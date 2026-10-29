//! `procfs` source for Joule Profiler.
//!
//! Reads the memory, I/O and CPU time of the profiled program and of every child processes,
//! from the `/proc` files of Linux, and optionally the memory of the whole machine.
//!
//! # Example
//!
//! ```no_run
//! use std::time::Duration;
//!
//! use joule_profiler_source_procfs::Procfs;
//!
//! let source = Procfs::new()
//!     .global(true)
//!     .process_poll_interval(Duration::from_millis(50))
//!     .build()
//!     .unwrap();
//! ```

use std::time::Duration;

use joule_profiler_core::source::Source;
use procfs::{Current, Meminfo};
use serde::Deserialize;

pub mod error;
mod hierarchy;
mod poller;
pub mod processor;
pub mod sensor;

pub use error::ProcfsError;
pub use processor::ProcfsProcessor;
pub use sensor::{ProcfsSensor, Snapshot};

use crate::error::Result;

const DEFAULT_MACHINE_POLL: Duration = Duration::from_millis(10);
const DEFAULT_PROCESS_POLL: Duration = Duration::from_millis(100);
const DEFAULT_HIERARCHY_POLL: Duration = Duration::from_millis(500);

/// Configuration of the procfs source.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Procfs {
    #[serde(with = "humantime_serde")]
    global_poll_interval: Duration,

    #[serde(with = "humantime_serde")]
    process_poll_interval: Duration,

    #[serde(with = "humantime_serde")]
    hierarchy_poll_interval: Duration,

    global: bool,
}

impl Default for Procfs {
    fn default() -> Self {
        Self {
            global_poll_interval: DEFAULT_MACHINE_POLL,
            process_poll_interval: DEFAULT_PROCESS_POLL,
            hierarchy_poll_interval: DEFAULT_HIERARCHY_POLL,
            global: false,
        }
    }
}

impl Procfs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets how often the memory of the machine is read, when `global` is set.
    pub fn global_poll_interval(mut self, interval: Duration) -> Self {
        self.global_poll_interval = interval;
        self
    }

    /// Sets how often the memory of the process tree is read.
    pub fn process_poll_interval(mut self, interval: Duration) -> Self {
        self.process_poll_interval = interval;
        self
    }

    /// Sets how often the process tree is listed again. A process that starts and
    /// ends in between is missed.
    pub fn hierarchy_poll_interval(mut self, interval: Duration) -> Self {
        self.hierarchy_poll_interval = interval;
        self
    }

    /// Sets whether the memory of the whole machine is reported too. Defaults to `false`.
    pub fn global(mut self, global: bool) -> Self {
        self.global = global;
        self
    }

    /// Builds the source.
    ///
    /// # Errors
    ///
    /// Returns [`ProcfsError::Procfs`] if `/proc/meminfo` cannot be read.
    pub fn build(self) -> Result<Source> {
        let mem_total = Meminfo::current()?.mem_total;
        let ticks = procfs::ticks_per_second();
        let tick_ms = 1000 / ticks.max(1);

        log::debug!("procfs: {ticks} clock ticks per second, {mem_total} bytes of memory");

        let info = processor::info(&self, tick_ms, mem_total);
        Ok(Source::new(
            ProcfsSensor::new(
                self.global_poll_interval,
                self.process_poll_interval,
                self.hierarchy_poll_interval,
                self.global,
                mem_total,
            ),
            ProcfsProcessor::new(self, tick_ms),
        )
        .with_info(info))
    }
}
