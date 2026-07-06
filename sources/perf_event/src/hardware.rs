use crate::{Result, error::PerfEventError, event::Event, snapshot::Snapshot};
use futures::future::try_join_all;
use log::{debug, info, trace};
use perf_event::{
    Builder, Counter, Group, ReadFormat,
    events::{Hardware, Software},
};
use std::{
    collections::HashMap,
    fs::{self, File},
    sync::Arc,
};
use tokio::task::spawn_blocking;

/// What `perf_event` counters should be scoped to.
pub enum Target {
    /// Track a single process and any children it spawns, across every CPU
    /// it runs on (via `inherit`).
    Pid(i32),
    /// Track every process inside a cgroup. Cgroup-scoped events don't
    /// support `inherit`, so this is monitored with one group per online CPU.
    Cgroup(File),
}

#[cfg_attr(test, mockall::automock)]
pub trait PerfEventHardware: Send + Default {
    fn init_counters(
        &mut self,
        events: &[Event],
        target: Target,
    ) -> impl Future<Output = Result<()>> + Send;
    fn read_snapshot(&mut self) -> impl Future<Output = Result<Snapshot>> + Send;
}

/// Per-CPU `perf_event` groups scoped to a cgroup. Kept alongside the cgroup's
/// file descriptor, which must stay open for as long as the groups are.
struct CgroupGroups {
    _cgroup_fd: Arc<File>,
    groups: HashMap<usize, Group>,
    counters: HashMap<usize, HashMap<Event, Counter>>,
}

enum State {
    /// Individual per-event counters, not yet initialized or PID-scoped.
    Pid(HashMap<Event, Counter>),
    Cgroup(CgroupGroups),
}

impl Default for State {
    fn default() -> Self {
        State::Pid(HashMap::new())
    }
}

/// Implicit CPU key used for PID-scoped counters, which have
/// no real per-CPU breakdown (`inherit` follows the process across CPUs).
const PID_CPU: usize = 0;

#[derive(Default)]
pub struct PerfEventCounters {
    state: State,
}

impl PerfEventHardware for PerfEventCounters {
    async fn init_counters(&mut self, events: &[Event], target: Target) -> Result<()> {
        match target {
            Target::Pid(pid) => self.init_pid_counters(events, pid).await,
            Target::Cgroup(cgroup_fd) => self.init_cgroup_counters(events, cgroup_fd).await,
        }
    }

    /// Reads all performance counters and returns a snapshot.
    ///
    /// Values are kept per-CPU rather than summed here: summing on every
    /// read would redo the same work for both the begin and end snapshot of
    /// each phase, whereas `PerfEvent::to_metrics` only needs to do it once,
    /// on the already-diffed deltas.
    ///
    /// An error can occur if an event counter cannot be read.
    async fn read_snapshot(&mut self) -> Result<Snapshot> {
        match &mut self.state {
            State::Pid(counters) => {
                let taken = std::mem::take(counters);
                let (metrics, restored) = spawn_blocking(move || {
                    let mut counters = taken;
                    let result = counters
                        .iter_mut()
                        .map(|(event, counter)| {
                            let value = counter
                                .read()
                                .map_err(|_| PerfEventError::ErrorReadingCounter(*event))?;
                            Ok((*event, HashMap::from([(PID_CPU, value)])))
                        })
                        .collect::<Result<HashMap<_, _>>>();
                    (result, counters)
                })
                .await?;
                *counters = restored;
                Ok(Snapshot { metrics: metrics? })
            }
            State::Cgroup(CgroupGroups {
                groups, counters, ..
            }) => {
                let taken_groups: Vec<(usize, Group)> =
                    std::mem::take(groups).into_iter().collect();

                let tasks = taken_groups.into_iter().map(|(cpu, mut group)| {
                    spawn_blocking(move || {
                        let result = group.read();
                        (cpu, group, result)
                    })
                });

                let mut metrics: HashMap<Event, HashMap<usize, u64>> = HashMap::new();
                for (cpu, group, group_data) in try_join_all(tasks).await? {
                    groups.insert(cpu, group);
                    let group_data = group_data?;
                    for (event, counter) in counters.get(&cpu).into_iter().flatten() {
                        metrics
                            .entry(*event)
                            .or_default()
                            .insert(cpu, group_data[counter]);
                    }
                }

                Ok(Snapshot { metrics })
            }
        }
    }
}

