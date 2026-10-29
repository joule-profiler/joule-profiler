#[cfg(feature = "backend-powercap")]
use std::{path::PathBuf, time::Duration};

#[cfg(feature = "backend-powercap")]
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_mins(1);

/// How the energy counters are read.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "backend-perf", derive(Default))]
pub enum BackendType {
    /// The `power` PMU, using the `perf_event` interface.
    #[cfg(feature = "backend-perf")]
    #[cfg_attr(feature = "backend-perf", default)]
    Perf,

    /// The powercap framework using the `/sys/class/powercap` sysfs.
    #[cfg(feature = "backend-powercap")]
    Powercap(Powercap),
}

#[cfg(not(feature = "backend-perf"))]
impl Default for BackendType {
    fn default() -> Self {
        Self::Powercap(Powercap::default())
    }
}

/// Configuration of the powercap backend.
#[cfg(feature = "backend-powercap")]
#[derive(Debug, Clone)]
pub struct Powercap {
    pub(crate) root: Option<PathBuf>,
    pub(crate) poll_interval: Duration,
}

#[cfg(feature = "backend-powercap")]
impl Default for Powercap {
    fn default() -> Self {
        Self {
            root: None,
            poll_interval: DEFAULT_POLL_INTERVAL,
        }
    }
}

#[cfg(feature = "backend-powercap")]
impl Powercap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the powercap root. Defaults to `/sys/class/powercap`.
    pub fn root(mut self, root: impl Into<PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    /// Sets how often the counters are read, so that counter wraps are seen. Defaults to 1min.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }
}

#[cfg(feature = "backend-powercap")]
impl From<Powercap> for BackendType {
    fn from(powercap: Powercap) -> Self {
        Self::Powercap(powercap)
    }
}
