use std::fs;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use joule_profiler_core::injector::Target;
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::util::poller::Poller;
use joule_profiler_core::util::shared::{Consumer, shared};
use joule_profiler_core::util::stats::MinMaxMean;

use crate::error::{ProcfsError, Result};
use crate::hierarchy::get_process_descendants;
use crate::poller::{poll_hierarchy, poll_machine_memory, poll_process_memory};

/// The metrics related to the memory of the process tree.
pub(crate) const PROCESS_METRICS: [&str; 5] = [
    "proc_vm_size",
    "proc_rss",
    "proc_pss",
    "proc_shared",
    "proc_anon",
];

/// The metrics related to the memory of the machine.
pub(crate) const GLOBAL_MEMORY_METRICS: [&str; 4] = [
    "global_mem_used",
    "global_cached",
    "global_anon",
    "global_swap_free",
];

/// The metrics related to the I/O of the process tree.
pub(crate) const IO_METRICS: [&str; 2] = ["proc_io_read_bytes", "proc_io_write_bytes"];

/// The metrics related to the CPU time of the process tree, read in clock ticks.
pub(crate) const CPU_METRICS: [&str; 2] = ["proc_cpu_user", "proc_cpu_system"];

/// What the polling threads read since the last boundary.
#[derive(Default, Clone)]
pub(crate) struct MemorySnapshot {
    pub(crate) proc: [MinMaxMean; PROCESS_METRICS.len()],
    pub(crate) global: [MinMaxMean; GLOBAL_MEMORY_METRICS.len()],
}

/// What the sensor reads at a phase boundary.
pub struct Snapshot {
    pub(crate) memory: MemorySnapshot,
    pub(crate) io: [u64; IO_METRICS.len()],
    pub(crate) cpu: [u64; CPU_METRICS.len()],
}

/// Reads the I/O and CPU time of the process tree at each boundary. Three polling threads, each
/// at its own interval, read the memory of the process tree, list the tree again, and read the
/// memory of the machine when `global` is set.
pub struct ProcfsSensor {
    poll_interval: Duration,
    process_poll_interval: Duration,
    hierarchy_poll_interval: Duration,
    global: bool,
    mem_total: u64,

    /// The profiled process.
    root_pid: i32,

    /// The process tree.
    pids: Arc<ArcSwap<Vec<i32>>>,

    process_memory: Option<Consumer<[MinMaxMean; PROCESS_METRICS.len()]>>,
    machine_memory: Option<Consumer<[MinMaxMean; GLOBAL_MEMORY_METRICS.len()]>>,
    pollers: Vec<Poller>,
}

impl ProcfsSensor {
    pub(crate) fn new(
        poll_interval: Duration,
        process_poll_interval: Duration,
        hierarchy_poll_interval: Duration,
        global: bool,
        mem_total: u64,
    ) -> Self {
        Self {
            poll_interval,
            process_poll_interval,
            hierarchy_poll_interval,
            global,
            mem_total,
            root_pid: 0,
            pids: Arc::default(),
            process_memory: None,
            machine_memory: None,
            pollers: Vec::new(),
        }
    }
}

impl Sensor for ProcfsSensor {
    type Snapshot = Snapshot;
    type Error = ProcfsError;

    fn name(&self) -> &'static str {
        "procfs"
    }

    fn attach(&mut self, target: Target) -> Result<()> {
        let root_pid = target.pid;
        self.root_pid = root_pid;
        self.pids.store(Arc::new(get_process_descendants(root_pid)));

        let (mut process_writer, process_reader) = shared();
        self.process_memory = Some(process_reader);
        let pids = Arc::clone(&self.pids);
        self.pollers
            .push(Poller::start(self.process_poll_interval, move || {
                poll_process_memory(&pids, &mut process_writer);
                Ok::<(), ProcfsError>(())
            }));

        let pids = Arc::clone(&self.pids);
        self.pollers
            .push(Poller::start(self.hierarchy_poll_interval, move || {
                poll_hierarchy(root_pid, &pids);
                Ok::<(), ProcfsError>(())
            }));

        if self.global {
            let (mut machine_writer, machine_reader) = shared();
            self.machine_memory = Some(machine_reader);
            let mem_total = self.mem_total;
            self.pollers
                .push(Poller::start(self.poll_interval, move || {
                    poll_machine_memory(mem_total, &mut machine_writer)
                }));
        }

        Ok(())
    }

    fn measure(&mut self) -> Result<Snapshot> {
        let pids = self.pids.load_full();
        let (io, cpu) = read_cpu_and_io_counters(self.root_pid, &pids);

        let proc = self
            .process_memory
            .as_mut()
            .ok_or(ProcfsError::NotAttached)?
            .consume();
        let global = self
            .machine_memory
            .as_mut()
            .map(Consumer::consume)
            .unwrap_or_default();

        Ok(Snapshot {
            memory: MemorySnapshot { proc, global },
            io,
            cpu,
        })
    }

    fn close(&mut self) -> Result<()> {
        self.pollers.clear();
        Ok(())
    }
}

/// Reads CPU and I/O counters for the process hierarchy.
/// Only the root, because the child would be counted twice.
#[allow(clippy::similar_names)]
fn read_cpu_and_io_counters(
    root: i32,
    processes: &[i32],
) -> ([u64; IO_METRICS.len()], [u64; CPU_METRICS.len()]) {
    let mut io = [0u64; IO_METRICS.len()];
    let mut cpu = [0u64; CPU_METRICS.len()];

    for pid in processes.iter().copied() {
        if let Some(text) = read_proc_file(pid, "io") {
            for (total, value) in io.iter_mut().zip(read_io(&text)) {
                *total += value;
            }
        }

        let Some(stats) = read_proc_file(pid, "stat") else {
            continue;
        };

        if pid == root {
            let Some((utime, stime, cutime, cstime)) = read_cpu_full(&stats) else {
                continue;
            };
            cpu[0] += utime;
            cpu[1] += stime;
            cpu[0] += cutime;
            cpu[1] += cstime;
        } else {
            let Some((utime, stime)) = read_cpu(&stats) else {
                continue;
            };
            cpu[0] += utime;
            cpu[1] += stime;
        }
    }

    (io, cpu)
}

