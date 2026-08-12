//! Cgroup v2 management utilities.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use log::{debug, warn};

use crate::{
    Result,
    error::CgroupError,
    snapshot::{CpuSnapshot, IoSnapshot, MemorySnapshot},
    util::{is_cgroup_dir, read_flat_keyed_file, read_io_stat, read_u64_opt},
};

/// Interface for reading cgroup statistics.
pub trait CgroupBackend: Send + Sync + Clone + Default + 'static {
    /// Checks that `path` is a cgroup the backend can read, `root` being the
    /// hierarchy it belongs to.
    fn verify(&self, _path: &Path, _root: &Path) -> Result<()> {
        Ok(())
    }

    /// Initializes the backend.
    fn create(&self, path: &Path) -> Result<()>;

    /// Attaches the provided pid to the cgroup.
    fn attach_pid(&self, path: &Path, pid: i32) -> Result<()>;

    /// Cleanup the backend.
    fn cleanup(&self, path: &Path, root: &Path) -> Result<()>;

    /// Returns memory statistics.
    fn memory(&self, path: &Path) -> Result<MemorySnapshot>;

    /// Returns CPU statistics.
    fn cpu(&self, path: &Path) -> Result<CpuSnapshot>;

    /// Returns I/O statistics.
    fn io(&self, path: &Path) -> Result<IoSnapshot>;
}

/// Cgroup sysfs backend.
#[derive(Debug, Clone, Copy, Default)]
pub struct SysFsBackend;

impl SysFsBackend {
    /// Returns all PIDs attached to the cgroup.
    fn pids(path: &Path) -> Result<Vec<i32>> {
        debug!("Retrieving cgroup `{}` PIDs.", path.display());
        let path = path.join("cgroup.procs");
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(CgroupError::io(&path, err)),
        };
        Ok(content
            .lines()
            .filter_map(|l| l.trim().parse::<i32>().ok())
            .collect())
    }
}

impl CgroupBackend for SysFsBackend {
    /// Checks that the hierarchy is mounted and that the cgroup exists in it.
    fn verify(&self, path: &Path, root: &Path) -> Result<()> {
        if !is_cgroup_dir(root) {
            return Err(CgroupError::NotAvailable(root.to_path_buf()));
        }
        if !path.exists() {
            return Err(CgroupError::NotFound(path.to_path_buf()));
        }
        if !is_cgroup_dir(path) {
            return Err(CgroupError::NotAvailable(path.to_path_buf()));
        }
        Ok(())
    }

    /// Creates the cgroup directory.
    fn create(&self, path: &Path) -> Result<()> {
        debug!("Initializing cgroup at \"{}\"", path.display());
        fs::create_dir_all(path).map_err(|err| match err.kind() {
            ErrorKind::PermissionDenied => {
                CgroupError::PermissionDenied("creating a cgroup requires root privileges.")
            }
            _ => CgroupError::io(path, err),
        })?;
        Ok(())
    }

    /// Attaches a process PID to the cgroup.
    fn attach_pid(&self, path: &Path, pid: i32) -> Result<()> {
        let procs_path = path.join("cgroup.procs");
        fs::write(&procs_path, pid.to_string()).map_err(|err| match err.kind() {
            ErrorKind::PermissionDenied => CgroupError::PermissionDenied(
                "attaching a process to this cgroup requires root privileges.",
            ),
            // The kernel reports a missing `cgroup.procs` and a process that
            // died before being attached the same way, so only the file tells
            // them apart.
            ErrorKind::NotFound if !procs_path.exists() => {
                CgroupError::NotFound(path.to_path_buf())
            }
            _ => CgroupError::FailedToAttachPid {
                pid,
                path: procs_path,
                source: err,
            },
        })?;
        debug!("Attached PID {pid} to cgroup {}", path.display());
        Ok(())
    }

