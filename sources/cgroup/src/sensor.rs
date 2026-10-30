use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use joule_profiler_core::{
    Sensor, Target,
    util::{
        cgroup::Cgroup,
        poller::Poller,
        shared::{Consumer, Producer, shared},
        stats::MinMaxMean,
    },
};

use crate::{
    cgroup::{CPU_COUNTERS, IO_COUNTERS, MEMORY, Usage},
    error::{CgroupError, Result},
    layout::Layout,
};

/// What one cgroup reported at a boundary.
pub(crate) struct Reading {
    /// The memory polled since the previous boundary.
    pub(crate) memory: [MinMaxMean; MEMORY.len()],

    /// The counters of `cpu.stat` at the boundary.
    pub(crate) cpu: [Option<u64>; CPU_COUNTERS.len()],

    /// The counters of `io.stat` at the boundary.
    pub(crate) io: [Option<u64>; IO_COUNTERS.len()],
}

/// What the sensor reads at a boundary.
pub(crate) struct Snapshot {
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

/// Reads the CPU time and I/O of the cgroup at each boundary, and polls its memory.
pub(crate) struct CgroupSensor {
    proc: Arc<Cgroup>,
    global: Arc<Cgroup>,
    layout: Arc<Layout>,

    process_poll_interval: Duration,
    global_poll_interval: Duration,

    memory_reader: Option<Consumer<PolledMemory>>,
    poller: Option<Poller<CgroupError>>,
}

impl CgroupSensor {
    pub(crate) fn new(
        proc: Arc<Cgroup>,
        global: Arc<Cgroup>,
        layout: Arc<Layout>,
        process_poll_interval: Duration,
        global_poll_interval: Duration,
    ) -> Self {
        Self {
            proc,
            global,
            layout,
            process_poll_interval,
            global_poll_interval,
            memory_reader: None,
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

    /// Checks that the cgroup exists and that its memory, and that of the root cgroup when
    /// `global`, can be read.
    fn init(&mut self) -> Result<()> {
        if !self.proc.exists() {
            return Err(CgroupError::NotFound(self.proc.path().to_path_buf()));
        }

        self.proc.read_memory_usage()?;
        if self.layout.global.is_some() {
            self.global.read_memory_usage()?;
        }

        Ok(())
    }

    /// Starts polling the memory, now that the target is in the cgroup.
    fn attach(&mut self, _target: Target) -> Result<()> {
        let (memory_writer, memory_reader) = shared();
        self.memory_reader = Some(memory_reader);

        let now = Instant::now();
        let mut memory_poller = MemoryPoller {
            proc: Arc::clone(&self.proc),
            process_poll_interval: self.process_poll_interval,
            next_process_poll: now + self.process_poll_interval,
            global: self
                .layout
                .global
                .is_some()
                .then(|| Arc::clone(&self.global)),
            global_poll_interval: self.global_poll_interval,
            next_global_poll: now + self.global_poll_interval,
            memory_writer,
        };

        let first_wait = memory_poller.next_poll().saturating_duration_since(now);
        self.poller = Some(Poller::start_scheduled(first_wait, move || {
            memory_poller.poll()
        }));
        Ok(())
    }

    fn measure(&mut self) -> Result<Snapshot> {
        let polled = self
            .memory_reader
            .as_mut()
            .ok_or(CgroupError::NotAttached)?
            .consume();

        let global = if self.layout.global.is_some() {
            Some(Reading {
                memory: polled.global,
                cpu: self.global.read_cpu_counters()?,
                io: self.global.read_io_counters()?,
            })
        } else {
            None
        };

        Ok(Snapshot {
            at: Instant::now(),
            proc: Reading {
                memory: polled.proc,
                cpu: self.proc.read_cpu_counters()?,
                io: self.proc.read_io_counters()?,
            },
            global,
        })
    }

    fn close(&mut self) -> Result<()> {
        if let Some(poller) = self.poller.take() {
            poller.stop()?;
        }
        Ok(())
    }
}

/// Polls the memory of the cgroup, and of the root cgroup when `global`, each at its own interval.
struct MemoryPoller {
    proc: Arc<Cgroup>,
    process_poll_interval: Duration,
    next_process_poll: Instant,
    global: Option<Arc<Cgroup>>,
    global_poll_interval: Duration,
    next_global_poll: Instant,
    memory_writer: Producer<PolledMemory>,
}

impl MemoryPoller {
    /// Reads the memory that is due, adds it to the stats of the phase, and returns how long to
    /// wait for the next poll.
    fn poll(&mut self) -> Result<Duration> {
        let now = Instant::now();

        let proc_memory = if now >= self.next_process_poll {
            self.next_process_poll = now + self.process_poll_interval;
            Some(self.proc.read_memory_usage()?)
        } else {
            None
        };

        let global_memory = match &self.global {
            Some(global) if now >= self.next_global_poll => {
                self.next_global_poll = now + self.global_poll_interval;
                Some(global.read_memory_usage()?)
            }
            _ => None,
        };

        if proc_memory.is_some() || global_memory.is_some() {
            self.memory_writer.write(|polled| {
                if let Some(memory) = proc_memory {
                    add_to_stats(&mut polled.proc, memory);
                }
                if let Some(memory) = global_memory {
                    add_to_stats(&mut polled.global, memory);
                }
            });
        }

        Ok(self.next_poll().saturating_duration_since(Instant::now()))
    }

    /// Returns the nearest deadline among the polls.
    fn next_poll(&self) -> Instant {
        if self.global.is_some() {
            self.next_process_poll.min(self.next_global_poll)
        } else {
            self.next_process_poll
        }
    }
}

fn add_to_stats(stats: &mut [MinMaxMean; MEMORY.len()], values: [Option<u64>; MEMORY.len()]) {
    for (stat, value) in stats.iter_mut().zip(values) {
        if let Some(value) = value {
            stat.add(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process;

    use joule_profiler_core::MemoryUnit;

    use super::*;

    fn sensor(path: &std::path::Path) -> CgroupSensor {
        let layout = Layout {
            proc: crate::layout::AvailableValues::expected(&[]),
            global: None,
            process_memory_unit: MemoryUnit::default(),
            global_memory_unit: MemoryUnit::default(),
        };

        CgroupSensor::new(
            Arc::new(Cgroup::at(path)),
            Arc::new(Cgroup::at(std::env::temp_dir())),
            Arc::new(layout),
            Duration::from_millis(10),
            Duration::from_millis(10),
        )
    }

    #[test]
    fn missing_cgroup_fails_init() {
        let path = std::env::temp_dir().join(format!("joule-cgroup-absent-{}", process::id()));

        assert!(matches!(
            sensor(&path).init(),
            Err(CgroupError::NotFound(_))
        ));
    }

    #[test]
    fn waits_nearest_deadline() {
        let absent = Arc::new(Cgroup::at(std::env::temp_dir().join("joule-cgroup-none")));
        let (memory_writer, _memory_reader) = shared();
        let now = Instant::now();
        let mut memory_poller = MemoryPoller {
            proc: Arc::clone(&absent),
            process_poll_interval: Duration::from_secs(10),
            next_process_poll: now,
            global: Some(absent),
            global_poll_interval: Duration::from_secs(1),
            next_global_poll: now,
            memory_writer,
        };

        let wait = memory_poller.poll().unwrap();

        assert!(
            wait > Duration::from_millis(900) && wait <= Duration::from_secs(1),
            "{wait:?}"
        );
    }
}
