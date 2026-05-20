use crate::error::CgroupError;
use crate::snapshot::Snapshot;
use crate::util::{read_flat_keyed, read_io_stat, read_u64_opt};
use futures::StreamExt;
use joule_profiler_core::sensor::{Sensor, Sensors};
use joule_profiler_core::source::MetricReader;
use joule_profiler_core::types::{Metric, Metrics};
use joule_profiler_core::unit::{MetricUnit, Unit, UnitPrefix};
use log::{debug, trace, warn};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_timerfd::Interval;

mod error;
mod snapshot;
mod util;

const SOURCE_NAME: &str = "CGroup";

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

#[derive(Debug, Clone)]
pub struct CgroupConfig {
    pub cgroup_root: PathBuf,
    pub cgroup_name: String,
    pub poll_interval: Option<Duration>,
}

impl Default for CgroupConfig {
    fn default() -> Self {
        Self {
            cgroup_root: PathBuf::from("/sys/fs/cgroup"),
            cgroup_name: format!("joule-profiler-{}", std::process::id()),
            poll_interval: None,
        }
    }
}

pub struct CgroupSource {
    config: CgroupConfig,
    cgroup_path: PathBuf,
    shared_snapshot: Arc<Mutex<Snapshot>>,
    handle: Option<JoinHandle<Result<(), CgroupError>>>,
    last_ponctual: Snapshot,
}

impl CgroupSource {
    pub fn new(config: CgroupConfig) -> Result<Self, CgroupError> {
        let controllers = config.cgroup_root.join("cgroup.controllers");
        if !controllers.exists() {
            return Err(CgroupError::NotAvailable(
                config.cgroup_root.display().to_string(),
            ));
        }

        let cgroup_path = config.cgroup_root.join(&config.cgroup_name);

        Ok(Self {
            config,
            cgroup_path,
            shared_snapshot: Arc::new(Mutex::new(Snapshot::default())),
            handle: None,
            last_ponctual: Snapshot::default(),
        })
    }

    pub fn try_default() -> Result<Self, CgroupError> {
        Self::new(CgroupConfig::default())
    }

    fn create_and_enable_controllers(&self) -> Result<(), CgroupError> {
        if !self.cgroup_path.exists() {
            fs::create_dir_all(&self.cgroup_path).map_err(|e| CgroupError::IoPath {
                path: self.cgroup_path.display().to_string(),
                source: e,
            })?;
            debug!("Created cgroup at {}", self.cgroup_path.display());
        }

        let subtree_control = self.config.cgroup_root.join("cgroup.subtree_control");
        for controller in ["memory", "cpu", "io"] {
            let token = format!("+{controller}");
            let controller_static: &'static str = match controller {
                "memory" => "memory",
                "cpu" => "cpu",
                "io" => "io",
                _ => unreachable!(),
            };
            fs::write(&subtree_control, &token).map_err(|e| CgroupError::EnableController {
                controller: controller_static,
                path: subtree_control.display().to_string(),
                source: e,
            })?;
            trace!(
                "Enabled controller `{controller}` in {}",
                subtree_control.display()
            );
        }

