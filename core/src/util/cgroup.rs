//! Cgroup v2 helpers.

use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

use crate::error::ErrorChain;

pub const ROOT_PATH: &str = "/sys/fs/cgroup";

const PROCS: &str = "cgroup.procs";
const CONTROLLERS: &str = "cgroup.controllers";
const SUBTREE: &str = "cgroup.subtree_control";
const FREEZE: &str = "cgroup.freeze";

/// The controllers a run cgroup gets when none are configured.
const RUN_CONTROLLERS: [&str; 3] = ["cpu", "memory", "io"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cgroup {
    path: PathBuf,
}

impl Cgroup {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn child(&self, name: impl AsRef<Path>) -> Self {
        Self::at(self.path.join(name))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.is_dir()
    }

    /// Returns whether it is a cgroup v2.
    pub fn is_cgroup(&self) -> bool {
        self.path.join(CONTROLLERS).is_file()
    }

    pub fn create(&self) -> io::Result<()> {
        fs::create_dir_all(&self.path)
    }

    pub fn remove(&self) -> io::Result<()> {
        fs::remove_dir(&self.path)
    }

    pub fn attach(&self, pid: i32) -> io::Result<()> {
        fs::write(self.path.join(PROCS), pid.to_string())
    }

    pub fn processes(&self) -> io::Result<Vec<i32>> {
        Ok(fs::read_to_string(self.path.join(PROCS))?
            .lines()
            .filter_map(|pid| pid.trim().parse().ok())
            .collect())
    }

    /// Moves every process in it to `other`. A failure is logged.
    pub fn move_processes_to(&self, other: &Self) {
        let processes = match self.processes() {
            Ok(processes) => processes,
            Err(error) => {
                log::warn!("reading the processes of {}: {error}", self.path.display());
                return;
            }
        };

        for pid in processes {
            if let Err(error) = other.attach(pid) {
                log::warn!(
                    "moving pid {pid} to {} failed: {error}",
                    other.path.display()
                );
            }
        }
    }

    pub fn controllers(&self) -> io::Result<Vec<String>> {
        Ok(split_words(&self.path.join(CONTROLLERS))?.unwrap_or_default())
    }

    /// Returns the controllers enabled for its children.
    pub fn handed_down(&self) -> io::Result<Vec<String>> {
        Ok(split_words(&self.path.join(SUBTREE))?.unwrap_or_default())
    }

    /// Enables `controller` for its children.
    pub fn enable_controller(&self, controller: &str) -> io::Result<()> {
        self.write_subtree(&format!("+{controller}"))
    }

    /// Disables `controller` for its children.
    pub fn disable_controller(&self, controller: &str) -> io::Result<()> {
        self.write_subtree(&format!("-{controller}"))
    }

    fn write_subtree(&self, content: &str) -> io::Result<()> {
        fs::write(self.path.join(SUBTREE), content)
    }

    /// Freezes or thaws every process in it. Freezing is not immediate: `frozen` in
    /// `cgroup.events` says when it is done.
    pub fn set_frozen(&self, frozen: bool) -> io::Result<()> {
        fs::write(self.path.join(FREEZE), if frozen { "1" } else { "0" })
    }

    /// Reads a file holding a single number, such as `memory.current`. Returns `None` if it
    /// does not exist.
    pub fn read_u64(&self, file: &str) -> io::Result<Option<u64>> {
        Ok(self
            .read(file)?
            .and_then(|content| content.trim().parse().ok()))
    }

    /// Reads a file of `key value` lines, such as `cpu.stat`. Returns an empty map if it does
    /// not exist.
    pub fn read_keyed(&self, file: &str) -> io::Result<HashMap<String, u64>> {
        let Some(content) = self.read(file)? else {
            return Ok(HashMap::new());
        };

        Ok(content
            .lines()
            .filter_map(|line| {
                let (key, value) = line.split_once(' ')?;
                Some((key.to_owned(), value.trim().parse().ok()?))
            })
            .collect())
    }

    /// Reads one of its files. Returns `None` if it does not exist, as for a disabled
    /// controller.
    pub fn read(&self, file: &str) -> io::Result<Option<String>> {
        match fs::read_to_string(self.path.join(file)) {
            Ok(content) => Ok(Some(content)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[derive(Debug, Error)]
pub enum CgroupError {
    #[error("{0} is not a cgroup v2 directory: is the unified hierarchy mounted there?")]
    NotACgroup(PathBuf),

    #[error("reading {0}")]
    Read(PathBuf, #[source] io::Error),

    #[error(
        "the `{controller}` controller is not available in {path}: its own parent does not hand it down"
    )]
    Unavailable { controller: String, path: PathBuf },

    #[error(
        "handing the `{controller}` controller down from {path}: this needs root, or a delegated subtree whose processes all live in its children"
    )]
    HandDown {
        controller: String,
        path: PathBuf,
        #[source]
        error: io::Error,
    },

    #[error("creating the cgroup {path}: this needs root, or a delegated subtree")]
    Create {
        path: PathBuf,
        #[source]
        error: io::Error,
    },

    #[error("moving pid {pid} into {path}: this needs root, or a delegated subtree")]
    Attach {
        pid: i32,
        path: PathBuf,
        #[source]
        error: io::Error,
    },

    #[error("freezing {path}: this needs Linux 5.2, and root or a delegated subtree")]
    Freeze {
        path: PathBuf,
        #[source]
        error: io::Error,
    },
}

/// The cgroup made for a run: both the builder and the `[cgroup]` table.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CgroupConfig {
    parent: PathBuf,
    name: String,
    controllers: Option<Vec<String>>,
    attach: bool,
    freeze: bool,
}

impl Default for CgroupConfig {
    fn default() -> Self {
        Self {
            parent: PathBuf::from(ROOT_PATH),
            name: format!("joule-profiler-{}", std::process::id()),
            controllers: None,
            attach: true,
            freeze: false,
        }
    }
}

impl CgroupConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the cgroup the run cgroup is created in. Defaults to `/sys/fs/cgroup`.
    pub fn parent(mut self, parent: impl Into<PathBuf>) -> Self {
        self.parent = parent.into();
        self
    }

    /// Sets the name of the run cgroup. Defaults to `joule-profiler-<pid>`.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Sets the controllers that must be enabled: one that cannot be fails the creation.
    /// Defaults to `cpu`, `memory` and `io`, each left out with a warning if it cannot be.
    pub fn controllers(mut self, controllers: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.controllers = Some(controllers.into_iter().map(Into::into).collect());
        self
    }

    /// Sets whether the profiled program is moved into the cgroup. Defaults to `true`.
    pub fn attach(mut self, attach: bool) -> Self {
        self.attach = attach;
        self
    }

    /// Sets whether the cgroup is frozen before the program starts, and thawed once every
    /// source is ready. Defaults to `false`.
    pub fn freeze(mut self, freeze: bool) -> Self {
        self.freeze = freeze;
        self
    }

    pub fn path(&self) -> PathBuf {
        self.parent.join(&self.name)
    }

    /// Enables the controllers on the parent, then creates the cgroup. An existing one is used
    /// as is and kept afterwards.
    ///
    /// # Errors
    ///
    /// Returns [`CgroupError::NotACgroup`] if the parent is not a cgroup v2,
    /// [`CgroupError::Unavailable`] or [`CgroupError::HandDown`] if a required controller cannot
    /// be enabled, and [`CgroupError::Create`] if the cgroup cannot be created.
    pub fn create(self) -> Result<RunCgroup, CgroupError> {
        let parent = Cgroup::at(&self.parent);
        if !parent.is_cgroup() {
            return Err(CgroupError::NotACgroup(self.parent));
        }

        let read_failed = |file: &str, error| CgroupError::Read(parent.path().join(file), error);
        let available = parent
            .controllers()
            .map_err(|error| read_failed(CONTROLLERS, error))?;
        let handed_down = parent
            .handed_down()
            .map_err(|error| read_failed(SUBTREE, error))?;

        // Built first so that its `Drop` undoes any change if a later step fails.
        let mut run_cgroup = RunCgroup {
            cgroup: parent.child(&self.name),
            parent,
            attach: self.attach,
            freeze: self.freeze,
            created: false,
            controllers: Vec::new(),
        };

        let required = self.controllers.is_some();
        let wanted = self
            .controllers
            .unwrap_or_else(|| RUN_CONTROLLERS.map(String::from).to_vec());

        for controller in wanted {
            if handed_down.contains(&controller) {
                continue;
            }

            match run_cgroup.enable_controller(&controller, &available) {
                Ok(()) => run_cgroup.controllers.push(controller),
                Err(error) if !required => log::warn!("{}", ErrorChain(&error)),
                Err(error) => return Err(error),
            }
        }

        if run_cgroup.cgroup.exists() {
            log::debug!(
                "using the cgroup {}, which exists already",
                run_cgroup.path().display()
            );
        } else {
            run_cgroup
                .cgroup
                .create()
                .map_err(|error| CgroupError::Create {
                    path: run_cgroup.cgroup.path().to_path_buf(),
                    error,
                })?;
            run_cgroup.created = true;
        }

        Ok(run_cgroup)
    }
}