    /// Moves processes back to the root cgroup and removes the directory.
    fn cleanup(&self, path: &Path, root: &Path) -> Result<()> {
        if !path.exists() {
            debug!(
                "Cgroup `{}` is already gone, nothing to clean.",
                path.display()
            );
            return Ok(());
        }

        debug!("Cleaning cgroup `{}`.", path.display());

        let root_procs = root.join("cgroup.procs");
        for pid in Self::pids(path)? {
            if let Err(e) = fs::write(&root_procs, pid.to_string()) {
                warn!("Could not move PID {pid} back to root cgroup: {e}");
            }
        }
        match fs::remove_dir(path) {
            Ok(()) => debug!("Removed cgroup {}", path.display()),
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => warn!(
                "Could not remove cgroup {} (may still have live tasks): {e}",
                path.display()
            ),
        }
        Ok(())
    }

    /// Reads memory statistics from cgroup memory files.
    fn memory(&self, path: &Path) -> Result<MemorySnapshot> {
        let mut stat = read_flat_keyed_file(&path.join("memory.stat"))
            .map_err(|e| e.into_controller_error("memory", path))?;
        Ok(MemorySnapshot {
            current: read_u64_opt(&path.join("memory.current"))?,
            swap_current: read_u64_opt(&path.join("memory.swap.current"))?,
            peak: read_u64_opt(&path.join("memory.peak"))?,
            anon: stat.remove("anon"),
            file: stat.remove("file"),
            shmem: stat.remove("shmem"),
            kernel: stat.remove("kernel"),
            kernel_stack: stat.remove("kernel_stack"),
            slab: stat.remove("slab"),
        })
    }

    /// Reads CPU statistics from `cpu.stat`.
    fn cpu(&self, path: &Path) -> Result<CpuSnapshot> {
        let mut stat = read_flat_keyed_file(&path.join("cpu.stat"))
            .map_err(|e| e.into_controller_error("cpu", path))?;

        Ok(CpuSnapshot {
            usage_usec: stat
                .remove("usage_usec")
                .ok_or(CgroupError::MissingAlwaysPresentMetric("usage_usec"))?,
            user_usec: stat
                .remove("user_usec")
                .ok_or(CgroupError::MissingAlwaysPresentMetric("user_usec"))?,
            system_usec: stat
                .remove("system_usec")
                .ok_or(CgroupError::MissingAlwaysPresentMetric("system_usec"))?,
            nr_periods: stat.remove("nr_periods"),
            nr_throttled: stat.remove("nr_throttled"),
            throttled_usec: stat.remove("throttled_usec"),
            nr_bursts: stat.remove("nr_bursts"),
            burst_usec: stat.remove("burst_usec"),
        })
    }

    /// Reads I/O statistics from `io.stat`.
    fn io(&self, path: &Path) -> Result<IoSnapshot> {
        let (rbytes, wbytes) =
            read_io_stat(&path.join("io.stat")).map_err(|e| e.into_controller_error("io", path))?;
        Ok(IoSnapshot { rbytes, wbytes })
    }
}

/// Structure representing the root cgroup.
pub struct RootCgroup<B: CgroupBackend = SysFsBackend> {
    /// The path to the cgroup.
    path: PathBuf,

    /// The backend to use to query cgroup interface (used mainly for testing).
    backend: B,
}

impl RootCgroup {
    /// Builds a cgroup handle based on the provided directory.
    pub fn at(path: PathBuf) -> Self {
        Self {
            path,
            backend: SysFsBackend,
        }
    }
}

impl<B: CgroupBackend> RootCgroup<B> {
    pub fn new(path: PathBuf, backend: B) -> Self {
        Self { path, backend }
    }

    /// Builds the root cgroup and its named child in one go, using a default backend.
    pub fn build(cgroup_root: Option<PathBuf>, cgroup_name: &str) -> (Self, ChildCgroup<B>) {
        let root = Self::new(
            cgroup_root.unwrap_or_else(|| PathBuf::from(DEFAULT_CGROUP_ROOT)),
            B::default(),
        );
        let child = root.child(cgroup_name);
        (root, child)
    }

    /// Gets a child handle of the current cgroup based on its name, sharing
    /// this cgroup's backend instance.
    pub fn child(&self, name: &str) -> ChildCgroup<B> {
        ChildCgroup::new(
            self.path.join(name),
            self.path.clone(),
            self.backend.clone(),
        )
    }

