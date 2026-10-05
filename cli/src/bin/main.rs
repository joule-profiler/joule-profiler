use anyhow::Result;
use joule_profiler_cli::cli::{Cli, Command};
use joule_profiler_cli::config::Settings;
use joule_profiler_cli::{exporter, init_logging, sources};
use joule_profiler_core::profiler::JouleProfiler;
use joule_profiler_core::util::cgroup::CgroupConfig;
use joule_profiler_exporter_terminal::{print_info, print_schema};
use joule_profiler_injector_stdout::StdoutInjector;

fn main() -> Result<()> {
    let cli = Cli::parsed()?;
    init_logging(cli.verbose);

    let settings = Settings::resolve(&cli)?;
    let profiling = matches!(cli.command, Command::Profile(_));

    // Only a run creates its cgroup, which needs root. It is created before the sources so that
    // the cgroup source can read it.
    let cgroup = settings
        .cgroup
        .clone()
        .filter(|_| profiling)
        .map(CgroupConfig::create)
        .transpose()?;
    let cgroup_path = settings.cgroup.as_ref().map(CgroupConfig::path);

    let mut joule_profiler = JouleProfiler::new();
    sources::add(&mut joule_profiler, &settings, cgroup_path.as_deref())?;
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
            let injector = StdoutInjector::new(profile_args.cmd, &settings.token_pattern)?
                .use_root(settings.use_root)
                .output_file(settings.stdout_file.clone());

            exporter::set(&mut joule_profiler, &settings)?;
            joule_profiler.set_injector(injector);
            joule_profiler.set_defer(settings.defer);

            print_info("Information", &joule_profiler.info())?;
            joule_profiler.profile()?;
        }
    }

    Ok(())
}
