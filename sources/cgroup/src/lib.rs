use futures::StreamExt;
use joule_profiler_core::sensor::{Sensor, Sensors};
use joule_profiler_core::source::MetricReader;
use joule_profiler_core::types::{Metric, Metrics};
use joule_profiler_core::unit::{MetricUnit, Unit, UnitPrefix};
use log::{debug, trace};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_timerfd::Interval;
use tokio_util::sync::CancellationToken;

use crate::cgroup::{Cgroup, Controller};
use crate::counters::{Counters, CpuCounters, IoCounters, MemoryCounters};
use crate::error::CgroupError;

mod cgroup;
mod counters;
mod error;
mod snapshot;
mod util;

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

#[derive(Debug, Clone)]
pub struct CgroupConfig {
    pub cgroup_root: PathBuf,
    pub cgroup_name: String,
    pub poll_interval: Option<Duration>,
    pub controllers: HashSet<Controller>,
}

impl Default for CgroupConfig {
    fn default() -> Self {
        Self {
            cgroup_root: PathBuf::from("/sys/fs/cgroup"),
            cgroup_name: format!("joule-profiler-{}", std::process::id()),
            poll_interval: None,
            controllers: vec![Controller::Io, Controller::Mem, Controller::Cpu]
                .into_iter()
                .collect(),
        }
    }
}

pub struct CgroupSource {
    config: CgroupConfig,
    handle: Option<WorkerHandle>,
    cgroup: Arc<Cgroup>,
    shared_memory_counters: Arc<Mutex<MemoryCounters>>,
    cpu_counters: CpuCounters,
    io_counters: IoCounters,
}

impl CgroupSource {
    pub fn new(config: CgroupConfig) -> Result<Self> {
        let cgroup = Arc::new(Cgroup::new(
            config.cgroup_root.clone(),
            &config.cgroup_name,
        )?);

        Ok(Self {
            config,
            handle: None,
            cgroup,
            shared_memory_counters: Arc::default(),
            cpu_counters: CpuCounters::default(),
            io_counters: IoCounters::default(),
        })
    }

    pub fn create_worker(
        cgroup: Arc<Cgroup>,
        shared_memory_counters: Arc<Mutex<MemoryCounters>>,
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
                        let memory_snapshot = cgroup.read_memory()?;
                        let mut lock = shared_memory_counters.lock().await;
                        lock.update(&memory_snapshot);
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

    async fn init(&mut self, pid: i32) -> Result<()> {
        self.cgroup.initialize_cgroup()?;
        for controller in &self.config.controllers {
            self.cgroup.activate_controller(*controller)?;
        }

        self.cgroup.attach_pid(pid)?;

        if let Some(poll_interval) = self.config.poll_interval {
            self.handle = Some(Self::create_worker(
                self.cgroup.clone(),
                self.shared_memory_counters.clone(),
                poll_interval,
            )?);
        }

        debug!("Cgroup source initialised for pid {pid}");
        Ok(())
    }

    async fn measure(&mut self) -> Result<()> {
        let memory_snapshot = self.cgroup.read_memory()?;
        {
            let mut lock = self.shared_memory_counters.lock().await;
            lock.update(&memory_snapshot);
        }
        let cpu_snapshot = self.cgroup.read_cpu()?;
        self.cpu_counters.update(&cpu_snapshot);

        let io_snapshot = self.cgroup.read_io()?;
        self.io_counters.update(&io_snapshot);

        Ok(())
    }

    async fn retrieve(&mut self) -> Result<Self::Type> {
        let memory = {
            let mut lock = self.shared_memory_counters.lock().await;
            let memory_counters = *lock;
            lock.reset();
            memory_counters
        };

        let cpu = self.cpu_counters;
        self.cpu_counters.new_phase();

        let io = self.io_counters;
        self.io_counters.new_phase();

        Ok(Counters { memory, cpu, io })
    }

    async fn join(&mut self) -> Result<()> {
        if let Some((cancellation_token, handle)) = self.handle.take() {
            cancellation_token.cancel();
            handle.await??;
        }
        self.cgroup.cleanup()?;
        Ok(())
    }

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

    fn to_metrics(&self, counters: Self::Type) -> Result<Metrics> {
        macro_rules! push {
            ($metrics:expr, $name:expr, $value:expr, $unit:expr) => {
                $metrics.push(Metric::new($name, $value, $unit, Self::get_name()));
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
        let m = counters.memory;

        push_minmax!(metrics, m.anon, "anon", BYTE_UNIT);
        push_minmax!(metrics, m.file, "file", BYTE_UNIT);
        push_minmax!(metrics, m.kernel, "kernel", BYTE_UNIT);
        push_minmax!(metrics, m.kernel_stack, "kernel_stack", BYTE_UNIT);
        push_minmax!(metrics, m.peak, "peak", BYTE_UNIT);
        push_minmax!(metrics, m.shmem, "shmem", BYTE_UNIT);
        push_minmax!(metrics, m.slab, "slab", BYTE_UNIT);
        push_minmax!(metrics, m.swap_current, "swap_current", BYTE_UNIT);

        let cpu = counters.cpu;

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

        let io = counters.io;

        push_begin_end!(metrics, io.rbytes, "read_bytes", BYTE_UNIT);
        push_begin_end!(metrics, io.wbytes, "write_bytes", BYTE_UNIT);

        Ok(metrics)
    }

    fn get_name() -> &'static str {
        SOURCE_NAME
    }
}
