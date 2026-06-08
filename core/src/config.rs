//! Configuration for Joule Profiler.
//!
//! This module defines the configuration structures used to run the profiler:
//! which command to execute, output options, and RAPL settings.
//!
//! # Example
//!
//! ```no_run
//! use joule_profiler_core::config::{Config, Command, ProfileConfig};
//!
//! let profile = ProfileConfig {
//!     stdout_file: None,
//!     cmd: vec!["sleep".into(), "1".into()],
//!     token_pattern: "__[A-Z0-9_]+__".into(),
//!     use_root: false,
//!     init_timeout: std::time::Duration::from_secs(1),
//! };
//!
//! let config = Config {
//!     command: Command::Profile(profile),
//!     rapl_path: None,
//! };
//! ```

use std::time::Duration;

use derive_builder::Builder;

const PHASE_TOKEN_DEFAULT_REGEX_PATTERN: &str = "__[A-Z0-9_]+__";
const DEFAULT_INIT_TIMEOUT: Duration = Duration::from_secs(1);

/// Top-level configuration for Joule Profiler.
#[derive(Debug)]
pub struct Config {
    /// Action to run (profile a program or list sensors).
    pub command: Command,

    /// Override the base path used to read Intel RAPL counters.
    pub rapl_path: Option<String>,
}

/// Command executed by the profiler.
#[derive(Debug, Clone)]
pub enum Command {
    /// Run a program and collect metrics.
    Profile(ProfileConfig),

    /// List available sensors.
    ListSensors,
}

/// Configuration for program profiling.
#[derive(Debug, Clone, Builder)]
pub struct ProfileConfig {
    /// Optional file to redirect the profiled program stdout.
    pub stdout_file: Option<String>,

    /// Command and arguments to execute.
    pub cmd: Vec<String>,

    /// Regex used to detect phase tokens in program output.
    #[builder(default = PHASE_TOKEN_DEFAULT_REGEX_PATTERN.to_string())]
    pub token_pattern: String,

    /// Executes the profiled command with root privileges if true and Joule Profiler is launched as root.
    pub use_root: bool,

    /// Duration before aborting sources initialization.
    #[builder(default = DEFAULT_INIT_TIMEOUT)]
    pub init_timeout: Duration,
}