        Ok(())
    }

    fn attach_pid(&self, pid: i32) -> Result<(), CgroupError> {
        let procs_path = self.cgroup_path.join("cgroup.procs");
        fs::write(&procs_path, pid.to_string())
            .map_err(|e| CgroupError::Attach { pid, source: e })?;
        debug!(
            "Attached PID {pid} to cgroup {}",
            self.cgroup_path.display()
        );
        Ok(())
    }

    fn live_pids(&self) -> Vec<i32> {
        let path = self.cgroup_path.join("cgroup.procs");
        match fs::read_to_string(&path) {
            Ok(raw) => raw
                .lines()
                .filter_map(|l| l.trim().parse::<i32>().ok())
                .collect(),
            Err(e) => {
                warn!("Could not read cgroup.procs ({}): {e}", path.display());
                Vec::new()
            }
        }
    }

    fn cleanup(&self) {
        let root_procs = self.config.cgroup_root.join("cgroup.procs");
        for pid in self.live_pids() {
            if let Err(e) = fs::write(&root_procs, pid.to_string()) {
                warn!("Could not move PID {pid} back to root cgroup: {e}");
            }
        }
        if self.cgroup_path.exists() {
            if let Err(e) = fs::remove_dir(&self.cgroup_path) {
                warn!(
                    "Could not remove cgroup {} (may still have live tasks): {e}",
                    self.cgroup_path.display()
                );
            } else {
                debug!("Removed cgroup {}", self.cgroup_path.display());
            }
        }
    }

    fn read_memory_fields(cgroup_path: &Path) -> (u64, u64, u64, u64) {
        let memory_current = read_u64_opt(&cgroup_path.join("memory.current"))
            .ok()
            .flatten()
            .unwrap_or(0);

        let memory_swap_current = read_u64_opt(&cgroup_path.join("memory.swap.current"))
            .ok()
            .flatten()
            .unwrap_or(0);

        let (memory_anon, memory_file) =
            if let Ok(stat) = read_flat_keyed(&cgroup_path.join("memory.stat")) {
                (
                    stat.get("anon").copied().unwrap_or(0),
                    stat.get("file").copied().unwrap_or(0),
                )
            } else {
                (0, 0)
            };

        (
            memory_current,
            memory_anon,
            memory_file,
            memory_swap_current,
        )
    }

    fn read_not_polled_fields(&self) -> Snapshot {
        let mut snap = Snapshot::default();

        if let Ok(stat) = read_flat_keyed(&self.cgroup_path.join("memory.stat")) {
            snap.memory_kernel_stack = stat.get("kernel_stack").copied();
            snap.memory_slab = stat.get("slab").copied();
        }

        if let Ok(stat) = read_flat_keyed(&self.cgroup_path.join("cpu.stat")) {
            snap.cpu_usage_usec = stat.get("usage_usec").copied();
            snap.cpu_user_usec = stat.get("user_usec").copied();
            snap.cpu_system_usec = stat.get("system_usec").copied();
            snap.cpu_nr_periods = stat.get("nr_periods").copied();
            snap.cpu_nr_throttled = stat.get("nr_throttled").copied();
            snap.cpu_throttled_usec = stat.get("throttled_usec").copied();
        }

        if let Ok((rb, wb)) = read_io_stat(&self.cgroup_path.join("io.stat")) {
            snap.io_rbytes = Some(rb);
            snap.io_wbytes = Some(wb);
        }

        snap.memory_peak = read_u64_opt(&self.cgroup_path.join("memory.peak"))
            .ok()
            .flatten();

        snap
    }
}

impl MetricReader for CgroupSource {
    type Type = Snapshot;
    type Error = CgroupError;

    async fn init(&mut self, pid: i32) -> Result<(), Self::Error> {
        self.create_and_enable_controllers()?;
        self.attach_pid(pid)?;

        if let Some(interval) = self.config.poll_interval {
            let shared = self.shared_snapshot.clone();
            let cgroup_path = self.cgroup_path.clone();

            let handle = tokio::spawn(async move {
                debug!("Starting cgroup memory polling.");
                let mut ticker = Interval::new_interval(interval)?;

                loop {
                    ticker.next().await;

                    let (memory_current, memory_anon, memory_file, memory_swap_current) =
                        CgroupSource::read_memory_fields(&cgroup_path);

                    trace!(
                        "cgroup poll — current:{memory_current} anon:{memory_anon} \
                         file:{memory_file} swap:{memory_swap_current}"
                    );

                    let mut snap = shared.lock().await;
                    snap.update(
                        memory_current,
                        memory_anon,
                        memory_file,
                        memory_swap_current,
                    );
                }
            });

            self.handle = Some(handle);
        }

        debug!("CgroupSource initialised for PID {pid}");
        Ok(())
    }

    async fn join(&mut self) -> Result<(), Self::Error> {
        if let Some(handle) = self.handle.take() {
            if handle.is_finished() {
                if let Ok(res) = handle.await {
                    return res;
                }
            } else {
                handle.abort();
            }
        }
        self.cleanup();
        Ok(())
    }

    async fn measure(&mut self) -> Result<(), Self::Error> {
        if self.config.poll_interval.is_none() {
            let (memory_current, memory_anon, memory_file, memory_swap_current) =
                CgroupSource::read_memory_fields(&self.cgroup_path);
            let mut snap = self.shared_snapshot.lock().await;
            snap.update(
                memory_current,
                memory_anon,
                memory_file,
                memory_swap_current,
            );
        }

        self.last_ponctual = self.read_not_polled_fields();
        Ok(())
    }

    async fn retrieve(&mut self) -> Result<Self::Type, Self::Error> {
        let mut shared = self.shared_snapshot.lock().await;
        shared.remove_sentinel_values();

        let mut snap = shared.clone();
        shared.reset_phase();
        drop(shared);

        snap.memory_peak = self.last_ponctual.memory_peak;
        snap.memory_kernel_stack = self.last_ponctual.memory_kernel_stack;
        snap.memory_slab = self.last_ponctual.memory_slab;
        snap.cpu_usage_usec = self.last_ponctual.cpu_usage_usec;
        snap.cpu_user_usec = self.last_ponctual.cpu_user_usec;
        snap.cpu_system_usec = self.last_ponctual.cpu_system_usec;
        snap.cpu_nr_periods = self.last_ponctual.cpu_nr_periods;
        snap.cpu_nr_throttled = self.last_ponctual.cpu_nr_throttled;
        snap.cpu_throttled_usec = self.last_ponctual.cpu_throttled_usec;
        snap.io_rbytes = self.last_ponctual.io_rbytes;
        snap.io_wbytes = self.last_ponctual.io_wbytes;

        Ok(snap)
    }

