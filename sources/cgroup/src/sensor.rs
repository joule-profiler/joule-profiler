use std::sync::Arc;
use std::time::{Duration, Instant};

use joule_profiler_core::injector::Target;
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::util::cgroup::Cgroup;
use joule_profiler_core::util::poller::Poller;
use joule_profiler_core::util::shared::{Consumer, Producer, shared};
use joule_profiler_core::util::stats::MinMaxMean;

use crate::cgroup::{COUNTERS, MEMORY, Usage};
use crate::error::{CgroupError, Result};
use crate::layout::Layout;

/// What one cgroup reported at a boundary.
pub struct Reading {
    /// The memory polled since the previous boundary.
    pub(crate) memory: [MinMaxMean; MEMORY.len()],

    /// The counters at the boundary, in [`COUNTERS`] order.
    pub(crate) counters: [Option<u64>; COUNTERS.len()],
}

pub struct Snapshot {
    pub(crate) at: Instant,
    pub(crate) proc: Reading,

    /// The whole hierarchy, when `global`.
    pub(crate) global: Option<Reading>,
}

/// The memory polled since the last boundary.
#[derive(Default, Clone)]
struct PolledMemory {
    proc: [MinMaxMean; MEMORY.len()],
    global: [MinMaxMean; MEMORY.len()],
}

pub struct CgroupSensor {
    proc: Arc<Cgroup>,
    global: Arc<Cgroup>,
    layout: Arc<Layout>,

    poll_interval: Duration,

    polled: Option<Consumer<PolledMemory>>,
    poller: Option<Poller>,
}

impl CgroupSensor {
    pub(crate) fn new(
        proc: Arc<Cgroup>,
        global: Arc<Cgroup>,
        layout: Arc<Layout>,
        poll_interval: Duration,
    ) -> Self {
        Self {
            proc,
            global,
            layout,
            poll_interval,
            polled: None,
            poller: None,
        }
    }
}

impl Sensor for CgroupSensor {
    type Snapshot = Snapshot;
    type Error = CgroupError;

    fn name(&self) -> &'static str {
        "cgroup"
    }

    /// The cgroup may not have existed when the source was built, but it must now.
    fn init(&mut self) -> Result<()> {
        if self.proc.exists() {
            Ok(())
        } else {
            Err(CgroupError::NotFound(self.proc.path().to_path_buf()))
        }
    }

    /// Starts polling the memory, now that the target is in the cgroup.
    fn attach(&mut self, _target: Target) -> Result<()> {
        let (mut polled, consumer) = shared();
        self.polled = Some(consumer);
        let proc = Arc::clone(&self.proc);

        let global = self.layout.global.then(|| Arc::clone(&self.global));

        self.poller = Some(Poller::start(self.poll_interval, move || {
            sample(&mut polled, &proc, global.as_deref())
        }));

        Ok(())
    }

    fn measure(&mut self) -> Result<Snapshot> {
        let polled = self
            .polled
            .as_mut()
            .ok_or(CgroupError::NotAttached)?
            .consume();

        let global = if self.layout.global {
            Some(Reading {
                memory: polled.global,
                counters: self.global.counters()?,
            })
        } else {
            None
        };

        Ok(Snapshot {
            at: Instant::now(),
            proc: Reading {
                memory: polled.proc,
                counters: self.proc.counters()?,
            },
            global,
        })
    }

    fn close(&mut self) -> Result<()> {
        // Dropping the poller joins its thread.
        self.poller = None;
        Ok(())
    }
}

/// Polls the memory of the cgroup, and of the hierarchy if `global`.
fn sample(
    polled: &mut Producer<PolledMemory>,
    proc: &Cgroup,
    global: Option<&Cgroup>,
) -> Result<()> {
    // Both are read before anything is written, so a failed read drops the whole poll.
    let proc = proc.memory_usage()?;
    let global = global.map(Usage::memory_usage).transpose()?;

    polled.write(|polled| {
        fill(&mut polled.proc, proc);
        if let Some(global) = global {
            fill(&mut polled.global, global);
        }
    });

    Ok(())
}

fn fill(stats: &mut [MinMaxMean; MEMORY.len()], values: [Option<u64>; MEMORY.len()]) {
    for (stat, value) in stats.iter_mut().zip(values) {
        if let Some(value) = value {
            stat.add(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process;

    use super::*;

    fn sensor(path: &std::path::Path) -> CgroupSensor {
        let layout = Layout::expected(&[], false);

        CgroupSensor::new(
            Arc::new(Cgroup::at(path)),
            Arc::new(Cgroup::at(std::env::temp_dir())),
            Arc::new(layout),
            Duration::from_millis(10),
        )
    }

    #[test]
    fn a_cgroup_that_does_not_exist_fails_at_init() {
        let path = std::env::temp_dir().join(format!("joule-cgroup-absent-{}", process::id()));

        assert!(matches!(
            sensor(&path).init(),
            Err(CgroupError::NotFound(_))
        ));
    }
}
