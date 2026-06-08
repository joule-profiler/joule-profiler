use std::time::Duration;

use clap::Parser;

/// Arguments for profiling mode.
#[derive(Parser, Debug)]
pub struct ProfileArgs {
    /// Regex pattern to detect phase tokens in program output.
    ///
    /// Matches tokens in stdout; if the pattern has a capture group, the
    /// captured text is used as the token name. Energy phases computed:
    ///   - global (START -> END)
    ///   - START -> `first_token`
    ///   - `token_i` -> `token_i+1`
    ///   - `last_token` -> END
    #[arg(
        long = "token-pattern",
        default_value = "__[A-Z0-9_]+__",
        value_name = "REGEX"
    )]
    pub token_pattern: String,

    /// Redirect profiled program stdout to this file.
    #[arg(short = 'o', long = "stdout-file")]
    pub stdout_file: Option<String>,

    /// Command to execute (everything after `--`).
    #[arg(last = true, required = true)]
    pub cmd: Vec<String>,

    /// Rapl polling frequency in second.
    #[arg(long = "rapl-polling")]
    pub rapl_polling: Option<f64>,

    /// Executes the profiled command with root privileges if true and Joule Profiler is launched as root.
    #[arg(long = "use-root")]
    pub use_root: bool,

    /// Duration before aborting sources initialization.
    #[clap(value_parser = humantime::parse_duration)]
    #[arg(long = "init-timeout", default_value = "1s")]
    pub init_timeout: Duration,
}
