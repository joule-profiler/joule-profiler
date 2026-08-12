use std::{
    io::ErrorKind,
    num::ParseIntError,
    path::{Path, PathBuf},
};

use thiserror::Error;
use tokio::task::JoinError;

use crate::util::{is_cgroup_dir, read_controllers};

/// Main error type for all cgroup-related operations.
#[derive(Debug, Error)]
pub enum CgroupError {
    /// cgroup v2 is not available or not mounted as a unified hierarchy.
    #[error(
        "`{0}` is not a cgroup v2 directory. Check that the unified hierarchy is mounted \
        (`mount -t cgroup2`) and that `cgroup_root` points to it."
    )]
    NotAvailable(PathBuf),

    /// The cgroup directory does not exist.
    #[error(
        "Cgroup `{0}` does not exist. Create it beforehand or let the source do it \
        with `create_cgroup = true`."
    )]
    NotFound(PathBuf),

    /// Attempted to create a cgroup that already exists.
    #[error("Cgroup with name \"{0}\" already created.")]
    AlreadyCreated(String),

    /// Failed to attach a PID to a cgroup.
    #[error("Failed to attach PID `{pid}` to cgroup on path `{path}`. Cause: {source}")]
    FailedToAttachPid {
        pid: i32,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A cgroup controller is not enabled in `cgroup.subtree_control`.
    #[error(
        "Cgroup controller `{controller}` is not enabled for the parent cgroup. \
        Enable it with: echo \"+{controller}\" | sudo tee {path}/cgroup.subtree_control"
    )]
    ControllerNotEnabled {
        controller: &'static str,
        path: PathBuf,
    },

    /// A cgroup controller is not available in the hierarchy at all.
    #[error(
        "Cgroup controller `{controller}` is not available in `{path}`. \
        The kernel was built without it or it is not delegated to this hierarchy."
    )]
    ControllerNotAvailable {
        controller: &'static str,
        path: PathBuf,
    },

    /// A metric expected to always exist in kernel stats was missing.
    #[error("Missing always present metric `{0}`")]
    MissingAlwaysPresentMetric(&'static str),

    /// I/O error tied to a specific file path.
    #[error("I/O error on `{path}`: {source}")]
    IoPath {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Failed to create the timer driving the background memory polling.
    ///
    /// Not built from [`From`] on purpose: every I/O error of the source must
    /// name the file or the operation it comes from.
    #[error("Failed to create the cgroup polling timer: {0}")]
    Timer(#[source] std::io::Error),

    /// Failed to parse a numeric value from a cgroup file.
    #[error("Failed to parse `{path}`: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: ParseIntError,
    },

    #[error("Permission denied. Cause: {0}")]
    PermissionDenied(&'static str),

    /// Tokio task join error (async execution failure).
    #[error("Failed to join tokio task: {0}")]
    JoinError(
        #[from]
        #[source]
        JoinError,
    ),

    #[error("cgroup source mutex poisoned.")]
    MutexPoisoned,
}

impl CgroupError {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> Self {
        CgroupError::IoPath {
            path: path.to_path_buf(),
            source,
        }
    }

    pub(crate) fn into_controller_error(self, controller: &'static str, cgroup: &Path) -> Self {
        let CgroupError::IoPath { ref source, .. } = self else {
            return self;
        };

        if source.kind() != ErrorKind::NotFound {
            return self;
        }

        if !cgroup.exists() {
            return CgroupError::NotFound(cgroup.to_path_buf());
        }

        if !is_cgroup_dir(cgroup) {
            return CgroupError::NotAvailable(cgroup.to_path_buf());
        }

        let parent = cgroup.parent().filter(|parent| is_cgroup_dir(parent));
        let scope = parent.unwrap_or(cgroup);

        let Ok(available) = read_controllers(&scope.join("cgroup.controllers")) else {
            return self;
        };

        if !available.iter().any(|name| name == controller) {
            return CgroupError::ControllerNotAvailable {
                controller,
                path: scope.to_path_buf(),
            };
        }

        let Some(parent) = parent else {
            return self;
        };

        let Ok(enabled) = read_controllers(&parent.join("cgroup.subtree_control")) else {
            return self;
        };

        if enabled.iter().any(|name| name == controller) {
            return self;
        }

        CgroupError::ControllerNotEnabled {
            controller,
            path: parent.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// Builds a directory that looks like a cgroup v2 one.
    fn cgroup_dir(path: &Path, available: &str, enabled: &str) {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("cgroup.controllers"), available).unwrap();
        fs::write(path.join("cgroup.subtree_control"), enabled).unwrap();
    }

    fn not_found(path: &Path) -> CgroupError {
        CgroupError::io(path, ErrorKind::NotFound.into())
    }

    #[test]
    fn keeps_errors_that_are_not_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let err = CgroupError::io(dir.path(), ErrorKind::PermissionDenied.into());

        let err = err.into_controller_error("memory", dir.path());

        assert!(matches!(err, CgroupError::IoPath { .. }));
    }

    #[test]
    fn reports_a_missing_cgroup() {
        let dir = tempfile::tempdir().unwrap();
        let cgroup = dir.path().join("gone");

        let err = not_found(&cgroup.join("memory.stat")).into_controller_error("memory", &cgroup);

        assert!(matches!(err, CgroupError::NotFound(path) if path == cgroup));
    }

    #[test]
    fn reports_a_directory_that_is_not_a_cgroup() {
        let dir = tempfile::tempdir().unwrap();

        let err =
            not_found(&dir.path().join("memory.stat")).into_controller_error("memory", dir.path());

        assert!(matches!(err, CgroupError::NotAvailable(path) if path == dir.path()));
    }

    #[test]
    fn reports_a_controller_missing_from_the_hierarchy() {
        let dir = tempfile::tempdir().unwrap();
        let cgroup = dir.path().join("child");
        cgroup_dir(dir.path(), "cpu io", "cpu io");
        cgroup_dir(&cgroup, "cpu io", "");

        let err = not_found(&cgroup.join("memory.stat")).into_controller_error("memory", &cgroup);

        assert!(
            matches!(err, CgroupError::ControllerNotAvailable { controller, path }
                if controller == "memory" && path == dir.path())
        );
    }

    #[test]
    fn reports_a_controller_the_parent_does_not_delegate() {
        let dir = tempfile::tempdir().unwrap();
        let cgroup = dir.path().join("child");
        cgroup_dir(dir.path(), "cpu io memory", "cpu");
        cgroup_dir(&cgroup, "cpu", "");

        let err = not_found(&cgroup.join("memory.stat")).into_controller_error("memory", &cgroup);

        assert!(
            matches!(err, CgroupError::ControllerNotEnabled { controller, path }
                if controller == "memory" && path == dir.path())
        );
    }

    /// The root of a hierarchy has no parent to enable controllers for it, so
    /// nothing more precise than the failing read can be reported.
    #[test]
    fn keeps_the_io_error_at_the_hierarchy_root() {
        let dir = tempfile::tempdir().unwrap();
        cgroup_dir(dir.path(), "cpu io memory", "cpu io memory");

        let err =
            not_found(&dir.path().join("memory.stat")).into_controller_error("memory", dir.path());

        assert!(matches!(err, CgroupError::IoPath { .. }));
    }

    /// A controller enabled by the parent but whose file is still missing is
    /// not something the source can explain: the original error is kept.
    #[test]
    fn keeps_the_io_error_when_nothing_explains_it() {
        let dir = tempfile::tempdir().unwrap();
        let cgroup = dir.path().join("child");
        cgroup_dir(dir.path(), "cpu io memory", "cpu io memory");
        cgroup_dir(&cgroup, "cpu io memory", "");

        let err = not_found(&cgroup.join("memory.stat")).into_controller_error("memory", &cgroup);

        assert!(matches!(err, CgroupError::IoPath { .. }));
    }
}
