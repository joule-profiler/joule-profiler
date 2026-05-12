use futures::StreamExt;
use joule_profiler_core::{
    sensor::{Sensor, Sensors},
    source::MetricReader,
    types::{Metric, MetricValue, Metrics},
    unit::{MetricUnit, Unit, UnitPrefix},
};
use procfs::process::Process;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_timerfd::Interval;

use crate::counters::{Counters, IoCounters, MemoryCounters};

pub mod counters;

const IO_COUNTERS_METRIC_UNIT: MetricUnit = MetricUnit {prefix: UnitPrefix::None, unit: Unit::Byte};

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

#[derive(Debug, Default)]
pub struct Procfs {
    process: Option<Process>,
    poll_interval: Option<Duration>,
    memory_counters: Arc<Mutex<MemoryCounters>>,
    io_counters: IoCounters,
    handle: Option<JoinHandle<Result<()>>>,
    memory_unit: MemoryUnit,
}

fn read_memory(process: &Process) -> Result<(u64, u64, u64, u64, u64)> {
    let vm_size = process.stat()?.vsize;
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

fn read_io(process: &Process) -> Result<(u64, u64)> {
    let io = process.io()?;
    Ok((io.rchar, io.wchar))
}

impl Procfs {
    pub fn new(poll_interval: Option<Duration>) -> Self {
        Self {
            poll_interval,
            ..Default::default()
        }
    }
}

impl MetricReader for Procfs {
    type Type = Counters;
    type Error = ProcfsError;

    async fn init(&mut self, pid: i32) -> Result<()> {
        self.process = Some(Process::new(pid)?);

        let Some(interval) = self.poll_interval else {
            return Ok(());
        };

        let current_counters = self.memory_counters.clone();

        let handle = tokio::spawn(async move {
            let process = Process::new(pid)?;
            let mut ticker = Interval::new_interval(interval)?;

            loop {
                ticker.next().await;

                let Ok((vm_size, rss, pss, shared, anon)) = read_memory(&process) else {
                    continue;
                };

                let mut counters = current_counters.lock().await;
                counters.update(vm_size, rss, pss, shared, anon);
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
        let process = self.process.as_ref().ok_or(ProcfsError::NotInitialized)?;
        
        match read_io(process) {
            Ok((read_bytes, write_bytes)) => {
                self.io_counters.end_read_bytes = read_bytes;
                self.io_counters.end_write_bytes = write_bytes;
            }
            Err(ProcfsError::Procfs(_)) => {}
            Err(e) => return Err(e),
        }

        match read_memory(process) {
            Ok((vm_size, rss, pss, shared, anon)) => {
                let mut counters = self.memory_counters.lock().await;
                counters.update(vm_size, rss, pss, shared, anon);
            }
            Err(ProcfsError::Procfs(_)) => {}
            Err(e) => return Err(e),
        }

        Ok(())
    }

    async fn retrieve(&mut self) -> Result<Self::Type> {
        let mut lock = self.memory_counters.lock().await;
        let memory_counters = lock.clone();
        lock.reset_phase();

        let counters = Counters { memory_counters, io_counters: self.io_counters };

        self.io_counters.begin_read_bytes = self.io_counters.end_read_bytes;
        self.io_counters.begin_write_bytes = self.io_counters.end_write_bytes;

        Ok(counters)
    }

    fn get_sensors(&self) -> Result<Sensors> {
        Ok([
            "vm_size",
            "vm_size_max",
            "rss",
            "rss_max",
            "pss",
            "pss_max",
            "shared",
            "shared_max",
            "anon",
            "anon_max",
        ]
        .into_iter()
        .map(|name| Sensor::new(name, self.memory_unit.into(), Self::get_name()))
        .collect())
    }

    fn to_metrics(&self, counters: Self::Type) -> Result<Metrics> {
        let memory_unit = self.memory_unit.into();

        let memory_counters = counters.memory_counters;

        let (conv, conv_delta): (fn(u64) -> MetricValue, fn(i64) -> MetricValue) =
            match self.memory_unit {
                MemoryUnit::Bytes => (
                    |b| MetricValue::UnsignedInteger(b),
                    |s| MetricValue::SignedInteger(s),
                ),
                MemoryUnit::Kilo => (
                    |b| MetricValue::UnsignedInteger(b / 1024),
                    |s| MetricValue::SignedInteger(s / 1024),
                ),
                MemoryUnit::Mega => (
                    |b| MetricValue::Float(b as f64 / (1024.0 * 1024.0), Some(2)),
                    |s| MetricValue::Float(s as f64 / (1024.0 * 1024.0), Some(2)),
                ),
                MemoryUnit::Giga => (
                    |b| MetricValue::Float(b as f64 / (1024.0 * 1024.0 * 1024.0), Some(2)),
                    |s| MetricValue::Float(s as f64 / (1024.0 * 1024.0 * 1024.0), Some(2)),
                ),
            };

        let memory_metrics: Metrics = [
            ("vm_size", conv(memory_counters.vm_size)),
            ("vm_size_max", conv(memory_counters.max_vm_size)),
            ("rss", conv(memory_counters.rss)),
            ("rss_max", conv(memory_counters.max_rss)),
            ("pss", conv(memory_counters.pss)),
            ("pss_max", conv(memory_counters.max_pss)),
            ("shared", conv(memory_counters.shared)),
            ("shared_max", conv(memory_counters.max_shared)),
            ("anon", conv(memory_counters.anon)),
            ("anon_max", conv(memory_counters.max_anon)),
        ]
        .into_iter()
        .map(|(name, value)| Metric::new(name, value, memory_unit, Self::get_name()))
        .collect();

        let memory_deltas: Metrics = [
            (
                "vm_size_delta",
                conv_delta(MemoryCounters::delta(memory_counters.vm_size, memory_counters.phase_start_vm_size)),
            ),
            (
                "rss_delta",
                conv_delta(MemoryCounters::delta(memory_counters.rss, memory_counters.phase_start_rss)),
            ),
            (
                "pss_delta",
                conv_delta(MemoryCounters::delta(memory_counters.pss, memory_counters.phase_start_pss)),
            ),
            (
                "shared_delta",
                conv_delta(MemoryCounters::delta(memory_counters.shared, memory_counters.phase_start_shared)),
            ),
            (
                "anon_delta",
                conv_delta(MemoryCounters::delta(memory_counters.anon, memory_counters.phase_start_anon)),
            ),
        ]
        .into_iter()
        .map(|(name, value)| Metric::new(name, value, memory_unit, Self::get_name()))
        .collect();

        let io_counters = counters.io_counters;
        let read_bytes =
    io_counters
        .end_read_bytes
        .saturating_sub(io_counters.begin_read_bytes);

        let write_bytes =
    io_counters
        .end_write_bytes
        .saturating_sub(io_counters.begin_write_bytes);

        let io_metrics = vec![
            Metric::new("read_bytes", read_bytes, IO_COUNTERS_METRIC_UNIT, Self::get_name()),
            Metric::new("write_bytes", write_bytes, IO_COUNTERS_METRIC_UNIT, Self::get_name()),
        ];

        Ok(memory_metrics.into_iter().chain(memory_deltas).chain(io_metrics).collect())
    }

    fn get_name() -> &'static str {
        "procfs"
    }
}
