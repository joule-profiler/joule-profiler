use std::{collections::HashSet, path::PathBuf};

use clap::{ArgAction, Parser, ValueEnum};

use anyhow::{Result, bail};
pub use commands::ProfilerCommand;
#[cfg(feature = "rapl")]
use serde::Deserialize;

use crate::{
    config::{overrides::ConfigOverride, table::ConfigTable},
    output::{
        displayer::Displayer,
        formats::{OutputFormat, csv::CsvOutput, json::JsonOutput, terminal::TerminalOutput},
    },
};

mod commands;
pub mod config;
mod logging;
mod output;

/// joule-profiler: measure program energy consumption
#[allow(clippy::struct_excessive_bools)]
#[derive(Parser, Debug)]
#[command(name = "joule-profiler")]
#[command(
    version,
    about = "Measure program metrics from various sources like RAPL, perf_event or NVML"
)]
pub struct CliArgs {
    /// Verbosity (-v, -vv, -vvv)
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbose: u8,

    /// Output format to export the results in. (e.g., terminal, json, csv)
    #[arg(long = "output-format")]
    pub output_format: Option<OutputFormat>,

    /// Output file for CSV/JSON. (else `data<TIMESTAMP>`.csv/json)
    #[arg(short = 'o', long = "output-file")]
    pub output_file: Option<String>,

    /// Sources activation list. All sources must be separated with a comma (e.g., "rapl,nvml").
    #[cfg_attr(
        feature = "rapl",
        arg(long, value_delimiter = ',', default_value = "rapl")
    )]
    #[cfg_attr(not(feature = "rapl"), arg(long, value_delimiter = ','))]
    pub sources: Vec<Source>,

    #[allow(clippy::doc_markdown)]
    /// Override a configuration key, repeatable. (e.g., -D profiler.rapl_backend=powercap)
    ///
    /// KEY is the dotted path of the key in the configuration file, so any
    /// key a configuration file can set is settable here:
    ///   -D profiler.output_format=json
    ///   -D sources.rapl.sockets_spec=[0,1]
    ///   -D sources.cgroup.create_cgroup=false
    ///
    /// VALUE is read as TOML, falling back to a plain string, so durations,
    /// regexes and paths need no quoting. Overrides are applied on top of the
    /// --config file, and configuring a source enables it.
    #[arg(
        short = 'D',
        long = "define",
        value_name = "KEY=VALUE",
        verbatim_doc_comment
    )]
    pub overrides: Vec<ConfigOverride>,

    /// TOML configuration file. Every key it sets can also be set with -D.
    #[arg(long = "config")]
    pub config_file: Option<PathBuf>,

    /// The command to execute.
    #[command(subcommand)]
    pub command: ProfilerCommand,
}

impl CliArgs {
    pub fn from_args() -> Self {
        Self::parse()
    }

    pub fn validate(&self) -> Result<()> {
        let mut seen = HashSet::new();

        for source in &self.sources {
            if !seen.insert(source) {
                bail!("Duplicate source specified: {source}");
            }
        }

        Ok(())
    }
}

#[cfg(not(any(
    feature = "rapl",
    feature = "perf_event",
    feature = "nvml",
    feature = "amdsmi",
    feature = "procfs",
    feature = "cgroup",
)))]
compile_error!("At least one source feature must be enabled");

#[derive(Clone, Debug, PartialEq, Eq, Hash, ValueEnum)]
pub enum Source {
    #[cfg(feature = "rapl")]
    Rapl,

    #[cfg(feature = "perf_event")]
    #[value(alias = "perf_event")]
    Perf,

    #[cfg(feature = "nvml")]
    Nvml,

    #[cfg(feature = "amdsmi")]
    #[value(name = "amdsmi")]
    AmdSmi,

    #[cfg(feature = "procfs")]
    Procfs,

    #[cfg(feature = "cgroup")]
    Cgroup,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            #[cfg(feature = "rapl")]
            Source::Rapl => "rapl",

            #[cfg(feature = "perf_event")]
            Source::Perf => "perf",

            #[cfg(feature = "nvml")]
            Source::Nvml => "nvml",

            #[cfg(feature = "amdsmi")]
            Source::AmdSmi => "amdsmi",

            #[cfg(feature = "procfs")]
            Source::Procfs => "procfs",

            #[cfg(feature = "cgroup")]
            Source::Cgroup => "cgroup",
        };
        write!(f, "{s}")
    }
}

