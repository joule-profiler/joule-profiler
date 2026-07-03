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
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_timerfd::Interval;
use tokio_util::sync::CancellationToken;

use crate::{
    backend::{Backend, ProcfsBackend},
    config::ProcfsConfig,
    counters::{Counters, compute_mem_used},
    error::ProcfsError,
    snapshot::ProcSnapshot,
    utils::make_conversion,
};

mod backend;
pub mod config;
pub mod counters;
pub mod error;
mod snapshot;
mod utils;

const IO_COUNTERS_METRIC_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::None,
    unit: Unit::Byte,
};

const PROCFS_SOURCE_NAME: &str = "procfs";

type Result<T> = std::result::Result<T, ProcfsError>;
type WorkerHandle = (CancellationToken, JoinHandle<Result<()>>);

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
#[derive(Debug)]
pub struct Procfs<B: Backend = ProcfsBackend> {
    /// procfs source configuration.
    config: ProcfsConfig,

    /// The backend used to interract with procfs. Used for testing.
    backend: Arc<B>,

    /// Current metrics counters.
    counters: Arc<Mutex<Counters>>,

    /// The handle to the polling task and it's cancellation token.
    polling_task_handle: Option<WorkerHandle>,

    /// The handle to the polling task and it's cancellation token.
    process_discovery_task_handle: Option<WorkerHandle>,

    /// Total physical memory in bytes, read once at construction from `/proc/meminfo`.
    mem_total: u64,

    /// The current processes to be measured, updated at each poll from the process discovery task.
    detected_processes: Arc<Mutex<HashSet<i32>>>,
}

impl Procfs {
    /// Creates a new `Procfs` source from the given configuration.
    ///
    /// Reads `MemTotal` from `/proc/meminfo` at construction time.
    pub fn new(config: ProcfsConfig) -> Result<Self> {
        Ok(Self {
            config,
            mem_total: 0,
            backend: Arc::new(ProcfsBackend),
            counters: Arc::default(),
            detected_processes: Arc::default(),
            polling_task_handle: None,
            process_discovery_task_handle: None,
        })
    }
}

impl<B: Backend> Procfs<B> {
    fn spawn_polling_worker(
        backend: Arc<B>,
        counters: Arc<Mutex<Counters>>,
        detected_processes: Arc<Mutex<HashSet<i32>>>,
        poll_interval: Duration,
    ) -> Result<WorkerHandle> {
        let mut ticker = Interval::new_interval(poll_interval)?;
        let cancellation_token = CancellationToken::new();
        let cancellation_token_clone = cancellation_token.clone();

        let handle = tokio::spawn(async move {
            debug!("Starting procfs polling.");

            loop {
                tokio::select! {
                    _ = ticker.next() => {
                        trace!("Polled procfs source.");
                        Self::measure_and_update_pids(&backend, &counters, &detected_processes).await?;
                    }
                    () = cancellation_token.cancelled() => {
                        debug!("procfs worker stopped.");
                        break;
                    }
                }
            }
            Ok(())
        });
        Ok((cancellation_token_clone, handle))
    }

    fn spawn_process_discovery_worker(
        backend: Arc<B>,
        pid: i32,
        detected_processes: Arc<Mutex<HashSet<i32>>>,
        poll_interval: Duration,
    ) -> Result<WorkerHandle> {
        let mut ticker = Interval::new_interval(poll_interval)?;
        let cancellation_token = CancellationToken::new();
        let cancellation_token_clone = cancellation_token.clone();

        let handle = tokio::spawn(async move {
            debug!("Starting procfs process discovery worker.");

            loop {
                tokio::select! {
                    _ = ticker.next() => {
                        trace!("Polled procfs discovery task.");
                        *detected_processes.lock().await = backend.collect_children(pid);
                    }
                    () = cancellation_token.cancelled() => {
                        debug!("procfs process discovery worker stopped.");
                        break;
                    }
                }
            }
            Ok(())
        });
        Ok((cancellation_token_clone, handle))
    }

