//! cgroup metric source for Joule Profiler.
//!
//! This module implements a [`MetricReader`] and uses Linux cgroup v2 to
//! collect per-process and global system metrics using kernel cgroup files.
//!
//! An asynchronous tokio task runs to poll non monotonic metrics.

use futures::StreamExt;
use joule_profiler_core::sensor::{Sensor, Sensors};
use joule_profiler_core::source::MetricReader;
use joule_profiler_core::time::get_timestamp_micros;
use joule_profiler_core::types::{Metric, MetricValue, Metrics};
use joule_profiler_core::unit::{MetricUnit, Unit, UnitPrefix};
use log::{debug, trace};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_timerfd::Interval;
use tokio_util::sync::CancellationToken;

use crate::cgroup::{Cgroup, RootCgroup, StatsReader};
use crate::counters::{Counters, CpuCounters, IoCounters, MemoryCounters};
use crate::error::CgroupError;

mod cgroup;
mod config;
mod counters;
mod error;
mod snapshot;
mod util;

pub use config::CgroupConfig;

const SOURCE_NAME: &str = "cgroup";

const BYTE_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::None,
    unit: Unit::Byte,
};

const MICRO_SECOND_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::Micro,
    unit: Unit::Second,
};

const COUNT_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::None,
    unit: Unit::Count,
};

pub(crate) type Result<T> = std::result::Result<T, CgroupError>;
pub(crate) type WorkerHandle = (CancellationToken, JoinHandle<Result<()>>);

/// cgroup metrics source.
///
/// Owns the process cgroup and root cgroup handles, and maintains
/// internal counters for both process-level and system-wide metrics.
pub struct CgroupSource {
    config: CgroupConfig,
    handle: Option<WorkerHandle>,
    pub proc_cgroup: Arc<Cgroup>,
    pub root_cgroup: Arc<RootCgroup>,
    proc_memory_counters: Arc<Mutex<MemoryCounters>>,
    global_memory_counters: Arc<Mutex<MemoryCounters>>,
    proc_cpu_counters: CpuCounters,
    global_cpu_counters: CpuCounters,
    proc_io_counters: IoCounters,
    global_io_counters: IoCounters,
    begin_timestamp: u128,
    end_timestamp: u128,
}

impl CgroupSource {
    /// Creates a new cgroup metric source.
    ///
    /// Initializes internal cgroup handles but does initialize and attach any PID yet.
    pub fn new(config: CgroupConfig) -> Result<Self> {
        let root_cgroup = Arc::new(RootCgroup::new(config.cgroup_root.clone()));
        let proc_cgroup = Arc::new(root_cgroup.child(&config.cgroup_name));

        Ok(Self {
            config,
            handle: None,
            root_cgroup,
            proc_cgroup,
            proc_memory_counters: Arc::default(),
            global_memory_counters: Arc::default(),
            proc_cpu_counters: CpuCounters::default(),
            global_cpu_counters: CpuCounters::default(),
            proc_io_counters: IoCounters::default(),
            global_io_counters: IoCounters::default(),
            begin_timestamp: 0,
            end_timestamp: 0,
        })
    }

    /// Creates a background worker that periodically samples memory usage.
    ///
    /// This worker updates process and global memory counters at a fixed interval.
    /// It can be cancelled using the returned `CancellationToken`.
    pub fn create_worker(
        root_cgroup: Arc<RootCgroup>,
        proc_cgroup: Arc<Cgroup>,
        proc_memory_counters: Arc<Mutex<MemoryCounters>>,
        global_memory_counters: Arc<Mutex<MemoryCounters>>,
        poll_interval: Duration,
    ) -> Result<WorkerHandle> {
        let mut ticker = Interval::new_interval(poll_interval)?;

        let cancellation_token = CancellationToken::new();
        let cancellation_token_clone = cancellation_token.clone();

        let handle = tokio::spawn(async move {
            debug!("Starting cgroup memory polling.");

            loop {
                tokio::select! {
                    _ = ticker.next() => {
                        trace!("Polled cgroup source.");
                        {
                            let snapshot = proc_cgroup.stats().memory()?;
                            let mut lock = proc_memory_counters.lock().await;
                            lock.update(&snapshot);
                        }
                        {
                            let snapshot = root_cgroup.stats().memory()?;
                            let mut lock = global_memory_counters.lock().await;
                            lock.update(&snapshot);
                        }
                    }

                    () = cancellation_token.cancelled() => {
                        debug!("Cgroup worker stopped.");
                        break;
                    }
                }
            }

            Ok(())
        });

        Ok((cancellation_token_clone, handle))
    }
}

