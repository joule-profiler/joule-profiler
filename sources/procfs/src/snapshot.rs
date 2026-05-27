use log::trace;
use procfs::{Current, FromRead, Meminfo, process::Process};

use crate::{Result, utils::collect_all_children};

/// Snapshot of memory and I/O measurements for a process hierarchy.
///
/// All fields are summed across the process and all its descendants.
/// Memory values are in bytes.
#[derive(Default)]
pub struct ProcSnapshot {
    /// Virtual memory size, from `/proc/{pid}/stat`.
    pub vm_size: u64,

    /// Resident set size.
    pub rss: u64,

    /// Proportional set size (shared pages divided by the count of process using them).
    pub pss: u64,

    /// Shared memory (clean + dirty).
    pub shared: u64,

    /// Anonymous memory (not backed by a file).
    pub anon: u64,

    /// Cumulative bytes read since process start.
    pub read_bytes: u64,

    /// Cumulative bytes written since process start.
    pub write_bytes: u64,
}

/// Reads memory and I/O stats for a single pid and adds them into `snapshot`.
///
/// Silently ignores `PermissionDenied` on I/O reads, which can happen
/// if the process exits between the smaps and io reads.
fn read_proc_pid(pid: i32, snapshot: &mut ProcSnapshot) -> Result<()> {
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

pub fn measure_proc(pid: i32) -> Result<ProcSnapshot> {
    trace!("Retrieving process hierarchy from pid {pid}.");
    let pids = collect_all_children(pid);
    trace!("Found pids {pids:?}. Reading process procfs counters.");

    let mut snapshot = ProcSnapshot::default();
    for pid in pids {
        read_proc_pid(pid, &mut snapshot)?;
    }
    Ok(snapshot)
}

/// Point-in-time system-wide memory statistics from `/proc/meminfo`.
///
/// All values are in bytes. Optional fields are absent when the kernel
/// does not expose them (e.g. `MemAvailable` on old kernels, `AnonPages`
/// on some minimal configurations).
pub struct GlobalSnapshot {
    /// Estimate of available memory.
    /// Preferred over `mem_free + cached` for computing used memory.
    pub mem_available: Option<u64>,

    /// Free memory, used if `mem_available` is not accessible.
    pub mem_free: u64,

    /// Cached memory.
    pub cached: u64,

    /// Anonymous pages (heap, stack, mmaps).
    pub anon: Option<u64>,

    /// Free swap space.
    pub swap_free: u64,
}

/// Reads system-wide memory statistics from `/proc/meminfo`.
pub fn measure_global() -> Result<GlobalSnapshot> {
    let meminfo = Meminfo::from_file(Meminfo::PATH)?;
    trace!("Querying global meminfo from {}", procfs::Meminfo::PATH);

    Ok(GlobalSnapshot {
        mem_available: meminfo.mem_available,
        mem_free: meminfo.mem_free,
        cached: meminfo.cached,
        anon: meminfo.anon_pages,
        swap_free: meminfo.swap_free,
    })
}