/// A cgroup made for a run. Dropping it undoes what [`CgroupConfig::create`] did.
#[derive(Debug)]
pub struct RunCgroup {
    cgroup: Cgroup,
    parent: Cgroup,
    attach: bool,
    freeze: bool,
    created: bool,
    controllers: Vec<String>,
}

impl RunCgroup {
    pub fn path(&self) -> &Path {
        self.cgroup.path()
    }

    /// Moves the profiled program into it, unless `attach` is false.
    pub fn attach(&self, pid: i32) -> Result<(), CgroupError> {
        if !self.attach {
            return Ok(());
        }

        self.cgroup
            .attach(pid)
            .map_err(|error| CgroupError::Attach {
                pid,
                path: self.cgroup.path().to_path_buf(),
                error,
            })
    }

    /// Freezes it before the program is started, unless `freeze` is false. The program is then
    /// frozen as soon as it is moved in.
    pub fn freeze(&self) -> Result<(), CgroupError> {
        self.set_frozen(true)
    }

    /// Thaws it once every source is ready, unless `freeze` is false.
    pub fn thaw(&self) -> Result<(), CgroupError> {
        self.set_frozen(false)
    }

    fn set_frozen(&self, frozen: bool) -> Result<(), CgroupError> {
        if !self.freeze {
            return Ok(());
        }

        self.cgroup
            .set_frozen(frozen)
            .map_err(|error| CgroupError::Freeze {
                path: self.cgroup.path().to_path_buf(),
                error,
            })
    }