    async fn measure_and_update_pids(
        backend: &B,
        counters: &Arc<Mutex<Counters>>,
        detected_processes: &Arc<Mutex<HashSet<i32>>>,
    ) -> Result<()> {
        let mut present_pids = HashSet::new();
        let pids = detected_processes.lock().await.clone();
        let mut snapshot = ProcSnapshot::default();
        let mut pids_updated = false;

        for pid in pids {
            match backend.read_proc(pid, &mut snapshot) {
                Ok(()) => {
                    present_pids.insert(pid);
                    Ok(())
                }
                Err(ProcfsError::Procfs(procfs::ProcError::NotFound(_))) => {
                    trace!("PID {pid} not present, removing it from processes list.");
                    pids_updated = true;
                    Ok(())
                }
                Err(ProcfsError::Procfs(procfs::ProcError::Incomplete(_))) => {
                    trace!(
                        "PID {pid} data incomplete (process exited during read), removing it from processes list."
                    );
                    pids_updated = true;
                    Ok(())
                }
                r => r,
            }?;
        }

        let global = backend.measure_global()?;
        let mut counters = counters.lock().await;
        counters.update(&snapshot, &global);

        if pids_updated {
            trace!("Updating processes list: {present_pids:?}");
            *detected_processes.lock().await = present_pids;
        }

        Ok(())
    }
}

impl<B: Backend> MetricReader for Procfs<B> {
    type Type = Counters;
    type Error = ProcfsError;
    type Config = ProcfsConfig;

    fn from_config(config: ProcfsConfig) -> Result<Self> {
        Ok(Self {
            config,
            mem_total: 0,
            backend: Arc::new(B::default()),
            counters: Arc::default(),
            detected_processes: Arc::default(),
            polling_task_handle: None,
            process_discovery_task_handle: None,
        })
    }

    /// Initializes the source to `pid` and starts the background poller if a `poll_interval` is configured.
    async fn init(&mut self, pid: i32) -> Result<()> {
        debug!("Initializing procfs source.");
        self.mem_total = self.backend.mem_total()?;
        *self.detected_processes.lock().await = self.backend.collect_children(pid);

        self.process_discovery_task_handle = Some(Self::spawn_process_discovery_worker(
            self.backend.clone(),
            pid,
            self.detected_processes.clone(),
            self.config.process_detection_poll_interval,
        )?);

        let counters = self.counters.clone();

        self.polling_task_handle = Some(Self::spawn_polling_worker(
            self.backend.clone(),
            counters,
            self.detected_processes.clone(),
            self.config.poll_interval,
        )?);

        Ok(())
    }

    /// Aborts the background poller if still running, or propagates its error if it already finished.
    async fn join(&mut self) -> Result<()> {
        debug!("Joining procfs source.");
        if let Some((cancellation_token, handle)) = self.polling_task_handle.take() {
            cancellation_token.cancel();
            handle.await??;
        }
        Ok(())
    }

