use std::{collections::HashMap, fs, path::Path, time::Duration};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::{RaplBackend, config::overrides::ConfigOverride, output::formats::OutputFormat};

pub mod overrides;
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

/// Unknown keys are rejected: a misspelled one would otherwise be dropped
/// without a trace, which is easy to miss on a `-D` override.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct GlobalConfig {
    /// The global configuration of the profiler.
    #[serde(default)]
    pub profiler: ProfilerConfig,

    /// The sources configurations.
    #[serde(default)]
    pub sources: HashMap<String, toml::Value>,
}

/// Loads the configuration from `config_file`, with the `-D` overrides applied
/// on top of it.
///
/// Overrides are applied to the raw TOML rather than to the deserialized
/// configuration, so a `-D` reaches any key a configuration file can set,
/// including per-source ones, and goes through the same checks.
pub fn load_global_config(
    config_file: Option<&Path>,
    overrides: &[ConfigOverride],
) -> Result<GlobalConfig> {
    let mut value = match config_file {
        Some(path) => {
            let content = fs::read_to_string(path)
                .with_context(|| format!("configuration file error on `{}`", path.display()))?;
            toml::from_str(&content).context("error parsing configuration file")?
        }
        None => toml::Value::Table(toml::map::Map::new()),
    };

    for config_override in overrides {
        config_override.apply(&mut value)?;
    }

    value
        .try_into()
        .context("error applying the configuration. Check the `-D` overrides")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn overrides(raw: &[&str]) -> Vec<ConfigOverride> {
        raw.iter().map(|s| s.parse().unwrap()).collect()
    }

    fn config_file(content: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "{content}").unwrap();
        file
    }

    #[test]
    fn load_without_a_file_nor_overrides_is_the_default_config() {
        let config = load_global_config(None, &[]).unwrap();

        assert_eq!(config.profiler.token_pattern, DEFAULT_TOKEN_PATTERN);
        assert!(config.sources.is_empty());
    }

    #[test]
    fn load_applies_overrides_without_a_config_file() {
        let config = load_global_config(
            None,
            &overrides(&[
                "profiler.token_pattern=__CLI__",
                "sources.cgroup.create_cgroup=false",
            ]),
        )
        .unwrap();

        assert_eq!(config.profiler.token_pattern, "__CLI__");
        assert_eq!(
            config.sources["cgroup"]["create_cgroup"],
            toml::Value::Boolean(false)
        );
    }

    #[test]
    fn load_overrides_take_precedence_over_the_file() {
        let file = config_file(
            "[profiler]\ntoken_pattern = \"__FILE__\"\nuse_root = true\n\
             [sources.cgroup]\ncgroup_name = \"from-file\"\n",
        );

        let config = load_global_config(
            Some(file.path()),
            &overrides(&["profiler.token_pattern=__CLI__"]),
        )
        .unwrap();

        assert_eq!(config.profiler.token_pattern, "__CLI__");
        assert!(config.profiler.use_root);
        assert_eq!(
            config.sources["cgroup"]["cgroup_name"],
            toml::Value::String("from-file".to_owned())
        );
    }

    #[test]
    fn load_overrides_a_source_absent_from_the_file() {
        let file = config_file("[sources.rapl]\nsockets_spec = [0]\n");

        let config = load_global_config(
            Some(file.path()),
            &overrides(&["sources.cgroup.poll_interval=20ms"]),
        )
        .unwrap();

        assert_eq!(
            config.sources["cgroup"]["poll_interval"],
            toml::Value::String("20ms".to_owned())
        );
        assert_eq!(
            config.sources["rapl"]["sockets_spec"][0],
            toml::Value::Integer(0)
        );
    }

    #[test]
    fn load_reports_a_missing_config_file() {
        let err = load_global_config(Some(Path::new("/does/not/exist.toml")), &[]).unwrap_err();

        assert!(err.to_string().contains("/does/not/exist.toml"));
    }

    #[test]
    fn load_reports_an_override_of_the_wrong_type() {
        let err = load_global_config(None, &overrides(&["profiler.use_root=42"])).unwrap_err();

        assert!(err.to_string().contains("-D"));
    }

    #[test]
    fn load_rejects_an_unknown_profiler_key() {
        let err =
            load_global_config(None, &overrides(&["profiler.output_forrmat=json"])).unwrap_err();

        assert!(format!("{err:#}").contains("output_forrmat"));
    }

    #[test]
    fn load_rejects_an_unknown_top_level_key() {
        let err = load_global_config(None, &overrides(&["profilr.use_root=true"])).unwrap_err();

        assert!(format!("{err:#}").contains("profilr"));
    }

    #[test]
    fn load_rejects_an_unknown_key_of_a_config_file() {
        let file = config_file("[profiler]\nuse_rooot = true\n");

        let err = load_global_config(Some(file.path()), &[]).unwrap_err();

        assert!(format!("{err:#}").contains("use_rooot"));
    }
}
