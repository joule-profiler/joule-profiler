use std::{
    collections::HashSet,
    fs::File,
    io::ErrorKind,
    iter,
    path::{Path, PathBuf},
};

use joule_profiler_core::{
    info::Info,
    util::sys::{cpu_list, read_cpu_list},
};
use perf_event::{Builder, Counter, ReadFormat};

use crate::{
    counters::RawCount,
    error::{PerfEventError, Result},
    event::Event,
    scope::{Leader, PerfScope, counter_builder, open_counter},
};

const ONLINE_CPUS_PATH: &str = "/sys/devices/system/cpu/online";

/// Counts every process in a cgroup, on each selected CPU.
pub(crate) struct CgroupScope {
    path: PathBuf,
    cpus: Option<HashSet<u32>>,
    group: bool,
    opened: Option<Vec<CpuCounters>>,
}

impl CgroupScope {
    pub(crate) fn new(path: PathBuf, cpus: Option<HashSet<u32>>, group: bool) -> Self {
        Self {
            path,
            cpus,
            group,
            opened: None,
        }
    }
}

impl PerfScope for CgroupScope {
    fn info(&self) -> Info {
        Info::new()
            .with("scope", "cgroup")
            .with("cgroup", self.path.display().to_string())
            .with(
                "cpus",
                resolve_cpus(self.cpus.as_ref()).map(|cpus| cpu_list(&cpus)),
            )
    }

    fn open(&mut self, events: &[Event]) -> Result<()> {
        let cgroup = File::open(&self.path)
            .map_err(|error| PerfEventError::OpenCgroup(self.path.clone(), error))?;

        let groups = resolve_cpus(self.cpus.as_ref())?
            .into_iter()
            .map(|cpu| CpuCounters::open(cpu, &cgroup, events, self.group))
            .collect::<Result<_>>()?;

        self.opened = Some(groups);
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        self.opened = None;
        Ok(())
    }

    fn read(&mut self, events: &[Event]) -> Result<Vec<RawCount>> {
        let opened = self.opened.as_mut().ok_or(PerfEventError::NotOpened)?;
        let mut totals = vec![RawCount::default(); events.len()];

        for cpu_counters in opened {
            cpu_counters.add_to_totals(&mut totals, events)?;
        }

        Ok(totals)
    }
}

struct CpuCounters {
    cpu: u32,
    leader: Leader,
    counters: Vec<Counter>,
}

impl CpuCounters {
    /// Opens a counter for each event on the given `cpu`.
    fn open(cpu: u32, cgroup: &File, events: &[Event], group: bool) -> Result<Self> {
        let (first, others) = events.split_first().ok_or(PerfEventError::NoEvent)?;

        let mut leader_builder = cgroup_counter_builder(*first, cpu, cgroup);
        let mut leader = if group {
            Leader::ReadGrouped(
                leader_builder
                    .read_format(
                        ReadFormat::GROUP
                            | ReadFormat::ID
                            | ReadFormat::TOTAL_TIME_ENABLED
                            | ReadFormat::TOTAL_TIME_RUNNING,
                    )
                    .build_group()
                    .map_err(|err| match err.kind() {
                        ErrorKind::PermissionDenied => PerfEventError::PermissionDenied(err),
                        _ => PerfEventError::OpenGroup(cpu, err),
                    })?,
            )
        } else {
            Leader::ReadEach(open_counter(&leader_builder, *first, None)?)
        };

        let mut group_leader = group.then(|| leader.as_counter_mut());

        let counters = others
            .iter()
            .map(|event| {
                let builder = cgroup_counter_builder(*event, cpu, cgroup);
                open_counter(&builder, *event, group_leader.as_deref_mut())
            })
            .collect::<Result<_>>()?;

        Ok(Self {
            cpu,
            leader,
            counters,
        })
    }

    /// Reads the counters and adds their values to `totals`.
    fn add_to_totals(&mut self, totals: &mut [RawCount], events: &[Event]) -> Result<()> {
        match &mut self.leader {
            Leader::ReadGrouped(group) => {
                let data = group
                    .read()
                    .map_err(|error| PerfEventError::ReadGroup(self.cpu, error))?;
                let (enabled, running) = (data.time_enabled(), data.time_running());
                let counters = iter::once(group.as_counter()).chain(&self.counters);

                for (total, counter) in totals.iter_mut().zip(counters) {
                    *total += RawCount::new(data[counter], enabled, running);
                }
            }

            Leader::ReadEach(leader) => {
                let counters = iter::once(leader).chain(&mut self.counters);

                for ((total, counter), event) in totals.iter_mut().zip(counters).zip(events) {
                    *total += counter
                        .read_full()
                        .map(RawCount::from)
                        .map_err(|error| PerfEventError::ReadCounter(*event, error))?;
                }
            }
        }

        Ok(())
    }
}

/// Returns a builder that counts `event` on `cpu`, for the processes of `cgroup` only.
fn cgroup_counter_builder(event: Event, cpu: u32, cgroup: &File) -> Builder<'_> {
    let mut builder = counter_builder(event);
    builder.one_cpu(cpu as usize).observe_cgroup(cgroup);
    builder
}

/// Returns the requested CPUs, or every online CPU, sorted. Fails if a requested CPU is offline.
fn resolve_cpus(cpus_spec: Option<&HashSet<u32>>) -> Result<Vec<u32>> {
    let path = Path::new(ONLINE_CPUS_PATH);
    let online_cpus =
        read_cpu_list(path).map_err(|error| PerfEventError::Read(path.into(), error))?;

    let Some(cpus_spec) = cpus_spec else {
        return Ok(online_cpus);
    };

    if let Some(offline_cpus) = cpus_spec.iter().find(|cpu| !online_cpus.contains(cpu)) {
        return Err(PerfEventError::OfflineCpu(*offline_cpus));
    }

    let mut cpus: Vec<u32> = cpus_spec.iter().copied().collect();
    cpus.sort_unstable();
    Ok(cpus)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cpu_that_is_not_online_is_refused() {
        let result = resolve_cpus(Some(&HashSet::from([u32::MAX])));

        assert!(matches!(result, Err(PerfEventError::OfflineCpu(u32::MAX))));
    }

    #[test]
    fn every_online_cpu_is_counted_by_default() {
        let online = read_cpu_list(Path::new(ONLINE_CPUS_PATH)).unwrap();

        assert_eq!(resolve_cpus(None).unwrap(), online);
    }

    #[test]
    fn the_requested_cpus_come_out_sorted() {
        let online = resolve_cpus(None).unwrap();
        let requested: HashSet<u32> = online.iter().copied().collect();

        assert_eq!(resolve_cpus(Some(&requested)).unwrap(), online);
    }
}
