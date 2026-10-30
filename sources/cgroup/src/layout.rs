use joule_profiler_core::{MemoryUnit, MetricInfo, MetricUnit, util::cgroup::Cgroup};

use crate::{
    cgroup::{CPU_COUNTERS, IO_COUNTERS, MEMORY, Usage},
    error::Result,
};

/// The values a cgroup reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AvailableValues {
    pub(crate) memory: [bool; MEMORY.len()],
    pub(crate) cpu: [bool; CPU_COUNTERS.len()],
    pub(crate) io: [bool; IO_COUNTERS.len()],
}

impl AvailableValues {
    /// Returns the values the cgroup really reports.
    pub(crate) fn probe(cgroup: &Cgroup) -> Result<Self> {
        Ok(Self {
            memory: cgroup.read_memory_usage()?.map(|value| value.is_some()),
            cpu: cgroup.read_cpu_counters()?.map(|value| value.is_some()),
            io: cgroup.read_io_counters()?.map(|value| value.is_some()),
        })
    }

    /// Returns the values a cgroup with these controllers will have.
    pub(crate) fn expected(controllers: &[String]) -> Self {
        let has = |controller: &str| controllers.iter().any(|name| name == controller);

        Self {
            memory: [has("memory"); MEMORY.len()],
            cpu: CPU_COUNTERS.map(|(name, _)| {
                matches!(name, "usage_usec" | "user_usec" | "system_usec") || has("cpu")
            }),
            io: [has("io"); IO_COUNTERS.len()],
        }
    }

    /// Returns the available memory values, with their index.
    pub(crate) fn memory(&self) -> impl Iterator<Item = (&'static str, usize)> + '_ {
        MEMORY
            .iter()
            .enumerate()
            .filter(|(index, _)| self.memory[*index])
            .map(|(index, name)| (*name, index))
    }

    /// Returns the available counters of `cpu.stat`, with their index.
    pub(crate) fn cpu_counters(
        &self,
    ) -> impl Iterator<Item = ((&'static str, MetricUnit), usize)> + '_ {
        available_counters(&CPU_COUNTERS, &self.cpu)
    }

    /// Returns the available counters of `io.stat`, with their index.
    pub(crate) fn io_counters(
        &self,
    ) -> impl Iterator<Item = ((&'static str, MetricUnit), usize)> + '_ {
        available_counters(&IO_COUNTERS, &self.io)
    }

    /// Adds the metrics of these values, named after `scope`, with the memory in `memory_unit`.
    fn add_metrics(&self, scope: &str, memory_unit: MemoryUnit, metrics: &mut Vec<MetricInfo>) {
        for (name, _) in self.memory() {
            metrics.extend(["min", "max", "avg"].map(|end| {
                MetricInfo::new(
                    format!("{scope}_{name}_{end}"),
                    MetricUnit::from(memory_unit),
                )
            }));
        }

        for ((name, unit), _) in self.cpu_counters().chain(self.io_counters()) {
            metrics.push(MetricInfo::new(format!("{scope}_{name}"), unit));
        }

        metrics.push(MetricInfo::new(
            format!("{scope}_cpu_usage"),
            MetricUnit::PERCENT,
        ));
    }
}

/// Returns the counters whose flag is set, with their index.
fn available_counters<'a>(
    counters: &'a [(&'static str, MetricUnit)],
    flags: &'a [bool],
) -> impl Iterator<Item = ((&'static str, MetricUnit), usize)> + 'a {
    counters
        .iter()
        .zip(flags)
        .enumerate()
        .filter(|(_, (_, available))| **available)
        .map(|(index, (counter, _))| (*counter, index))
}

/// The values the source reports: those of the cgroup and those of the root cgroup when
/// `global`.
pub(crate) struct Layout {
    pub(crate) proc: AvailableValues,
    pub(crate) global: Option<AvailableValues>,
    pub(crate) process_memory_unit: MemoryUnit,
    pub(crate) global_memory_unit: MemoryUnit,
}

impl Layout {
    /// Returns the metrics, in the order the processor writes them.
    pub(crate) fn metrics(&self) -> Vec<MetricInfo> {
        let mut metrics = Vec::new();

        self.proc
            .add_metrics("proc", self.process_memory_unit, &mut metrics);
        if let Some(global) = &self.global {
            global.add_metrics("global", self.global_memory_unit, &mut metrics);
        }

        metrics
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controllers(names: &[&str]) -> Vec<String> {
        names.iter().map(ToString::to_string).collect()
    }

    fn names(layout: &Layout) -> Vec<String> {
        layout
            .metrics()
            .into_iter()
            .map(|metric| metric.name)
            .collect()
    }

    #[test]
    fn no_controllers_reports_cpu_time() {
        let layout = Layout {
            proc: AvailableValues::expected(&[]),
            global: None,
            process_memory_unit: MemoryUnit::default(),
            global_memory_unit: MemoryUnit::default(),
        };

        assert_eq!(
            names(&layout),
            [
                "proc_usage_usec",
                "proc_user_usec",
                "proc_system_usec",
                "proc_cpu_usage"
            ]
        );
    }

    #[test]
    fn controllers_bring_values() {
        let values = AvailableValues::expected(&controllers(&["cpu", "memory", "io"]));

        assert!(values.memory.iter().all(|available| *available));
        assert!(values.cpu.iter().all(|available| *available));
        assert!(values.io.iter().all(|available| *available));
    }

    #[test]
    fn root_declares_available_values() {
        let mut root = AvailableValues::expected(&controllers(&["cpu", "memory", "io"]));
        root.memory[0] = false;

        let layout = Layout {
            proc: AvailableValues::expected(&controllers(&["cpu", "memory", "io"])),
            global: Some(root),
            process_memory_unit: MemoryUnit::default(),
            global_memory_unit: MemoryUnit::default(),
        };

        let names = names(&layout);
        assert!(names.iter().any(|name| name == "proc_current_max"));
        assert!(!names.iter().any(|name| name == "global_current_max"));
        assert!(names.iter().any(|name| name == "global_anon_max"));
    }
}