    /// Enables `controller` on the parent, if it is among the `available` ones.
    fn enable_controller(&self, controller: &str, available: &[String]) -> Result<(), CgroupError> {
        if !available.iter().any(|name| name == controller) {
            return Err(CgroupError::Unavailable {
                controller: controller.to_owned(),
                path: self.parent.path().to_path_buf(),
            });
        }

        self.parent
            .enable_controller(controller)
            .map_err(|error| CgroupError::HandDown {
                controller: controller.to_owned(),
                path: self.parent.path().to_path_buf(),
                error,
            })
    }
}

impl Drop for RunCgroup {
    fn drop(&mut self) {
        // A run that failed before the sources were ready leaves it frozen.
        if let Err(error) = self.thaw() {
            log::warn!("{}", ErrorChain(&error));
        }

        if self.created {
            // The root is the only cgroup that can hold processes and enable controllers at once.
            self.cgroup.move_processes_to(&Cgroup::at(ROOT_PATH));

            if let Err(error) = self.cgroup.remove() {
                log::warn!("removing {} failed: {error}", self.cgroup.path.display());
            }
        }

        for controller in &self.controllers {
            if let Err(error) = self.parent.disable_controller(controller) {
                log::warn!(
                    "taking the `{controller}` controller back from {} failed: {error}",
                    self.parent.path.display()
                );
            }
        }
    }
}

