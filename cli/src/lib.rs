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

    /// Sockets to measure (e.g., 0 or 0,1)
    #[arg(short = 's', long = "sockets")]
    pub sockets: Option<String>,

    /// Output format to export the results in. (e.g., terminal, json, csv)
    #[arg(long = "output-format")]
    pub output_format: Option<OutputFormat>,

    /// Output file for CSV/JSON (else `data<TIMESTAMP>`.csv/json)
    #[arg(short = 'o', long = "output-file")]
    pub output_file: Option<String>,

    /// Choose RAPL backend between powercap or perf
    #[arg(long = "rapl-backend", value_enum)]
    pub rapl_backend: Option<RaplBackend>,

    /// Sources activation list. All sources must be separated with a comma (e.g., "perf,nvml").
    #[arg(long, value_delimiter = ',', default_value = "rapl")]
    pub sources: Vec<Source>,

    #[arg(long = "config")]
    pub config_file: Option<PathBuf>,

    /// The command to execute
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
            Source::Perf => "perf | perf_event",
            Source::Procfs => "procfs",
            Source::Cgroup => "cgroup",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Default, Clone, ValueEnum, Deserialize)]
pub enum RaplBackend {
    #[default]
    #[serde(rename = "perf", alias = "perf_event")]
    Perf,
    #[serde(rename = "powercap")]
    Powercap,
}

pub fn config_table_to_displayer(
    config_table: &ConfigTable,
    cli: &CliArgs,
) -> Result<Box<dyn Displayer>> {
    let output_format = cli
        .output_format
        .unwrap_or(config_table.profiler_config.output_format);
    let output_file = cli
        .output_file
        .as_ref()
        .or(config_table.profiler_config.output_file.as_ref())
        .cloned();

    let displayer = match output_format {
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
