use anyhow::Result;
use joule_profiler_cli::{
    CliArgs, ProfilerCommand, RaplBackend, init_logging, output_format_to_displayer,
    parse_sockets_spec,
};
use joule_profiler_core::JouleProfiler;
use joule_profiler_core::config::{Command, Config};
use joule_profiler_source_nvml::Nvml;
use joule_profiler_source_perf_event::PerfEvent;
use joule_profiler_source_rapl::{perf, powercap};
use log::{trace, warn};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = CliArgs::from_args();
    init_logging(cli.verbose);

    let mut displayer = output_format_to_displayer(&cli)?;
    let mut profiler = JouleProfiler::new();

    let rapl_path = cli.rapl_path.as_deref();
    let rapl_sockets_spec = parse_sockets_spec(cli.sockets.as_deref());
    let rapl_polling = match &cli.command {
        ProfilerCommand::Profile(profile_args) => profile_args.rapl_polling,
        ProfilerCommand::ListSensors => None,
    };

    match cli.rapl_backend {
        RaplBackend::Perf => {
            if let Err(err) = perf::Rapl::check_perf_access() {
                warn!("Cannot initialize RAPL with perf_event, switching to powercap: {err}");
                let rapl_powercap =
                    powercap::Rapl::new(rapl_path, rapl_sockets_spec.as_ref(), rapl_polling)?;
                profiler.add_source(rapl_powercap);
            } else {
                trace!("Using perf_event for RAPL profiling");
                let perf_rapl = perf::Rapl::new(rapl_sockets_spec.as_ref())?;
                profiler.add_source(perf_rapl);
            }
        }
        RaplBackend::Powercap => {
            trace!("Using Powercap for RAPL profiling");
            let rapl_powercap =
                powercap::Rapl::new(rapl_path, rapl_sockets_spec.as_ref(), rapl_polling)?;
            profiler.add_source(rapl_powercap);
        }
    }

    if cli.gpu {
        match Nvml::new() {
            Ok(nvml) => {
                trace!("Using NVML for Nvidia GPU profiling");
                profiler.add_source(nvml);
            }
            Err(err) => warn!("{err}"),
        }
    }

    if cli.perf {
        trace!("Initializing perf_event source");
        let perf_event = PerfEvent::new()?;
        profiler.add_source(perf_event);
    }

    let config = Config::from(cli);

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
