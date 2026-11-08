use std::collections::{HashSet, VecDeque};

use log::trace;
use procfs::{Current, FromRead, Meminfo, process::Process};

use crate::{
    Result,
    snapshot::{GlobalSnapshot, ProcSnapshot},
    utils::read_child_processes,
};

/// Trait abstracting procfs for testing efficiently.
#[cfg_attr(test, mockall::automock)]
pub trait Backend: Default + Send + Sync + 'static {
    // Reads memory and I/O stats for a single pid and adds them into `snapshot`.
    fn read_proc(&self, pid: i32) -> Result<ProcSnapshot>;

    /// Reads the current global system memory.
    fn measure_global(&self) -> Result<GlobalSnapshot>;

    /// Collects the root pid and all its descendant recursively.
    fn collect_children(&self, pid: i32) -> HashSet<i32>;

    /// Retrieve the total memory of the hardware.
    fn mem_total(&self) -> Result<u64>;
}

#[derive(Default, Clone, Copy)]
pub struct ProcfsBackend;

impl Backend for ProcfsBackend {
    /// Reads memory and I/O stats for a single pid.
    ///
    /// Silently ignores `PermissionDenied` and `NotFound` on I/O reads. The
    /// latter happens when the kernel has no task I/O accounting (no
    /// `/proc/<pid>/io`, e.g. WSL2) or when the process exits between the
    /// smaps and io reads. Memory metrics are still valid in both cases.
    fn read_proc(&self, pid: i32) -> Result<ProcSnapshot> {
        let process = Process::new(pid)?;
        let mut snapshot = ProcSnapshot::default();

        trace!("Querying process {} stat.", process.pid);
        snapshot.vm_size += process.stat()?.vsize;

        trace!("Querying process {} smaps_rollup.", process.pid);
        let smaps = process.smaps_rollup()?;
        if let Some(entry) = smaps.memory_map_rollup.0.first() {
            let map = &entry.extension.map;
            snapshot.rss += map.get("Rss").copied().unwrap_or(0);
            snapshot.pss += map.get("Pss").copied().unwrap_or(0);
            snapshot.anon += map.get("Anonymous").copied().unwrap_or(0);
            snapshot.shared += map.get("Shared_Clean").copied().unwrap_or(0)
                + map.get("Shared_Dirty").copied().unwrap_or(0);
        }

        trace!("Querying process {} io.", process.pid);
        match process.io() {
            Ok(io) => {
                snapshot.read_bytes += io.rchar;
                snapshot.write_bytes += io.wchar;
            }
            Err(procfs::ProcError::PermissionDenied(_) | procfs::ProcError::NotFound(_)) => {}
            Err(err) => return Err(err.into()),
        }

        Ok(snapshot)
    }

    /// Reads system-wide memory statistics from `/proc/meminfo`.
    fn measure_global(&self) -> Result<GlobalSnapshot> {
        let meminfo = Meminfo::from_file(Meminfo::PATH)?;
        trace!("Querying global meminfo from {}.", procfs::Meminfo::PATH);

        Ok(GlobalSnapshot {
            mem_available: meminfo.mem_available,
            mem_free: meminfo.mem_free,
            cached: meminfo.cached,
            anon: meminfo.anon_pages,
            swap_free: meminfo.swap_free,
        })
    }

    /// Collects `root_pid` and all its descendant recursively.
    /// Threads are excluded, only process leaders are returned.
    fn collect_children(&self, pid: i32) -> HashSet<i32> {
        let mut pids: HashSet<i32> = HashSet::new();
        pids.insert(pid);
        let mut queue: VecDeque<i32> = VecDeque::from([pid]);

        while let Some(pid) = queue.pop_front() {
            for child_pid in read_child_processes(pid) {
                if !pids.contains(&child_pid) {
                    pids.insert(child_pid);
                    queue.push_back(child_pid);
                }
            }
        }
        pids
    }

    fn mem_total(&self) -> Result<u64> {
        trace!("Retrieving Meminfo from {}.", Meminfo::PATH);
        Ok(Meminfo::from_file(Meminfo::PATH)?.mem_total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Memory metrics must not depend on `/proc/<pid>/io`, which kernels built
    /// without task I/O accounting (e.g. WSL2) do not provide.
    #[test]
    fn read_proc_reports_memory_of_the_current_process() {
        let pid = i32::try_from(std::process::id()).unwrap();

        let snapshot = ProcfsBackend.read_proc(pid).unwrap();

        assert!(snapshot.vm_size > 0);
        assert!(snapshot.rss > 0);
    }
}
