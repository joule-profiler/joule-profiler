//! Procfs metric source for Joule Profiler.
//!
//! Provides memory and I/O metrics for a process and it's children and system-wide,
//! by reading Linux's `/proc` filesystem via the `procfs` crate.

use futures::StreamExt;
use joule_profiler_core::{
    sensor::{Sensor, Sensors},
    source::MetricReader,
    types::{Metric, Metrics},
    unit::{MetricUnit, Unit, UnitPrefix},
};
use log::{debug, trace};
use procfs::{Current, FromRead, Meminfo};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_timerfd::Interval;

use crate::{
    config::ProcfsConfig,
    counters::{Counters, MinMax},
    error::ProcfsError,
    snapshot::{ProcSnapshot, measure_global, measure_proc},
    utils::make_conversion,
};

pub mod config;
pub mod counters;
pub mod error;
mod snapshot;
mod utils;

const IO_COUNTERS_METRIC_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::None,
    unit: Unit::Byte,
};

type Result<T> = std::result::Result<T, ProcfsError>;

/// Procfs-based metric source.
///
/// Reads memory and I/O metrics from `/proc` for a target process and its
/// entire child hierarchy, as well as system-wide memory statistics.
///
/// Metrics are accumulated as min/max over each measurement phase.
/// A phase starts on [`MetricReader::measure`] and ends on [`MetricReader::retrieve`],
/// which returns the counters and resets them for the next phase.
///
/// If a `poll_interval` is configured, a background task polls `/proc` continuously
/// at that interval in addition to explicit [`MetricReader::measure`] calls.
///
/// # Example
///
/// ```no_run
/// use joule_profiler_source_procfs::{Procfs, config::ProcfsConfig};
///
/// let source = Procfs::new(ProcfsConfig::default()).unwrap();
/// ```
#[derive(Debug, Default)]
pub struct Procfs {
    /// procfs source configuration.
    config: ProcfsConfig,

    /// pid of the profiled process, initialized at -1.
    pid: i32,

    /// Current metrics counters.
    counters: Arc<Mutex<Counters>>,

    /// The handle to the polling task.
    handle: Option<JoinHandle<Result<()>>>,

    /// Total physical memory in bytes, read once at construction from `/proc/meminfo`.
    mem_total: u64,
}

impl Procfs {
    /// Creates a new `Procfs` source from the given configuration.
    ///
    /// Reads `MemTotal` from `/proc/meminfo` at construction time.
    pub fn new(config: ProcfsConfig) -> Result<Self> {
        let mem_total = Meminfo::from_file(Meminfo::PATH)?.mem_total;
        Ok(Self {
            config,
            pid: -1,
            mem_total,
            ..Default::default()
        })
    }
}

impl MetricReader for Procfs {
    type Type = Counters;
    type Error = ProcfsError;

    /// Initializes the source to `pid` and starts the background poller if a `poll_interval` is configured.
    async fn init(&mut self, pid: i32) -> Result<()> {
        self.pid = pid;

        let Some(interval) = self.config.poll_interval else {
            return Ok(());
        };

        let counters = self.counters.clone();

        let handle = tokio::spawn(async move {
            debug!("Starting procfs polling.");
            let mut ticker = Interval::new_interval(interval)?;

            loop {
                ticker.next().await;
                trace!("procfs source poll.");

                let proc = match measure_proc(pid) {
                    Err(ProcfsError::Procfs(procfs::ProcError::NotFound(_))) => {
                        Ok(ProcSnapshot::default())
                    }
                    r => r,
                }?;

                let global = measure_global()?;

                let mut counters = counters.lock().await;
                counters.update(&proc, &global);
            }
        });

        self.handle = Some(handle);
        Ok(())
    }

    /// Aborts the background poller if still running, or propagates its error if it already finished.
    async fn join(&mut self) -> Result<()> {
        if let Some(handle) = self.handle.take() {
            if handle.is_finished() {
                trace!("Joining polling task.");
                if let Ok(res) = handle.await {
                    return res;
                }
            } else {
                trace!("Aborting polling task.");
                handle.abort();
            }
        }
        Ok(())
    }

    /// Takes a single snapshot of process and global counters and updates them.
    ///
    /// If the process is not found, an empty snapshot is used.
    async fn measure(&mut self) -> Result<()> {
        let proc = match measure_proc(self.pid) {
            Err(ProcfsError::Procfs(procfs::ProcError::NotFound(_))) => Ok(ProcSnapshot::default()),
            r => r,
        }?;

        let global = measure_global()?;

        let mut counters = self.counters.lock().await;
        counters.update(&proc, &global);

        Ok(())
    }

    /// Returns the accumulated counters for the current phase and resets them.
    async fn retrieve(&mut self) -> Result<Self::Type> {
        let mut lock = self.counters.lock().await;
        let counters = *lock;
        lock.reset();
        Ok(counters)
    }

