use joule_profiler_core::unit::MetricUnit;
use joule_profiler_core::util::cgroup::Cgroup;

use crate::error::{CgroupError, Result};

/// The memory values, in snapshot order.
pub(crate) const MEMORY: [&str; 9] = [
    "current",
    "anon",
    "file",
    "kernel",
    "kernel_stack",
    "peak",
    "shmem",
    "slab",
    "swap_current",
];

/// The counters of `cpu.stat`, then the bytes of `io.stat` summed over the devices.
pub(crate) const COUNTERS: [(&str, MetricUnit); 10] = [
    ("usage_usec", MetricUnit::MICROSECOND),
    ("user_usec", MetricUnit::MICROSECOND),
    ("system_usec", MetricUnit::MICROSECOND),
    ("nr_periods", MetricUnit::COUNT),
    ("nr_throttled", MetricUnit::COUNT),
    ("throttled_usec", MetricUnit::MICROSECOND),
    ("nr_bursts", MetricUnit::COUNT),
    ("burst_usec", MetricUnit::MICROSECOND),
    ("read_bytes", MetricUnit::BYTE),
    ("write_bytes", MetricUnit::BYTE),
];

/// The index of `usage_usec` in [`COUNTERS`].
pub(crate) const USAGE_USEC_INDEX: usize = 0;

pub(crate) trait Usage {
    fn memory_usage(&self) -> Result<[Option<u64>; MEMORY.len()]>;
    fn counters(&self) -> Result<[Option<u64>; COUNTERS.len()]>;
}

impl Usage for Cgroup {
    fn memory_usage(&self) -> Result<[Option<u64>; MEMORY.len()]> {
        let read = |file: &str| {
            self.read_u64(file)
                .map_err(|error| read_failed(self, file, error))
        };
        let stat = self
            .read_keyed("memory.stat")
            .map_err(|error| read_failed(self, "memory.stat", error))?;

        Ok([
            read("memory.current")?,
            stat.get("anon").copied(),
            stat.get("file").copied(),
            stat.get("kernel").copied(),
            stat.get("kernel_stack").copied(),
            read("memory.peak")?,
            stat.get("shmem").copied(),
            stat.get("slab").copied(),
            read("memory.swap.current")?,
        ])
    }

    fn counters(&self) -> Result<[Option<u64>; COUNTERS.len()]> {
        let stat = self
            .read_keyed("cpu.stat")
            .map_err(|error| read_failed(self, "cpu.stat", error))?;
        let [read, written] = io_bytes(self)?;

        Ok(COUNTERS.map(|(name, _)| match name {
            "read_bytes" => read,
            "write_bytes" => written,
            _ => stat.get(name).copied(),
        }))
    }
}

/// The bytes read and written, summed over the devices of `io.stat`.
fn io_bytes(cgroup: &Cgroup) -> Result<[Option<u64>; 2]> {
    let Some(content) = cgroup
        .read("io.stat")
        .map_err(|error| read_failed(cgroup, "io.stat", error))?
    else {
        return Ok([None, None]);
    };

    let mut totals = [None, None];

    for field in content.split_whitespace() {
        for (total, prefix) in totals.iter_mut().zip(["rbytes=", "wbytes="]) {
            if let Some(value) = field.strip_prefix(prefix)
                && let Ok(value) = value.parse::<u64>()
            {
                *total = Some(total.unwrap_or_default() + value);
            }
        }
    }

    Ok(totals)
}

/// The controllers of the cgroup, or those its parent can give it if it does not exist yet.
pub(crate) fn controllers(cgroup: &Cgroup) -> Result<Vec<String>> {
    match cgroup.path().parent() {
        Some(parent) if !cgroup.exists() => {
            let parent = Cgroup::at(parent);
            parent
                .controllers()
                .map_err(|error| read_failed(&parent, "cgroup.controllers", error))
        }
        _ => cgroup
            .controllers()
            .map_err(|error| read_failed(cgroup, "cgroup.controllers", error)),
    }
}

fn read_failed(cgroup: &Cgroup, file: &str, error: std::io::Error) -> CgroupError {
    CgroupError::Read(cgroup.path().join(file), error)
}
