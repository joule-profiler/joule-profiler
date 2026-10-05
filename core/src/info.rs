use std::fmt::{self, Display};
use std::time::Duration;

use serde::{Serialize, Serializer};

use crate::metric::MetricValue;
use crate::util::sys;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Info {
    entries: Vec<(String, InfoValue)>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum InfoValue {
    Bool(bool),
    Number(MetricValue),
    Text(String),
    List(Vec<String>),
}

impl Info {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, key: impl Into<String>, value: impl Into<InfoValue>) -> Self {
        self.entries.push((key.into(), value.into()));
        self
    }

    pub fn entries(&self) -> &[(String, InfoValue)] {
        &self.entries
    }

    pub fn get(&self, key: &str) -> Option<&InfoValue> {
        self.entries
            .iter()
            .find_map(|(name, value)| (name == key).then_some(value))
    }
}

/// Serialized as a JSON object, keeping the order of the entries.
impl Serialize for Info {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.entries.iter().map(|(key, value)| (key, value)))
    }
}

impl InfoValue {
    pub fn list<T: Display>(items: impl IntoIterator<Item = T>) -> Self {
        Self::List(items.into_iter().map(|item| item.to_string()).collect())
    }
}

impl Display for InfoValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => value.fmt(f),
            Self::Number(value) => value.fmt(f),
            Self::Text(text) => f.write_str(text),
            Self::List(items) => f.write_str(&items.join(", ")),
        }
    }
}

impl From<bool> for InfoValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<u64> for InfoValue {
    fn from(value: u64) -> Self {
        Self::Number(value.into())
    }
}

impl From<i64> for InfoValue {
    fn from(value: i64) -> Self {
        Self::Number(value.into())
    }
}

impl From<&str> for InfoValue {
    fn from(text: &str) -> Self {
        Self::Text(text.to_owned())
    }
}

impl From<String> for InfoValue {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

/// Written as in the configuration, such as `10ms`.
impl From<Duration> for InfoValue {
    fn from(duration: Duration) -> Self {
        Self::Text(format!("{duration:?}"))
    }
}

/// An error becomes `unavailable (<error>)`, so one failed read does not hide the rest.
impl<T: Into<InfoValue>, E: Display> From<Result<T, E>> for InfoValue {
    fn from(value: Result<T, E>) -> Self {
        value.map_or_else(
            |error| Self::Text(format!("unavailable ({error})")),
            Into::into,
        )
    }
}

/// The CPU model and the CPUs of each socket.
pub(crate) fn machine() -> Info {
    let sockets = sys::socket_topology().map(|sockets| {
        InfoValue::list(
            sockets
                .iter()
                .map(|socket| format!("{} (cpus {})", socket.id, sys::cpu_list(&socket.cpus))),
        )
    });

    Info::new()
        .with("cpu", sys::cpu_model())
        .with("sockets", sockets)
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    #[test]
    fn entries_keep_the_order_they_were_added_in() {
        let info = Info::new().with("b", "2").with("a", "1");

        let keys: Vec<&str> = info.entries().iter().map(|(key, _)| key.as_str()).collect();

        assert_eq!(keys, ["b", "a"]);
    }

    #[test]
    fn a_value_that_could_not_be_read_says_why() {
        let failed: io::Result<String> = Err(io::Error::other("no such file"));

        let info = Info::new().with("pmus", failed);

        assert_eq!(
            info.get("pmus"),
            Some(&InfoValue::from("unavailable (no such file)"))
        );
    }

    #[test]
    fn values_are_written_plainly() {
        assert_eq!(InfoValue::from(true).to_string(), "true");
        assert_eq!(InfoValue::from(42_u64).to_string(), "42");
        assert_eq!(
            InfoValue::from(Duration::from_millis(10)).to_string(),
            "10ms"
        );
        assert_eq!(InfoValue::list(["cpu", "power"]).to_string(), "cpu, power");
    }

    #[test]
    fn a_description_is_serialized_as_an_object_of_typed_values() {
        let info = Info::new()
            .with("backend", "perf")
            .with("domains", InfoValue::list(["PACKAGE-0", "DRAM-0"]))
            .with("global", false)
            .with("perf_event_paranoid", -1_i64);

        assert_eq!(
            serde_json::to_string(&info).unwrap(),
            r#"{"backend":"perf","domains":["PACKAGE-0","DRAM-0"],"global":false,"perf_event_paranoid":-1}"#
        );
    }

    #[test]
    fn the_machine_tells_its_processors_and_its_sockets() {
        let machine = machine();

        assert!(machine.get("cpu").is_some());
        assert!(matches!(machine.get("sockets"), Some(InfoValue::List(_))));
    }
}