    fn get_sensors(&self) -> Result<Sensors, Self::Error> {
        let sensors = vec![
            Sensor::new("memory_current_min", BYTE_UNIT, Self::get_name()),
            Sensor::new("memory_current_max", BYTE_UNIT, Self::get_name()),
            Sensor::new("memory_anon_min", BYTE_UNIT, Self::get_name()),
            Sensor::new("memory_anon_max", BYTE_UNIT, Self::get_name()),
            Sensor::new("memory_file_min", BYTE_UNIT, Self::get_name()),
            Sensor::new("memory_file_max", BYTE_UNIT, Self::get_name()),
            Sensor::new("memory_swap_current_min", BYTE_UNIT, Self::get_name()),
            Sensor::new("memory_swap_current_max", BYTE_UNIT, Self::get_name()),
            Sensor::new("memory_peak", BYTE_UNIT, Self::get_name()),
            Sensor::new("memory_kernel_stack", BYTE_UNIT, Self::get_name()),
            Sensor::new("memory_slab", BYTE_UNIT, Self::get_name()),
            Sensor::new("cpu_usage_usec", MICRO_SECOND_UNIT, Self::get_name()),
            Sensor::new("cpu_user_usec", MICRO_SECOND_UNIT, Self::get_name()),
            Sensor::new("cpu_system_usec", MICRO_SECOND_UNIT, Self::get_name()),
            Sensor::new("cpu_nr_periods", COUNT_UNIT, Self::get_name()),
            Sensor::new("cpu_nr_throttled", COUNT_UNIT, Self::get_name()),
            Sensor::new("cpu_throttled_usec", MICRO_SECOND_UNIT, Self::get_name()),
            Sensor::new("io_rbytes", BYTE_UNIT, Self::get_name()),
            Sensor::new("io_wbytes", BYTE_UNIT, Self::get_name()),
        ];
        Ok(sensors)
    }

    fn to_metrics(&self, snap: Self::Type) -> Result<Metrics, Self::Error> {
        let mut metrics = Vec::new();

        macro_rules! push {
            ($field:expr, $name:expr, $unit:expr) => {
                if let Some(v) = $field {
                    metrics.push(Metric::new($name, v, $unit, Self::get_name()));
                }
            };
            (direct $field:expr, $name:expr, $unit:expr) => {
                metrics.push(Metric::new($name, $field, $unit, Self::get_name()));
            };
        }

        push!(direct snap.memory_current_min,      "memory_current_min",      BYTE_UNIT);
        push!(direct snap.memory_current_max,      "memory_current_max",      BYTE_UNIT);
        push!(direct snap.memory_anon_min,         "memory_anon_min",         BYTE_UNIT);
        push!(direct snap.memory_anon_max,         "memory_anon_max",         BYTE_UNIT);
        push!(direct snap.memory_file_min,         "memory_file_min",         BYTE_UNIT);
        push!(direct snap.memory_file_max,         "memory_file_max",         BYTE_UNIT);
        push!(direct snap.memory_swap_current_min, "memory_swap_current_min", BYTE_UNIT);
        push!(direct snap.memory_swap_current_max, "memory_swap_current_max", BYTE_UNIT);
        push!(snap.memory_peak, "memory_peak", BYTE_UNIT);
        push!(snap.memory_kernel_stack, "memory_kernel_stack", BYTE_UNIT);
        push!(snap.memory_slab, "memory_slab", BYTE_UNIT);
        push!(snap.cpu_usage_usec, "cpu_usage_usec", MICRO_SECOND_UNIT);
        push!(snap.cpu_user_usec, "cpu_user_usec", MICRO_SECOND_UNIT);
        push!(snap.cpu_system_usec, "cpu_system_usec", MICRO_SECOND_UNIT);
        push!(snap.cpu_nr_periods, "cpu_nr_periods", COUNT_UNIT);
        push!(snap.cpu_nr_throttled, "cpu_nr_throttled", COUNT_UNIT);
        push!(
            snap.cpu_throttled_usec,
            "cpu_throttled_usec",
            MICRO_SECOND_UNIT
        );
        push!(snap.io_rbytes, "io_rbytes", BYTE_UNIT);
        push!(snap.io_wbytes, "io_wbytes", BYTE_UNIT);

        Ok(metrics)
    }

    fn get_name() -> &'static str {
        SOURCE_NAME
    }
}

impl Drop for CgroupSource {
    fn drop(&mut self) {
        if self.cgroup_path.exists() {
            self.cleanup();
        }
    }
}
