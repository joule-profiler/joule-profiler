use std::fmt::{self, Display};

use joule_profiler_core::unit::MetricUnit;
use perf_event::Builder;
use perf_event::events::{Hardware, Software};
use serde::Deserialize;

/// A hardware or software performance event.
///
/// Events are deserialized from their snake case name (`cpu_cycles`), their `perf` name
/// (`cycles`, `cpu-cycles`) or their uppercase name (`CPU_CYCLES`). Metrics are named after the
/// uppercase name.
///
/// Software events are counted by the kernel and always available. Hardware events need a PMU
/// that supports them, which only has a few counters (typically 4 to 8 per core). To count more
/// events than that, disable [`group`](crate::PerfEvent::group) so the kernel multiplexes them.
///
/// Events are sorted in declaration order. This sets the order of the metrics and the group
/// leader, which is why hardware events come first. See
/// [`PerfEvent::group`](crate::PerfEvent::group).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    /// Total cycles. Affected by CPU frequency scaling.
    #[serde(alias = "CPU_CYCLES", alias = "cpu-cycles", alias = "cycles")]
    CpuCycles,

    /// Retired instructions.
    #[serde(alias = "INSTRUCTIONS")]
    Instructions,

    /// Cache accesses, usually to the last level cache.
    #[serde(alias = "CACHE_REFERENCES", alias = "cache-references")]
    CacheReferences,

    /// Cache misses, usually in the last level cache.
    #[serde(alias = "CACHE_MISSES", alias = "cache-misses")]
    CacheMisses,

    /// Retired branch instructions.
    #[serde(
        alias = "BRANCH_INSTRUCTIONS",
        alias = "branch-instructions",
        alias = "branches"
    )]
    BranchInstructions,

    /// Mispredicted branch instructions.
    #[serde(alias = "BRANCH_MISSES", alias = "branch-misses")]
    BranchMisses,

    /// Bus cycles, which can differ from total cycles.
    #[serde(alias = "BUS_CYCLES", alias = "bus-cycles")]
    BusCycles,

    /// Total cycles, not affected by CPU frequency scaling.
    #[serde(alias = "REF_CPU_CYCLES", alias = "ref-cycles")]
    RefCpuCycles,

    /// Time measured by the per-CPU high-resolution timer, in nanoseconds.
    #[serde(alias = "CPU_CLOCK", alias = "cpu-clock")]
    CpuClock,

    /// Time spent on a CPU by the observed tasks, in nanoseconds.
    #[serde(alias = "TASK_CLOCK", alias = "task-clock")]
    TaskClock,

    /// Page faults.
    #[serde(alias = "PAGE_FAULTS", alias = "page-faults", alias = "faults")]
    PageFaults,

    /// Context switches.
    #[serde(alias = "CONTEXT_SWITCHES", alias = "context-switches", alias = "cs")]
    ContextSwitches,

    /// Migrations of a task to another CPU.
    #[serde(
        alias = "CPU_MIGRATIONS",
        alias = "cpu-migrations",
        alias = "migrations"
    )]
    CpuMigrations,

    /// Page faults served without disk I/O.
    #[serde(alias = "MINOR_FAULTS", alias = "minor-faults")]
    MinorFaults,

    /// Page faults that required disk I/O.
    #[serde(alias = "MAJOR_FAULTS", alias = "major-faults")]
    MajorFaults,

    /// Unaligned memory accesses handled by the kernel. Never occurs on x86.
    #[serde(alias = "ALIGNMENT_FAULTS", alias = "alignment-faults")]
    AlignmentFaults,

    /// Unimplemented instructions emulated by the kernel.
    #[serde(alias = "EMULATION_FAULTS", alias = "emulation-faults")]
    EmulationFaults,

    /// Context switches to a task in another cgroup. Requires Linux 5.13 or later.
    #[serde(alias = "CGROUP_SWITCHES", alias = "cgroup-switches")]
    CgroupSwitches,
}

impl Event {
    /// Events counted when none are configured.
    pub const DEFAULT: [Self; 4] = [
        Self::CpuCycles,
        Self::Instructions,
        Self::CacheMisses,
        Self::BranchMisses,
    ];

    /// Returns the unit of the metric reported for this event:
    /// nanoseconds for the clocks and count otherwise.
    pub(crate) fn unit(self) -> MetricUnit {
        match self {
            Event::CpuClock | Event::TaskClock => MetricUnit::NANOSECOND,
            _ => MetricUnit::COUNT,
        }
    }

