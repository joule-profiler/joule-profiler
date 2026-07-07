use anyhow::{Result, anyhow};
use joule_profiler_cli::config::GlobalConfig;
use joule_profiler_cli::config::source::{register_source, register_source_override};
use joule_profiler_cli::config::table::ConfigTable;
use joule_profiler_cli::{
    CliArgs, ProfilerCommand, RaplBackend, config_table_to_displayer, init_logging,
    parse_sockets_spec,
};
use joule_profiler_core::JouleProfiler;
use joule_profiler_core::config::Command;
use joule_profiler_source_amdsmi::AmdSmi;
use joule_profiler_source_cgroup::Cgroup;
use joule_profiler_source_nvml::Nvml;
use joule_profiler_source_perf_event::PerfEvent;
use joule_profiler_source_procfs::Procfs;
use joule_profiler_source_rapl::{perf, powercap};

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = CliArgs::from_args();
    cli.validate()?;

    init_logging(cli.verbose);

    let mut profiler = JouleProfiler::new();

    let mut config_table = if let Some(config_file) = &cli.config_file {
        let content = std::fs::read_to_string(config_file)
            .map_err(|e| anyhow!("configuration file error. Cause: {e}"))?;
        let value: GlobalConfig = toml::from_str(&content)
            .map_err(|e| anyhow!("error parsing configuration file. Cause: {e}"))?;

        ConfigTable::new(value, &cli.sources)
    } else {
        ConfigTable::new(GlobalConfig::default(), &cli.sources)
    };

    config_table.apply_cli(&mut cli);

    match config_table.profiler_config.rapl_backend {
        RaplBackend::Perf => register_source_override::<perf::Rapl, CliArgs>(
            &mut profiler,
            &mut config_table,
            |config| {
                if let Some(sockets_spec) = &cli.sockets {
                    config.sockets_spec = Some(parse_sockets_spec(sockets_spec));
                }
            },
        ),

        RaplBackend::Powercap => register_source_override::<powercap::Rapl, CliArgs>(
            &mut profiler,
            &mut config_table,
            |config| {
                if let Some(rapl_path) = cli.rapl_path.take() {
                    config.rapl_path = Some(rapl_path);
                }
                if let Some(sockets_spec) = &cli.sockets {
                    config.sockets_spec = Some(parse_sockets_spec(sockets_spec));
                }
                if let ProfilerCommand::Profile(profile_args) = &cli.command
                    && let Some(rapl_polling) = profile_args.rapl_polling
                {
                    config.poll_interval = Some(rapl_polling);
                }
            },
        ),
    }?;

    register_source::<PerfEvent>(&mut profiler, &mut config_table)?;
    register_source::<Cgroup>(&mut profiler, &mut config_table)?;
    register_source::<Procfs>(&mut profiler, &mut config_table)?;
    register_source::<Nvml>(&mut profiler, &mut config_table)?;
    register_source::<AmdSmi>(&mut profiler, &mut config_table)?;

    let mut displayer = config_table_to_displayer(&config_table)?;
    let config = config_table.to_config(cli)?;

    match config.command {
        Command::Profile(profile_config) => {
            let results = profiler.profile(&profile_config).await?;
            displayer.display_results(
                &profile_config.cmd,
                &profile_config.token_pattern,
                &results,
            )?;
        }
        Command::ListSensors => {
            let sensors = profiler.list_sensors()?;
            displayer.list_sensors(&sensors)?;
        }
    }

    Ok(())
}
