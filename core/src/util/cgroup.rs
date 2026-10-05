//! Cgroup v2 helpers.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

pub const ROOT: &str = "/sys/fs/cgroup";

const PROCS: &str = "cgroup.procs";
const CONTROLLERS: &str = "cgroup.controllers";
const SUBTREE: &str = "cgroup.subtree_control";

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

    pub fn move_processes_to(&self, other: &Self) {
        let Ok(processes) = self.processes() else {
            return;
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
        Ok(words(&self.path.join(CONTROLLERS))?.unwrap_or_default())
    }

    /// The controllers enabled for its children.
    pub fn handed_down(&self) -> io::Result<Vec<String>> {
        Ok(words(&self.path.join(SUBTREE))?.unwrap_or_default())
    }

    /// Enables `controller` for its children.
    pub fn hand_down(&self, controller: &str) -> io::Result<()> {
        self.subtree(&format!("+{controller}"))
    }

    /// Disables `controller` for its children.
    pub fn take_back(&self, controller: &str) -> io::Result<()> {
        self.subtree(&format!("-{controller}"))
    }

    fn subtree(&self, change: &str) -> io::Result<()> {
        fs::write(self.path.join(SUBTREE), change)
    }

    /// A file holding a single number, such as `memory.current`. `None` if it does not exist.
    pub fn read_u64(&self, file: &str) -> io::Result<Option<u64>> {
        Ok(self
            .read(file)?
            .and_then(|content| content.trim().parse().ok()))
    }

    /// A `key value` file, such as `cpu.stat`. Empty if it does not exist.
    pub fn keyed(&self, file: &str) -> io::Result<HashMap<String, u64>> {
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

    /// A missing file is `None`: that is how a disabled controller looks.
    pub fn read(&self, file: &str) -> io::Result<Option<String>> {
        match fs::read_to_string(self.path.join(file)) {
            Ok(content) => Ok(Some(content)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn words(path: &Path) -> io::Result<Option<Vec<String>>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(
            content.split_whitespace().map(ToOwned::to_owned).collect(),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
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
}

/// The cgroup made for a run: both the builder and the `[cgroup]` table.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CgroupConfig {
    parent: PathBuf,
    name: String,
    controllers: Option<Vec<String>>,
    attach: bool,
}

impl Default for CgroupConfig {
    fn default() -> Self {
        Self {
            parent: PathBuf::from(ROOT),
            name: format!("joule-profiler-{}", std::process::id()),
            controllers: None,
            attach: true,
        }
    }
}

impl CgroupConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parent(mut self, parent: impl Into<PathBuf>) -> Self {
        self.parent = parent.into();
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Controllers that must be enabled. Fails if one is not available.
    pub fn controllers(mut self, controllers: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.controllers = Some(controllers.into_iter().map(Into::into).collect());
        self
    }

    /// Whether the profiled program is moved into it.
    pub fn attach(mut self, attach: bool) -> Self {
        self.attach = attach;
        self
    }

    pub fn path(&self) -> PathBuf {
        self.parent.join(&self.name)
    }

    /// Enables the controllers on the parent, then creates the cgroup. An existing one is used
    /// as is and kept afterwards.
    pub fn create(self) -> Result<RunCgroup, CgroupError> {
        let parent = Cgroup::at(&self.parent);
        if !parent.is_cgroup() {
            return Err(CgroupError::NotACgroup(self.parent));
        }

        let read = |file: &str, error| CgroupError::Read(parent.path().join(file), error);
        let available = parent.controllers().map_err(|e| read(CONTROLLERS, e))?;
        let handed_down = parent.handed_down().map_err(|e| read(SUBTREE, e))?;

        // Built first so that its `Drop` undoes any change if a later step fails.
        let mut run = RunCgroup {
            cgroup: parent.child(&self.name),
            parent,
            attach: self.attach,
            made: false,
            handed: Vec::new(),
        };

        let required = self.controllers.is_some();
        let wanted = self
            .controllers
            .unwrap_or_else(|| RUN_CONTROLLERS.map(String::from).to_vec());

        for controller in wanted {
            if handed_down.contains(&controller) {
                continue;
            }

            match run.hand_down(&controller, &available) {
                Ok(()) => run.handed.push(controller),
                Err(error) if !required => log::warn!("{error}"),
                Err(error) => return Err(error),
            }
        }

        if run.cgroup.exists() {
            log::debug!(
                "using the cgroup {}, which exists already",
                run.path().display()
            );
        } else {
            run.cgroup.create().map_err(|error| CgroupError::Create {
                path: run.cgroup.path().to_path_buf(),
                error,
            })?;
            run.made = true;
        }

        Ok(run)
    }
}

/// A cgroup made for a run. Dropping it undoes what [`CgroupConfig::create`] did.
#[derive(Debug)]
pub struct RunCgroup {
    cgroup: Cgroup,
    parent: Cgroup,
    attach: bool,
    made: bool,
    handed: Vec<String>,
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

    fn hand_down(&self, controller: &str, available: &[String]) -> Result<(), CgroupError> {
        if !available.iter().any(|name| name == controller) {
            return Err(CgroupError::Unavailable {
                controller: controller.to_owned(),
                path: self.parent.path().to_path_buf(),
            });
        }

        self.parent
            .hand_down(controller)
            .map_err(|error| CgroupError::HandDown {
                controller: controller.to_owned(),
                path: self.parent.path().to_path_buf(),
                error,
            })
    }
}

impl Drop for RunCgroup {
    fn drop(&mut self) {
        if self.made {
            // The root is the only cgroup that can hold processes and enable controllers at once.
            self.cgroup.move_processes_to(&Cgroup::at(ROOT));

            if let Err(error) = self.cgroup.remove() {
                log::warn!("removing {} failed: {error}", self.cgroup.path.display());
            }
        }

        for controller in &self.handed {
            if let Err(error) = self.parent.take_back(controller) {
                log::warn!(
                    "taking the `{controller}` controller back from {} failed: {error}",
                    self.parent.path.display()
                );
            }
        }
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
    fn a_cgroup_is_only_a_path_until_it_is_asked_for_something() {
        let cgroup = fake("absent");

        assert!(!cgroup.exists());
        assert_eq!(cgroup.read("memory.current").unwrap(), None);
        assert_eq!(cgroup.controllers().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn creating_and_removing_one() {
        let cgroup = fake("lifecycle");

        cgroup.create().unwrap();
        assert!(cgroup.exists());

        cgroup.remove().unwrap();
        assert!(!cgroup.exists());
    }

    #[test]
    fn a_keyed_file_is_read_as_its_pairs() {
        let cgroup = fake("keyed");
        cgroup.create().unwrap();
        fs::write(
            cgroup.path().join("cpu.stat"),
            "usage_usec 42\nuser_usec 30\nnr_periods 0\n",
        )
        .unwrap();

        let stat = cgroup.keyed("cpu.stat").unwrap();

        assert_eq!(stat.get("usage_usec"), Some(&42));
        assert_eq!(stat.get("user_usec"), Some(&30));
        assert_eq!(stat.get("nothing"), None);

        fs::remove_dir_all(cgroup.path()).unwrap();
    }

    #[test]
    fn a_missing_controller_reads_as_nothing_rather_than_failing() {
        let cgroup = fake("no-controller");
        cgroup.create().unwrap();

        assert!(cgroup.keyed("memory.stat").unwrap().is_empty());
        assert_eq!(cgroup.read_u64("memory.current").unwrap(), None);

        fs::remove_dir_all(cgroup.path()).unwrap();
    }

    #[test]
    fn processes_are_read_as_numbers() {
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
    fn a_run_cgroup_is_made_with_what_its_parent_can_hand_down_and_taken_away_after() {
        let parent = fake_parent("run-parent", "cpu memory", "memory");

        let run = CgroupConfig::new()
            .parent(parent.path())
            .name("a-run")
            .create()
            .unwrap();

        assert!(run.path().is_dir());
        assert_eq!(
            run.handed,
            ["cpu"],
            "memory was handed down already, io is not there"
        );
        assert_eq!(
            fs::read_to_string(parent.path().join(SUBTREE)).unwrap(),
            "+cpu"
        );

        drop(run);

        assert!(!parent.child("a-run").exists());
        assert_eq!(
            fs::read_to_string(parent.path().join(SUBTREE)).unwrap(),
            "-cpu",
            "what was handed down is taken back"
        );
        fs::remove_dir_all(parent.path()).unwrap();
    }

    #[test]
    fn a_controller_asked_for_by_name_has_to_be_there() {
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
    fn a_cgroup_that_exists_already_is_used_and_left_in_place() {
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
    fn a_parent_that_is_not_a_cgroup_is_refused() {
        let parent = fake("run-plain");
        parent.create().unwrap();

        assert!(matches!(
            CgroupConfig::new().parent(parent.path()).create(),
            Err(CgroupError::NotACgroup(_))
        ));
        fs::remove_dir_all(parent.path()).unwrap();
    }

    #[test]
    fn a_child_hangs_off_its_parent() {
        let cgroup = Cgroup::at("/sys/fs/cgroup/one");

        assert_eq!(
            cgroup.child("two").path(),
            Path::new("/sys/fs/cgroup/one/two")
        );
    }
}
