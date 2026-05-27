use futures::StreamExt;
use joule_profiler_core::{
    sensor::{Sensor, Sensors},
    source::MetricReader,
    types::{Metric, MetricValue, Metrics},
    unit::{MetricUnit, Unit, UnitPrefix},
};
use log::{debug, trace};
use procfs::{Current, FromRead, Meminfo};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_timerfd::Interval;

use crate::{
    counters::{Counters, MinMax},
    error::ProcfsError,
    snapshot::{GlobalSnapshot, ProcSnapshot, read_global, read_proc},
    utils::{MemoryUnit, collect_all_children},
};

pub mod counters;
pub mod error;
mod snapshot;
mod utils;

const IO_COUNTERS_METRIC_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::None,
    unit: Unit::Byte,
};

type Result<T> = std::result::Result<T, ProcfsError>;

#[derive(Debug, Default)]
pub struct Procfs {
    pid: i32,

    poll_interval: Option<Duration>,
    counters: Arc<Mutex<Counters>>,
    handle: Option<JoinHandle<Result<()>>>,

    proc_memory_unit: MemoryUnit,
    global_memory_unit: MemoryUnit,

    mem_total: u64,
}

impl Procfs {
    pub fn new(poll_interval: Option<Duration>) -> Result<Self> {
        let mem_total = Meminfo::from_file(Meminfo::PATH)?.mem_total;
        Ok(Self {
            pid: -1,
            mem_total,
            poll_interval,
            global_memory_unit: MemoryUnit::Giga,
            ..Default::default()
        })
    }
}

impl MetricReader for Procfs {
    type Type = Counters;
    type Error = ProcfsError;

    async fn init(&mut self, pid: i32) -> Result<()> {
        self.pid = pid;

        let Some(interval) = self.poll_interval else {
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
        let proc = match measure_proc(self.pid) {
            Err(ProcfsError::Procfs(procfs::ProcError::NotFound(_))) => Ok(ProcSnapshot::default()),
            r => r,
        }?;

        let global = measure_global()?;

        let mut counters = self.counters.lock().await;
        counters.update(&proc, &global);

        Ok(())
    }

    async fn retrieve(&mut self) -> Result<Self::Type> {
        let mut lock = self.counters.lock().await;
        let counters = *lock;
        lock.reset();
        Ok(counters)
    }

    fn get_sensors(&self) -> Result<Sensors> {
        let proc_memory_unit: MetricUnit = self.proc_memory_unit.into();
        let global_memory_unit: MetricUnit = self.global_memory_unit.into();

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
        let proc_unit: MetricUnit = self.proc_memory_unit.into();
        let global_unit: MetricUnit = self.global_memory_unit.into();
        let proc_conv = make_conv(self.proc_memory_unit);
        let global_conv = make_conv(self.global_memory_unit);

        counters.remove_sentinels();
        let proc = counters.proc;

        let proc_memory_metrics: Metrics = [
            ("proc_vm_size_min", proc_conv(proc.vm_size.min())),
            ("proc_vm_size_max", proc_conv(proc.vm_size.max())),
            ("proc_rss_min", proc_conv(proc.rss.min())),
            ("proc_rss_max", proc_conv(proc.rss.max())),
            ("proc_pss_min", proc_conv(proc.pss.min())),
            ("proc_pss_max", proc_conv(proc.pss.max())),
            ("proc_shared_min", proc_conv(proc.shared.min())),
            ("proc_shared_max", proc_conv(proc.shared.max())),
            ("proc_anon_min", proc_conv(proc.anon.min())),
            ("proc_anon_max", proc_conv(proc.anon.max())),
        ]
        .into_iter()
        .map(|(name, value)| Metric::new(name, value, proc_unit, Self::get_name()))
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

        let mem_used =
            |mem_available: Option<MinMax>, mem_free: MinMax, cached: MinMax| -> MinMax {
                if let Some(available) = mem_available {
                    MinMax(
                        self.mem_total.saturating_sub(available.max()),
                        self.mem_total.saturating_sub(available.min()),
                    )
                } else {
                    MinMax(
                        self.mem_total.saturating_sub(mem_free.min() + cached.min()),
                        self.mem_total.saturating_sub(mem_free.max() + cached.max()),
                    )
                }
            };

        let mem_used = mem_used(global.mem_available, global.mem_free, global.cached);

        let mut global_memory_metrics: Metrics = [
            ("global_mem_used_min", global_conv(mem_used.min())),
            ("global_mem_used_max", global_conv(mem_used.max())),
            ("global_cached_min", global_conv(global.cached.min())),
            ("global_cached_max", global_conv(global.cached.max())),
            ("global_swap_free_min", global_conv(global.swap_free.min())),
            ("global_swap_free_max", global_conv(global.swap_free.max())),
        ]
        .into_iter()
        .map(|(name, value)| Metric::new(name, value, global_unit, Self::get_name()))
        .collect();

        if let Some(anon) = global.anon {
            let min_anon = global_conv(anon.min());
            let max_anon = global_conv(anon.max());

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

fn measure_proc(pid: i32) -> Result<ProcSnapshot> {
    trace!("Retrieving process hierarchy from pid {pid}.");
    let pids = collect_all_children(pid);
    trace!("Found pids {pids:?}. Reading process procfs counters.");
    read_proc(&pids)
}

fn measure_global() -> Result<GlobalSnapshot> {
    trace!("Reading global procfs counters.");
    read_global()
}
