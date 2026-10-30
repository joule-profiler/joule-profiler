use std::{fs, path::Path};

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::error::{ConfigError, Result};

/// A format a table can be read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    #[cfg(feature = "toml")]
    Toml,

    #[cfg(feature = "json")]
    Json,
}

impl Format {
    /// Returns the format of a configuration file from its extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            #[cfg(feature = "toml")]
            "toml" => Some(Self::Toml),
            #[cfg(feature = "json")]
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    #[cfg_attr(
        not(any(feature = "toml", feature = "json")),
        allow(unused_variables, clippy::unnecessary_wraps)
    )]
    fn parse(
        self,
        text: &str,
    ) -> std::result::Result<Map<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
        match self {
            #[cfg(feature = "toml")]
            Self::Toml => Ok(toml::from_str(text)?),
            #[cfg(feature = "json")]
            Self::Json => Ok(serde_json::from_str(text)?),
        }
    }
}

/// A configuration before it is deserialized, whatever the format it was read from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigTable(Map<String, Value>);

impl ConfigTable {
    /// Reads a table from a file, in the format of its extension.
    pub fn read(path: &Path) -> Result<Self> {
        let format =
            Format::from_path(path).ok_or_else(|| ConfigError::UnknownFormat(path.into()))?;
        let text =
            fs::read_to_string(path).map_err(|error| ConfigError::Read(path.into(), error))?;

        format
            .parse(&text)
            .map(Self)
            .map_err(|error| ConfigError::Parse(path.into(), error))
    }

    /// Sets the value of a dotted `key`, such as `sources.rapl.sockets`, creating the missing
    /// sections.
    pub fn set(&mut self, key: &str, value: Value) -> Result<()> {
        let segments: Vec<&str> = key.split('.').map(str::trim).collect();
        if segments.iter().any(|segment| segment.is_empty()) {
            return Err(ConfigError::Key(key.to_owned()));
        }
        let Some((last, parents)) = segments.split_last() else {
            return Err(ConfigError::Key(key.to_owned()));
        };

        let mut table = &mut self.0;
        for segment in parents {
            table = table
                .entry(*segment)
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .ok_or_else(|| ConfigError::NotATable((*segment).to_owned()))?;
        }
        table.insert((*last).to_owned(), value);
        Ok(())
    }

    /// Sets a `key=value` definition. The value is read as a TOML value, or as JSON without the
    /// `toml` feature, and as a string if it is neither.
    pub fn define(&mut self, definition: &str) -> Result<()> {
        let (key, raw) = definition
            .split_once('=')
            .ok_or_else(|| ConfigError::Definition(definition.to_owned()))?;

        self.set(key, parse_value(raw))
    }

    /// Deserializes the table.
    pub fn deserialize<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_value(Value::Object(self.0.clone())).map_err(ConfigError::Invalid)
    }
}

#[cfg(feature = "toml")]
fn parse_value(raw: &str) -> Value {
    toml::from_str::<Map<String, Value>>(&format!("value = {raw}"))
        .ok()
        .and_then(|mut table| table.remove("value"))
        .unwrap_or_else(|| Value::String(raw.to_owned()))
}

#[cfg(not(feature = "toml"))]
fn parse_value(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn define_creates_nested_tables() {
        let mut table = ConfigTable::default();

        table.define("sources.perf_event.scope=cgroup").unwrap();

        assert_eq!(table.0["sources"]["perf_event"]["scope"], json!("cgroup"));
    }

    #[test]
    fn define_parses_values() {
        let mut table = ConfigTable::default();

        table.define("profiler.defer=true").unwrap();
        table.define("sources.perf_event.cpus=[0, 1]").unwrap();
        table.define("injector.token_pattern=__[A-Z]+__").unwrap();

        assert_eq!(table.0["profiler"]["defer"], json!(true));
        assert_eq!(table.0["sources"]["perf_event"]["cpus"], json!([0, 1]));
        assert_eq!(table.0["injector"]["token_pattern"], json!("__[A-Z]+__"));
    }

    #[test]
    fn invalid_keys() {
        let mut table = ConfigTable::default();
        table.define("profiler.defer=true").unwrap();

        assert!(matches!(
            table.define("profiler.defer.deeper=1"),
            Err(ConfigError::NotATable(_))
        ));
        assert!(matches!(
            table.define("profiler..defer=1"),
            Err(ConfigError::Key(_))
        ));
        assert!(matches!(
            table.define("profiler.defer"),
            Err(ConfigError::Definition(_))
        ));
    }

    #[cfg(feature = "toml")]
    #[test]
    fn toml_reads_as_json() {
        let table = Format::Toml
            .parse("[sources.rapl]\nsockets = [0]\nunit = \"joule\"")
            .unwrap();

        assert_eq!(
            Value::Object(table),
            json!({ "sources": { "rapl": { "sockets": [0], "unit": "joule" } } })
        );
    }
}