    /// Returns a builder for a counter of this event.
    pub(crate) fn builder(self) -> Builder<'static> {
        macro_rules! event_builder {
            ($event:expr) => {
                Builder::new($event)
            };
        }
        match self {
            Self::CpuCycles => event_builder!(Hardware::CPU_CYCLES),
            Self::Instructions => event_builder!(Hardware::INSTRUCTIONS),
            Self::CacheReferences => event_builder!(Hardware::CACHE_REFERENCES),
            Self::CacheMisses => event_builder!(Hardware::CACHE_MISSES),
            Self::BranchInstructions => event_builder!(Hardware::BRANCH_INSTRUCTIONS),
            Self::BranchMisses => event_builder!(Hardware::BRANCH_MISSES),
            Self::BusCycles => event_builder!(Hardware::BUS_CYCLES),
            Self::RefCpuCycles => event_builder!(Hardware::REF_CPU_CYCLES),
            Self::CpuClock => event_builder!(Software::CPU_CLOCK),
            Self::TaskClock => event_builder!(Software::TASK_CLOCK),
            Self::PageFaults => event_builder!(Software::PAGE_FAULTS),
            Self::ContextSwitches => event_builder!(Software::CONTEXT_SWITCHES),
            Self::CpuMigrations => event_builder!(Software::CPU_MIGRATIONS),
            Self::MinorFaults => event_builder!(Software::PAGE_FAULTS_MIN),
            Self::MajorFaults => event_builder!(Software::PAGE_FAULTS_MAJ),
            Self::AlignmentFaults => event_builder!(Software::ALIGNMENT_FAULTS),
            Self::EmulationFaults => event_builder!(Software::EMULATION_FAULTS),
            Self::CgroupSwitches => event_builder!(Software::CGROUP_SWITCHES),
        }
    }
}

impl Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::CpuCycles => "CPU_CYCLES",
            Self::Instructions => "INSTRUCTIONS",
            Self::CacheReferences => "CACHE_REFERENCES",
            Self::CacheMisses => "CACHE_MISSES",
            Self::BranchInstructions => "BRANCH_INSTRUCTIONS",
            Self::BranchMisses => "BRANCH_MISSES",
            Self::BusCycles => "BUS_CYCLES",
            Self::RefCpuCycles => "REF_CPU_CYCLES",
            Self::CpuClock => "CPU_CLOCK",
            Self::TaskClock => "TASK_CLOCK",
            Self::PageFaults => "PAGE_FAULTS",
            Self::ContextSwitches => "CONTEXT_SWITCHES",
            Self::CpuMigrations => "CPU_MIGRATIONS",
            Self::MinorFaults => "MINOR_FAULTS",
            Self::MajorFaults => "MAJOR_FAULTS",
            Self::AlignmentFaults => "ALIGNMENT_FAULTS",
            Self::EmulationFaults => "EMULATION_FAULTS",
            Self::CgroupSwitches => "CGROUP_SWITCHES",
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const ALL: [Event; 18] = [
        Event::CpuCycles,
        Event::Instructions,
        Event::CacheReferences,
        Event::CacheMisses,
        Event::BranchInstructions,
        Event::BranchMisses,
        Event::BusCycles,
        Event::RefCpuCycles,
        Event::CpuClock,
        Event::TaskClock,
        Event::PageFaults,
        Event::ContextSwitches,
        Event::CpuMigrations,
        Event::MinorFaults,
        Event::MajorFaults,
        Event::AlignmentFaults,
        Event::EmulationFaults,
        Event::CgroupSwitches,
    ];

    fn read(name: &str) -> serde_json::Result<Event> {
        serde_json::from_value(json!(name))
    }

    #[test]
    fn an_event_is_read_from_its_snake_case_perf_or_uppercase_name() {
        for name in ["cpu_cycles", "cycles", "cpu-cycles", "CPU_CYCLES"] {
            assert_eq!(read(name).unwrap(), Event::CpuCycles, "{name}");
        }
    }

    #[test]
    fn every_event_is_read_back_from_its_metric_name() {
        for event in ALL {
            assert_eq!(read(&event.to_string()).unwrap(), event);
        }
    }

    #[test]
    fn an_unknown_event_is_refused() {
        assert!(read("cycle").is_err());
    }
}