/// Reads `/proc/<pid>/<file>`, or `None` if the process has ended.
fn read_proc_file(pid: i32, file: &str) -> Option<String> {
    fs::read_to_string(format!("/proc/{pid}/{file}")).ok()
}

/// The bytes a process read and wrote, from `/proc/<pid>/io`.
fn read_io(text: &str) -> [u64; IO_METRICS.len()] {
    let mut io = [0u64; IO_METRICS.len()];

    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };

        let slot = match key {
            "read_bytes" => 0,
            "write_bytes" => 1,
            _ => continue,
        };
        io[slot] = value.trim_start().parse().unwrap_or_default();
    }

    io
}

/// Returns CPU metrics `utime`, `stime`.
fn read_cpu(text: &str) -> Option<(u64, u64)> {
    let mut fields = cpu_fields(text)?;
    let mut next_unsigned = || fields.next()?.parse::<u64>().ok();
    Some((next_unsigned()?, next_unsigned()?))
}

/// Returns CPU metrics `utime`, `stime`, `cutime` and `cstime`.
/// Negative `cutime` and `cstime` values count as 0.
#[allow(clippy::similar_names)]
fn read_cpu_full(text: &str) -> Option<(u64, u64, u64, u64)> {
    let mut fields = cpu_fields(text)?;

    let mut next_unsigned = || fields.next()?.parse::<u64>().ok();
    let utime = next_unsigned()?;
    let stime = next_unsigned()?;

    let mut next_signed = || u64::try_from(fields.next()?.parse::<i64>().ok()?.max(0)).ok();
    let cutime = next_signed()?;
    let cstime = next_signed()?;

    Some((utime, stime, cutime, cstime))
}

fn cpu_fields(text: &str) -> Option<impl Iterator<Item = &str>> {
    const UTIME_INDEX: usize = 11;
    Some(
        text.get(text.rfind(')')? + 1..)?
            .split_ascii_whitespace()
            .skip(UTIME_INDEX),
    )
}

#[cfg(test)]
mod tests {
    use procfs::process::Process;

    use super::*;

    const STAT: &str = "42 (my (odd) prog) S 1 42 42 0 -1 4194304 100 0 0 0 \
                           11 22 33 44 20 0 1 0 900 2621440 371 0 0 0 0 0";

    #[test]
    fn the_processor_time_is_read_past_a_command_that_holds_spaces() {
        assert_eq!(read_cpu_full(STAT), Some((11, 22, 33, 44)));
    }

    #[test]
    fn a_reaped_child_that_reads_negative_counts_as_nothing() {
        let odd = STAT.replace(" 11 22 33 44 ", " 11 22 -1 -2 ");

        assert_eq!(read_cpu_full(&odd), Some((11, 22, 0, 0)));
    }

    #[test]
    fn a_stat_that_is_not_one_is_not_read() {
        assert_eq!(read_cpu(""), None);
        assert_eq!(read_cpu("42 (prog) S 1"), None);
    }

    #[test]
    fn the_bytes_are_read_by_name_and_not_by_position() {
        let io = "rchar: 1\nwchar: 2\nsyscr: 3\nsyscw: 4\n\
                  read_bytes: 4096\nwrite_bytes: 8192\ncancelled_write_bytes: 0\n";

        assert_eq!(read_io(io), [4096, 8192]);
    }

    #[test]
    fn an_io_missing_what_is_wanted_reads_as_nothing_rather_than_failing() {
        assert_eq!(read_io("rchar: 1\nwchar: 2\n"), [0, 0]);
        assert_eq!(read_io(""), [0, 0]);
    }

    #[test]
    #[allow(clippy::similar_names)]
    fn reading_proc_straight_says_what_the_crate_says() {
        let pid = i32::try_from(std::process::id()).expect("a pid fits in an i32");
        let process = Process::new(pid).expect("this process is in /proc");

        let stat = process.stat().expect("its stat");
        let (utime, stime, cutime, cstime) =
            read_cpu_full(&read_proc_file(pid, "stat").expect("its stat, read straight"))
                .expect("four numbers");

        assert!(
            utime.abs_diff(stat.utime) <= 1 && stime.abs_diff(stat.stime) <= 1,
            "utime/stime: straight ({utime} {stime}), crate {} {}",
            stat.utime,
            stat.stime
        );
        assert_eq!(cutime, u64::try_from(stat.cutime).unwrap_or_default());
        assert_eq!(cstime, u64::try_from(stat.cstime).unwrap_or_default());

        if let Ok(io) = process.io() {
            let io_counters = read_io(&read_proc_file(pid, "io").expect("its io, read straight"));

            assert_eq!(io_counters[0], io.read_bytes, "read_bytes");
            assert_eq!(io_counters[1], io.write_bytes, "write_bytes");
        }
    }

    #[test]
    fn a_process_that_is_gone_is_one_less_rather_than_a_failure() {
        assert!(read_proc_file(-1, "stat").is_none());
        let (io, cpu) = read_cpu_and_io_counters(-1, &[-1]);

        assert_eq!(io, [0, 0]);
        assert_eq!(cpu, [0, 0]);
    }
}