    /// Takes a single snapshot of process and global counters and updates them.
    ///
    /// If the process is not found, an empty snapshot is used.
    async fn measure(&mut self) -> Result<()> {
        Self::measure_and_update_pids(&self.backend, &self.counters, &self.detected_processes)
            .await?;
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

    fn to_metrics(&self, counters: Self::Type) -> Result<Metrics> {
        let proc_unit: MetricUnit = self.config.proc_memory_unit.into();
        let global_unit: MetricUnit = self.config.global_memory_unit.into();

        let proc = counters.proc;

        let proc_memory_metrics: Metrics = [
            ("proc_vm_size_min", proc.vm_size.min().unwrap_or_default()),
            ("proc_vm_size_max", proc.vm_size.max().unwrap_or_default()),
            ("proc_rss_min", proc.rss.min().unwrap_or_default()),
            ("proc_rss_max", proc.rss.max().unwrap_or_default()),
            ("proc_pss_min", proc.pss.min().unwrap_or_default()),
            ("proc_pss_max", proc.pss.max().unwrap_or_default()),
            ("proc_shared_min", proc.shared.min().unwrap_or_default()),
            ("proc_shared_max", proc.shared.max().unwrap_or_default()),
            ("proc_anon_min", proc.anon.min().unwrap_or_default()),
            ("proc_anon_max", proc.anon.max().unwrap_or_default()),
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
        let global_memory_unit = self.config.global_memory_unit;
        let mem_used = compute_mem_used(
            self.mem_total,
            global.mem_available,
            global.mem_free,
            global.cached,
        );

        let mut global_memory_metrics: Metrics = [
            (
                "global_mem_used_min",
                make_conversion(global_memory_unit, mem_used.min().unwrap_or_default()),
            ),
            (
                "global_mem_used_max",
                make_conversion(global_memory_unit, mem_used.max().unwrap_or_default()),
            ),
            (
                "global_cached_min",
                make_conversion(global_memory_unit, global.cached.min().unwrap_or_default()),
            ),
            (
                "global_cached_max",
                make_conversion(global_memory_unit, global.cached.max().unwrap_or_default()),
            ),
            (
                "global_swap_free_min",
                make_conversion(
                    global_memory_unit,
                    global.swap_free.min().unwrap_or_default(),
                ),
            ),
            (
                "global_swap_free_max",
                make_conversion(
                    global_memory_unit,
                    global.swap_free.max().unwrap_or_default(),
                ),
            ),
        ]
        .into_iter()
        .map(|(name, value)| Metric::new(name, value, global_unit, Self::get_name()))
        .collect();

        if let Some(anon) = global.anon {
            let anon: Vec<_> = [
                ("global_anon_min", anon.min().unwrap_or_default()),
                ("global_anon_max", anon.max().unwrap_or_default()),
            ]
            .into_iter()
            .map(|(name, value)| {
                Metric::new(
                    name,
                    make_conversion(global_memory_unit, value),
                    global_unit,
                    Self::get_name(),
                )
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
        PROCFS_SOURCE_NAME
    }

    fn get_id() -> &'static str {
        PROCFS_SOURCE_NAME
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, path::PathBuf, sync::Arc, time::Duration};

    use joule_profiler_core::source::MetricReader;
    use tokio::time::sleep;

    use crate::{
        Procfs, backend::MockBackend, config::ProcfsConfig, error::ProcfsError,
        snapshot::GlobalSnapshot,
    };

    fn create_source(backend: MockBackend) -> Procfs<MockBackend> {
        let source = Procfs {
            backend: Arc::new(backend),
            config: ProcfsConfig::default(),
            mem_total: 0,
            counters: Arc::default(),
            detected_processes: Arc::default(),
            polling_task_handle: None,
            process_discovery_task_handle: None,
        };
        source
    }

    #[tokio::test]
    async fn test_init_function_initializes_field_correctly() {
        let mem_total = 4096;
        let pid = 1;
        let children: HashSet<_> = vec![pid, 2].into_iter().collect();

        let mut backend = MockBackend::new();

        backend
            .expect_collect_children()
            .returning(move |_| vec![pid, 2].into_iter().collect());

        backend
            .expect_mem_total()
            .once()
            .returning(move || Ok(mem_total));

        let mut source = create_source(backend);

        source.init(pid).await.unwrap();

        assert_eq!(source.mem_total, mem_total);
        assert!(source.detected_processes.lock().await.eq(&children));
    }

    #[tokio::test]
    async fn test_process_detection_polling_updates_processes_list() {
        let children: HashSet<_> = vec![1, 2, 3, 4].into_iter().collect();

        let mut backend = MockBackend::new();
        backend
            .expect_collect_children()
            .once()
            .returning(|_| HashSet::new());

        backend
            .expect_collect_children()
            .returning(|_| vec![1, 2, 3, 4].into_iter().collect());

        backend.expect_mem_total().once().returning(|| Ok(0));
        let mut source = create_source(backend);
        source.config.process_detection_poll_interval = Duration::from_millis(1);
        assert!(source.detected_processes.lock().await.eq(&HashSet::new()));

        source.init(1).await.unwrap();

        sleep(Duration::from_millis(5)).await;

        assert!(source.detected_processes.lock().await.eq(&children));
    }

    #[tokio::test]
    async fn test_polling_task_updates_counters() {
        let mut backend = MockBackend::new();
        backend
            .expect_collect_children()
            .returning(|_| vec![1].into_iter().collect());

        backend.expect_measure_global().once().returning(|| {
            Ok(GlobalSnapshot {
                anon: Some(100),
                cached: 200,
                mem_available: Some(300),
                mem_free: 400,
                swap_free: 500,
            })
        });

        backend.expect_read_proc().returning(|_, _| Ok(()));

        backend.expect_measure_global().once().returning(|| {
            Ok(GlobalSnapshot {
                anon: Some(200),
                cached: 400,
                mem_available: Some(600),
                mem_free: 800,
                swap_free: 1000,
            })
        });

        backend.expect_mem_total().once().returning(|| Ok(0));

        let mut source = create_source(backend);
        let global_counters = source.counters.lock().await.global;
        assert!(global_counters.anon.is_none());
        assert!(global_counters.cached.min().is_none());
        assert!(global_counters.cached.max().is_none());
        assert!(global_counters.mem_available.is_none());
        assert!(global_counters.mem_free.min().is_none());
        assert!(global_counters.mem_free.max().is_none());
        assert!(global_counters.swap_free.min().is_none());
        assert!(global_counters.swap_free.max().is_none());

        source.config.poll_interval = Duration::from_millis(10);
        source.init(1).await.unwrap();

        sleep(Duration::from_millis(20)).await;

        let global_counters = source.counters.lock().await.global;

        assert_eq!(global_counters.anon.unwrap().min(), Some(100));
        assert_eq!(global_counters.anon.unwrap().max(), Some(200));
        assert_eq!(global_counters.cached.min(), Some(200));
        assert_eq!(global_counters.cached.max(), Some(400));
        assert_eq!(global_counters.mem_available.unwrap().min(), Some(300));
        assert_eq!(global_counters.mem_available.unwrap().max(), Some(600));
        assert_eq!(global_counters.mem_free.min(), Some(400));
        assert_eq!(global_counters.mem_free.max(), Some(800));
        assert_eq!(global_counters.swap_free.min(), Some(500));
        assert_eq!(global_counters.swap_free.max(), Some(1000));
    }

    #[tokio::test]
    async fn test_measure_updates_counters_once() {
        let mut backend = MockBackend::new();

        backend
            .expect_collect_children()
            .returning(|_| vec![1].into_iter().collect());

        backend.expect_read_proc().returning(|_, _| Ok(()));

        backend.expect_measure_global().returning(|| {
            Ok(GlobalSnapshot {
                anon: Some(100),
                cached: 200,
                mem_available: Some(300),
                mem_free: 400,
                swap_free: 500,
            })
        });

        backend.expect_mem_total().once().returning(|| Ok(1000));

        let mut source = create_source(backend);

        source.init(1).await.unwrap();
        source.measure().await.unwrap();

        let counters = source.counters.lock().await;

        assert_eq!(counters.global.cached.max(), Some(200));
        assert_eq!(counters.global.cached.max(), Some(200));
        assert_eq!(counters.global.mem_available.unwrap().max(), Some(300));
        assert_eq!(counters.global.mem_free.max(), Some(400));
        assert_eq!(counters.global.swap_free.max(), Some(500));
    }

    #[tokio::test]
    async fn test_retrieve_resets_counters() {
        let mut backend = MockBackend::new();

        backend
            .expect_collect_children()
            .returning(|_| vec![1].into_iter().collect());

        backend.expect_read_proc().returning(|_, _| Ok(()));

        backend.expect_measure_global().returning(|| {
            Ok(GlobalSnapshot {
                anon: Some(10),
                cached: 20,
                mem_available: Some(30),
                mem_free: 40,
                swap_free: 50,
            })
        });

        backend.expect_mem_total().once().returning(|| Ok(1000));

        let mut source = create_source(backend);
        source.init(1).await.unwrap();

        source.measure().await.unwrap();

        let first = source.retrieve().await.unwrap();
        let second = source.retrieve().await.unwrap();

        assert!(first.global.cached.max().is_some());
        assert!(second.global.cached.max().is_none());
    }

    #[tokio::test]
    async fn test_dead_pid_removed_from_detected_processes() {
        use procfs::ProcError;

        let mut backend = MockBackend::new();

        backend
            .expect_collect_children()
            .returning(|_| vec![1, 2].into_iter().collect());

        backend.expect_read_proc().returning(|pid, _| {
            if pid == 2 {
                Err(ProcfsError::Procfs(ProcError::NotFound(Some(
                    PathBuf::new(),
                ))))
            } else {
                Ok(())
            }
        });

        backend
            .expect_measure_global()
            .returning(|| Ok(GlobalSnapshot::default()));

        backend.expect_mem_total().once().returning(|| Ok(1000));

        let mut source = create_source(backend);
        source.init(1).await.unwrap();

        source.measure().await.unwrap();

        let processes = source.detected_processes.lock().await;

        assert!(processes.contains(&1));
        assert!(!processes.contains(&2));
    }

    #[tokio::test]
    async fn test_join_stops_polling_task() {
        let mut backend = MockBackend::new();

        backend
            .expect_collect_children()
            .returning(|_| vec![1].into_iter().collect());

        backend.expect_mem_total().once().returning(|| Ok(0));

        backend.expect_read_proc().returning(|_, _| Ok(()));

        backend
            .expect_measure_global()
            .returning(|| Ok(GlobalSnapshot::default()));

        let mut source = create_source(backend);
        source.config.poll_interval = Duration::from_millis(5);
        source.init(1).await.unwrap();
        sleep(Duration::from_millis(15)).await;

        source.join().await.unwrap();
    }
}
