use futures::StreamExt;
use joule_profiler_core::{
    sensor::{Sensor, Sensors},
    source::MetricReader,
    types::{Metric, MetricValue, Metrics},
    unit::{MetricUnit, Unit, UnitPrefix},
};
use log::{debug, trace, warn};
use procfs::{Current, FromRead, process::Process};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_timerfd::Interval;

use crate::{counters::Counters, utils::collect_all_children};

pub mod counters;
mod utils;

const IO_COUNTERS_METRIC_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::None,
    unit: Unit::Byte,
};

#[derive(Error, Debug)]
pub enum ProcfsError {
    #[error("Procfs not initialized, call init first")]
    NotInitialized,
    #[error(transparent)]
    Procfs(#[from] procfs::ProcError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MemoryUnit {
    Bytes,
    Kilo,
    #[default]
    Mega,
    Giga,
}

impl From<MemoryUnit> for MetricUnit {
    fn from(unit: MemoryUnit) -> Self {
        match unit {
            MemoryUnit::Bytes => MetricUnit {
                prefix: UnitPrefix::None,
                unit: Unit::Byte,
            },
            MemoryUnit::Kilo => MetricUnit {
                prefix: UnitPrefix::Kilo,
                unit: Unit::Byte,
            },
            MemoryUnit::Mega => MetricUnit {
                prefix: UnitPrefix::Mega,
                unit: Unit::Byte,
            },
            MemoryUnit::Giga => MetricUnit {
                prefix: UnitPrefix::Giga,
                unit: Unit::Byte,
            },
        }
    }
}

type Result<T> = std::result::Result<T, ProcfsError>;

pub struct MemorySnapshot {
    proc_vm_size: u64,
    proc_rss: u64,
    proc_pss: u64,
    proc_shared: u64,
    proc_anon: u64,

    global_mem_available: Option<u64>,
    global_mem_free: u64,
    global_cached: u64,
    global_anon: Option<u64>,
    global_swap_free: u64,
}

#[derive(Debug, Default)]
pub struct Procfs {
    pid: i32,
    poll_interval: Option<Duration>,
    counters: Arc<Mutex<Counters>>,
    handle: Option<JoinHandle<Result<()>>>,
    proc_memory_unit: MemoryUnit,
    global_memory_unit: MemoryUnit,
}

fn read_proc_memory(process: &Process) -> Result<(u64, u64, u64, u64, u64)> {
    let vm_size = process.stat()?.vsize;
    trace!("Querying process {} smaps_rollup.", process.pid);
    let smaps = process.smaps_rollup()?;

    let (rss, pss, anon, shared) = smaps
        .memory_map_rollup
        .iter()
        .next()
        .map(|entry| {
            let m = &entry.extension.map;
            let rss = m.get("Rss").copied().unwrap_or(0);
            let pss = m.get("Pss").copied().unwrap_or(0);
            let anon = m.get("Anonymous").copied().unwrap_or(0);
            let shared = m.get("Shared_Clean").copied().unwrap_or(0)
                + m.get("Shared_Dirty").copied().unwrap_or(0);
            (rss, pss, anon, shared)
        })
        .unwrap_or_default();

    Ok((vm_size, rss, pss, shared, anon))
}

fn read_global_memory() -> Result<(Option<u64>, u64, u64, Option<u64>, u64)> {
    let meminfo = procfs::Meminfo::from_file(procfs::Meminfo::PATH)?;
    trace!("Querying global meminfo from {}", procfs::Meminfo::PATH);

    Ok((
        meminfo.mem_available,
        meminfo.mem_free,
        meminfo.cached,
        meminfo.anon_pages,
        meminfo.swap_free,
    ))
}

fn read_io(process: &Process) -> Result<(u64, u64)> {
    trace!("Querying process {} io.", process.pid);
    let io = process.io()?;
    Ok((io.rchar, io.wchar))
}

impl Procfs {
    pub fn new(poll_interval: Option<Duration>) -> Self {
        Self {
            poll_interval,
            global_memory_unit: MemoryUnit::Giga,
            ..Default::default()
        }
    }
}

fn read_memory_counters(pids: &[i32]) -> Result<MemorySnapshot> {
    let (proc_vm_size, proc_rss, proc_pss, proc_shared, proc_anon) = pids
        .iter()
        .filter_map(|p| Process::new(*p).ok())
        .filter_map(|p| read_proc_memory(&p).ok())
        .fold((0, 0, 0, 0, 0), |acc, (vm, r, ps, sh, an)| {
            (acc.0 + vm, acc.1 + r, acc.2 + ps, acc.3 + sh, acc.4 + an)
        });
    trace!(
        "vm_size: {proc_vm_size}, rss: {proc_rss}, pss: {proc_pss}, shared: {proc_shared}, anon: {proc_anon}."
    );

    let (global_mem_available, global_mem_free, global_cached, global_anon, global_swap_free) =
        read_global_memory()?;
    trace!(
        "mem_available: {global_mem_available:?}, mem_free: {global_mem_free}, cached: {global_cached}, anon: {global_anon:?}, swap_free: {global_swap_free}."
    );

    Ok(MemorySnapshot {
        proc_vm_size,
        proc_rss,
        proc_pss,
        proc_shared,
        proc_anon,
        global_mem_available,
        global_mem_free,
        global_cached,
        global_anon,
        global_swap_free,
    })
}

impl MetricReader for Procfs {
    type Type = Counters;
    type Error = ProcfsError;