    /// Get memory stats.
    pub fn memory(&self) -> Result<MemorySnapshot> {
        self.backend.memory(&self.path)
    }

    /// Get CPU stats.
    pub fn cpu(&self) -> Result<CpuSnapshot> {
        self.backend.cpu(&self.path)
    }

    /// Get I/O stats.
    pub fn io(&self) -> Result<IoSnapshot> {
        self.backend.io(&self.path)
    }

    /// Checks that the cgroup v2 hierarchy is mounted at this path.
    pub fn verify(&self) -> Result<()> {
        self.backend.verify(&self.path, &self.path)
    }
}

const DEFAULT_CGROUP_ROOT: &str = "/sys/fs/cgroup";

impl Default for RootCgroup {
    fn default() -> Self {
        Self::at(PathBuf::from(DEFAULT_CGROUP_ROOT))
    }
}

/// Structure representing a child cgroup.
pub struct ChildCgroup<B: CgroupBackend = SysFsBackend> {
    /// The path to the cgroup.
    path: PathBuf,

    /// The path to the root cgroup.
    root: PathBuf,

    /// The backend to use to query cgroup interface (used mainly for testing).
    backend: B,
}

impl<B: CgroupBackend> ChildCgroup<B> {
    pub fn new(path: PathBuf, root: PathBuf, backend: B) -> Self {
        Self {
            path,
            root,
            backend,
        }
    }

    /// Get memory stats.
    pub fn memory(&self) -> Result<MemorySnapshot> {
        self.backend.memory(&self.path)
    }

    /// Get CPU stats.
    pub fn cpu(&self) -> Result<CpuSnapshot> {
        self.backend.cpu(&self.path)
    }

    /// Get I/O stats.
    pub fn io(&self) -> Result<IoSnapshot> {
        self.backend.io(&self.path)
    }

    /// Checks that the cgroup exists in a mounted cgroup v2 hierarchy.
    pub fn verify(&self) -> Result<()> {
        self.backend.verify(&self.path, &self.root)
    }

    /// Initializes the cgroup backend.
    pub fn create(&self) -> Result<()> {
        self.backend.create(&self.path)
    }

    /// Attaches a process PID to the cgroup.
    pub fn attach_pid(&self, pid: i32) -> Result<()> {
        self.backend.attach_pid(&self.path, pid)
    }

    /// Cleanup the cgroup backend.
    pub fn cleanup(&self) -> Result<()> {
        self.backend.cleanup(&self.path, &self.root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default, Clone)]
    struct MockCgroupBackend {
        memory: MemorySnapshot,
        cpu: CpuSnapshot,
        io: IoSnapshot,
    }

    impl CgroupBackend for MockCgroupBackend {
        fn memory(&self, _path: &Path) -> Result<MemorySnapshot> {
            Ok(self.memory)
        }

        fn cpu(&self, _path: &Path) -> Result<CpuSnapshot> {
            Ok(self.cpu)
        }

        fn io(&self, _path: &Path) -> Result<IoSnapshot> {
            Ok(self.io)
        }

        fn create(&self, _path: &Path) -> Result<()> {
            Ok(())
        }

        fn attach_pid(&self, _path: &Path, _pid: i32) -> Result<()> {
            Ok(())
        }

        fn cleanup(&self, _path: &Path, _root: &Path) -> Result<()> {
            Ok(())
        }
    }

    fn mock_cgroup(name: &str, backend: MockCgroupBackend) -> ChildCgroup<MockCgroupBackend> {
        let path = PathBuf::from(name);
        ChildCgroup {
            path: path.clone(),
            root: path,
            backend,
        }
    }

