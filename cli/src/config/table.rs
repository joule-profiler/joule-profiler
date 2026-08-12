//! Configuration management for Joule Profiler.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use clap::ValueEnum;
use joule_profiler_core::{
    config::{Command, Config, ProfileConfigBuilder},
    source::MetricReader,
};
use log::warn;

use crate::{
    CliArgs, ProfilerCommand, Source,
    config::{GlobalConfig, ProfilerConfig, source::MetricSourceConfig},
};

/// The structure used to resolve sources from toml configuration and the profiler config.
#[derive(Debug)]
pub struct ConfigTable {
    /// Global Joule Profiler configuration.
    pub profiler_config: ProfilerConfig,

    /// Raw TOML values for each named metric source, keyed by source ID.
    pub sources_config: HashMap<String, toml::Value>,

    /// The set of source IDs that are enabled.
    enabled_sources: HashSet<String>,
}

impl ConfigTable {
    /// Creates a new [`ConfigTable`] from a [`GlobalConfig`] and the
    /// sources enabled from the CLI.
    ///
    /// The `enabled_sources` set is the union of:
    /// - sources declared in the global config file, and
    /// - sources passed directly via the `sources` CLI argument.
    pub fn new(global_config: GlobalConfig, sources: &[Source]) -> Self {
        let enabled_sources: HashSet<_> = global_config
            .sources
            .keys()
            .cloned()
            .chain(sources.iter().map(Source::to_string))
            .collect();

        Self {
            profiler_config: global_config.profiler,
            sources_config: global_config.sources,
            enabled_sources,
        }
    }

    /// Applies every CLI override of the global (profiler-level) configuration
    /// in one pass, so [`ConfigTable::to_config`] can read `profiler_config`
    /// directly afterwards instead of re-merging `cli` itself.
    ///
    /// Only the flags that have no configuration key of their own go through
    /// here: the `-D` overrides are already merged into the configuration
    /// before the table is built.
    pub fn apply_cli(&mut self, cli: &mut CliArgs) {
        if let Some(output_format) = cli.output_format.take() {
            self.profiler_config.output_format = output_format;
        }

        if let Some(output_file) = cli.output_file.take() {
            self.profiler_config.output_file = Some(output_file);
        }

        if let ProfilerCommand::Profile(profile_args) = &mut cli.command {
            if let Some(stdout_file) = profile_args.stdout_file.take() {
                self.profiler_config.stdout_file = Some(stdout_file);
            }

            if let Some(token_pattern) = profile_args.token_pattern.take() {
                self.profiler_config.token_pattern = token_pattern;
            }

            if let Some(init_timeout) = profile_args.init_timeout.take() {
                self.profiler_config.init_timeout = init_timeout;
            }

            self.profiler_config.use_root |= profile_args.use_root;
        }
    }

    /// Builds a metric source reader using the provided configuration
    /// into the config table, or default configuration if not configured.
    ///
    /// It returns an error if the source initialization fails and `ignore_on_failure` is not set.
    pub fn build_source<R>(&mut self) -> Result<Option<R>>
    where
        R: MetricReader,
    {
        if !self.enabled_sources.contains(R::get_id()) {
            return Ok(None);
        }

        let config_wrapper = match self.sources_config.remove(R::get_id()) {
            Some(v) => v.try_into(),
            None => Ok(MetricSourceConfig::default()),
        }?;

        let config = config_wrapper.inner;

        match R::from_config(config) {
            Ok(reader) => Ok(Some(reader)),
            Err(e) => {
                if config_wrapper.ignore_on_failure {
                    warn!(
                        "Failed to initialize source {}, skipping. Error: {e}",
                        R::get_name()
                    );
                    Ok(None)
                } else {
                    Err(e.into())
                }
            }
        }
    }

    /// Builds a metric source reader, applying an external config
    /// override through a caller-supplied closure before construction.
    ///
    /// It returns an error if the source initialization fails and `ignore_on_failure` is not set.
    pub fn build_source_override<R>(
        &mut self,
        config_override_fn: impl FnOnce(&mut R::Config),
    ) -> Result<Option<R>>
    where
        R: MetricReader,
    {
        if !self.enabled_sources.contains(R::get_id()) {
            return Ok(None);
        }

        let config_wrapper = match self.sources_config.remove(R::get_id()) {
            Some(v) => v.try_into(),
            None => Ok(MetricSourceConfig::default()),
        }?;

        let mut config = config_wrapper.inner;

        config_override_fn(&mut config);

        match R::from_config(config) {
            Ok(reader) => Ok(Some(reader)),
            Err(e) => {
                if config_wrapper.ignore_on_failure {
                    warn!(
                        "Failed to initialize source {}, skipping. Error: {e}",
                        R::get_name()
                    );
                    Ok(None)
                } else {
                    Err(e.into())
                }
            }
        }
    }
}