    async fn init(&mut self, pid: i32) -> Result<()> {
        self.pid = pid;

        let Some(interval) = self.poll_interval else {
            return Ok(());
        };

        let current_counters = self.counters.clone();

        let handle = tokio::spawn(async move {
            debug!("Starting procfs polling.");
            let mut ticker = Interval::new_interval(interval)?;

            loop {
                ticker.next().await;

                let pids = collect_all_children(pid);
                trace!("Found pids {pids:?}.");

                let snapshot = read_memory_counters(&pids)?;

                let (read_bytes, write_bytes) = pids
                    .into_iter()
                    .filter_map(|p| Process::new(p).ok())
                    .filter_map(|p| read_io(&p).ok())
                    .fold((0, 0), |acc, (r, w)| (acc.0 + r, acc.1 + w));

                let mut counters = current_counters.lock().await;

                counters.memory_counters.update_proc(
                    snapshot.proc_vm_size,
                    snapshot.proc_rss,
                    snapshot.proc_pss,
                    snapshot.proc_shared,
                    snapshot.proc_anon,
                );
                counters.memory_counters.update_global(
                    snapshot.global_mem_available,
                    snapshot.global_mem_free,
                    snapshot.global_cached,
                    snapshot.global_anon,
                    snapshot.global_swap_free,
                );

                counters.io_counters.end_read_bytes =
                    counters.io_counters.end_read_bytes.max(read_bytes);
                counters.io_counters.end_write_bytes =
                    counters.io_counters.end_write_bytes.max(write_bytes);
            }
        });

        self.handle = Some(handle);
        Ok(())
    }

    async fn join(&mut self) -> Result<()> {
        if let Some(handle) = self.handle.take() {
            if handle.is_finished() {
                if let Ok(res) = handle.await {
                    return res;
                }
            } else {
                handle.abort();
            }
        }
        Ok(())
    }

    async fn measure(&mut self) -> Result<()> {
        let pids = collect_all_children(self.pid);

        let snapshot = read_memory_counters(&pids)?;
        let mut counters = self.counters.lock().await;
        counters.memory_counters.update_proc(
            snapshot.proc_vm_size,
            snapshot.proc_rss,
            snapshot.proc_pss,
            snapshot.proc_shared,
            snapshot.proc_anon,
        );
        counters.memory_counters.update_global(
            snapshot.global_mem_available,
            snapshot.global_mem_free,
            snapshot.global_cached,
            snapshot.global_anon,
            snapshot.global_swap_free,
        );

        let (read_bytes, write_bytes) = pids
            .into_iter()
            .filter_map(|p| Process::new(p).ok())
            .filter_map(|p| read_io(&p).ok())
            .fold((0, 0), |acc, (r, w)| (acc.0 + r, acc.1 + w));

        counters.io_counters.end_read_bytes = counters.io_counters.end_read_bytes.max(read_bytes);
        counters.io_counters.end_write_bytes =
            counters.io_counters.end_write_bytes.max(write_bytes);

        warn!("{read_bytes} {write_bytes}");

        Ok(())
    }

    async fn retrieve(&mut self) -> Result<Self::Type> {
        let mut lock = self.counters.lock().await;
        let counters = *lock;
        lock.memory_counters.reset_phase();

        lock.io_counters.begin_read_bytes = counters.io_counters.end_read_bytes;
        lock.io_counters.begin_write_bytes = counters.io_counters.end_write_bytes;

        Ok(counters)
    }

