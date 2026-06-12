use std::{collections::HashMap, time::Duration};

use serde::Deserialize;

use crate::output::formats::OutputFormat;

pub mod source;
pub mod table;

const DEFAULT_INIT_TIMEOUT: Duration = Duration::from_secs(1);
const DEFAULT_TOKEN_PATTERN: &str = "__[A-Z0-9_]+__";

fn default_timeout() -> Duration {
    DEFAULT_INIT_TIMEOUT
}

fn default_token_pattern() -> String {
    DEFAULT_TOKEN_PATTERN.to_owned()
}

#[derive(Debug, Default, Deserialize)]
pub struct ProfilerConfig {
    /// Optional file to redirect the profiled program stdout.
    pub stdout_file: Option<String>,

    /// Regex used to detect phase tokens in program output.
    #[serde(default = "default_token_pattern")]
    pub token_pattern: String,

    /// Executes the profiled command with root privileges if true and Joule Profiler is launched as root.
    #[serde(default)]
    pub use_root: bool,

    pub output_file: Option<String>,

    #[serde(default)]
    pub output_format: OutputFormat,

    #[serde(default = "default_timeout", with = "humantime_serde")]
    pub init_timeout: Duration,
}

#[derive(Debug, Default, Deserialize)]
pub struct GlobalConfig {
    #[serde(default)]
    pub profiler: ProfilerConfig,
    pub sources: HashMap<String, toml::Value>,
}