/// RAPL backend, selected with `profiler.rapl_backend` in the configuration.
///
/// The RAPL source always comes with a backend, so this enum always has at
/// least one variant.
#[cfg(feature = "_rapl")]
#[derive(Debug, Default, Clone, Deserialize)]
pub enum RaplBackend {
    #[cfg(feature = "rapl-perf")]
    #[default]
    #[serde(rename = "perf")]
    Perf,

    #[cfg(feature = "rapl-powercap")]
    #[cfg_attr(not(feature = "rapl-perf"), default)]
    #[serde(rename = "powercap")]
    Powercap,
}

/// Builds the displayer for the configured output format.
///
/// Reads only from `config_table`: `apply_cli` must have already resolved
/// any CLI override into `profiler_config` beforehand.
pub fn config_table_to_displayer(config_table: &ConfigTable) -> Result<Box<dyn Displayer>> {
    let output_file = config_table.profiler_config.output_file.clone();

    let displayer = match config_table.profiler_config.output_format {
        OutputFormat::Terminal => TerminalOutput.into(),
        OutputFormat::Json => JsonOutput::new(output_file)?.into(),
        OutputFormat::Csv => CsvOutput::try_new(output_file)?.into(),
    };

    Ok(displayer)
}

pub fn init_logging(verbose: u8) {
    logging::init_logging(verbose);
}

#[cfg(test)]
mod tests {
    use clap::ValueEnum;
    use joule_profiler_core::source::MetricReader;

    use super::*;

    fn cli_args_with_sources(sources: Vec<Source>) -> CliArgs {
        CliArgs {
            verbose: 0,
            output_format: None,
            output_file: None,
            sources,
            overrides: Vec::new(),
            config_file: None,
            command: ProfilerCommand::ListSensors,
        }
    }

    #[test]
    fn source_display_matches_every_metric_reader_get_id() {
        #[cfg(feature = "rapl-perf")]
        assert_eq!(
            Source::Rapl.to_string(),
            joule_profiler_source_rapl::perf::Rapl::get_id()
        );
        #[cfg(feature = "rapl-powercap")]
        assert_eq!(
            Source::Rapl.to_string(),
            joule_profiler_source_rapl::powercap::Rapl::get_id()
        );
        #[cfg(feature = "perf_event")]
        {
            type DefaultPerfEvent = joule_profiler_source_perf_event::PerfEvent;
            assert_eq!(Source::Perf.to_string(), DefaultPerfEvent::get_id());
        }
        #[cfg(feature = "cgroup")]
        {
            type DefaultCgroup = joule_profiler_source_cgroup::Cgroup;
            assert_eq!(Source::Cgroup.to_string(), DefaultCgroup::get_id());
        }
        #[cfg(feature = "procfs")]
        {
            type DefaultProcfs = joule_profiler_source_procfs::Procfs;
            assert_eq!(Source::Procfs.to_string(), DefaultProcfs::get_id());
        }
        #[cfg(feature = "nvml")]
        {
            type DefaultNvml = joule_profiler_source_nvml::Nvml;
            assert_eq!(Source::Nvml.to_string(), DefaultNvml::get_id());
        }
        #[cfg(feature = "amdsmi")]
        {
            type DefaultAmdSmi = joule_profiler_source_amdsmi::AmdSmi;
            assert_eq!(Source::AmdSmi.to_string(), DefaultAmdSmi::get_id());
        }
    }

    #[cfg(feature = "perf_event")]
    #[test]
    fn source_value_enum_accepts_perf_event_alias() {
        let parsed = Source::from_str("perf_event", false).unwrap();
        assert_eq!(parsed, Source::Perf);
    }

    /// Every variant this build has must parse back from the name it displays.
    #[test]
    fn source_value_enum_accepts_canonical_names() {
        for source in Source::value_variants() {
            let parsed = Source::from_str(&source.to_string(), false).unwrap();
            assert_eq!(&parsed, source);
        }
    }

    #[test]
    fn validate_rejects_duplicate_sources() {
        let source = Source::value_variants()[0].clone();
        let cli = cli_args_with_sources(vec![source.clone(), source]);
        assert!(cli.validate().is_err());
    }

    #[test]
    fn validate_accepts_distinct_sources() {
        let cli = cli_args_with_sources(Source::value_variants().to_vec());
        assert!(cli.validate().is_ok());
    }
}
