use std::fmt::{self, Display};

use perf_event::events::Hardware;
use serde::Deserialize;

/// A hardware event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    #[serde(alias = "CPU_CYCLES", alias = "cpu-cycles", alias = "cycles")]
    CpuCycles,

    #[serde(alias = "INSTRUCTIONS")]
    Instructions,

    #[serde(alias = "CACHE_REFERENCES", alias = "cache-references")]
    CacheReferences,

    #[serde(alias = "CACHE_MISSES", alias = "cache-misses")]
    CacheMisses,

    #[serde(
        alias = "BRANCH_INSTRUCTIONS",
        alias = "branch-instructions",
        alias = "branches"
    )]
    BranchInstructions,

    #[serde(alias = "BRANCH_MISSES", alias = "branch-misses")]
    BranchMisses,

    #[serde(alias = "BUS_CYCLES", alias = "bus-cycles")]
    BusCycles,

    #[serde(alias = "REF_CPU_CYCLES", alias = "ref-cycles")]
    RefCpuCycles,
}

impl Event {
    /// The events counted when none are configured.
    pub const DEFAULT: [Self; 4] = [
        Self::CpuCycles,
        Self::Instructions,
        Self::CacheMisses,
        Self::BranchMisses,
    ];
}

impl From<Event> for Hardware {
    fn from(event: Event) -> Self {
        match event {
            Event::CpuCycles => Self::CPU_CYCLES,
            Event::Instructions => Self::INSTRUCTIONS,
            Event::CacheReferences => Self::CACHE_REFERENCES,
            Event::CacheMisses => Self::CACHE_MISSES,
            Event::BranchInstructions => Self::BRANCH_INSTRUCTIONS,
            Event::BranchMisses => Self::BRANCH_MISSES,
            Event::BusCycles => Self::BUS_CYCLES,
            Event::RefCpuCycles => Self::REF_CPU_CYCLES,
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
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_slice_contains_all_default_variants() {
        assert!(Event::DEFAULT.contains(&Event::CpuCycles));
        assert!(Event::DEFAULT.contains(&Event::Instructions));
        assert!(Event::DEFAULT.contains(&Event::CacheMisses));
        assert!(Event::DEFAULT.contains(&Event::BranchMisses));
    }

    #[test]
    fn an_event_is_reported_in_capitals() {
        assert_eq!(Event::BranchMisses.to_string(), "BRANCH_MISSES");
    }
}
