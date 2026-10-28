use std::{collections::HashSet, path::PathBuf};

/// The scope to use to open counters, either the process or a cgroup.
#[derive(Debug, Default, Clone)]
pub enum ScopeType {
    /// The profiled process and its children.
    #[default]
    Pid,

    /// Every process in a cgroup v2.
    Cgroup(Cgroup),
}

/// A cgroup v2 scope.
#[derive(Debug, Clone)]
pub struct Cgroup {
    pub(crate) name: PathBuf,
    pub(crate) root: Option<PathBuf>,
    pub(crate) cpus: Option<HashSet<u32>>,
}

impl Cgroup {
    /// Creates a scope for the cgroup `name`, relative to the root.
    pub fn new(name: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            root: None,
            cpus: None,
        }
    }

    /// Sets the cgroup root. Defaults to `/sys/fs/cgroup`.
    pub fn root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    /// Sets the CPUs to count on. Defaults to every online CPU.
    pub fn cpus(mut self, cpus: impl IntoIterator<Item = u32>) -> Self {
        self.cpus = Some(cpus.into_iter().collect());
        self
    }
}

impl From<Cgroup> for ScopeType {
    fn from(cgroup: Cgroup) -> Self {
        Self::Cgroup(cgroup)
    }
}