impl ConfigTable {
    /// Errors on the configured sources no registered source claimed.
    ///
    /// Must be called once every source has been registered: `build_source`
    /// takes its own section out of the table, so what is left can only be
    /// misspelled source names.
    pub fn ensure_sources_are_known(&self) -> Result<()> {
        if self.sources_config.is_empty() {
            return Ok(());
        }

        let mut unknown: Vec<&str> = self.sources_config.keys().map(String::as_str).collect();
        unknown.sort_unstable();

        let known: Vec<String> = Source::value_variants()
            .iter()
            .map(Source::to_string)
            .collect();

        bail!(
            "unknown metric source `{}`. Available sources: {}.",
            unknown.join("`, `"),
            known.join(", "),
        )
    }

    /// Consumes the [`ConfigTable`] and a final [`CliArgs`] to produce the
    /// core [`Config`].
    ///
    /// Returns an error if the configuration is invalid.
    pub fn to_config(self, cli: CliArgs) -> Result<Config> {
        let command = match cli.command {
            ProfilerCommand::Profile(profile_args) => {
                let mut builder = ProfileConfigBuilder::default();

                let config = builder
                    .cmd(profile_args.cmd)
                    .stdout_file(self.profiler_config.stdout_file)
                    .token_pattern(self.profiler_config.token_pattern)
                    .use_root(self.profiler_config.use_root)
                    .init_timeout(self.profiler_config.init_timeout)
                    .build()?;

                Command::Profile(config)
            }
            ProfilerCommand::ListSensors => Command::ListSensors,
        };

        Ok(Config { command })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use joule_profiler_core::{sensor::Sensors, types::Metrics};
    use serde::Deserialize;

    use super::*;
    use crate::{commands::profile::ProfileArgs, output::formats::OutputFormat};

    #[derive(Debug, Default, Deserialize)]
    struct MockConfig {
        #[serde(default)]
        fail: bool,
        #[serde(default)]
        value: u32,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("mock source failure")]
    struct MockError;

    struct MockSource {
        value: u32,
    }

    impl MetricReader for MockSource {
        type Type = ();
        type Error = MockError;
        type Config = MockConfig;

        fn from_config(config: MockConfig) -> std::result::Result<Self, MockError> {
            if config.fail {
                Err(MockError)
            } else {
                Ok(Self {
                    value: config.value,
                })
            }
        }

        async fn measure(&mut self) -> std::result::Result<(), MockError> {
            Ok(())
        }

        async fn retrieve(&mut self) -> std::result::Result<(), MockError> {
            Ok(())
        }

        fn get_sensors(&self) -> std::result::Result<Sensors, MockError> {
            Ok(Vec::new())
        }

        fn to_metrics(&self, (): ()) -> std::result::Result<Metrics, MockError> {
            Ok(Metrics::default())
        }

        fn get_name() -> &'static str {
            "Mock"
        }

        fn get_id() -> &'static str {
            "mock"
        }
    }

    fn config_table_with(profiler_config: ProfilerConfig) -> ConfigTable {
        ConfigTable {
            profiler_config,
            sources_config: HashMap::new(),
            enabled_sources: HashSet::new(),
        }
    }

    fn cli_args(command: ProfilerCommand) -> CliArgs {
        CliArgs {
            verbose: 0,
            output_format: None,
            output_file: None,
            sources: Vec::new(),
            overrides: Vec::new(),
            config_file: None,
            command,
        }
    }

    fn configured_profiler_config() -> ProfilerConfig {
        ProfilerConfig {
            stdout_file: Some("configured_stdout.txt".to_owned()),
            token_pattern: "__CONFIGURED__".to_owned(),
            use_root: false,
            output_file: Some("configured_output.json".to_owned()),
            output_format: OutputFormat::Json,
            init_timeout: Duration::from_secs(5),
            #[cfg(feature = "rapl")]
            rapl_backend: crate::RaplBackend::default(),
        }
    }

    #[test]
    fn profiler_config_uses_default_config_when_not_override() {
        let config = ProfilerConfig::default();

        assert_eq!(config.stdout_file, None);
        assert_eq!(config.output_file, None);
        assert_eq!(config.token_pattern, "__[A-Z0-9_]+__");
        assert_eq!(config.init_timeout, Duration::from_secs(1));
        assert!(!config.use_root);
        assert!(matches!(config.output_format, OutputFormat::Terminal));
    }

    #[test]
    fn new_enabled_sources_is_union_of_config_file_and_cli() {
        let from_cli = Source::value_variants()[0].clone();

        let mut sources = HashMap::new();
        sources.insert(
            "from_file".to_owned(),
            toml::Value::Table(toml::map::Map::default()),
        );

        let global = GlobalConfig {
            profiler: ProfilerConfig::default(),
            sources,
        };

        let table = ConfigTable::new(global, std::slice::from_ref(&from_cli));

        assert!(table.enabled_sources.contains("from_file"));
        assert!(table.enabled_sources.contains(&from_cli.to_string()));
        assert!(!table.enabled_sources.contains("not_configured"));
    }

    #[test]
    fn new_enabled_sources_empty_by_default() {
        let table = ConfigTable::new(GlobalConfig::default(), &[]);
        assert!(table.enabled_sources.is_empty());
    }

    #[test]
    fn apply_cli_with_no_overrides_keeps_the_base_config() {
        let mut table = config_table_with(configured_profiler_config());
        let mut cli = cli_args(ProfilerCommand::Profile(ProfileArgs {
            cmd: vec!["echo".to_owned()],
            ..Default::default()
        }));

        table.apply_cli(&mut cli);

        assert_eq!(
            table.profiler_config.stdout_file.as_deref(),
            Some("configured_stdout.txt")
        );
        assert_eq!(table.profiler_config.token_pattern, "__CONFIGURED__");
        assert_eq!(
            table.profiler_config.output_file.as_deref(),
            Some("configured_output.json")
        );
        assert!(matches!(
            table.profiler_config.output_format,
            OutputFormat::Json
        ));
        assert_eq!(table.profiler_config.init_timeout, Duration::from_secs(5));
        assert!(!table.profiler_config.use_root);
    }

    #[test]
    fn apply_cli_overrides_output_format_and_consumes_it() {
        let mut table = config_table_with(ProfilerConfig::default());
        let mut cli = cli_args(ProfilerCommand::ListSensors);
        cli.output_format = Some(OutputFormat::Csv);

        table.apply_cli(&mut cli);

        assert!(matches!(
            table.profiler_config.output_format,
            OutputFormat::Csv
        ));
        assert!(cli.output_format.is_none());
    }

    #[test]
    fn apply_cli_overrides_output_file_and_consumes_it() {
        let mut table = config_table_with(ProfilerConfig::default());
        let mut cli = cli_args(ProfilerCommand::ListSensors);
        cli.output_file = Some("cli_output.csv".to_owned());

        table.apply_cli(&mut cli);

        assert_eq!(
            table.profiler_config.output_file.as_deref(),
            Some("cli_output.csv")
        );
        assert!(cli.output_file.is_none());
    }

    #[test]
    fn apply_cli_overrides_profile_only_fields_and_consumes_them() {
        let mut table = config_table_with(ProfilerConfig::default());
        let mut cli = cli_args(ProfilerCommand::Profile(ProfileArgs {
            cmd: vec!["sleep".to_owned(), "1".to_owned()],
            stdout_file: Some("cli_stdout.txt".to_owned()),
            token_pattern: Some("__CLI__".to_owned()),
            init_timeout: Some(Duration::from_secs(9)),
            use_root: true,
        }));

        table.apply_cli(&mut cli);

        assert_eq!(
            table.profiler_config.stdout_file.as_deref(),
            Some("cli_stdout.txt")
        );
        assert_eq!(table.profiler_config.token_pattern, "__CLI__");
        assert_eq!(table.profiler_config.init_timeout, Duration::from_secs(9));
        assert!(table.profiler_config.use_root);

        let ProfilerCommand::Profile(args) = &cli.command else {
            panic!("expected a Profile command");
        };
        assert!(args.stdout_file.is_none());
        assert!(args.token_pattern.is_none());
        assert!(args.init_timeout.is_none());
    }

    #[test]
    fn apply_cli_use_root_not_overwritten() {
        let mut table = config_table_with(ProfilerConfig {
            use_root: true,
            ..ProfilerConfig::default()
        });
        let mut cli = cli_args(ProfilerCommand::Profile(ProfileArgs {
            cmd: vec!["true".to_owned()],
            use_root: false,
            ..Default::default()
        }));

        table.apply_cli(&mut cli);

        assert!(table.profiler_config.use_root);
    }

    #[test]
    fn to_config_profile_reads_resolved_fields_from_profiler_config() {
        let table = config_table_with(configured_profiler_config());
        let cli = cli_args(ProfilerCommand::Profile(ProfileArgs {
            cmd: vec!["sleep".to_owned(), "1".to_owned()],
            ..Default::default()
        }));

        let config = table.to_config(cli).unwrap();

        let Command::Profile(profile_config) = config.command else {
            panic!("expected a Profile command");
        };
        assert_eq!(profile_config.cmd, vec!["sleep".to_owned(), "1".to_owned()]);
        assert_eq!(
            profile_config.stdout_file.as_deref(),
            Some("configured_stdout.txt")
        );
        assert_eq!(profile_config.token_pattern, "__CONFIGURED__");
        assert_eq!(profile_config.init_timeout, Duration::from_secs(5));
        assert!(!profile_config.use_root);
    }

    #[test]
    fn build_source_returns_none_when_not_enabled() {
        let mut table = config_table_with(ProfilerConfig::default());
        assert!(table.build_source::<MockSource>().unwrap().is_none());
    }

    #[test]
    fn build_source_uses_default_config_when_no_section_present() {
        let mut table = config_table_with(ProfilerConfig::default());
        table.enabled_sources.insert("mock".to_owned());

        let source = table.build_source::<MockSource>().unwrap().unwrap();
        assert_eq!(source.value, 0);
    }

    #[test]
    fn build_source_uses_the_matching_config_section() {
        let mut table = config_table_with(ProfilerConfig::default());
        table.enabled_sources.insert("mock".to_owned());
        table
            .sources_config
            .insert("mock".to_owned(), toml::from_str("value = 42").unwrap());

        let source = table.build_source::<MockSource>().unwrap().unwrap();
        assert_eq!(source.value, 42);
    }

    #[test]
    fn build_source_ignore_on_failure_ignores_the_error() {
        let mut table = config_table_with(ProfilerConfig::default());
        table.enabled_sources.insert("mock".to_owned());
        table.sources_config.insert(
            "mock".to_owned(),
            toml::from_str("fail = true\nignore_on_failure = true").unwrap(),
        );

        assert!(table.build_source::<MockSource>().unwrap().is_none());
    }

    #[test]
    fn build_source_without_ignore_on_failure_propagates_the_error() {
        let mut table = config_table_with(ProfilerConfig::default());
        table.enabled_sources.insert("mock".to_owned());
        table
            .sources_config
            .insert("mock".to_owned(), toml::from_str("fail = true").unwrap());

        assert!(table.build_source::<MockSource>().is_err());
    }

    #[test]
    fn build_source_override_applies_the_closure_before_construction() {
        let mut table = config_table_with(ProfilerConfig::default());
        table.enabled_sources.insert("mock".to_owned());

        let source = table
            .build_source_override::<MockSource>(|config| config.value = 99)
            .unwrap()
            .unwrap();

        assert_eq!(source.value, 99);
    }

    #[test]
    fn ensure_sources_are_known_accepts_a_fully_consumed_table() {
        let mut table = config_table_with(ProfilerConfig::default());
        table.enabled_sources.insert("mock".to_owned());
        table
            .sources_config
            .insert("mock".to_owned(), toml::from_str("value = 1").unwrap());

        table.build_source::<MockSource>().unwrap();

        table.ensure_sources_are_known().unwrap();
    }

    #[test]
    fn ensure_sources_are_known_rejects_an_unclaimed_source() {
        let mut table = config_table_with(ProfilerConfig::default());
        table
            .sources_config
            .insert("nope".to_owned(), toml::from_str("value = 1").unwrap());

        let err = table.ensure_sources_are_known().unwrap_err();

        assert!(err.to_string().contains("nope"));
        assert!(
            err.to_string()
                .contains(&Source::value_variants()[0].to_string())
        );
    }
}
