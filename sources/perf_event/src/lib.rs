use std::collections::HashSet;
use std::path::PathBuf;

use joule_profiler_core::info::Info;
use joule_profiler_core::source::Source;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

pub mod cgroup;
pub mod error;
pub mod event;
pub mod pid;
pub mod scope;

pub use cgroup::CgroupScope;
pub use error::PerfEventError;
pub use event::Event;
pub use pid::PidScope;
pub use scope::{PerfProcessor, PerfScope, PerfSensor};

use crate::cgroup::CGROUP_ROOT;

/// Hardware counters on the profiled program (`scope = "pid"`, the default) or on a cgroup.
#[derive(Debug, Clone)]
pub enum PerfEvent {
    Pid(Pid),
    Cgroup(Cgroup),
}

impl Default for PerfEvent {
    fn default() -> Self {
        Self::Pid(Pid::default())
    }
}

impl PerfEvent {
    pub fn build(self) -> Source {
        match self {
            Self::Pid(pid) => pid.build(),
            Self::Cgroup(cgroup) => cgroup.build(),
        }
    }
}

impl<'de> Deserialize<'de> for PerfEvent {
    /// `scope` picks the variant and defaults to `pid`.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "scope", rename_all = "lowercase")]
        enum Scoped {
            Pid(Pid),
            Cgroup(Cgroup),
        }

        let mut table = serde_json::Map::deserialize(deserializer)?;
        table.entry("scope").or_insert_with(|| "pid".into());

        match Scoped::deserialize(serde_json::Value::Object(table)).map_err(D::Error::custom)? {
            Scoped::Pid(pid) => Ok(Self::Pid(pid)),
            Scoped::Cgroup(cgroup) => Ok(Self::Cgroup(cgroup)),
        }
    }
}

/// Counters on the profiled program and the processes it starts.
#[must_use]
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pid {
    /// Default: [`Event::DEFAULT`].
    events: Option<HashSet<Event>>,
}

impl Pid {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(mut self, events: impl IntoIterator<Item = Event>) -> Self {
        self.events = Some(events.into_iter().collect());
        self
    }

    pub fn build(self) -> Source {
        let events = ordered(self.events);
        log::debug!("perf_event: counting {} events on a pid", events.len());

        Source::new(
            PerfSensor::new(PidScope::new(events.clone())),
            PerfProcessor::new(events, Info::new().with("scope", "pid")),
        )
    }
}

/// Counters on an existing cgroup.
#[must_use]
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cgroup {
    /// Default: [`Event::DEFAULT`].
    #[serde(default)]
    events: Option<HashSet<Event>>,

    /// Relative to `root`.
    name: PathBuf,

    /// Default: `/sys/fs/cgroup`.
    #[serde(default)]
    root: Option<PathBuf>,

    /// Default: every online CPU.
    #[serde(default)]
    cpus: Option<HashSet<u32>>,
}

impl Cgroup {
    pub fn new(name: impl Into<PathBuf>) -> Self {
        Self {
            events: None,
            name: name.into(),
            root: None,
            cpus: None,
        }
    }

    pub fn events(mut self, events: impl IntoIterator<Item = Event>) -> Self {
        self.events = Some(events.into_iter().collect());
        self
    }

    pub fn root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    pub fn cpus(mut self, cpus: impl IntoIterator<Item = u32>) -> Self {
        self.cpus = Some(cpus.into_iter().collect());
        self
    }

    pub fn build(self) -> Source {
        let events = ordered(self.events);
        let path = self
            .root
            .unwrap_or_else(|| PathBuf::from(CGROUP_ROOT))
            .join(self.name);
        log::debug!(
            "perf_event: counting {} events on cgroup {}",
            events.len(),
            path.display()
        );

        let scope = CgroupScope::new(events.clone(), path, self.cpus);
        let info = scope.info();

        Source::new(PerfSensor::new(scope), PerfProcessor::new(events, info))
    }
}

/// The events, sorted so that the metrics keep the same order.
fn ordered(events: Option<HashSet<Event>>) -> Vec<Event> {
    let Some(events) = events else {
        return Event::DEFAULT.to_vec();
    };

    let mut events: Vec<Event> = events.into_iter().collect();
    events.sort_unstable();
    events
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::*;

    fn read(table: serde_json::Value) -> serde_json::Result<PerfEvent> {
        serde_json::from_value(table)
    }

    fn names(perf: PerfEvent) -> Vec<String> {
        perf.build()
            .metrics()
            .into_iter()
            .map(|metric| metric.name)
            .collect()
    }

    #[test]
    fn an_empty_table_counts_the_default_events_on_the_program() {
        let perf = read(json!({})).unwrap();

        assert!(matches!(perf, PerfEvent::Pid(_)));
        assert_eq!(names(perf).len(), Event::DEFAULT.len());
    }

    #[test]
    fn the_events_of_the_program_are_set_without_naming_its_scope() {
        let perf = read(json!({ "events": ["instructions"] })).unwrap();

        assert!(matches!(perf, PerfEvent::Pid(_)));
        assert_eq!(names(perf), ["INSTRUCTIONS"]);
    }

    #[test]
    fn the_scope_of_the_program_can_still_be_named() {
        let perf = read(json!({ "scope": "pid", "events": ["instructions"] })).unwrap();

        assert!(matches!(perf, PerfEvent::Pid(_)));
    }

    #[test]
    fn a_cgroup_scope_reads_its_own_keys() {
        let perf = read(json!({
            "scope": "cgroup",
            "name": "my-run",
            "root": "/elsewhere",
            "cpus": [0, 1],
        }))
        .unwrap();

        let PerfEvent::Cgroup(cgroup) = perf else {
            panic!("a cgroup scope");
        };
        assert_eq!(cgroup.name, Path::new("my-run"));
        assert_eq!(cgroup.cpus, Some(HashSet::from([0, 1])));
    }

    #[test]
    fn the_program_has_no_cpus_to_count_on() {
        let error = read(json!({ "cpus": [0] })).unwrap_err();

        assert!(
            error.to_string().contains("unknown field `cpus`"),
            "{error}"
        );
    }

    #[test]
    fn a_cgroup_scope_names_its_cgroup() {
        let error = read(json!({ "scope": "cgroup" })).unwrap_err();

        assert!(
            error.to_string().contains("missing field `name`"),
            "{error}"
        );
    }

    #[test]
    fn the_events_come_out_in_the_same_order_whatever_order_they_were_given_in() {
        let given = read(json!({ "events": ["branch_misses", "instructions", "cpu_cycles"] }));

        assert_eq!(
            names(given.unwrap()),
            ["CPU_CYCLES", "INSTRUCTIONS", "BRANCH_MISSES"]
        );
    }
}
