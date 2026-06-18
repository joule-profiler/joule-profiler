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
pub trait Backend: Send + Sync + 'static {
    // Reads memory and I/O stats for a single pid and adds them into `snapshot`.
    fn read_proc(&self, pid: i32, snapshot: &mut ProcSnapshot) -> Result<()>;

    /// Reads the current global system memory.
    fn measure_global(&self) -> Result<GlobalSnapshot>;

    /// Collects the root pid and all its descendant recursively.
    fn collect_children(&self, pid: i32) -> HashSet<i32>;

    /// Retrieve the total memory of the hardware.
    fn mem_total(&self) -> Result<u64>;
}

#[derive(Clone, Copy)]
pub struct ProcfsBackend;

impl Backend for ProcfsBackend {
    /// Reads memory and I/O stats for a single pid and adds them into `snapshot`.
    ///
    /// Silently ignores `PermissionDenied` on I/O reads, which can happen
    /// if the process exits between the smaps and io reads.
    fn read_proc(&self, pid: i32, snapshot: &mut ProcSnapshot) -> Result<()> {
        let process = Process::new(pid)?;

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
            Err(procfs::ProcError::PermissionDenied(_)) => {}
            Err(err) => return Err(err.into()),
        }

        Ok(())
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
