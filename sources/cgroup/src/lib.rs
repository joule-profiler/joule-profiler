//! cgroup metric source for Joule Profiler.
//!
//! This module implements a [`MetricReader`] and uses Linux cgroup v2 to
//! collect per-process and global system metrics using kernel cgroup files.
//!
//! An asynchronous tokio task runs to poll non monotonic metrics.

use futures::StreamExt;
use joule_profiler_core::sensor::{Sensor, Sensors};
use joule_profiler_core::source::MetricReader;
use joule_profiler_core::sys::is_root;
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

use crate::cgroup::{CgroupBackend, ChildCgroup, RootCgroup, SysFsBackend};
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
pub struct Cgroup<B = SysFsBackend>
where
    B: CgroupBackend,
{
    config: CgroupConfig,
    handle: Option<WorkerHandle>,
    pub proc_cgroup: Arc<ChildCgroup<B>>,
    pub root_cgroup: Arc<RootCgroup<B>>,
    proc_memory_counters: Arc<Mutex<MemoryCounters>>,
    global_memory_counters: Arc<Mutex<MemoryCounters>>,
    proc_cpu_counters: CpuCounters,
    global_cpu_counters: CpuCounters,
    proc_io_counters: IoCounters,
    global_io_counters: IoCounters,
    begin_timestamp: u128,
    end_timestamp: u128,
}

