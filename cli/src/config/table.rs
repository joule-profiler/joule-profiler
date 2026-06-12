use std::collections::HashSet;

use anyhow::Result;
use joule_profiler_core::{
    config::{Command, Config, ProfileConfigBuilder},
    source::MetricReader,
};
use log::warn;

use crate::{
    CliArgs, ProfilerCommand, Source,
    config::{GlobalConfig, source::MetricSourceConfig},
    config_to_displayer,
    output::displayer::Displayer,
};

pub struct ConfigTable {
    pub config: GlobalConfig,
    enabled_sources: HashSet<String>,
}

impl ConfigTable {
    pub fn new(config: GlobalConfig, sources: &[Source]) -> Self {
        let enabled_sources: HashSet<_> = config
            .sources
            .keys()
            .cloned()
            .chain(sources.iter().map(|s| s.to_string()))
            .collect();

        Self {
            config,
            enabled_sources,
        }
    }

    pub fn build_source_override<R, O>(
        &mut self,
        config_override: &mut O,
        config_override_fn: impl FnOnce(&mut O, &mut R::Config),
    ) -> Result<Option<R>>
    where
        R: MetricReader,
    {
        if !self.enabled_sources.contains(R::get_id()) {
            return Ok(None);
        }

        let config_wrapper = match self.config.sources.remove(R::get_id()) {
            Some(v) => v.try_into(),
            None => Ok(MetricSourceConfig::default()),
        }?;

        let mut config = config_wrapper.inner;

        config_override_fn(config_override, &mut config);

        match R::from_config(config) {
            Ok(reader) => Ok(Some(reader)),
            Err(e) => {
                if config_wrapper.ignore_on_failure {
                    warn!(
                        "Failed to initialize source {}, skipping. Cause: {e}.",
                        R::get_name()
                    );
                    Ok(None)
                } else {
                    Err(e.into())
                }
            }
        }
    }

    pub fn build_source<R>(&mut self) -> Result<Option<R>>
    where
        R: MetricReader,
    {
        if !self.enabled_sources.contains(R::get_id()) {
            return Ok(None);
        }

        let config_wrapper = match self.config.sources.remove(R::get_id()) {
            Some(v) => v.try_into(),
            None => Ok(MetricSourceConfig::default()),
        }?;

        let config = config_wrapper.inner;

        match R::from_config(config) {
            Ok(reader) => Ok(Some(reader)),
            Err(e) => {
                if config_wrapper.ignore_on_failure {
                    warn!(
                        "Failed to initialize source {}, skipping. Cause: {e}.",
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
    pub fn to_config(self, cli: CliArgs) -> Result<(Config, Box<dyn Displayer>)> {
        let displayer = config_to_displayer(&self, &cli)?;

        let command = match cli.command {
            ProfilerCommand::Profile(profile_args) => {
                let profiler_config = self.config.profiler;
                let stdout_file = profile_args.stdout_file.or(profiler_config.stdout_file);

                let use_root = profile_args.use_root || profiler_config.use_root;

                let init_timeout = profile_args
                    .init_timeout
                    .unwrap_or(profiler_config.init_timeout);

                let token_pattern = profile_args
                    .token_pattern
                    .unwrap_or(profiler_config.token_pattern);

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

        Ok((Config { command }, displayer))
    }
}
