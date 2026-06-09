use std::fmt::Display;

use perf_event::events::Hardware;
use serde::Deserialize;

/// Hardware performance counter event types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum Event {
    #[serde(alias = "CPU_CYCLES", alias = "cpu-cycles")]
    CpuCycles,

    #[serde(alias = "INSTRUCTIONS", alias = "instructions")]
    Instructions,

    #[serde(alias = "CACHE_MISSES", alias = "cache-misses")]
    CacheMisses,

    #[serde(alias = "BRANCH_MISSES", alias = "branch-misses")]
    BranchMisses,
}

/// Default hardware events to be used in `perf_event`.
pub static EVENTS: &[Event] = &[
    Event::CpuCycles,
    Event::Instructions,
    Event::CacheMisses,
    Event::BranchMisses,
];

impl From<Event> for Hardware {
    fn from(event: Event) -> Self {
        match event {
            Event::CpuCycles => Hardware::CPU_CYCLES,
            Event::Instructions => Hardware::INSTRUCTIONS,
            Event::CacheMisses => Hardware::CACHE_MISSES,
            Event::BranchMisses => Hardware::BRANCH_MISSES,
        }
    }
}

impl Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Event::CpuCycles => "CPU_CYCLES",
            Event::Instructions => "INSTRUCTIONS",
            Event::CacheMisses => "CACHE_MISSES",
            Event::BranchMisses => "BRANCH_MISSES",
        })
    }
}

impl From<Event> for String {
    fn from(event: Event) -> Self {
        match event {
            Event::CpuCycles => "CPU_CYCLES".into(),
            Event::Instructions => "INSTRUCTIONS".into(),
            Event::CacheMisses => "CACHE_MISSES".into(),
            Event::BranchMisses => "BRANCH_MISSES".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_slice_contains_all_variants() {
        assert!(EVENTS.contains(&Event::CpuCycles));
        assert!(EVENTS.contains(&Event::Instructions));
        assert!(EVENTS.contains(&Event::CacheMisses));
        assert!(EVENTS.contains(&Event::BranchMisses));
    }
}
