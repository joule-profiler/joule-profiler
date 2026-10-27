use std::time::Duration;

use joule_profiler_core::source::Source;
use procfs::{Current, Meminfo};
use serde::Deserialize;

pub mod error;
pub mod processor;
pub mod sensor;

pub use error::ProcfsError;
pub use processor::ProcfsProcessor;
pub use sensor::{ProcfsSensor, Snapshot};

use crate::error::Result;

const MACHINE_POLL: Duration = Duration::from_millis(10);
// Slower than the machine: `smaps_rollup` walks the page tables of every process of the tree.
const PROCESS_POLL: Duration = Duration::from_millis(20);
const HIERARCHY: Duration = Duration::from_secs(1);

/// Memory, CPU time and I/O of the profiled process tree, from `/proc`. Both the builder and the
/// `[sources.procfs]` table.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Procfs {
    #[serde(with = "humantime_serde")]
    poll_interval: Duration,

    #[serde(with = "humantime_serde")]
    process_poll_interval: Duration,

    #[serde(with = "humantime_serde")]
    hierarchy_interval: Duration,

    global: bool,
}

impl Default for Procfs {
    fn default() -> Self {
        Self {
            poll_interval: MACHINE_POLL,
            process_poll_interval: PROCESS_POLL,
            hierarchy_interval: HIERARCHY,
            global: false,
        }
    }
}

impl Procfs {
    pub fn new() -> Self {
        Self::default()
    }

    /// How often the machine memory is polled, when `global`.
    pub fn poll(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// How often the memory of the process tree is polled.
    pub fn process_poll(mut self, interval: Duration) -> Self {
        self.process_poll_interval = interval;
        self
    }

    /// How often the process tree is listed again. A process that starts and ends in between
    /// is missed.
    pub fn hierarchy_poll(mut self, interval: Duration) -> Self {
        self.hierarchy_interval = interval;
        self
    }

    /// Also reports the memory of the machine.
    pub fn global(mut self, global: bool) -> Self {
        self.global = global;
        self
    }

    pub fn build(self) -> Result<Source> {
        let mem_total = Meminfo::current()?.mem_total;
        let ticks = procfs::ticks_per_second();
        let tick_ms = 1000 / ticks.max(1);

        log::debug!("procfs: {ticks} clock ticks a second, {mem_total} bytes of memory");

        let info = processor::info(&self, tick_ms, mem_total);
        Ok(Source::new(
            ProcfsSensor::new(
                self.poll_interval,
                self.process_poll_interval,
                self.hierarchy_interval,
                self.global,
                mem_total,
            ),
            ProcfsProcessor::new(self, tick_ms),
        )
        .with_info(info))
    }
}