impl MetricReader for CgroupSource {
    type Type = Counters;
    type Error = CgroupError;

    /// Initializes the cgroup for the given process and enables controllers.
    ///
    /// - creates the process cgroup directory
    /// - enables requested controllers on root cgroup
    /// - attaches the target PID
    /// - optionally starts background polling worker
    async fn init(&mut self, pid: i32) -> Result<()> {
        self.proc_cgroup.initialize()?;
        for controller in &self.config.controllers {
            self.root_cgroup.activate_controller(*controller)?;
        }

        self.proc_cgroup.attach_pid(pid)?;

        if let Some(poll_interval) = self.config.poll_interval {
            self.handle = Some(Self::create_worker(
                self.root_cgroup.clone(),
                self.proc_cgroup.clone(),
                self.proc_memory_counters.clone(),
                self.global_memory_counters.clone(),
                poll_interval,
            )?);
        }

        self.begin_timestamp = get_timestamp_micros();

        debug!("Cgroup source initialised for pid {pid}");
        Ok(())
    }

    /// Performs a measurement of all available metrics.
    ///
    /// Updates internal CPU, memory, and I/O counters.
    async fn measure(&mut self) -> Result<()> {
        self.end_timestamp = get_timestamp_micros();
        {
            let snapshot = self.proc_cgroup.stats().memory()?;
            let mut lock = self.proc_memory_counters.lock().await;
            lock.update(&snapshot);
        }
        self.proc_cpu_counters
            .update(&self.proc_cgroup.stats().cpu()?);
        self.proc_io_counters
            .update(&self.proc_cgroup.stats().io()?);

        {
            let snapshot = self.root_cgroup.stats().memory()?;
            let mut lock = self.global_memory_counters.lock().await;
            lock.update(&snapshot);
        }
        self.global_cpu_counters
            .update(&self.root_cgroup.stats().cpu()?);
        self.global_io_counters
            .update(&self.root_cgroup.stats().io()?);

        Ok(())
    }

    /// Returns collected metrics and resets per-phase counters.
    async fn retrieve(&mut self) -> Result<Self::Type> {
        let proc_memory = {
            let mut lock = self.proc_memory_counters.lock().await;
            let counters = *lock;
            lock.reset();
            counters
        };

        let proc_cpu = self.proc_cpu_counters;
        self.proc_cpu_counters.new_phase();

        let proc_io = self.proc_io_counters;
        self.proc_io_counters.new_phase();

        let global_memory = {
            let mut lock = self.global_memory_counters.lock().await;
            let counters = *lock;
            lock.reset();
            counters
        };

        let global_cpu = self.global_cpu_counters;
        self.global_cpu_counters.new_phase();

        let global_io = self.global_io_counters;
        self.global_io_counters.new_phase();

        let begin_timestamp = self.begin_timestamp;
        self.begin_timestamp = self.end_timestamp;

        Ok(Counters {
            proc_memory,
            proc_cpu,
            proc_io,
            global_memory,
            global_cpu,
            global_io,
            begin_timestamp,
            end_timestamp: self.end_timestamp,
        })
    }

    /// Stops background worker and cleans up the cgroup.
    async fn join(&mut self) -> Result<()> {
        if let Some((cancellation_token, handle)) = self.handle.take() {
            cancellation_token.cancel();
            handle.await??;
        }
        self.proc_cgroup.cleanup()?;
        Ok(())
    }

