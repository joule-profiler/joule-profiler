use std::{collections::HashSet, path::PathBuf};

use clap::{ArgAction, Parser, ValueEnum};

use anyhow::{Result, bail};
pub use commands::ProfilerCommand;
use serde::Deserialize;

use crate::{
    config::table::ConfigTable,
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
    about = "Measure program metrics from various sources like RAPL"
)]
pub struct CliArgs {
    /// Verbosity (-v, -vv, -vvv)
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbose: u8,

    /// Override the base path used to read Intel RAPL counters.
    ///
    /// By default, the profiler reads from:
    ///   /sys/devices/virtual/powercap/intel-rapl
    ///
    /// If not provided, the profiler uses (by priority):
    ///   1. $`JOULE_PROFILER_RAPL_PATH` (if set)
    ///   2. /sys/devices/virtual/powercap/intel-rapl
    #[arg(long = "rapl-path")]
    pub rapl_path: Option<String>,

    /// Sockets to measure. (e.g., 0 or 0,1)
    #[arg(short = 's', long = "sockets")]
    pub sockets: Option<String>,

    /// Output format to export the results in. (e.g., terminal, json, csv)
    #[arg(long = "output-format")]
    pub output_format: Option<OutputFormat>,

    /// Output file for CSV/JSON. (else `data<TIMESTAMP>`.csv/json)
    #[arg(short = 'o', long = "output-file")]
    pub output_file: Option<String>,

    /// Choose RAPL backend between powercap or perf.
    #[arg(long = "rapl-backend", value_enum)]
    pub rapl_backend: Option<RaplBackend>,

    /// Sources activation list. All sources must be separated with a comma (e.g., "perf,nvml").
    #[arg(long, value_delimiter = ',', default_value = "rapl")]
    pub sources: Vec<Source>,

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

#[derive(Clone, Debug, PartialEq, Eq, Hash, ValueEnum)]
pub enum Source {
    Rapl,
    Nvml,
    #[value(name = "amdsmi")]
    AmdSmi,
    #[value(alias = "perf_event")]
    Perf,
    Procfs,
    Cgroup,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Source::Rapl => "rapl",
            Source::Nvml => "nvml",
            Source::AmdSmi => "amdsmi",
            Source::Perf => "perf",
            Source::Procfs => "procfs",
            Source::Cgroup => "cgroup",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Default, Clone, ValueEnum, Deserialize)]
pub enum RaplBackend {
    #[default]
    #[serde(rename = "perf")]
    Perf,
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

pub fn parse_sockets_spec(sockets_spec: &str) -> HashSet<u32> {
    sockets_spec
        .split(',')
        .filter_map(|x| x.trim().parse::<u32>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use clap::ValueEnum;
    use joule_profiler_core::source::MetricReader;

    use super::*;

    fn cli_args_with_sources(sources: Vec<Source>) -> CliArgs {
        CliArgs {
            verbose: 0,
            rapl_path: None,
            sockets: None,
            output_format: None,
            output_file: None,
            rapl_backend: None,
            sources,
            config_file: None,
            command: ProfilerCommand::ListSensors,
        }
    }

    #[test]
    fn source_display_matches_every_metric_reader_get_id() {
        type DefaultPerfEvent = joule_profiler_source_perf_event::PerfEvent;
        type DefaultCgroup = joule_profiler_source_cgroup::Cgroup;
        type DefaultProcfs = joule_profiler_source_procfs::Procfs;
        type DefaultNvml = joule_profiler_source_nvml::Nvml;
        type DefaultAmdSmi = joule_profiler_source_amdsmi::AmdSmi;

        assert_eq!(
            Source::Rapl.to_string(),
            joule_profiler_source_rapl::perf::Rapl::get_id()
        );
        assert_eq!(
            Source::Rapl.to_string(),
            joule_profiler_source_rapl::powercap::Rapl::get_id()
        );
        assert_eq!(Source::Perf.to_string(), DefaultPerfEvent::get_id());
        assert_eq!(Source::Cgroup.to_string(), DefaultCgroup::get_id());
        assert_eq!(Source::Procfs.to_string(), DefaultProcfs::get_id());
        assert_eq!(Source::Nvml.to_string(), DefaultNvml::get_id());
        assert_eq!(Source::AmdSmi.to_string(), DefaultAmdSmi::get_id());
    }

    #[test]
    fn source_value_enum_accepts_perf_event_alias() {
        let parsed = Source::from_str("perf_event", false).unwrap();
        assert_eq!(parsed, Source::Perf);
    }

    #[test]
    fn source_value_enum_accepts_canonical_names() {
        assert_eq!(Source::from_str("rapl", false).unwrap(), Source::Rapl);
        assert_eq!(Source::from_str("nvml", false).unwrap(), Source::Nvml);
        assert_eq!(Source::from_str("amdsmi", false).unwrap(), Source::AmdSmi);
        assert_eq!(Source::from_str("perf", false).unwrap(), Source::Perf);
        assert_eq!(Source::from_str("procfs", false).unwrap(), Source::Procfs);
        assert_eq!(Source::from_str("cgroup", false).unwrap(), Source::Cgroup);
    }

    #[test]
    fn parse_sockets_spec_parses_comma_separated_values() {
        let sockets = parse_sockets_spec("0,1,2");
        assert_eq!(sockets, HashSet::from([0, 1, 2]));
    }

    #[test]
    fn parse_sockets_spec_trims_whitespace() {
        let sockets = parse_sockets_spec(" 0 , 1 ");
        assert_eq!(sockets, HashSet::from([0, 1]));
    }

    #[test]
    fn parse_sockets_spec_ignores_invalid_entries() {
        let sockets = parse_sockets_spec("0,not_a_number,2");
        assert_eq!(sockets, HashSet::from([0, 2]));
    }

    #[test]
    fn parse_sockets_spec_empty_string_returns_empty_set() {
        assert!(parse_sockets_spec("").is_empty());
    }

    #[test]
    fn validate_rejects_duplicate_sources() {
        let cli = cli_args_with_sources(vec![Source::Rapl, Source::Rapl]);
        assert!(cli.validate().is_err());
    }

    #[test]
    fn validate_accepts_distinct_sources() {
        let cli = cli_args_with_sources(vec![Source::Rapl, Source::Nvml]);
        assert!(cli.validate().is_ok());
    }
}
