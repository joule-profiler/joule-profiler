use anyhow::Result;
use joule_profiler_cli::config::GlobalConfig;
use joule_profiler_cli::config::source::register_source;
use joule_profiler_cli::config::table::ConfigTable;
use joule_profiler_cli::{CliArgs, config_to_displayer, init_logging, register_sources};
use joule_profiler_core::JouleProfiler;
use joule_profiler_core::config::{Command, Config};
use joule_profiler_source_amdsmi::AmdSmi;
use joule_profiler_source_amdsmi::config::AmdSmiConfig;
use joule_profiler_source_cgroup::{CgroupConfig, CgroupSource};
use joule_profiler_source_nvml::Nvml;
use joule_profiler_source_nvml::config::NvmlConfig;
use joule_profiler_source_perf_event::PerfEvent;
use joule_profiler_source_procfs::Procfs;
use joule_profiler_source_procfs::config::ProcfsConfig;
use joule_profiler_source_rapl::{perf, powercap};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = CliArgs::from_args();
    cli.validate()?;

    init_logging(cli.verbose);

    let mut profiler = JouleProfiler::new();

    let mut config_table = if let Some(config_file) = &cli.config_file {
        let content = std::fs::read_to_string(config_file)?;
        let value: GlobalConfig = toml::from_str(&content)?;
        ConfigTable::new(value, cli)
    } else {
        ConfigTable::new(GlobalConfig::default(), cli)
    };

    register_sources!(
        &mut profiler,
        &mut config_table,
        [PerfEvent, Nvml, perf::Rapl, powercap::Rapl, CgroupSource, AmdSmi, Procfs]
    );

    let mut displayer = config_to_displayer(&config_table)?;
    let config: Config = config_table.try_into()?;

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
