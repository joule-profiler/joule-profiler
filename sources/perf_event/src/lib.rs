//! `perf_event` source for Joule Profiler.
//!
//! Counts hardware and software events, such as CPU cycles, instructions, cache misses or
//! the task clock time spent, using the `perf_event` Linux interface.
//!
//! # Scopes
//!
//! - [`ScopeType::Pid`] (default): the profiled process and its children. The counters are
//!   opened when it starts.
//! - [`ScopeType::Cgroup`]: every process in a cgroup v2. The counters are opened on each CPU
//!   before the process starts.
//!
//! # Groups
//!
//! By default the counters are opened as one group, but the source can be configured to not group them.
//! It can be useful in case you have more counters to open than the processor can actually handle
//! simultaneously. See [`PerfEvent::group`].
//!
//! # Permissions
//!
//! Requires `kernel.perf_event_paranoid` at most 1 for the pid scope and 0 for
//! the cgroup scope or `CAP_PERFMON` capability.
//!
//! # Example
//!
//! ```no_run
//! use joule_profiler_source_perf_event::{Cgroup, Event, PerfEvent};
//!
//! let source = PerfEvent::new()
//!     .events([Event::CpuCycles, Event::Instructions, Event::TaskClock])
//!     .scope(Cgroup::new("my.slice").cpus([0, 1]))
//!     .build().unwrap();
//! ```

use std::{
    collections::{BTreeSet, HashSet},
    path::PathBuf,
    sync::Arc,
};

use joule_profiler_core::{info::InfoValue, source::Source, util::cgroup::ROOT_PATH};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use crate::{
    processor::PerfProcessor,
    scope::{PerfScope, cgroup::CgroupScope, pid::PidScope},
    sensor::PerfSensor,
    util::{read_paranoid_level, read_pmus_list},
};

mod config;
mod counters;
mod error;
mod event;
mod processor;
mod scope;
mod sensor;
mod util;

pub use config::{Cgroup, ScopeType};
pub use error::{PerfEventError, Result};
pub use event::Event;

/// Configuration of the `perf_event` source..
#[derive(Debug, Clone)]
pub struct PerfEvent {
    events: Option<BTreeSet<Event>>,
    scope: ScopeType,
    group: bool,
}

impl Default for PerfEvent {
    fn default() -> Self {
        Self {
            events: None,
            scope: ScopeType::default(),
            group: true,
        }
    }
}

impl PerfEvent {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the events to count. The order and duplicates are ignored.
    pub fn events(mut self, events: impl IntoIterator<Item = Event>) -> Self {
        self.events = Some(events.into_iter().collect());
        self
    }

    /// Sets the scope of the counters.
    pub fn scope(mut self, scope: impl Into<ScopeType>) -> Self {
        self.scope = scope.into();
        self
    }

    /// Sets whether the counters are opened as one group. Defaults to `true`.
    ///
    /// Grouped counters are scheduled and read together, so they cover the same time. A group
    /// must fit in the hardware counters, otherwise it never runs and reports 0. Ungrouped
    /// counters are multiplexed by the kernel when needed, and their counts are scaled.
    ///
    /// A hardware event leads the group whenever there is one: two groups led by a software
    /// event on the same process are never scheduled.
    pub fn group(mut self, group: bool) -> Self {
        self.group = group;
        self
    }

    /// Builds the source.
    ///
    /// # Errors
    ///
    /// Returns [`PerfEventError::NoEvent`] if the list of events is empty.
    pub fn build(self) -> Result<Source> {
        let events: Arc<Vec<Event>> = match self.events {
            None => Event::DEFAULT.to_vec(),
            Some(events) if events.is_empty() => return Err(PerfEventError::NoEvent),
            Some(events) => events.into_iter().collect(),
        }
        .into();

        Ok(match self.scope {
            ScopeType::Pid => build_perf_source(events, self.group, PidScope::new(self.group)),
            ScopeType::Cgroup(cgroup) => {
                let path = cgroup
                    .root
                    .unwrap_or_else(|| ROOT_PATH.into())
                    .join(cgroup.name);
                build_perf_source(
                    events,
                    self.group,
                    CgroupScope::new(path, cgroup.cpus, self.group),
                )
            }
        })
    }
}

