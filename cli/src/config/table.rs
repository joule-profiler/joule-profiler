//! Configuration management for the profiler CLI.
//!
//! This module provides [`ConfigTable`], which bridges raw CLI arguments and
//! global config file settings into a unified [`Config`] object consumed by
//! the profiler core.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use joule_profiler_core::{
    config::{Command, Config, ProfileConfigBuilder},
    source::MetricReader,
};
use log::warn;

use crate::{
    CliArgs, ProfilerCommand, Source,
    config::{GlobalConfig, ProfilerConfig, source::MetricSourceConfig},
    output::formats::OutputFormat,
};

/// Holds the resolved configuration state, merging values from the global
/// config file and CLI arguments before they are built into a final.
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

    /// Applies CLI overrides to Joule Profiler global configuration.
    pub fn apply_cli(&mut self, cli: &mut CliArgs) {
        if cli.csv {
            self.profiler_config.output_format = OutputFormat::Csv;
        } else if cli.json {
            self.profiler_config.output_format = OutputFormat::Json;
        }

        if let Some(output_file) = cli.output_file.take() {
            self.profiler_config.output_file = Some(output_file);
        }

        if let Some(rapl_backend) = cli.rapl_backend.take() {
            self.profiler_config.rapl_backend = rapl_backend;
        }

        if let ProfilerCommand::Profile(profile_args) = &cli.command {
            self.profiler_config.use_root |= profile_args.use_root;
        }
    }

    /// Builds a metric source reader of type `R` using the provided configuration
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

    /// Builds a metric source reader of type `R`, applying an external config
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
    /// Consumes the [`ConfigTable`] and a final [`CliArgs`] to produce the
    /// core [`Config`].
    ///
    /// CLI values override config file values for all fields
    /// resolved here (stdout file, root mode, timeouts, token patterns).
    ///
    /// Returns an error if the configuration is invalid.
    pub fn to_config(self, cli: CliArgs) -> Result<Config> {
        let command = match cli.command {
            ProfilerCommand::Profile(profile_args) => {
                let stdout_file = profile_args
                    .stdout_file
                    .or(self.profiler_config.stdout_file);

                let use_root = profile_args.use_root || self.profiler_config.use_root;

                let init_timeout = profile_args
                    .init_timeout
                    .unwrap_or(self.profiler_config.init_timeout);

                let token_pattern = profile_args
                    .token_pattern
                    .unwrap_or(self.profiler_config.token_pattern);

                let mut builder = ProfileConfigBuilder::default();

                let config = builder
                    .cmd(profile_args.cmd)
                    .stdout_file(stdout_file)
                    .token_pattern(token_pattern)
                    .use_root(use_root)
                    .init_timeout(init_timeout)
                    .build()?;

                Command::Profile(config)
            }
            ProfilerCommand::ListSensors => Command::ListSensors,
        };

        Ok(Config { command })
    }
}