/// Reads the words of a file. Returns `None` if it does not exist.
fn split_words(path: &Path) -> io::Result<Option<Vec<String>>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(
            content.split_whitespace().map(ToOwned::to_owned).collect(),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake(name: &str) -> Cgroup {
        let path = std::env::temp_dir().join(format!("joule-cg-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);

        Cgroup::at(path)
    }

    #[test]
    fn absent_cgroup_reads_nothing() {
        let cgroup = fake("absent");

        assert!(!cgroup.exists());
        assert_eq!(cgroup.read("memory.current").unwrap(), None);
        assert_eq!(cgroup.controllers().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn read_keyed() {
        let cgroup = fake("keyed");
        cgroup.create().unwrap();
        fs::write(
            cgroup.path().join("cpu.stat"),
            "usage_usec 42\nuser_usec 30\nnr_periods 0\n",
        )
        .unwrap();

        let stat = cgroup.read_keyed("cpu.stat").unwrap();

        assert_eq!(stat.get("usage_usec"), Some(&42));
        assert_eq!(stat.get("user_usec"), Some(&30));
        assert_eq!(stat.get("nothing"), None);

        fs::remove_dir_all(cgroup.path()).unwrap();
    }

    #[test]
    fn missing_controller_reads_none() {
        let cgroup = fake("no-controller");
        cgroup.create().unwrap();

        assert!(cgroup.read_keyed("memory.stat").unwrap().is_empty());
        assert_eq!(cgroup.read_u64("memory.current").unwrap(), None);

        fs::remove_dir_all(cgroup.path()).unwrap();
    }

    #[test]
    fn processes() {
        let cgroup = fake("procs");
        cgroup.create().unwrap();
        fs::write(cgroup.path().join(PROCS), "12\n34\n\n").unwrap();

        assert_eq!(cgroup.processes().unwrap(), vec![12, 34]);

        fs::remove_dir_all(cgroup.path()).unwrap();
    }

    fn fake_parent(name: &str, available: &str, handed: &str) -> Cgroup {
        let parent = fake(name);
        parent.create().unwrap();
        fs::write(parent.path().join(CONTROLLERS), available).unwrap();
        fs::write(parent.path().join(SUBTREE), handed).unwrap();

        parent
    }

    #[test]
    fn run_cgroup_enables_and_restores_controllers() {
        let parent = fake_parent("run-parent", "cpu memory", "memory");

        let run = CgroupConfig::new()
            .parent(parent.path())
            .name("a-run")
            .create()
            .unwrap();

        assert!(run.path().is_dir());
        assert_eq!(run.controllers, ["cpu"]);
        assert_eq!(
            fs::read_to_string(parent.path().join(SUBTREE)).unwrap(),
            "+cpu"
        );

        drop(run);

        assert!(!parent.child("a-run").exists());
        assert_eq!(
            fs::read_to_string(parent.path().join(SUBTREE)).unwrap(),
            "-cpu"
        );
        fs::remove_dir_all(parent.path()).unwrap();
    }

    #[test]
    fn required_controller_must_exist() {
        let parent = fake_parent("run-required", "cpu memory", "");

        let error = CgroupConfig::new()
            .parent(parent.path())
            .name("a-run")
            .controllers(["io"])
            .create()
            .unwrap_err();

        assert!(matches!(error, CgroupError::Unavailable { controller, .. } if controller == "io"));
        assert!(!parent.child("a-run").exists());
        fs::remove_dir_all(parent.path()).unwrap();
    }

    #[test]
    fn existing_cgroup_is_kept() {
        let parent = fake_parent("run-existing", "memory", "memory");
        parent.child("there").create().unwrap();

        let run = CgroupConfig::new()
            .parent(parent.path())
            .name("there")
            .create()
            .unwrap();
        drop(run);

        assert!(parent.child("there").exists());
        fs::remove_dir_all(parent.path()).unwrap();
    }

    #[test]
    fn freeze_and_thaw() {
        let parent = fake_parent("run-freeze", "", "");
        let freeze = parent.child("a-run").path().join(FREEZE);

        let run = CgroupConfig::new()
            .parent(parent.path())
            .name("a-run")
            .create()
            .unwrap();
        run.freeze().unwrap();
        assert!(!freeze.exists());
        drop(run);

        let mut run = CgroupConfig::new()
            .parent(parent.path())
            .name("a-run")
            .freeze(true)
            .create()
            .unwrap();
        run.freeze().unwrap();
        assert_eq!(fs::read_to_string(&freeze).unwrap(), "1");

        // Kept in place so that what `drop` wrote can be read.
        run.created = false;
        drop(run);
        assert_eq!(fs::read_to_string(&freeze).unwrap(), "0");
        fs::remove_dir_all(parent.path()).unwrap();
    }

    #[test]
    fn parent_must_be_cgroup() {
        let parent = fake("run-plain");
        parent.create().unwrap();

        assert!(matches!(
            CgroupConfig::new().parent(parent.path()).create(),
            Err(CgroupError::NotACgroup(_))
        ));
        fs::remove_dir_all(parent.path()).unwrap();
    }
}
