use anyhow::Result;
use joule_profiler_cli::{
    cli::{Cli, Command},
    config::Config,
    exporter, init_logging, sources,
};
use joule_profiler_core::{JouleProfiler, util::cgroup::CgroupConfig};
use joule_profiler_exporter_terminal::{print_info, print_schema};

fn main() -> Result<()> {
    let cli = Cli::parsed()?;
    init_logging(cli.verbose);

    let config = Config::load_from_cli(&cli)?;
    let profiling = matches!(cli.command, Command::Profile(_));

    let cgroup = config
        .cgroup
        .clone()
        .filter(|_| profiling)
        .map(CgroupConfig::create)
        .transpose()?;
    let cgroup_path = config.cgroup.as_ref().map(CgroupConfig::path);

    let mut joule_profiler = JouleProfiler::new();
    sources::add(&mut joule_profiler, config.sources, cgroup_path.as_deref())?;
    if let Some(cgroup) = cgroup {
        joule_profiler.set_cgroup(cgroup);
    }

    match cli.command {
        Command::ListSensors => {
            print_schema("Available sensors", &joule_profiler.schema())?;
        }
        Command::Info => {
            print_info("Information", &joule_profiler.info())?;
        }
        Command::Profile(profile_args) => {
            let injector = config.injector.build(profile_args.cmd)?;

            exporter::set_exporter(&mut joule_profiler, config.exporter)?;
            joule_profiler.set_injector(injector);
            joule_profiler.set_defer(config.defer);

            print_info("Information", &joule_profiler.info())?;
            joule_profiler.profile()?;
        }
    }

    Ok(())
}
