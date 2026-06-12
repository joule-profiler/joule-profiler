use anyhow::Result;
use joule_profiler_cli::config::GlobalConfig;
use joule_profiler_cli::config::source::{register_source, register_source_override};
use joule_profiler_cli::config::table::ConfigTable;
use joule_profiler_cli::{CliArgs, ProfilerCommand, RaplBackend, init_logging, parse_sockets_spec};
use joule_profiler_core::JouleProfiler;
use joule_profiler_core::config::{Command, Config};
use joule_profiler_source_amdsmi::AmdSmi;
use joule_profiler_source_amdsmi::config::AmdSmiConfig;
use joule_profiler_source_cgroup::{CgroupConfig, CgroupSource};
use joule_profiler_core::config::Command;
use joule_profiler_source_nvml::Nvml;
use joule_profiler_source_nvml::config::NvmlConfig;
use joule_profiler_source_perf_event::PerfEvent;
use joule_profiler_source_procfs::Procfs;
use joule_profiler_source_procfs::config::ProcfsConfig;
use joule_profiler_source_rapl::{perf, powercap};

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = CliArgs::from_args();
    cli.validate()?;

    init_logging(cli.verbose);

    let mut profiler = JouleProfiler::new();

    let mut config_table = if let Some(config_file) = &cli.config_file {
        let content = std::fs::read_to_string(config_file)?;
        let value: GlobalConfig = toml::from_str(&content)?;
        ConfigTable::new(value, &cli.sources)
    } else {
        ConfigTable::new(GlobalConfig::default(), &cli.sources)
    };

    match cli.rapl_backend {
        RaplBackend::Perf => register_source_override::<perf::Rapl, CliArgs>(
            &mut profiler,
            &mut config_table,
            &mut cli,
            |cli, config| {
                config.sockets_spec = parse_sockets_spec(cli.sockets.as_deref());
            },
        ),

        RaplBackend::Powercap => register_source_override::<powercap::Rapl, CliArgs>(
            &mut profiler,
            &mut config_table,
            &mut cli,
            |cli, config| {
                config.rapl_path = cli.rapl_path.take();
                config.sockets_spec = parse_sockets_spec(cli.sockets.as_deref());
                if let ProfilerCommand::Profile(profile_args) = &cli.command {
                    config.poll_interval = profile_args.rapl_polling
                }
            },
        ),
    }?;

    register_source::<PerfEvent>(&mut profiler, &mut config_table)?;
    register_source::<CgroupSource>(&mut profiler, &mut config_table)?;
    register_source::<Procfs>(&mut profiler, &mut config_table)?;
    register_source::<Nvml>(&mut profiler, &mut config_table)?;
    register_source::<AmdSmi>(&mut profiler, &mut config_table)?;

    let (config, mut displayer) = config_table.to_config(cli)?;

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
