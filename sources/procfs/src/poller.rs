use std::sync::Arc;

use arc_swap::ArcSwap;

use joule_profiler_core::util::{shared::Producer, stats::MinMaxMean};
use procfs::{Current, Meminfo, process::Process};

use crate::{
    error::Result,
    hierarchy::get_process_descendants,
    sensor::{GLOBAL_MEMORY_METRICS, PROCESS_METRICS},
};

/// Reads the memory of the process tree and adds it to the stats of the phase.
pub(crate) fn poll_process_memory(
    pids: &ArcSwap<Vec<i32>>,
    writer: &mut Producer<[MinMaxMean; PROCESS_METRICS.len()]>,
) {
    let memory = sum_process_memory(&pids.load_full());
    writer.write(|stats| add_to_stats(stats, memory));
}

/// Lists the process tree and swap the pids arc.
pub(crate) fn poll_hierarchy(root_pid: i32, pids: &ArcSwap<Vec<i32>>) {
    pids.store(Arc::new(get_process_descendants(root_pid)));
}

/// Reads the memory of the machine and adds it to the stats of the phase.
pub(crate) fn poll_machine_memory(
    mem_total: u64,
    writer: &mut Producer<[MinMaxMean; GLOBAL_MEMORY_METRICS.len()]>,
) -> Result<()> {
    let memory = read_global_memory(mem_total)?;

    writer.write(|stats| add_to_stats(stats, memory));
    Ok(())
}

fn add_to_stats<const N: usize>(stats: &mut [MinMaxMean; N], values: [u64; N]) {
    for (stat, value) in stats.iter_mut().zip(values) {
        stat.add(value);
    }
}

fn sum_process_memory(processes: &[i32]) -> [u64; PROCESS_METRICS.len()] {
    let mut total = [0u64; PROCESS_METRICS.len()];

    for pid in processes.iter().copied() {
        if let Some(memory) = read_process_memory(pid) {
            for (total, value) in total.iter_mut().zip(memory) {
                *total += value;
            }
        }
    }

    total
}

fn read_process_memory(pid: i32) -> Option<[u64; PROCESS_METRICS.len()]> {
    let process = Process::new(pid).ok()?;

    let mut memory = [0u64; PROCESS_METRICS.len()];
    memory[0] = process.stat().ok()?.vsize;

    if let Some(entry) = process
        .smaps_rollup()
        .ok()?
        .memory_map_rollup
        .0
        .first()
        .map(|entry| &entry.extension.map)
    {
        let read = |key: &str| entry.get(key).copied().unwrap_or_default();

        memory[1] = read("Rss");
        memory[2] = read("Pss");
        memory[3] = read("Shared_Clean") + read("Shared_Dirty");
        memory[4] = read("Anonymous");
    }

    Some(memory)
}

fn read_global_memory(mem_total: u64) -> Result<[u64; GLOBAL_MEMORY_METRICS.len()]> {
    let meminfo = Meminfo::current()?;

    let used = mem_total.saturating_sub(
        meminfo
            .mem_available
            .unwrap_or(meminfo.mem_free + meminfo.cached),
    );

    Ok([
        used,
        meminfo.cached,
        meminfo.anon_pages.unwrap_or_default(),
        meminfo.swap_free,
    ])
}
