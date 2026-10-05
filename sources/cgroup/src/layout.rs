use joule_profiler_core::metric::MetricInfo;
use joule_profiler_core::unit::MetricUnit;
use joule_profiler_core::util::cgroup::Cgroup;

use crate::cgroup::{COUNTERS, MEMORY, Usage};
use crate::error::Result;

/// The values the cgroup reports, which depend on its controllers.
pub(crate) struct Layout {
    pub(crate) memory: [bool; MEMORY.len()],
    pub(crate) counters: [bool; COUNTERS.len()],

    pub(crate) global: bool,
}

impl Layout {
    /// The values the cgroup has.
    pub(crate) fn probe(cgroup: &Cgroup, global: bool) -> Result<Self> {
        Ok(Self {
            memory: cgroup.memory_usage()?.map(|value| value.is_some()),
            counters: cgroup.counters()?.map(|value| value.is_some()),
            global,
        })
    }

    /// The values a cgroup with these controllers will have.
    pub(crate) fn expected(controllers: &[String], global: bool) -> Self {
        let has = |controller: &str| controllers.iter().any(|name| name == controller);

        Self {
            memory: [has("memory"); MEMORY.len()],
            counters: COUNTERS.map(|(name, _)| match name {
                // In every `cpu.stat`.
                "usage_usec" | "user_usec" | "system_usec" => true,
                "read_bytes" | "write_bytes" => has("io"),
                _ => has("cpu"),
            }),
            global,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        !self
            .memory
            .iter()
            .chain(&self.counters)
            .any(|reported| *reported)
    }

    /// The metrics, in the order the processor writes them.
    pub(crate) fn metrics(&self) -> Vec<MetricInfo> {
        let mut metrics = Vec::new();

        for scope in self.scopes() {
            for (name, _) in self.memory() {
                metrics.extend(
                    ["min", "max", "avg"].map(|end| {
                        MetricInfo::new(format!("{scope}_{name}_{end}"), MetricUnit::BYTE)
                    }),
                );
            }

            for ((name, unit), _) in self.counters() {
                metrics.push(MetricInfo::new(format!("{scope}_{name}"), unit));
            }

            metrics.push(MetricInfo::new(
                format!("{scope}_cpu_usage"),
                MetricUnit::PERCENT,
            ));
        }

        metrics
    }

    fn scopes(&self) -> &'static [&'static str] {
        if self.global {
            &["proc", "global"]
        } else {
            &["proc"]
        }
    }

    /// The reported memory values, with their index.
    pub(crate) fn memory(&self) -> impl Iterator<Item = (&'static str, usize)> + '_ {
        MEMORY
            .iter()
            .enumerate()
            .filter(|(index, _)| self.memory[*index])
            .map(|(index, name)| (*name, index))
    }

    /// The reported counters, with their index.
    pub(crate) fn counters(
        &self,
    ) -> impl Iterator<Item = ((&'static str, MetricUnit), usize)> + '_ {
        COUNTERS
            .iter()
            .enumerate()
            .filter(|(index, _)| self.counters[*index])
            .map(|(index, counter)| (*counter, index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controllers(names: &[&str]) -> Vec<String> {
        names.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn a_cgroup_without_controllers_is_expected_to_report_its_cpu_time_only() {
        let layout = Layout::expected(&[], false);

        let names: Vec<String> = layout.metrics().into_iter().map(|m| m.name).collect();

        assert_eq!(
            names,
            [
                "proc_usage_usec",
                "proc_user_usec",
                "proc_system_usec",
                "proc_cpu_usage"
            ]
        );
    }

    #[test]
    fn each_controller_brings_its_values() {
        let layout = Layout::expected(&controllers(&["cpu", "memory", "io"]), false);

        assert!(layout.memory.iter().all(|reported| *reported));
        assert!(layout.counters.iter().all(|reported| *reported));
    }
}
