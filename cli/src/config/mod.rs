use std::{collections::HashMap, time::Duration};

use serde::Deserialize;

use crate::{RaplBackend, output::formats::OutputFormat};

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

#[derive(Debug, Deserialize)]
pub struct ProfilerConfig {
    /// Optional file to redirect the profiled program stdout.
    pub stdout_file: Option<String>,

    /// Regex used to detect phase tokens in program output.
    #[serde(default = "default_token_pattern")]
    pub token_pattern: String,

    /// Executes the profiled command with root privileges if true and Joule Profiler is launched as root.
    #[serde(default)]
    pub use_root: bool,

    /// Output file for CSV/JSON. (else `data<TIMESTAMP>`.csv/json)
    pub output_file: Option<String>,

    /// The output format to use. (e.g., terminal, json, csv)
    #[serde(default)]
    pub output_format: OutputFormat,

    /// Duration before aborting sources initialization. (default: 1s)
    #[serde(default = "default_timeout", with = "humantime_serde")]
    pub init_timeout: Duration,

    #[serde(default)]
    pub rapl_backend: RaplBackend,
}

impl Default for ProfilerConfig {
    fn default() -> Self {
        Self {
            token_pattern: default_token_pattern(),
            init_timeout: default_timeout(),
            output_format: OutputFormat::Terminal,
            rapl_backend: RaplBackend::Perf,
            use_root: false,
            stdout_file: None,
            output_file: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct GlobalConfig {
    /// The global configuration of the profiler.
    #[serde(default)]
    pub profiler: ProfilerConfig,

    /// The sources configurations.
    #[serde(default)]
    pub sources: HashMap<String, toml::Value>,
}
