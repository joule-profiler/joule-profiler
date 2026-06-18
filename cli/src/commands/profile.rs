use std::time::Duration;

use clap::Parser;
use serde::Deserialize;

/// Arguments for profiling mode.
#[derive(Parser, Debug, Deserialize, Default)]
pub struct ProfileArgs {
    /// Regex pattern to detect phase tokens in program output. (default: __[A-Z0-9_]+__)
    ///
    /// Matches tokens in stdout; if the pattern has a capture group, the
    /// captured text is used as the token name. Energy phases computed:
    ///   - global (START -> END)
    ///   - START -> `first_token`
    ///   - `token_i` -> `token_i+1`
    ///   - `last_token` -> END
    #[arg(long = "token-pattern", value_name = "REGEX")]
    pub token_pattern: Option<String>,

    /// Redirect profiled program stdout to this file.
    #[arg(short = 'o', long = "stdout-file")]
    pub stdout_file: Option<String>,

    /// Command to execute (everything after `--`).
    #[arg(last = true, required = true)]
    pub cmd: Vec<String>,

    /// Rapl polling frequency in second. (default: 1s)
    #[arg(long = "rapl-polling")]
    #[clap(value_parser = humantime::parse_duration)]
    pub rapl_polling: Option<Duration>,

    /// Executes the profiled command with root privileges if true and Joule Profiler is launched as root.
    #[arg(long = "use-root")]
    pub use_root: bool,

    /// Duration before aborting sources initialization. (default: 1s)
    #[clap(value_parser = humantime::parse_duration)]
    #[arg(long = "init-timeout")]
    pub init_timeout: Option<Duration>,
}