    fn get_sensors(&self) -> Result<Sensors> {
        let proc_unit: MetricUnit = self.proc_memory_unit.into();
        let global_unit: MetricUnit = self.global_memory_unit.into();

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
        .map(|name| Sensor::new(name, proc_unit, Self::get_name()));

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
        .map(|name| Sensor::new(name, global_unit, Self::get_name()));

        Ok(proc_sensors
            .chain(global_sensors)
            .chain(io_sensors)
            .collect())
    }

    fn to_metrics(&self, counters: Self::Type) -> Result<Metrics> {
        let proc_unit: MetricUnit = self.proc_memory_unit.into();
        let global_unit: MetricUnit = self.global_memory_unit.into();
        let proc_conv = make_conv(self.proc_memory_unit);
        let global_conv = make_conv(self.global_memory_unit);

        let mut memory_counters = counters.memory_counters;
        memory_counters.remove_sentinel_values();

        let proc = &memory_counters.proc;
        let proc_memory_metrics: Metrics = [
            ("proc_vm_size_min", proc_conv(proc.min_vm_size)),
            ("proc_vm_size_max", proc_conv(proc.max_vm_size)),
            ("proc_rss_min", proc_conv(proc.min_rss)),
            ("proc_rss_max", proc_conv(proc.max_rss)),
            ("proc_pss_min", proc_conv(proc.min_pss)),
            ("proc_pss_max", proc_conv(proc.max_pss)),
            ("proc_shared_min", proc_conv(proc.min_shared)),
            ("proc_shared_max", proc_conv(proc.max_shared)),
            ("proc_anon_min", proc_conv(proc.min_anon)),
            ("proc_anon_max", proc_conv(proc.max_anon)),
        ]
        .into_iter()
        .map(|(name, value)| Metric::new(name, value, proc_unit, Self::get_name()))
        .collect();

        let global = &memory_counters.global;
        let mem_total = procfs::Meminfo::from_file(procfs::Meminfo::PATH)?.mem_total;

        let mem_used = |mem_available: Option<u64>, mem_free: u64, cached: u64| -> u64 {
            if let Some(available) = mem_available {
                mem_total - available
            } else {
                mem_total.saturating_sub(mem_free + cached)
            }
        };

        let max_mem_used = mem_used(
            global.min_mem_available,
            global.min_mem_free,
            global.min_cached,
        );
        let min_mem_used = mem_used(
            global.max_mem_available,
            global.max_mem_free,
            global.max_cached,
        );

        let mut global_memory_metrics: Metrics = [
            ("global_mem_used_min", global_conv(min_mem_used)),
            ("global_mem_used_max", global_conv(max_mem_used)),
            ("global_cached_min", global_conv(global.min_cached)),
            ("global_cached_max", global_conv(global.max_cached)),
            ("global_swap_free_min", global_conv(global.min_swap_free)),
            ("global_swap_free_max", global_conv(global.max_swap_free)),
        ]
        .into_iter()
        .map(|(name, value)| Metric::new(name, value, global_unit, Self::get_name()))
        .collect();

        if let Some(max_anon) = global.max_anon
            && let Some(min_anon) = global.min_anon
        {
            let max_anon = global_conv(max_anon);
            let min_anon = global_conv(min_anon);
            global_memory_metrics.push(Metric::new(
                "global_anon_max",
                max_anon,
                global_unit,
                Self::get_name(),
            ));
            global_memory_metrics.push(Metric::new(
                "global_anon_min",
                min_anon,
                global_unit,
                Self::get_name(),
            ));
        }

        let io = counters.io_counters;
        println!("{io:?}");
        let io_metrics: Metrics = [
            (
                "proc_io_read_bytes",
                io.end_read_bytes.saturating_sub(io.begin_read_bytes),
            ),
            (
                "proc_io_write_bytes",
                io.end_write_bytes.saturating_sub(io.begin_write_bytes),
            ),
        ]
        .into_iter()
        .map(|(name, value)| Metric::new(name, value, IO_COUNTERS_METRIC_UNIT, Self::get_name()))
        .collect();

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

fn make_conv(unit: MemoryUnit) -> fn(u64) -> MetricValue {
    #[allow(clippy::cast_precision_loss)]
    match unit {
        MemoryUnit::Bytes => |b| MetricValue::UnsignedInteger(b),
        MemoryUnit::Kilo => |b| MetricValue::UnsignedInteger(b / 1_024),
        MemoryUnit::Mega => |b| MetricValue::Float(b as f64 / 1_048_576.0, Some(2)),
        MemoryUnit::Giga => |b| MetricValue::Float(b as f64 / 1_073_741_824.0, Some(2)),
    }
}
