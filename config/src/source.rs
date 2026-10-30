use std::{
    fmt::{self, Display},
    str::FromStr,
};

use joule_profiler_core::Source;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::error::{ConfigError, Result};

/// The sources this build can measure with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum SourceName {
    #[cfg(feature = "_rapl")]
    Rapl,

    #[cfg(feature = "perf_event")]
    #[cfg_attr(feature = "clap", value(alias = "perf_event"))]
    Perf,

    #[cfg(feature = "procfs")]
    Procfs,

    #[cfg(feature = "cgroup")]
    Cgroup,

    #[cfg(feature = "nvml")]
    Nvml,

    #[cfg(feature = "amdsmi")]
    #[cfg_attr(feature = "clap", value(name = "amdsmi"))]
    AmdSmi,
}

impl SourceName {
    /// Every source supported.
    pub const ALL: &[Self] = &[
        #[cfg(feature = "_rapl")]
        Self::Rapl,
        #[cfg(feature = "perf_event")]
        Self::Perf,
        #[cfg(feature = "procfs")]
        Self::Procfs,
        #[cfg(feature = "cgroup")]
        Self::Cgroup,
        #[cfg(feature = "nvml")]
        Self::Nvml,
        #[cfg(feature = "amdsmi")]
        Self::AmdSmi,
    ];

    /// Returns the name of its `[sources.<key>]` table.
    pub fn key(self) -> &'static str {
        match self {
            #[cfg(feature = "perf_event")]
            Self::Perf => "perf_event",
            #[allow(
                unreachable_patterns,
                reason = "perf_event may be the only source built"
            )]
            other => other.as_str(),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            #[cfg(feature = "_rapl")]
            Self::Rapl => "rapl",
            #[cfg(feature = "perf_event")]
            Self::Perf => "perf",
            #[cfg(feature = "procfs")]
            Self::Procfs => "procfs",
            #[cfg(feature = "cgroup")]
            Self::Cgroup => "cgroup",
            #[cfg(feature = "nvml")]
            Self::Nvml => "nvml",
            #[cfg(feature = "amdsmi")]
            Self::AmdSmi => "amdsmi",
        }
    }

    /// Builds the source from its table, or with the defaults without one. A source that cannot
    /// be built is left out, with a warning, if its table sets `ignore_on_failure`.
    pub fn build(self, mut table: Option<Value>) -> Result<Option<Source>> {
        let ignore_on_failure = table
            .as_mut()
            .and_then(Value::as_object_mut)
            .and_then(|table| table.remove("ignore_on_failure"))
            .and_then(|ignore| ignore.as_bool())
            .unwrap_or(false);

        let table = table.unwrap_or_else(|| Value::Object(Map::new()));
        let built = match self {
            #[cfg(feature = "_rapl")]
            Self::Rapl => self.build_with(table, joule_profiler_source_rapl::Rapl::build),
            #[cfg(feature = "perf_event")]
            Self::Perf => {
                self.build_with(table, joule_profiler_source_perf_event::PerfEvent::build)
            }
            #[cfg(feature = "procfs")]
            Self::Procfs => self.build_with(table, joule_profiler_source_procfs::Procfs::build),
            #[cfg(feature = "cgroup")]
            Self::Cgroup => self.build_with(table, joule_profiler_source_cgroup::Cgroup::build),
            #[cfg(feature = "nvml")]
            Self::Nvml => self.build_with(table, joule_profiler_source_nvml::Nvml::build),
            #[cfg(feature = "amdsmi")]
            Self::AmdSmi => self.build_with(table, joule_profiler_source_amdsmi::AmdSmi::build),
        };

        match built {
            Ok(source) => Ok(Some(source)),
            Err(ConfigError::Unavailable(_, error)) if ignore_on_failure => {
                log::warn!("cannot build `{self}` source: {error}");
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn build_with<C, E>(
        self,
        table: Value,
        build: impl FnOnce(C) -> std::result::Result<Source, E>,
    ) -> Result<Source>
    where
        C: DeserializeOwned,
        E: std::error::Error + Send + Sync + 'static,
    {
        let config = serde_json::from_value(table)
            .map_err(|error| ConfigError::SourceTable(self.key(), error))?;

        build(config).map_err(|error| ConfigError::Unavailable(self.key(), Box::new(error)))
    }
}

impl Display for SourceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SourceName {
    type Err = ConfigError;

    /// Parses a source from its name or the key of its table.
    fn from_str(name: &str) -> Result<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|source| source.as_str() == name || source.key() == name)
            .ok_or_else(|| ConfigError::UnknownSource {
                name: name.to_owned(),
                known: Self::ALL
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            })
    }
}

/// Returns the `named` sources, then those with a table in `tables`, each with its table.
/// Without any, returns the default source.
pub fn configured_sources(
    named: &[SourceName],
    tables: &Map<String, Value>,
) -> Result<Vec<(SourceName, Option<Value>)>> {
    let mut sources: Vec<(SourceName, Option<Value>)> =
        named.iter().map(|name| (*name, None)).collect();

    for (key, table) in tables {
        let name: SourceName = key.parse()?;
        match sources.iter_mut().find(|(known, _)| *known == name) {
            Some((_, configured)) => *configured = Some(table.clone()),
            None => sources.push((name, Some(table.clone()))),
        }
    }

    if sources.is_empty()
        && let Some(default) = SourceName::ALL.first()
    {
        sources.push((*default, None));
    }

    Ok(sources)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_name_or_key() {
        for source in SourceName::ALL {
            assert_eq!(source.to_string().parse::<SourceName>().unwrap(), *source);
            assert_eq!(source.key().parse::<SourceName>().unwrap(), *source);
        }
        assert!(matches!(
            "nothing".parse::<SourceName>(),
            Err(ConfigError::UnknownSource { .. })
        ));
    }

    #[cfg(all(
        feature = "procfs",
        feature = "perf_event",
        feature = "nvml",
        feature = "_rapl"
    ))]
    #[test]
    fn configured_sources_order() {
        let sources = configured_sources(
            &[SourceName::Procfs, SourceName::Perf],
            serde_json::json!({ "nvml": { "minimal": false }, "perf_event": { "events": [] } })
                .as_object()
                .unwrap(),
        )
        .unwrap();

        let names: Vec<SourceName> = sources.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [SourceName::Procfs, SourceName::Perf, SourceName::Nvml]
        );
        assert!(sources[1].1.is_some());

        let default = configured_sources(&[], &Map::new()).unwrap();
        assert_eq!(default[0].0, SourceName::Rapl);
    }

    #[cfg(feature = "_rapl")]
    #[test]
    fn ignore_on_failure() {
        let broken =
            serde_json::json!({ "sockets": [4_294_967_295_u32], "ignore_on_failure": true });
        assert!(SourceName::Rapl.build(Some(broken)).unwrap().is_none());

        let broken = serde_json::json!({ "sockets": [4_294_967_295_u32] });
        assert!(matches!(
            SourceName::Rapl.build(Some(broken)),
            Err(ConfigError::Unavailable("rapl", _))
        ));
    }
}
