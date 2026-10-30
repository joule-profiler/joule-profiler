use anyhow::Result;
use joule_profiler_config::{
    ConfigTable, Map, ProfilerConfig, SourceName, Value, configured_sources,
};
use joule_profiler_core::util::cgroup::CgroupConfig;
use joule_profiler_injector_stdout::StdoutInjectorConfig;
use serde::Deserialize;

use crate::{
    cli::{Cli, Command},
    exporter::ExporterConfig,
};

/// The configuration file of the command line.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConfigFile {
    pub profiler: ProfilerConfig,

    pub exporter: ExporterConfig,

    pub injector: StdoutInjectorConfig,

    pub cgroup: Option<CgroupConfig>,

    pub sources: Map<String, Value>,
}

/// The merged configuration file and flags.
pub struct Config {
    pub sources: Vec<(SourceName, Option<Value>)>,

    pub exporter: ExporterConfig,

    pub injector: StdoutInjectorConfig,

    pub defer: bool,

    pub cgroup: Option<CgroupConfig>,
}

impl Config {
    /// Loads the configuration file, then applies the flags over it.
    pub fn load_from_cli(cli: &Cli) -> Result<Self> {
        let mut table = match &cli.config_file {
            Some(path) => ConfigTable::read(path)?,
            None => ConfigTable::default(),
        };
        for definition in &cli.definitions {
            table.define(definition)?;
        }
        let mut config_file: ConfigFile = table.deserialize()?;

        let mut defer = config_file.profiler.defer;
        if let Command::Profile(args) = &cli.command {
            defer |= args.defer;
            if let Some(pattern) = &args.token_pattern {
                config_file.injector.token_pattern.clone_from(pattern);
            }
            if let Some(output_format) = args.output_format {
                config_file.exporter.set_format(output_format);
            }
            if let Some(output_file) = &args.output_file {
                config_file.exporter.set_file(output_file.clone())?;
            }
            if let Some(stdout_file) = &args.stdout_file {
                config_file.injector.stdout_file = Some(stdout_file.clone());
            }
            config_file.injector.use_root |= args.use_root;
            config_file.injector.no_stdout |= args.no_stdout;
        }

        let sources = configured_sources(&cli.sources, &config_file.sources)?;
        let cgroup = config_file
            .cgroup
            .or_else(|| needs_run_cgroup(&sources).then(CgroupConfig::default));

        Ok(Self {
            sources,
            exporter: config_file.exporter,
            injector: config_file.injector,
            defer,
            cgroup,
        })
    }
}

/// Returns whether the cgroup source is asked for without naming a cgroup.
#[cfg_attr(not(feature = "cgroup"), allow(unused_variables))]
fn needs_run_cgroup(sources: &[(SourceName, Option<Value>)]) -> bool {
    #[cfg(feature = "cgroup")]
    {
        sources.iter().any(|(name, table)| {
            *name == SourceName::Cgroup
                && table.as_ref().and_then(|table| table.get("name")).is_none()
        })
    }

    #[cfg(not(feature = "cgroup"))]
    false
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[cfg(feature = "cgroup")]
    #[test]
    fn run_cgroup_for_unnamed_cgroup_source() {
        let unnamed = Some(json!({ "global": false }));
        let named = Some(json!({ "name": "my.slice" }));

        assert!(needs_run_cgroup(&[(SourceName::Cgroup, unnamed)]));
        assert!(!needs_run_cgroup(&[(SourceName::Cgroup, named)]));
        assert!(!needs_run_cgroup(&[]));
    }
}