    /// Builds a directory that looks like a cgroup v2 one.
    fn cgroup_dir(path: &Path) {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("cgroup.controllers"), "cpu io memory").unwrap();
    }

    #[test]
    fn verify_rejects_a_root_that_is_not_a_cgroup_hierarchy() {
        let dir = tempfile::tempdir().unwrap();
        let cgroup = dir.path().join("child");
        cgroup_dir(&cgroup);

        let err = SysFsBackend.verify(&cgroup, dir.path()).unwrap_err();

        assert!(matches!(err, CgroupError::NotAvailable(path) if path == dir.path()));
    }

    #[test]
    fn verify_rejects_a_missing_cgroup() {
        let dir = tempfile::tempdir().unwrap();
        cgroup_dir(dir.path());
        let cgroup = dir.path().join("child");

        let err = SysFsBackend.verify(&cgroup, dir.path()).unwrap_err();

        assert!(matches!(err, CgroupError::NotFound(path) if path == cgroup));
    }

    #[test]
    fn verify_rejects_a_plain_directory_inside_a_hierarchy() {
        let dir = tempfile::tempdir().unwrap();
        cgroup_dir(dir.path());
        let cgroup = dir.path().join("child");
        fs::create_dir(&cgroup).unwrap();

        let err = SysFsBackend.verify(&cgroup, dir.path()).unwrap_err();

        assert!(matches!(err, CgroupError::NotAvailable(path) if path == cgroup));
    }

    #[test]
    fn verify_accepts_a_cgroup_of_the_hierarchy() {
        let dir = tempfile::tempdir().unwrap();
        let cgroup = dir.path().join("child");
        cgroup_dir(dir.path());
        cgroup_dir(&cgroup);

        SysFsBackend.verify(&cgroup, dir.path()).unwrap();
    }

    /// Cleaning up a cgroup that the kernel already removed, or that was never
    /// created because the run failed earlier, is not a failure.
    #[test]
    fn cleanup_of_a_missing_cgroup_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        cgroup_dir(dir.path());

        SysFsBackend
            .cleanup(&dir.path().join("gone"), dir.path())
            .unwrap();
    }

    #[test]
    fn attach_pid_reports_a_missing_cgroup() {
        let dir = tempfile::tempdir().unwrap();
        let cgroup = dir.path().join("gone");

        let err = SysFsBackend.attach_pid(&cgroup, 1).unwrap_err();

        assert!(matches!(err, CgroupError::NotFound(path) if path == cgroup));
    }

    #[test]
    fn test_stats_reader_memory() {
        let backend = MockCgroupBackend {
            memory: MemorySnapshot {
                current: Some(123),
                swap_current: Some(456),
                peak: Some(789),
                anon: Some(100),
                file: Some(200),
                shmem: None,
                kernel: None,
                kernel_stack: None,
                slab: Some(300),
            },
            ..Default::default()
        };

        let cg = mock_cgroup("memory", backend);

        let stats = cg.memory().unwrap();

        assert_eq!(stats.current, Some(123));
        assert_eq!(stats.swap_current, Some(456));
        assert_eq!(stats.peak, Some(789));
        assert_eq!(stats.anon, Some(100));
        assert_eq!(stats.file, Some(200));
        assert_eq!(stats.slab, Some(300));
    }

    #[test]
    fn test_stats_reader_cpu() {
        let backend = MockCgroupBackend {
            cpu: CpuSnapshot {
                usage_usec: 1000,
                user_usec: 400,
                system_usec: 600,
                nr_periods: Some(10),
                nr_throttled: Some(2),
                throttled_usec: None,
                nr_bursts: None,
                burst_usec: None,
            },
            ..Default::default()
        };

        let cg = mock_cgroup("cpu", backend);

        let stats = cg.cpu().unwrap();

        assert_eq!(stats.usage_usec, 1000);
        assert_eq!(stats.user_usec, 400);
        assert_eq!(stats.system_usec, 600);
        assert_eq!(stats.nr_periods, Some(10));
        assert_eq!(stats.nr_throttled, Some(2));
    }

    #[test]
    fn test_stats_reader_io() {
        let backend = MockCgroupBackend {
            io: IoSnapshot {
                rbytes: Some(120),
                wbytes: Some(80),
            },
            ..Default::default()
        };

        let cg = mock_cgroup("io", backend);

        let stats = cg.io().unwrap();

        assert_eq!(stats.rbytes, Some(120));
        assert_eq!(stats.wbytes, Some(80));
    }
}
