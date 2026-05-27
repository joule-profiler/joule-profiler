use std::collections::HashSet;

use clap::{ArgAction, Parser, ValueEnum};

use anyhow::{Result, bail};
pub use commands::ProfilerCommand;
use joule_profiler_core::config::{Command, Config, ProfileConfig};

use crate::output::{
    displayer::Displayer,
    formats::{
        OutputFormat, csv::CsvOutput, json::JsonOutput, output_format, terminal::TerminalOutput,
    },
};

mod commands;
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

    /// Export results as JSON instead of pretty terminal output
    #[arg(long, conflicts_with = "csv")]
    pub json: bool,

    /// Export results as CSV (semicolon-separated values)
    #[arg(long, conflicts_with = "json")]
    pub csv: bool,

    /// Output file for CSV/JSON (else `data<TIMESTAMP>`.csv/json)
    #[arg(short = 'o', long = "output-file")]
    pub output_file: Option<String>,

    /// Choose RAPL backend between powercap or perf
    #[arg(long = "rapl-backend", value_enum, default_value_t = RaplBackend::Perf)]
    pub rapl_backend: RaplBackend,

    /// Sources activation list. All sources must be separated with a comma (e.g., "perf,nvml").
    #[arg(long, value_delimiter = ',', default_value = "rapl")]
    pub sources: Vec<Source>,

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

impl From<CliArgs> for Config {
    fn from(cli_args: CliArgs) -> Self {
        let command = match cli_args.command {
            ProfilerCommand::Profile(profile_args) => Command::Profile(ProfileConfig {
                stdout_file: profile_args.stdout_file,
                cmd: profile_args.cmd,
                token_pattern: profile_args.token_pattern,
                use_root: profile_args.use_root,
            }),

            ProfilerCommand::ListSensors => Command::ListSensors,
        };

        Config {
            command,
            rapl_path: cli_args.rapl_path,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, ValueEnum)]
pub enum Source {
    Rapl,
    Nvml,
    #[value(alias = "perf_event")]
    Perf,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Source::Rapl => "rapl",
            Source::Nvml => "nvml",
            Source::Perf => "perf | perf_event",
        };
        write!(f, "{s}")
    }
}

#[derive(Clone, Debug, ValueEnum)]
pub enum RaplBackend {
    Perf,
    Powercap,
}

pub fn output_format_to_displayer(cli: &CliArgs) -> Result<Box<dyn Displayer>> {
    let output_format = output_format(cli.json, cli.csv);
    let output_file = cli.output_file.clone();

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

pub fn parse_sockets_spec(sockets_spec: Option<&str>) -> Option<HashSet<u32>> {
    sockets_spec.map(|s| {
        s.split(',')
            .filter_map(|x| x.trim().parse::<u32>().ok())
            .collect()
    })
}