impl<B: CgroupBackend> Cgroup<B> {
    /// Creates a background worker that periodically samples memory usage.
    ///
    /// This worker updates process and global memory counters at a fixed interval.
    /// It can be cancelled using the returned `CancellationToken`.
    pub fn create_worker(
        proc_cgroup: Arc<ChildCgroup<B>>,
        root_cgroup: Arc<RootCgroup<B>>,
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
                            let snapshot = proc_cgroup.memory()?;
                            let mut lock = proc_memory_counters.lock().await;
                            lock.update(&snapshot);
                        }
                        {
                            let snapshot = root_cgroup.memory()?;
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

impl<B: CgroupBackend> MetricReader for Cgroup<B> {
    type Type = Counters;
    type Error = CgroupError;
    type Config = CgroupConfig;

    /// Builds the cgroup handles for `config`, generic over any [`CgroupBackend`].
    ///
    /// Does not initialize or attach any PID yet, see [`MetricReader::init`].
    fn from_config(config: Self::Config) -> Result<Self> {
        if (config.attach_pid || config.create_cgroup) && !is_root() {
            return Err(CgroupError::PermissionDenied(
                "the cgroup source requires root privileges to create a cgroup or attach a process.",
            ));
        }

        let (root_cgroup, proc_cgroup) =
            RootCgroup::build(config.cgroup_root.clone(), &config.cgroup_name);

        Ok(Self {
            config,
            handle: None,
            root_cgroup: Arc::new(root_cgroup),
            proc_cgroup: Arc::new(proc_cgroup),
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

    /// Creates the cgroup if configured.
    async fn pre_init(&mut self) -> Result<()> {
        if self.config.create_cgroup {
            self.proc_cgroup.create()?;
        }
        Ok(())
    }

    /// Initializes the cgroup source with the given pid.
    async fn init(&mut self, pid: i32) -> Result<()> {
        if self.config.attach_pid {
            self.proc_cgroup.attach_pid(pid)?;
        }

        self.handle = Some(Self::create_worker(
            self.proc_cgroup.clone(),
            self.root_cgroup.clone(),
            self.proc_memory_counters.clone(),
            self.global_memory_counters.clone(),
            self.config.poll_interval,
        )?);

        self.begin_timestamp = get_timestamp_micros();

        debug!("Cgroup source initialized for pid {pid}.");
        Ok(())
    }

    /// Performs a measurement of all available metrics.
    ///
    /// Updates internal CPU, memory, and I/O counters.
    async fn measure(&mut self) -> Result<()> {
        debug!("Measure cgroup source.");

        self.end_timestamp = get_timestamp_micros();
        {
            let snapshot = self.proc_cgroup.memory()?;
            let mut lock = self.proc_memory_counters.lock().await;
            lock.update(&snapshot);
        }
        self.proc_cpu_counters.update(&self.proc_cgroup.cpu()?);
        self.proc_io_counters.update(&self.proc_cgroup.io()?);

        {
            let snapshot = self.root_cgroup.memory()?;
            let mut lock = self.global_memory_counters.lock().await;
            lock.update(&snapshot);
        }
        self.global_cpu_counters.update(&self.root_cgroup.cpu()?);
        self.global_io_counters.update(&self.root_cgroup.io()?);

        Ok(())
    }

    /// Returns collected metrics and resets per-phase counters.
    async fn retrieve(&mut self) -> Result<Self::Type> {
        debug!("Retrieving cgroup counters.");

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
            debug!("Joining cgroup source polling task.");
            cancellation_token.cancel();
            handle.await??;
        }
        if self.config.create_cgroup {
            self.proc_cgroup.cleanup()?;
        }
        Ok(())
    }

    /// Returns the list of exported sensors.
    fn get_sensors(&self) -> Result<Sensors> {
        debug!("Retrieving cgroup source sensors.");
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

    fn get_id() -> &'static str {
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
                Cgroup::<SysFsBackend>::get_name(),
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
    use crate::snapshot::{CpuSnapshot, IoSnapshot, MemorySnapshot};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tokio::time::{Duration, sleep};

    #[derive(Default, Clone)]
    struct MockCgroupBackend {
        memory: Arc<std::sync::Mutex<MemorySnapshot>>,
        cpu: Arc<std::sync::Mutex<CpuSnapshot>>,
        io: Arc<std::sync::Mutex<IoSnapshot>>,
    }

    impl CgroupBackend for MockCgroupBackend {
        fn memory(&self, _path: &Path) -> Result<MemorySnapshot> {
            Ok(*self.memory.lock().unwrap())
        }

        fn cpu(&self, _path: &Path) -> Result<CpuSnapshot> {
            Ok(*self.cpu.lock().unwrap())
        }

        fn io(&self, _path: &Path) -> Result<IoSnapshot> {
            Ok(*self.io.lock().unwrap())
        }

        fn cleanup(&self, _path: &Path, _root: &Path) -> Result<()> {
            Ok(())
        }

        fn create(&self, _path: &Path) -> Result<()> {
            Ok(())
        }

        fn attach_pid(&self, _path: &Path, _pid: i32) -> Result<()> {
            Ok(())
        }
    }

    fn setup_source() -> (Cgroup<MockCgroupBackend>, MockCgroupBackend) {
        let backend = MockCgroupBackend::default();

        let root = Arc::new(RootCgroup::new(PathBuf::from("/tmp/root"), backend.clone()));

        let proc = Arc::new(ChildCgroup::new(
            PathBuf::from("/tmp/cgroup"),
            PathBuf::from("/tmp/root"),
            backend.clone(),
        ));

        let source = Cgroup {
            config: CgroupConfig::default(),
            handle: None,
            root_cgroup: root,
            proc_cgroup: proc,
            proc_memory_counters: Arc::default(),
            global_memory_counters: Arc::default(),
            proc_cpu_counters: CpuCounters::default(),
            global_cpu_counters: CpuCounters::default(),
            proc_io_counters: IoCounters::default(),
            global_io_counters: IoCounters::default(),
            begin_timestamp: 0,
            end_timestamp: 0,
        };

        (source, backend)
    }

    #[tokio::test]
    async fn test_measure_updates_counters() {
        let (mut src, backend) = setup_source();

        {
            let mem = src.proc_memory_counters.lock().await;
            assert!(mem.anon.is_none());
        }

        {
            backend.memory.lock().unwrap().anon = Some(200);
        }

        src.measure().await.unwrap();

        {
            let mem = src.proc_memory_counters.lock().await;
            let anon = mem.anon.unwrap();

            assert_eq!(anon.min(), Some(200));
            assert_eq!(anon.max(), Some(200));
        }
    }

    #[tokio::test]
    async fn test_measure_tracks_min_max() {
        let (mut src, backend) = setup_source();

        {
            let mut memory = backend.memory.lock().unwrap();
            memory.anon = Some(200);
            memory.current = Some(200);
            memory.peak = Some(200);
        }

        src.measure().await.unwrap();

        {
            let mut memory = backend.memory.lock().unwrap();
            memory.anon = Some(400);
            memory.current = Some(400);
            memory.peak = Some(400);
        }

        src.measure().await.unwrap();

        let mem = src.proc_memory_counters.lock().await;
        let anon = mem.anon.unwrap();
        let current = mem.current.unwrap();
        let peak = mem.peak.unwrap();

        assert_eq!(anon.min(), Some(200));
        assert_eq!(anon.max(), Some(400));

        assert_eq!(current.min(), Some(200));
        assert_eq!(current.max(), Some(400));

        assert_eq!(peak.min(), Some(200));
        assert_eq!(peak.max(), Some(400));
    }

    #[tokio::test]
    async fn test_retrieve_compute_diffs() {
        let (mut src, backend) = setup_source();

        {
            let mut cpu = backend.cpu.lock().unwrap();

            cpu.usage_usec = 1000;
            cpu.user_usec = 500;
            cpu.system_usec = 500;
            cpu.nr_periods = Some(2);
        }

        src.measure().await.unwrap();

        {
            let mut cpu = backend.cpu.lock().unwrap();

            cpu.usage_usec = 2000;
            cpu.user_usec = 1000;
            cpu.system_usec = 1000;
            cpu.nr_periods = Some(4);
        }

        src.measure().await.unwrap();

        let counters = src.retrieve().await.unwrap();

        assert_eq!(counters.proc_cpu.usage_usec.diff(), 1000);
        assert_eq!(counters.proc_cpu.user_usec.diff(), 500);
        assert_eq!(counters.proc_cpu.system_usec.diff(), 500);
        assert_eq!(counters.proc_cpu.nr_periods.unwrap().diff(), 2);
    }

    #[tokio::test]
    async fn test_worker_updates_counters() {
        let (src, backend) = setup_source();

        let (token, handle) = Cgroup::create_worker(
            src.proc_cgroup.clone(),
            src.root_cgroup.clone(),
            src.proc_memory_counters.clone(),
            src.global_memory_counters.clone(),
            Duration::from_millis(10),
        )
        .unwrap();

        {
            let mem = src.proc_memory_counters.lock().await;
            assert!(mem.anon.is_none());
        }

        {
            backend.memory.lock().unwrap().anon = Some(200);
        }

        sleep(Duration::from_millis(10)).await;

        {
            let mem = src.proc_memory_counters.lock().await;
            assert_eq!(mem.anon.unwrap().max().unwrap(), 200);
            assert_eq!(mem.anon.unwrap().min().unwrap(), 200);
        }

        {
            backend.memory.lock().unwrap().anon = Some(100);
        }

        sleep(Duration::from_millis(10)).await;

        {
            let mem = src.proc_memory_counters.lock().await;
            assert_eq!(mem.anon.unwrap().max().unwrap(), 200);
            assert_eq!(mem.anon.unwrap().min().unwrap(), 100);
        }

        token.cancel();
        handle.await.unwrap().unwrap();
    }
}