    fn get_sensors(&self) -> Result<Sensors> {
        let proc_memory_unit: MetricUnit = self.config.proc_memory_unit.into();
        let global_memory_unit: MetricUnit = self.config.global_memory_unit.into();

        let proc_sensors = [
            "proc_vm_size_min",
            "proc_vm_size_max",
            "proc_rss_min",
            "proc_rss_max",
            "proc_pss_min",
            "proc_pss_max",
            "proc_shared_min",
            "proc_shared_max",
            "proc_anon_min",
            "proc_anon_max",
        ]
        .into_iter()
        .map(|name| Sensor::new(name, proc_memory_unit, Self::get_name()));

        let io_sensors = vec![
            Sensor::new(
                "proc_io_read_bytes",
                IO_COUNTERS_METRIC_UNIT,
                Self::get_name(),
            ),
            Sensor::new(
                "proc_io_write_bytes",
                IO_COUNTERS_METRIC_UNIT,
                Self::get_name(),
            ),
        ];

        let global_sensors = [
            "global_mem_used_min",
            "global_mem_used_max",
            "global_cached_min",
            "global_cached_max",
            "global_anon_min",
            "global_anon_max",
            "global_swap_free_min",
            "global_swap_free_max",
        ]
        .into_iter()
        .map(|name| Sensor::new(name, global_memory_unit, Self::get_name()));

        Ok(proc_sensors
            .chain(global_sensors)
            .chain(io_sensors)
            .collect())
    }

    fn to_metrics(&self, mut counters: Self::Type) -> Result<Metrics> {
        let proc_unit: MetricUnit = self.config.proc_memory_unit.into();
        let global_unit: MetricUnit = self.config.global_memory_unit.into();

        counters.remove_sentinels();
        let proc = counters.proc;

        let proc_memory_metrics: Metrics = [
            ("proc_vm_size_min", proc.vm_size.min()),
            ("proc_vm_size_max", proc.vm_size.max()),
            ("proc_rss_min", proc.rss.min()),
            ("proc_rss_max", proc.rss.max()),
            ("proc_pss_min", proc.pss.min()),
            ("proc_pss_max", proc.pss.max()),
            ("proc_shared_min", proc.shared.min()),
            ("proc_shared_max", proc.shared.max()),
            ("proc_anon_min", proc.anon.min()),
            ("proc_anon_max", proc.anon.max()),
        ]
        .into_iter()
        .map(|(name, value)| {
            let value = make_conversion(self.config.proc_memory_unit, value);
            Metric::new(name, value, proc_unit, Self::get_name())
        })
        .collect();

        let io_metrics: Metrics = [
            (
                "proc_io_read_bytes",
                proc.end_read_bytes.saturating_sub(proc.begin_read_bytes),
            ),
            (
                "proc_io_write_bytes",
                proc.end_write_bytes.saturating_sub(proc.begin_write_bytes),
            ),
        ]
        .into_iter()
        .map(|(name, value)| Metric::new(name, value, IO_COUNTERS_METRIC_UNIT, Self::get_name()))
        .collect();

        let global = counters.global;
        let mem_used = compute_mem_used(
            self.mem_total,
            global.mem_available,
            global.mem_free,
            global.cached,
        );

        let global_memory_unit = self.config.global_memory_unit;

        let mut global_memory_metrics: Metrics = [
            (
                "global_mem_used_min",
                make_conversion(global_memory_unit, mem_used.min()),
            ),
            (
                "global_mem_used_max",
                make_conversion(global_memory_unit, mem_used.max()),
            ),
            (
                "global_cached_min",
                make_conversion(global_memory_unit, global.cached.min()),
            ),
            (
                "global_cached_max",
                make_conversion(global_memory_unit, global.cached.max()),
            ),
            (
                "global_swap_free_min",
                make_conversion(global_memory_unit, global.swap_free.min()),
            ),
            (
                "global_swap_free_max",
                make_conversion(global_memory_unit, global.swap_free.max()),
            ),
        ]
        .into_iter()
        .map(|(name, value)| Metric::new(name, value, global_unit, Self::get_name()))
        .collect();

        if let Some(anon) = global.anon {
            let anon: Vec<_> = [("global_anon_max", anon.0), ("global_anon_min", anon.1)]
                .into_iter()
                .map(|(name, value)| {
                    let value = make_conversion(global_memory_unit, value);
                    Metric::new(name, value, global_unit, Self::get_name())
                })
                .collect();
            global_memory_metrics.extend(anon);
        }

        Ok(proc_memory_metrics
            .into_iter()
            .chain(global_memory_metrics)
            .chain(io_metrics)
            .collect())
    }

    fn get_name() -> &'static str {
        "procfs"
    }
}

/// Computes used memory as `MemTotal - MemAvailable`.
///
/// If `MemAvailable` is present in `/proc/meminfo`, it is used directly (preferred).
/// Otherwise, falls back to `MemTotal - (MemFree + Cached)`, which is less accurate
/// but universally available.
///
/// Note: min/max are inverted relative to `available` since higher availability
/// means lower usage.
fn compute_mem_used(
    mem_total: u64,
    mem_available: Option<MinMax>,
    mem_free: MinMax,
    cached: MinMax,
) -> MinMax {
    if let Some(available) = mem_available {
        MinMax(
            mem_total.saturating_sub(available.max()),
            mem_total.saturating_sub(available.min()),
        )
    } else {
        MinMax(
            mem_total.saturating_sub(mem_free.max() + cached.max()),
            mem_total.saturating_sub(mem_free.min() + cached.min()),
        )
    }
}