impl PerfEventCounters {
    /// Create individual counters for all configured events, opened and
    /// enabled on a blocking thread since `perf_event_open`/ioctl calls block.
    ///
    /// Each counter is built separately with `inherit(true)` and `observe_pid`,
    /// since grouped counters do not support inheritance.
    async fn init_pid_counters(&mut self, events: &[Event], pid: i32) -> Result<()> {
        let events = events.to_vec();
        debug!("Adding {} individual performance counters", events.len());

        let counters = spawn_blocking(move || {
            let mut counters = HashMap::with_capacity(events.len());
            for event in events {
                trace!("Building counter: {event:?}");
                let counter = Builder::new(Hardware::from(event))
                    .inherit(true)
                    .observe_pid(pid)
                    .include_hv()
                    .include_kernel()
                    .build()?;
                counters.insert(event, counter);
            }
            for counter in counters.values_mut() {
                trace!("Enabling counter");
                counter.enable()?;
            }
            Ok::<_, PerfEventError>(counters)
        })
        .await??;

        debug!("All perf_event counters enabled");
        self.state = State::Pid(counters);
        Ok(())
    }

    /// Creates one counter group per online CPU, each scoped to `cgroup_fd`.
    ///
    /// Cgroup-scoped events can't use `inherit` to follow a process across
    /// CPUs the way PID-scoped ones do, so every online CPU needs its own
    /// group; their per-CPU reads are only summed into a single value per
    /// event later, in `PerfEvent::to_metrics`. Each group is opened on its
    /// own blocking thread, in parallel, since building it and adding its
    /// events are all blocking `perf_event_open`/ioctl calls.
    async fn init_cgroup_counters(&mut self, events: &[Event], cgroup_fd: File) -> Result<()> {
        let cpus = list_online_cpus()?;
        let cgroup_fd = Arc::new(cgroup_fd);

        let tasks = cpus.into_iter().map(|cpu| {
            let cgroup_fd = cgroup_fd.clone();
            let events = events.to_vec();
            spawn_blocking(move || -> Result<(usize, Group, HashMap<Event, Counter>)> {
                trace!("Building cgroup-scoped group for cpu {cpu}");

                let mut group = new_group_builder()
                    .one_cpu(cpu)
                    .observe_cgroup(&cgroup_fd)
                    .build_group()?;

                let mut cpu_counters = HashMap::with_capacity(events.len());
                for event in events {
                    trace!("Adding event {event:?} on cpu {cpu}");
                    let counter = group.add(
                        Builder::new(Hardware::from(event))
                            .observe_cgroup(&cgroup_fd)
                            .one_cpu(cpu),
                    )?;
                    cpu_counters.insert(event, counter);
                }

                group.enable()?;
                Ok((cpu, group, cpu_counters))
            })
        });

        let mut groups = HashMap::new();
        let mut counters = HashMap::new();
        for result in try_join_all(tasks).await? {
            let (cpu, group, cpu_counters) = result?;
            groups.insert(cpu, group);
            counters.insert(cpu, cpu_counters);
        }

        info!("Initialized perf_event groups for {} CPU(s)", groups.len());
        self.state = State::Cgroup(CgroupGroups {
            _cgroup_fd: cgroup_fd,
            groups,
            counters,
        });
        Ok(())
    }
}

/// Reads the set of online CPU ids from sysfs (e.g. `0-2,4`).
fn list_online_cpus() -> Result<Vec<usize>> {
    let content = fs::read_to_string("/sys/devices/system/cpu/online")?;

    let mut cpus = Vec::new();
    for part in content.trim().split(',') {
        if part.is_empty() {
            continue;
        }
        if let Some((start, end)) = part.split_once('-') {
            let start: usize = start.parse()?;
            let end: usize = end.parse()?;
            cpus.extend(start..=end);
        } else {
            cpus.push(part.parse()?);
        }
    }

    cpus.sort_unstable();
    cpus.dedup();
    Ok(cpus)
}

/// A dummy group-leader event: it doesn't itself count anything, it just
/// lets real per-CPU events be attached under it via `Group::add` with
/// `ReadFormat::GROUP` so they can be read back in one syscall.
fn new_group_builder<'a>() -> Builder<'a> {
    let mut builder = Builder::new(Software::DUMMY);
    builder.read_format(
        ReadFormat::GROUP
            | ReadFormat::TOTAL_TIME_ENABLED
            | ReadFormat::TOTAL_TIME_RUNNING
            | ReadFormat::ID,
    );
    builder
}
