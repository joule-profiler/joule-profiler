use anyhow::Result;

use joule_profiler_cli::{CliArgs, Source, init_logging};
use joule_profiler_core::{
    JouleProfiler,
    config::{Command, Config},
};
use joule_profiler_exporter_terminal::TerminalExporter;
use joule_profiler_injector_stdout::StdOutInjector;
use joule_profiler_source_nvml::NvmlSource;
use joule_profiler_source_perf_event::PerfEventSource;
use joule_profiler_source_rapl::RaplSource;
use log::trace;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = CliArgs::from_args();
    cli.validate()?;

    init_logging(cli.verbose);

    // let rapl_path = cli.rapl_path.as_deref();
    // let rapl_sockets_spec = parse_sockets_spec(cli.sockets.as_deref());
    // let rapl_polling = match &cli.command {
    //     ProfilerCommand::Profile(profile_args) => profile_args.rapl_polling,
    //     ProfilerCommand::ListSensors => None,
    // };

    let mut profiler = JouleProfiler::new();
    profiler.with_exporter(TerminalExporter);

    if cli.sources.contains(&Source::Rapl) {
        // match cli.rapl_backend {
        // RaplBackend::Perf => {
        //     if let Err(err) = perf::Rapl::check_perf_access() {
        //         warn!("Cannot initialize RAPL with perf_event, switching to powercap: {err}");
        //         let rapl_powercap =
        //             powercap::Rapl::new(rapl_path, rapl_sockets_spec.as_ref(), rapl_polling)?;
        //         profiler.add_source(rapl_powercap);
        //     } else {
        //         trace!("Using perf_event for RAPL profiling");
        //         let perf_rapl = perf::Rapl::new(rapl_sockets_spec.as_ref())?;
        //         profiler.add_source(perf_rapl);
        //     }
        // }
        // RaplBackend::Powercap => {
        //     trace!("Using Powercap for RAPL profiling");
        //     let rapl_powercap =
        //         powercap::Rapl::new(rapl_path, rapl_sockets_spec.as_ref(), rapl_polling)?;
        //     profiler.add_source(rapl_powercap);
        // }
        // }
        profiler.add_source::<RaplSource>(())?;
    }

    if cli.sources.contains(&Source::Nvml) {
        // match Nvml::new() {
        //     Ok(nvml) => {
        //         trace!("Using NVML for Nvidia GPU profiling");
        //         profiler.add_source(nvml);
        //     }
        //     Err(err) => warn!("{err}"),
        // }
        profiler.add_source::<NvmlSource>(())?;
    }

    if cli.sources.contains(&Source::Perf) {
        trace!("Initializing perf_event source");
        // let perf_event = PerfEvent::new()?;
        // profiler.add_source(perf_event);
        profiler.add_source::<PerfEventSource>(())?;
    }

    let config = Config::try_from(cli)?;

    match config.command {
        Command::Profile(profile_config) => {
            let injector =
                StdOutInjector::new(profile_config.cmd.clone(), &profile_config.token_pattern)?;
            let results = profiler.profile(&profile_config, injector).await?;
            println!("{results:?}");
        }
        Command::ListSensors => {
            let sensors = profiler.list_sensors()?;
            TerminalExporter::print_sensors(&mut std::io::stdout().lock(), &sensors)?;
        }
    }

    Ok(())
}
