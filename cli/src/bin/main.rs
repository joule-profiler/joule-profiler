use anyhow::Result;
use joule_profiler_cli::cli::{Cli, Command};
use joule_profiler_cli::config::ConfigTable;
use joule_profiler_cli::{exporter, init_logging, sources};
use joule_profiler_core::profiler::JouleProfiler;
use joule_profiler_core::util::cgroup::CgroupConfig;
use joule_profiler_exporter_terminal::{print_info, print_schema};
use joule_profiler_injector_stdout::StdoutInjector;

fn main() -> Result<()> {
    let cli = Cli::parsed()?;
    init_logging(cli.verbose);

    let config_table = ConfigTable::resolve(&cli)?;
    let profiling = matches!(cli.command, Command::Profile(_));

    let cgroup = config_table
        .cgroup
        .clone()
        .filter(|_| profiling)
        .map(CgroupConfig::create)
        .transpose()?;
    let cgroup_path = config_table.cgroup.as_ref().map(CgroupConfig::path);

    let mut joule_profiler = JouleProfiler::new();
    sources::add(&mut joule_profiler, &config_table, cgroup_path.as_deref())?;
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
            let injector = StdoutInjector::new(profile_args.cmd, &config_table.token_pattern)?
                .use_root(config_table.use_root)
                .output_file(config_table.stdout_file.clone());

            exporter::set(&mut joule_profiler, &config_table)?;
            joule_profiler.set_injector(injector);
            joule_profiler.set_defer(config_table.defer);

            print_info("Information", &joule_profiler.info())?;
            joule_profiler.profile()?;
        }
    }

    Ok(())
}
