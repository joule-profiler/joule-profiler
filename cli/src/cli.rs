use std::{collections::HashSet, path::PathBuf};

use anyhow::{Result, bail};
use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use joule_profiler_config::SourceName;
use serde::Deserialize;

#[derive(Parser, Debug)]
#[command(
    name = "joule-profiler",
    version,
    about = "Measure program metrics from various sources like RAPL, perf_event or NVML"
)]
pub struct Cli {
    /// Sources to activate, comma separated. (e.g., "rapl,perf,procfs") Without any, and without
    /// a `[sources.<name>]` table, RAPL is measured.
    #[arg(long, value_delimiter = ',')]
    pub sources: Vec<SourceName>,

    /// Configuration file.
    #[arg(long = "config", visible_alias = "config-file", value_name = "FILE")]
    pub config_file: Option<PathBuf>,

    /// Overrides one value of the configuration, as `section.key=value`. Repeatable.
    /// (e.g., `-D sources.perf_event.scope=cgroup`)
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

    /// Processes and exports the phases only once the command has ended.
    #[arg(long = "defer")]
    pub defer: bool,

    /// Discards what the command prints.
    #[arg(long = "no-stdout")]
    pub no_stdout: bool,

    /// The command to run, everything after `--`.
    #[arg(last = true, required = true)]
    pub cmd: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Human-readable, as the run goes.
    Terminal,

    /// JSON, one table for the run or one object per line. (see `lines` in `[exporter]`)
    Json,

    /// One row per metric.
    Csv,
}
