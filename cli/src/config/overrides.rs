//! CLI overrides of the TOML configuration (`-D KEY=VALUE`).

use std::str::FromStr;

use anyhow::{Result, anyhow, bail};
use toml::{Value, map::Map};

/// A single `-D KEY=VALUE` override, `KEY` being the dotted path of the key in
/// the configuration file.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigOverride {
    path: Vec<String>,
    value: Value,
}

impl ConfigOverride {
    /// Writes the override into `root`, creating the tables it goes through.
    ///
    /// Returns an error if the path traverses a key that is not a table.
    pub fn apply(&self, root: &mut Value) -> Result<()> {
        let (key, parents) = self
            .path
            .split_last()
            .expect("an override path always has at least one segment");

        let key_path = self.path.join(".");

        let mut table = root.as_table_mut().ok_or_else(|| {
            anyhow!("cannot override `{key_path}`: the configuration is not a table.")
        })?;

        for (depth, segment) in parents.iter().enumerate() {
            let node = table
                .entry(segment.clone())
                .or_insert_with(|| Value::Table(Map::new()));
            let kind = node.type_str();

            table = node.as_table_mut().ok_or_else(|| {
                anyhow!(
                    "cannot override `{key_path}`: `{}` is a {kind}, not a table.",
                    self.path[..=depth].join("."),
                )
            })?;
        }

        table.insert(key.clone(), self.value.clone());

        Ok(())
    }
}

impl FromStr for ConfigOverride {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let Some((key, value)) = s.split_once('=') else {
            bail!("expected KEY=VALUE, got `{s}`.");
        };

        let path: Vec<String> = key
            .trim()
            .split('.')
            .map(|segment| segment.trim().to_owned())
            .collect();

        if path.iter().any(String::is_empty) {
            bail!("`{key}` is not a valid configuration key.");
        }

        Ok(Self {
            path,
            value: parse_value(value),
        })
    }
}

/// Parses a `-D` value as a TOML value, falling back to a bare string.
///
/// The fallback is what lets durations, regexes and paths go unquoted while
/// `false`, `10` or `[0,1]` keep the type the sources expect.
fn parse_value(raw: &str) -> Value {
    let raw = raw.trim();

    toml::from_str::<Value>(&format!("value = {raw}"))
        .ok()
        .and_then(|mut parsed| parsed.as_table_mut()?.remove("value"))
        .unwrap_or_else(|| Value::String(raw.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> ConfigOverride {
        s.parse::<ConfigOverride>().unwrap()
    }

    fn applied(overrides: &[&str]) -> Value {
        let mut root = Value::Table(Map::new());
        for raw in overrides {
            parse(raw).apply(&mut root).unwrap();
        }
        root
    }

    #[test]
    fn parses_a_dotted_key_path() {
        let parsed = parse("sources.rapl.rapl_path=/sys/class/powercap");

        assert_eq!(parsed.path, ["sources", "rapl", "rapl_path"]);
        assert_eq!(
            parsed.value,
            Value::String("/sys/class/powercap".to_owned())
        );
    }

    #[test]
    fn parses_toml_typed_values() {
        assert_eq!(parse("a=42").value, Value::Integer(42));
        assert_eq!(parse("a=false").value, Value::Boolean(false));
        assert_eq!(
            parse("a=[0, 1]").value,
            Value::Array(vec![Value::Integer(0), Value::Integer(1)])
        );
    }

    #[test]
    fn falls_back_to_a_string_value() {
        assert_eq!(parse("a=10ms").value, Value::String("10ms".to_owned()));
        assert_eq!(
            parse("a=__[A-Z]+__").value,
            Value::String("__[A-Z]+__".to_owned())
        );
    }

    #[test]
    fn quoting_forces_a_string_value() {
        assert_eq!(parse("a=\"42\"").value, Value::String("42".to_owned()));
    }

    #[test]
    fn keeps_equal_signs_of_the_value() {
        assert_eq!(parse("a=b=c").value, Value::String("b=c".to_owned()));
    }

    #[test]
    fn rejects_a_value_less_override() {
        assert!("profiler.use_root".parse::<ConfigOverride>().is_err());
    }

    #[test]
    fn rejects_an_empty_key_segment() {
        assert!("sources..rapl=1".parse::<ConfigOverride>().is_err());
        assert!("=1".parse::<ConfigOverride>().is_err());
    }

    #[test]
    fn apply_creates_the_intermediate_tables() {
        let root = applied(&["sources.cgroup.create_cgroup=false"]);

        assert_eq!(
            root["sources"]["cgroup"]["create_cgroup"],
            Value::Boolean(false)
        );
    }

    #[test]
    fn apply_keeps_the_neighbouring_keys() {
        let root = applied(&[
            "sources.cgroup.cgroup_name=first",
            "sources.cgroup.create_cgroup=false",
            "sources.rapl.sockets_spec=[0]",
            "profiler.use_root=true",
        ]);

        assert_eq!(
            root["sources"]["cgroup"]["cgroup_name"],
            Value::String("first".to_owned())
        );
        assert_eq!(
            root["sources"]["cgroup"]["create_cgroup"],
            Value::Boolean(false)
        );
        assert_eq!(
            root["sources"]["rapl"]["sockets_spec"][0],
            Value::Integer(0)
        );
        assert_eq!(root["profiler"]["use_root"], Value::Boolean(true));
    }

    #[test]
    fn apply_replaces_an_existing_value() {
        let root = applied(&["profiler.output_format=csv", "profiler.output_format=json"]);

        assert_eq!(
            root["profiler"]["output_format"],
            Value::String("json".to_owned())
        );
    }

    #[test]
    fn apply_rejects_a_path_through_a_non_table() {
        let mut root = Value::Table(Map::new());
        parse("profiler.use_root=true").apply(&mut root).unwrap();

        let err = parse("profiler.use_root.nested=1")
            .apply(&mut root)
            .unwrap_err();

        assert!(err.to_string().contains("profiler.use_root"));
    }
}
