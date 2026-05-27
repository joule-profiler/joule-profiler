use log::trace;
use procfs::{Current, FromRead, Meminfo, process::Process};

use crate::Result;

#[derive(Default)]
pub struct ProcSnapshot {
    pub vm_size: u64,
    pub rss: u64,
    pub pss: u64,
    pub shared: u64,
    pub anon: u64,

    pub read_bytes: u64,
    pub write_bytes: u64,
}

pub fn read_proc(pids: &[i32]) -> Result<ProcSnapshot> {
    let mut snapshot = ProcSnapshot::default();

    for pid in pids {
        let process = Process::new(*pid)?;
        trace!("Querying process {} stat.", process.pid);
        let vm_size = process.stat()?.vsize;
        snapshot.vm_size += vm_size;

        trace!("Querying process {} smaps_rollup.", process.pid);
        let smaps = process.smaps_rollup()?;
        let map = if let Some(entry) = smaps.memory_map_rollup.0.first() {
            &entry.extension.map
        } else {
            continue;
        };

        snapshot.rss += map.get("Rss").copied().unwrap_or(0);
        snapshot.pss += map.get("Pss").copied().unwrap_or(0);
        snapshot.anon += map.get("Anonymous").copied().unwrap_or(0);
        snapshot.shared += map.get("Shared_Clean").copied().unwrap_or(0)
            + map.get("Shared_Dirty").copied().unwrap_or(0);

        trace!("Querying process {} io.", process.pid);
        let io = process.io()?;
        snapshot.read_bytes += io.rchar;
        snapshot.write_bytes += io.wchar;
    }

    Ok(snapshot)
}

pub struct GlobalSnapshot {
    pub mem_available: Option<u64>,
    pub mem_free: u64,
    pub cached: u64,
    pub anon: Option<u64>,
    pub swap_free: u64,
}

pub fn read_global() -> Result<GlobalSnapshot> {
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
