use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use serde::Deserialize;

#[cfg(not(any(
    feature = "rapl",
    feature = "perf_event",
    feature = "procfs",
    feature = "cgroup",
    feature = "nvml",
    feature = "amdsmi",
)))]
compile_error!("at least one source feature must be enabled");

#[derive(Parser, Debug)]
#[command(
    name = "joule-profiler",
    version,
    about = "Measure program metrics from various sources like RAPL, perf_event or NVML"
)]
pub struct Cli {
    /// Sources to activate, comma separated. (e.g. "rapl,perf,procfs")
    #[cfg_attr(
        feature = "rapl",
        arg(long, value_delimiter = ',', default_value = "rapl")
    )]
    #[cfg_attr(not(feature = "rapl"), arg(long, value_delimiter = ','))]
    pub sources: Vec<SourceName>,

    /// Configuration file. A `[sources.<name>]` table turns that source on.
    #[arg(long = "config", value_name = "FILE")]
    pub config_file: Option<PathBuf>,

    #[allow(clippy::doc_markdown, reason = "this is the help clap prints")]
    /// Overrides one value of the configuration, as `section.key=value`. Repeatable.
    /// (e.g. -D sources.perf_event.scope=cgroup)
    #[arg(short = 'D', long = "define", value_name = "KEY=VALUE")]
    pub set: Vec<String>,

    /// What the results are written as. (default: terminal)
    #[arg(long = "output-format", value_enum)]
    pub output_format: Option<OutputFormat>,

    /// Writes the results to this file instead of the terminal.
    #[arg(short = 'o', long = "output-file", value_name = "FILE")]
    pub output_file: Option<PathBuf>,

    /// Verbosity (-v, -vv, -vvv).
    #[arg(short = 'v', long = "verbose", action = ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Parses the arguments. A source named twice is an error.
    pub fn parsed() -> Result<Self> {
        let cli = Self::parse();

        let mut seen = HashSet::new();
        for source in &cli.sources {
            if !seen.insert(source) {
                bail!("source named twice: {source}");
            }
        }

        Ok(cli)
    }
}

/// The sources this build can measure with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, ValueEnum)]
pub enum SourceName {
    /// Energy per socket and domain, through the `power` PMU or powercap.
    #[cfg(feature = "rapl")]
    Rapl,

    /// Hardware counters, on the profiled process or on a cgroup.
    #[cfg(feature = "perf_event")]
    #[value(alias = "perf_event")]
    Perf,

    /// Memory, CPU time and I/O of the process tree, from `/proc`.
    #[cfg(feature = "procfs")]
    Procfs,

    /// Memory, CPU time and I/O of the cgroup the profiled process runs in.
    #[cfg(feature = "cgroup")]
    Cgroup,

    /// Energy, VRAM and utilization of the NVIDIA GPUs.
    #[cfg(feature = "nvml")]
    Nvml,

    /// Energy, VRAM and utilization of the AMD GPUs.
    #[cfg(feature = "amdsmi")]
    #[value(name = "amdsmi")]
    AmdSmi,
}

impl SourceName {
    /// The name of its `[sources.<key>]` table.
    pub fn key(self) -> &'static str {
        match self {
            #[cfg(feature = "perf_event")]
            SourceName::Perf => "perf_event",
            #[allow(
                unreachable_patterns,
                reason = "perf_event may be the only source built"
            )]
            other => other.as_str(),
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::value_variants()
            .iter()
            .copied()
            .find(|name| name.key() == key || name.as_str() == key)
    }

    fn as_str(self) -> &'static str {
        match self {
            #[cfg(feature = "rapl")]
            SourceName::Rapl => "rapl",
            #[cfg(feature = "perf_event")]
            SourceName::Perf => "perf",
            #[cfg(feature = "procfs")]
            SourceName::Procfs => "procfs",
            #[cfg(feature = "cgroup")]
            SourceName::Cgroup => "cgroup",
            #[cfg(feature = "nvml")]
            SourceName::Nvml => "nvml",
            #[cfg(feature = "amdsmi")]
            SourceName::AmdSmi => "amdsmi",
        }
    }
}

impl std::fmt::Display for SourceName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Runs a command and measures it.
    Profile(ProfileArgs),

    /// Lists the metrics each source reports.
    ListSensors,

    /// Prints the machine and the setup of each source.
    Info,
}

#[derive(Parser, Debug)]
pub struct ProfileArgs {
    /// The regular expression used to detect tokens on the standard output.
    /// (default: `__[A-Z0-9_]+__`)
    #[arg(long = "token-pattern", value_name = "REGEX")]
    pub token_pattern: Option<String>,

    #[arg(long = "defer")]
    pub defer: bool,

    /// The command to run, everything after `--`.
    #[arg(last = true, required = true)]
    pub cmd: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Human-readable, as the run goes.
    Terminal,

    /// JSON, one document for the run or one object per line. (see `[exporter] lines`)
    Json,

    /// One row per metric.
    Csv,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OutputFormat::Terminal => "terminal",
            OutputFormat::Json => "json",
            OutputFormat::Csv => "csv",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_is_found_from_its_name_and_its_table() {
        for source in SourceName::value_variants() {
            let parsed = SourceName::from_str(&source.to_string(), false).unwrap();

            assert_eq!(&parsed, source);
            assert_eq!(SourceName::from_key(source.key()), Some(*source));
        }
    }

    #[cfg(feature = "perf_event")]
    #[test]
    fn perf_is_also_called_perf_event() {
        assert_eq!(
            SourceName::from_str("perf_event", false).unwrap(),
            SourceName::Perf
        );
    }
}
