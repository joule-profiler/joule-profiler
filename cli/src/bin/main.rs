use anyhow::Result;
use joule_profiler_cli::config::load_global_config;
use joule_profiler_cli::config::source::register_source;
use joule_profiler_cli::config::table::ConfigTable;
use joule_profiler_cli::{CliArgs, RaplBackend, config_table_to_displayer, init_logging};
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

    let global_config = load_global_config(cli.config_file.as_deref(), &cli.overrides)?;
    let mut config_table = ConfigTable::new(global_config, &cli.sources);

    config_table.apply_cli(&mut cli);

    match config_table.profiler_config.rapl_backend {
        RaplBackend::Perf => register_source::<perf::Rapl>(&mut profiler, &mut config_table),
        RaplBackend::Powercap => {
            register_source::<powercap::Rapl>(&mut profiler, &mut config_table)
        }
    }?;

    register_source::<PerfEvent>(&mut profiler, &mut config_table)?;
    register_source::<Cgroup>(&mut profiler, &mut config_table)?;
    register_source::<Procfs>(&mut profiler, &mut config_table)?;
    register_source::<Nvml>(&mut profiler, &mut config_table)?;
    register_source::<AmdSmi>(&mut profiler, &mut config_table)?;

    config_table.ensure_sources_are_known()?;

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