fn build_perf_source<S: PerfScope>(events: Arc<Vec<Event>>, group: bool, scope: S) -> Source {
    let info = scope
        .info()
        .with("events", InfoValue::list(events.iter()))
        .with("group", group)
        .with("pmus", read_pmus_list().map(InfoValue::list))
        .with("perf_event_paranoid", read_paranoid_level());

    Source::new(
        PerfSensor::new(Arc::clone(&events), scope),
        PerfProcessor::new(events, group),
    )
    .with_info(info)
}

/// Deserializes a table with `events`, `group` and `scope` (`pid` by default).
/// A `cgroup` scope also takes `name`, `root` and `cpus`.
impl<'de> Deserialize<'de> for PerfEvent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Default, Deserialize)]
        #[serde(rename_all = "lowercase")]
        enum Scope {
            #[default]
            Pid,
            Cgroup,
        }

        #[derive(Default, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        struct Table {
            events: Option<BTreeSet<Event>>,
            scope: Scope,
            name: Option<PathBuf>,
            root: Option<PathBuf>,
            cpus: Option<HashSet<u32>>,
            group: Option<bool>,
        }

        let table = Table::deserialize(deserializer)?;

        let scope = match table.scope {
            Scope::Pid => {
                let cgroup_keys = [
                    ("name", table.name.is_some()),
                    ("root", table.root.is_some()),
                    ("cpus", table.cpus.is_some()),
                ];
                if let Some((key, _)) = cgroup_keys.iter().find(|(_, set)| *set) {
                    return Err(D::Error::custom(format!(
                        "`{key}` needs `scope = \"cgroup\"`"
                    )));
                }
                ScopeType::Pid
            }
            Scope::Cgroup => ScopeType::Cgroup(Cgroup {
                name: table.name.ok_or_else(|| D::Error::missing_field("name"))?,
                root: table.root,
                cpus: table.cpus,
            }),
        };

        Ok(Self {
            events: table.events,
            scope,
            group: table.group.unwrap_or(true),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use joule_profiler_core::unit::MetricUnit;
    use serde_json::json;

    use super::*;

    fn read(table: serde_json::Value) -> serde_json::Result<PerfEvent> {
        serde_json::from_value(table)
    }

    fn metric_names(perf: PerfEvent) -> Vec<String> {
        perf.build()
            .unwrap()
            .metrics()
            .into_iter()
            .map(|metric| metric.name)
            .collect()
    }

    #[test]
    fn the_counters_are_grouped_unless_told_otherwise() {
        let grouped = read(json!({})).unwrap();
        let ungrouped = read(json!({ "group": false })).unwrap();

        assert!(grouped.group);
        assert!(!ungrouped.group);
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

        let ScopeType::Cgroup(cgroup) = perf.scope else {
            panic!("a cgroup scope");
        };
        assert_eq!(cgroup.name, Path::new("my-run"));
        assert_eq!(cgroup.root.as_deref(), Some(Path::new("/elsewhere")));
        assert_eq!(cgroup.cpus, Some(HashSet::from([0, 1])));
    }

    #[test]
    fn the_keys_of_a_cgroup_need_a_cgroup_scope() {
        let error = read(json!({ "cpus": [0] })).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("`cpus` needs `scope = \"cgroup\"`"),
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
    fn a_hardware_event_leads_whenever_there_is_one() {
        let perf = read(json!({ "events": ["task_clock", "branch_misses"] })).unwrap();

        assert_eq!(metric_names(perf), ["BRANCH_MISSES", "TASK_CLOCK"]);
    }

    #[test]
    fn an_empty_list_of_events_is_refused() {
        let perf = read(json!({ "events": [] })).unwrap();

        assert!(matches!(perf.build(), Err(PerfEventError::NoEvent)));
    }

    #[test]
    fn the_clocks_are_reported_in_nanoseconds_and_the_other_events_as_counts() {
        let source = PerfEvent::new()
            .events([Event::CpuCycles, Event::CpuClock, Event::TaskClock])
            .build()
            .unwrap();

        let units: Vec<MetricUnit> = source
            .metrics()
            .into_iter()
            .map(|metric| metric.unit)
            .collect();
        assert_eq!(
            units,
            [
                MetricUnit::COUNT,
                MetricUnit::NANOSECOND,
                MetricUnit::NANOSECOND
            ]
        );
    }
}
