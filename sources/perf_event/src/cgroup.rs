use std::collections::HashSet;
use std::fs::File;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use joule_profiler_core::info::Info;
use joule_profiler_core::util::sys::{cpu_list, read_cpu_list};
use perf_event::events::{Hardware, Software};
use perf_event::{Builder, Counter, Group, ReadFormat};

use crate::error::{PerfEventError, Result};
use crate::event::Event;
use crate::scope::PerfScope;

const ONLINE_CPUS: &str = "/sys/devices/system/cpu/online";
pub(crate) const CGROUP_ROOT: &str = "/sys/fs/cgroup";

struct Opened {
    _cgroup: File,
    groups: Vec<CpuGroup>,
}

struct CpuGroup {
    cpu: u32,
    group: Group,
    counters: Vec<Counter>,
}

pub struct CgroupScope {
    events: Vec<Event>,
    path: PathBuf,
    cpus: Option<HashSet<u32>>,
    opened: Option<Opened>,
}

impl CgroupScope {
    pub(crate) fn new(events: Vec<Event>, path: PathBuf, cpus: Option<HashSet<u32>>) -> Self {
        Self {
            events,
            path,
            cpus,
            opened: None,
        }
    }
}

impl CgroupScope {
    /// The cgroup and its CPUs. An offline CPU is reported here.
    pub(crate) fn info(&self) -> Info {
        Info::new()
            .with("scope", "cgroup")
            .with("cgroup", self.path.display().to_string())
            .with(
                "cpus",
                resolve_cpus(self.cpus.as_ref()).map(|cpus| cpu_list(&cpus)),
            )
    }
}

impl PerfScope for CgroupScope {
    fn events(&self) -> &[Event] {
        &self.events
    }

    fn open(&mut self) -> Result<()> {
        let cgroup =
            File::open(&self.path).map_err(|e| PerfEventError::OpenCgroup(self.path.clone(), e))?;

        let mut groups = Vec::new();

        for cpu in resolve_cpus(self.cpus.as_ref())? {
            let mut group = Builder::new(Software::DUMMY)
                .read_format(ReadFormat::GROUP)
                .one_cpu(cpu as usize)
                .observe_cgroup(&cgroup)
                .include_hv()
                .include_kernel()
                .exclude_guest(false)
                .exclude_host(false)
                .build_group()
                .map_err(|e| match e.kind() {
                    ErrorKind::PermissionDenied => PerfEventError::PermissionDenied(e),
                    _ => PerfEventError::OpenGroup(cpu, e),
                })?;

            let mut counters = Vec::with_capacity(self.events.len());

            for event in &self.events {
                let counter = Builder::new(Hardware::from(*event))
                    .one_cpu(cpu as usize)
                    .observe_cgroup(&cgroup)
                    .include_hv()
                    .include_kernel()
                    .exclude_guest(false)
                    .exclude_host(false)
                    .build_with_group(&mut group)
                    .map_err(|e| PerfEventError::OpenCounter(*event, e))?;

                counters.push(counter);
            }

            group.enable().map_err(PerfEventError::Switch)?;
            groups.push(CpuGroup {
                cpu,
                group,
                counters,
            });
        }

        log::debug!("perf_event: {} cpu groups opened", groups.len());

        self.opened = Some(Opened {
            _cgroup: cgroup,
            groups,
        });

        Ok(())
    }

    fn read(&mut self) -> Result<Vec<u64>> {
        let opened = self.opened.as_mut().ok_or(PerfEventError::NotOpened)?;
        let mut raw = vec![0; self.events.len()];

        for cpu in &mut opened.groups {
            let data = cpu
                .group
                .read()
                .map_err(|e| PerfEventError::ReadGroup(cpu.cpu, e))?;

            for (counter, value) in cpu.counters.iter().zip(raw.iter_mut()) {
                *value += data.get(counter).map_or(0, |entry| entry.value());
            }
        }

        Ok(raw)
    }

    fn close(&mut self) -> Result<()> {
        if let Some(opened) = &mut self.opened {
            for cpu in &mut opened.groups {
                cpu.group.disable().map_err(PerfEventError::Switch)?;
            }
        }

        self.opened = None;
        Ok(())
    }
}

/// The wanted CPUs, or every online one, sorted.
fn resolve_cpus(wanted: Option<&HashSet<u32>>) -> Result<Vec<u32>> {
    let path = Path::new(ONLINE_CPUS);
    let online = read_cpu_list(path).map_err(|e| PerfEventError::Read(path.into(), e))?;

    let mut cpus: Vec<u32> = match wanted {
        Some(wanted) => {
            if let Some(offline) = wanted.iter().find(|cpu| !online.contains(*cpu)) {
                return Err(PerfEventError::OfflineCpu(*offline));
            }

            wanted.iter().copied().collect()
        }
        None => online,
    };

    cpus.sort_unstable();
    Ok(cpus)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(cpus: impl IntoIterator<Item = u32>) -> CgroupScope {
        CgroupScope::new(
            vec![Event::Instructions],
            PathBuf::from("/sys/fs/cgroup/a-run"),
            Some(cpus.into_iter().collect()),
        )
    }

    #[test]
    fn the_cgroup_and_its_cpus_are_described() {
        let info = scope([0]).info();

        assert_eq!(info.get("cgroup"), Some(&"/sys/fs/cgroup/a-run".into()));
        assert_eq!(info.get("cpus"), Some(&"0".into()));
    }

    #[test]
    fn an_offline_cpu_is_reported_in_the_description() {
        let info = scope([u32::MAX]).info();

        assert_eq!(
            info.get("cpus"),
            Some(&format!("unavailable (cpu {} is not online)", u32::MAX).into())
        );
    }
}
