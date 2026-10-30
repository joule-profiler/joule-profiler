use std::io::ErrorKind;

use joule_profiler_core::{Info, Target};
use perf_event::{Builder, Counter, Group};

use crate::{
    counters::RawCount,
    error::{PerfEventError, Result},
    event::Event,
};

pub(crate) mod cgroup;
pub(crate) mod pid;

/// What the counters are opened on: a process or a cgroup.
#[cfg_attr(test, mockall::automock)]
#[allow(clippy::used_underscore_binding)]
pub(crate) trait PerfScope: Send + 'static {
    fn info(&self) -> Info;

    /// Opens a counter for every event, before the target is known. Used by the `cgroup` scope.
    fn open(&mut self, _events: &[Event]) -> Result<()> {
        Ok(())
    }

    /// Opens a counter for every event on the target. Used by the `pid` scope.
    fn attach(&mut self, _target: Target, _events: &[Event]) -> Result<()> {
        Ok(())
    }

    /// Reads the counters, in the order of `events`.
    fn read(&mut self, events: &[Event]) -> Result<Vec<RawCount>>;

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Returns a builder with the options common to all scopes.
fn counter_builder<'a>(event: Event) -> Builder<'a> {
    let mut builder = event.builder();
    builder
        .enabled(true)
        .include_hv()
        .include_kernel()
        .exclude_guest(false)
        .exclude_host(false);
    builder
}

/// Opens a counter, in the given `group` if any.
fn open_counter(builder: &Builder, event: Event, group: Option<&mut Counter>) -> Result<Counter> {
    let counter = match group {
        Some(group) => builder.build_with_group(group),
        None => builder.build(),
    };

    counter.map_err(|err| match err.kind() {
        ErrorKind::PermissionDenied => PerfEventError::PermissionDenied(err),
        _ => PerfEventError::OpenCounter(event, err),
    })
}

/// The group leader: the counter of the first event.
enum Leader {
    /// Reads the whole group at once.
    ReadGrouped(Group),

    /// Reads each counter separately: when `group` is disabled, or on kernels older than 4.13
    /// that cannot read inherited counters as a group.
    ReadEach(Counter),
}

impl Leader {
    fn as_counter_mut(&mut self) -> &mut Counter {
        match self {
            Self::ReadGrouped(group) => group.as_counter_mut(),
            Self::ReadEach(counter) => counter,
        }
    }
}