    /// Returns the list of exported sensors.
    fn get_sensors(&self) -> Result<Sensors> {
        Ok(vec![
            Sensor::new("usage_usec", MICRO_SECOND_UNIT, SOURCE_NAME),
            Sensor::new("user_usec", MICRO_SECOND_UNIT, SOURCE_NAME),
            Sensor::new("system_usec", MICRO_SECOND_UNIT, SOURCE_NAME),
            Sensor::new("nr_periods", COUNT_UNIT, SOURCE_NAME),
            Sensor::new("nr_throttled", COUNT_UNIT, SOURCE_NAME),
            Sensor::new("throttled_usec", MICRO_SECOND_UNIT, SOURCE_NAME),
            Sensor::new("nr_bursts", COUNT_UNIT, SOURCE_NAME),
            Sensor::new("burst_usec", MICRO_SECOND_UNIT, SOURCE_NAME),
            Sensor::new("anon_min", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("anon_max", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("file_min", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("file_max", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("kernel_min", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("kernel_max", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("kernel_stack_min", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("kernel_stack_max", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("peak_min", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("peak_max", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("shmem_min", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("shmem_max", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("slab_min", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("slab_max", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("swap_current_min", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("swap_current_max", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("read_bytes", BYTE_UNIT, SOURCE_NAME),
            Sensor::new("write_bytes", BYTE_UNIT, SOURCE_NAME),
        ])
    }

    #[allow(clippy::cast_precision_loss)]
    /// Converts counters into metrics.
    fn to_metrics(&self, counters: Self::Type) -> Result<Metrics> {
        let phase_duration: u64 = counters
            .end_timestamp
            .saturating_sub(counters.begin_timestamp)
            .try_into()
            .unwrap_or_default();

        let proc_cpu_usage_usec = counters.proc_cpu.usage_usec.diff();

        let proc_usage_ratio = if phase_duration == 0 {
            0.0
        } else {
            proc_cpu_usage_usec as f64 / phase_duration as f64 * 100.0
        };

        let proc_cpu_usage = Metric::new(
            "proc_cpu_usage",
            MetricValue::Float(proc_usage_ratio, Some(2)),
            MetricUnit {
                prefix: UnitPrefix::None,
                unit: Unit::Percent,
            },
            Self::get_name(),
        );

        let global_cpu_usage_usec = counters.global_cpu.usage_usec.diff();

        let global_usage_ratio = if phase_duration == 0 {
            0.0
        } else {
            global_cpu_usage_usec as f64 / phase_duration as f64 * 100.0
        };

        let global_cpu_usage = Metric::new(
            "global_cpu_usage",
            MetricValue::Float(global_usage_ratio, Some(2)),
            MetricUnit {
                prefix: UnitPrefix::None,
                unit: Unit::Percent,
            },
            Self::get_name(),
        );

        let metrics = vec![proc_cpu_usage, global_cpu_usage];

        Ok(metrics
            .into_iter()
            .chain(to_metrics(
                &counters.proc_memory,
                &counters.proc_cpu,
                &counters.proc_io,
                "proc",
            ))
            .chain(to_metrics(
                &counters.global_memory,
                &counters.global_cpu,
                &counters.global_io,
                "global",
            ))
            .collect())
    }

    fn get_name() -> &'static str {
        SOURCE_NAME
    }
}

fn to_metrics(
    memory: &MemoryCounters,
    cpu: &CpuCounters,
    io: &IoCounters,
    prefix: &str,
) -> Metrics {
    macro_rules! push {
        ($metrics:expr, $name:expr, $value:expr, $unit:expr) => {
            $metrics.push(Metric::new(
                format!("{prefix}_{}", $name),
                $value,
                $unit,
                CgroupSource::get_name(),
            ));
        };
    }

    macro_rules! push_minmax {
        ($metrics:expr, $field:expr, $name:expr, $unit:expr) => {
            if let Some(mm) = $field {
                push!(
                    $metrics,
                    concat!($name, "_min"),
                    mm.min().unwrap_or_default(),
                    $unit
                );
                push!(
                    $metrics,
                    concat!($name, "_max"),
                    mm.max().unwrap_or_default(),
                    $unit
                );
            }
        };
    }

    macro_rules! push_begin_end {
        ($metrics:expr, $field:expr, $name:expr, $unit:expr) => {
            if let Some(be) = $field {
                push!($metrics, $name, be.diff(), $unit);
            }
        };
    }

    let mut metrics = Vec::new();

    push_minmax!(metrics, memory.anon, "anon", BYTE_UNIT);
    push_minmax!(metrics, memory.file, "file", BYTE_UNIT);
    push_minmax!(metrics, memory.kernel, "kernel", BYTE_UNIT);
    push_minmax!(metrics, memory.kernel_stack, "kernel_stack", BYTE_UNIT);
    push_minmax!(metrics, memory.peak, "peak", BYTE_UNIT);
    push_minmax!(metrics, memory.shmem, "shmem", BYTE_UNIT);
    push_minmax!(metrics, memory.slab, "slab", BYTE_UNIT);
    push_minmax!(metrics, memory.swap_current, "swap_current", BYTE_UNIT);

    push!(
        metrics,
        "usage_usec",
        cpu.usage_usec.diff(),
        MICRO_SECOND_UNIT
    );
    push!(
        metrics,
        "user_usec",
        cpu.user_usec.diff(),
        MICRO_SECOND_UNIT
    );
    push!(
        metrics,
        "system_usec",
        cpu.system_usec.diff(),
        MICRO_SECOND_UNIT
    );

    push_begin_end!(metrics, cpu.nr_periods, "nr_periods", COUNT_UNIT);
    push_begin_end!(metrics, cpu.nr_throttled, "nr_throttled", COUNT_UNIT);
    push_begin_end!(
        metrics,
        cpu.throttled_usec,
        "throttled_usec",
        MICRO_SECOND_UNIT
    );
    push_begin_end!(metrics, cpu.nr_bursts, "nr_bursts", COUNT_UNIT);
    push_begin_end!(metrics, cpu.burst_usec, "burst_usec", MICRO_SECOND_UNIT);

    push_begin_end!(metrics, io.rbytes, "read_bytes", BYTE_UNIT);
    push_begin_end!(metrics, io.wbytes, "write_bytes", BYTE_UNIT);

    metrics
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};
    use tokio::time::Duration;

    fn temp_root(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("cgroup_src_{name}"));
        let _ = fs::create_dir_all(&p);
        p
    }

    fn setup_source(name: &str) -> CgroupSource {
        let mut cfg = CgroupConfig::default();
        cfg.cgroup_root = temp_root(name);
        cfg.cgroup_name = "test".to_string();
        cfg.poll_interval = None;
        CgroupSource::new(cfg).unwrap()
    }

    #[tokio::test]
    async fn test_init_and_attach() {
        let mut src = setup_source("init");

        src.init(42).await.unwrap();

        let procs = src.proc_cgroup.path.join("cgroup.procs");
        let content = fs::read_to_string(procs).unwrap();

        assert!(content.contains("42"));

        src.join().await.unwrap();
    }

    #[tokio::test]
    async fn test_measure_updates_counters() {
        let mut src = setup_source("measure");
        let root = src.root_cgroup.path.clone();

        src.init(1).await.unwrap();

        fs::write(root.join("test/memory.stat"), "anon 200").unwrap();

        {
            let mem = src.proc_memory_counters.lock().await;
            assert!(mem.anon.is_none());
        }

        src.measure().await.unwrap();

        {
            let mem = src.proc_memory_counters.lock().await;
            assert_eq!(mem.anon.unwrap().max().unwrap(), 200);
            assert_eq!(mem.anon.unwrap().min().unwrap(), 200);
        }

        fs::write(root.join("test/memory.stat"), "anon 400").unwrap();

        src.measure().await.unwrap();

        {
            let mem = src.proc_memory_counters.lock().await;
            assert_eq!(mem.anon.unwrap().max().unwrap(), 400);
            assert_eq!(mem.anon.unwrap().min().unwrap(), 200);
        }

        src.join().await.unwrap();
    }

    #[tokio::test]
    async fn test_retrieve_compute_diffs() {
        let mut src = setup_source("retrieve");
        let root = src.root_cgroup.path.clone();

        fs::write(
            root.join("test/cpu.stat"),
            "usage_usec 1000\nuser_usec 500\nsystem_usec 500\nnr_periods 2\n",
        )
        .unwrap();

        src.init(1).await.unwrap();
        src.measure().await.unwrap();

        fs::write(
            root.join("test/cpu.stat"),
            "usage_usec 2000\nuser_usec 1000\nsystem_usec 1000\nnr_periods 4\n",
        )
        .unwrap();

        src.measure().await.unwrap();

        let counters = src.retrieve().await.unwrap();

        assert_eq!(counters.proc_cpu.usage_usec.diff(), 1000);
        assert_eq!(counters.proc_cpu.user_usec.diff(), 500);
        assert_eq!(counters.proc_cpu.system_usec.diff(), 500);
        assert_eq!(counters.proc_cpu.nr_periods.unwrap().diff(), 2);

        src.join().await.unwrap();
    }

    #[tokio::test]
    async fn test_join_stops_worker() {
        let mut src = setup_source("worker");

        src.config.poll_interval = Some(Duration::from_millis(10));

        src.init(1).await.unwrap();

        let res = src.join().await;

        assert!(res.is_ok());
    }
}
