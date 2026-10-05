use std::collections::{HashSet, VecDeque};
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read as _;
use std::time::{Duration, Instant};

use joule_profiler_core::injector::Target;
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::util::poller::Poller;
use joule_profiler_core::util::shared::{Consumer, shared};
use joule_profiler_core::util::stats::MinMaxMean;
use procfs::process::Process;
use procfs::{Current, Meminfo};

use crate::error::{ProcfsError, Result};

/// The memory of the process tree, in snapshot order.
pub(crate) const PROC: [&str; 5] = [
    "proc_vm_size",
    "proc_rss",
    "proc_pss",
    "proc_shared",
    "proc_anon",
];

/// The memory of the machine, in snapshot order.
pub(crate) const GLOBAL: [&str; 4] = [
    "global_mem_used",
    "global_cached",
    "global_anon",
    "global_swap_free",
];

pub(crate) const IO: [&str; 2] = ["proc_io_read_bytes", "proc_io_write_bytes"];

/// The CPU time of the process tree, read in clock ticks.
pub(crate) const CPU: [&str; 2] = ["proc_cpu_user", "proc_cpu_system"];

/// What the polling thread read since the last boundary.
#[derive(Default, Clone)]
pub(crate) struct PolledMemory {
    pub(crate) proc: [MinMaxMean; PROC.len()],
    pub(crate) global: [MinMaxMean; GLOBAL.len()],
}

pub struct Snapshot {
    pub(crate) polled: PolledMemory,
    pub(crate) io: [u64; IO.len()],
    pub(crate) cpu: [u64; CPU.len()],
}

/// The polling thread lists the process tree, which is costly, every `hierarchy_interval` only.
pub struct ProcfsSensor {
    poll_interval: Duration,
    process_poll_interval: Duration,
    global: bool,

    mem_total: u64,

    hierarchy_interval: Duration,

    /// The profiled process.
    root: i32,

    /// The process tree, as of the last listing.
    pids: Vec<i32>,

    polled: Option<Consumer<PolledMemory>>,

    /// A new process tree, if it was listed again since the last boundary.
    hierarchy: Option<Consumer<Option<Vec<i32>>>>,

    poller: Option<Poller>,

    /// Buffers reused at every boundary.
    files: ProcFiles,
}

impl ProcfsSensor {
    pub(crate) fn new(
        poll_interval: Duration,
        process_poll_interval: Duration,
        hierarchy_interval: Duration,
        global: bool,
        mem_total: u64,
    ) -> Self {
        Self {
            poll_interval,
            process_poll_interval,
            hierarchy_interval,
            global,
            mem_total,
            root: 0,
            pids: Vec::new(),
            polled: None,
            hierarchy: None,
            poller: None,
            files: ProcFiles::default(),
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
        let root = target.pid;
        let mut pids = descendants(root);

        self.root = root;
        self.pids.clone_from(&pids);

        let (mut polled, consumer) = shared();
        self.polled = Some(consumer);

        let (mut hierarchy, consumer) = shared();
        self.hierarchy = Some(consumer);

        let mem_total = self.mem_total;
        let global = self.global;
        let every = self.process_poll_interval;
        let rebuild_every = self.hierarchy_interval;

        let mut looked = None::<Instant>;
        let mut walked = Instant::now();

        self.poller = Some(Poller::start(self.poll_interval, move || {
            if walked.elapsed() >= rebuild_every {
                pids = descendants(root);
                hierarchy.write(|hierarchy| *hierarchy = Some(pids.clone()));
                walked = Instant::now();
            }

            let machine = global.then(|| read_global(mem_total)).transpose()?;
            let due = looked.is_none_or(|looked| looked.elapsed() >= every);
            let memory = due.then(|| sum_process(&pids));

            if due {
                looked = Some(Instant::now());
            }

            polled.write(|polled| {
                if let Some(machine) = machine {
                    fill(&mut polled.global, machine);
                }
                if let Some(memory) = memory {
                    fill(&mut polled.proc, memory);
                }
            });

            Ok::<(), ProcfsError>(())
        }));

        Ok(())
    }

    fn measure(&mut self) -> Result<Snapshot> {
        let polled = self
            .polled
            .as_mut()
            .ok_or(ProcfsError::NotAttached)?
            .consume();

        if let Some(pids) = self.hierarchy.as_mut().and_then(Consumer::consume) {
            self.pids = pids;
        }

        let (io, cpu) = counters(&mut self.files, self.root, &self.pids);

        Ok(Snapshot { polled, io, cpu })
    }

    fn close(&mut self) -> Result<()> {
        self.poller = None;
        Ok(())
    }
}

fn fill<const N: usize>(stats: &mut [MinMaxMean; N], values: [u64; N]) {
    for (stat, value) in stats.iter_mut().zip(values) {
        stat.add(value);
    }
}

fn sum_process(processes: &[i32]) -> [u64; PROC.len()] {
    let mut total = [0u64; PROC.len()];

    for pid in processes.iter().copied() {
        if let Some(memory) = read_process(pid) {
            for (total, value) in total.iter_mut().zip(memory) {
                *total += value;
            }
        }
    }

    total
}

fn read_process(pid: i32) -> Option<[u64; PROC.len()]> {
    let process = Process::new(pid).ok()?;

    let mut memory = [0u64; PROC.len()];
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

fn read_global(mem_total: u64) -> Result<[u64; GLOBAL.len()]> {
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

#[derive(Default)]
pub(crate) struct ProcFiles {
    path: String,
    text: String,
}

impl ProcFiles {
    fn read(&mut self, pid: i32, file: &str) -> Option<&str> {
        self.path.clear();
        let _ = write!(self.path, "/proc/{pid}/{file}");

        self.text.clear();
        File::open(&self.path)
            .ok()?
            .read_to_string(&mut self.text)
            .ok()?;

        Some(&self.text)
    }
}

/// The bytes a process read and wrote, from `/proc/<pid>/io`.
fn read_io(text: &str) -> [u64; IO.len()] {
    let mut io = [0u64; IO.len()];

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

/// `utime`, `stime`, `cutime` and `cstime`, counted from the last `)`: the name may hold one.
fn read_cpu(text: &str) -> Option<[u64; 4]> {
    const UTIME: usize = 11;

    let mut fields = text
        .get(text.rfind(')')? + 1..)?
        .split_ascii_whitespace()
        .skip(UTIME);

    // `cutime` and `cstime` are signed: a negative one counts as 0.
    let mut spent = [0u64; 4];
    for slot in &mut spent {
        *slot = u64::try_from(fields.next()?.parse::<i64>().ok()?).unwrap_or_default();
    }

    Some(spent)
}

/// The I/O of an exited child is lost, unlike its CPU time: the cgroup source counts both.
fn counters(
    files: &mut ProcFiles,
    root: i32,
    processes: &[i32],
) -> ([u64; IO.len()], [u64; CPU.len()]) {
    let mut io = [0u64; IO.len()];
    let mut cpu = [0u64; CPU.len()];

    for pid in processes.iter().copied() {
        if let Some(text) = files.read(pid, "io") {
            for (total, value) in io.iter_mut().zip(read_io(text)) {
                *total += value;
            }
        }

        let Some(spent) = files.read(pid, "stat").and_then(read_cpu) else {
            continue;
        };

        cpu[0] += spent[0];
        cpu[1] += spent[1];

        // Only the root's, or a reaped child would be counted twice.
        if pid == root {
            cpu[0] += spent[2];
            cpu[1] += spent[3];
        }
    }

    (io, cpu)
}

/// The process and its descendants, without threads.
fn descendants(root: i32) -> Vec<i32> {
    let mut found = HashSet::from([root]);
    let mut queue = VecDeque::from([root]);

    while let Some(pid) = queue.pop_front() {
        for child in children(pid) {
            if found.insert(child) {
                queue.push_back(child);
            }
        }
    }

    let mut found: Vec<i32> = found.into_iter().collect();
    found.sort_unstable();

    found
}

/// The children of a process, from the `children` file of each of its threads.
fn children(pid: i32) -> Vec<i32> {
    let Ok(process) = Process::new(pid) else {
        return Vec::new();
    };
    let Ok(tasks) = process.tasks() else {
        return Vec::new();
    };

    tasks
        .flatten()
        .flat_map(|task| {
            let path = format!("/proc/{pid}/task/{}/children", task.tid);

            std::fs::read_to_string(path)
                .unwrap_or_default()
                .split_whitespace()
                .filter_map(|child| child.parse().ok())
                .collect::<Vec<i32>>()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AWKWARD: &str = "42 (my (odd) prog) S 1 42 42 0 -1 4194304 100 0 0 0 \
                           11 22 33 44 20 0 1 0 900 2621440 371 0 0 0 0 0";

    #[test]
    fn the_processor_time_is_read_past_a_command_that_holds_spaces() {
        assert_eq!(read_cpu(AWKWARD), Some([11, 22, 33, 44]));
    }

    #[test]
    fn a_reaped_child_that_reads_negative_counts_as_nothing() {
        let odd = AWKWARD.replace(" 11 22 33 44 ", " 11 22 -1 -2 ");

        assert_eq!(read_cpu(&odd), Some([11, 22, 0, 0]));
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
    fn reading_proc_straight_says_what_the_crate_says() {
        let pid = i32::try_from(std::process::id()).expect("a pid fits in an i32");
        let mut files = ProcFiles::default();
        let process = Process::new(pid).expect("this process is in /proc");

        let stat = process.stat().expect("its stat");
        let ours = read_cpu(files.read(pid, "stat").expect("its stat, read straight"))
            .expect("four numbers");

        // The counters move between the two reads.
        assert!(
            ours[0].abs_diff(stat.utime) <= 1 && ours[1].abs_diff(stat.stime) <= 1,
            "utime/stime: straight {ours:?}, crate {} {}",
            stat.utime,
            stat.stime
        );
        assert_eq!(ours[2], u64::try_from(stat.cutime).unwrap_or_default());
        assert_eq!(ours[3], u64::try_from(stat.cstime).unwrap_or_default());

        if let Ok(io) = process.io() {
            let ours = read_io(files.read(pid, "io").expect("its io, read straight"));

            assert_eq!(ours[0], io.read_bytes, "read_bytes");
            assert_eq!(ours[1], io.write_bytes, "write_bytes");
        }
    }

    #[test]
    fn a_process_that_is_gone_is_one_less_rather_than_a_failure() {
        let mut files = ProcFiles::default();

        assert!(files.read(-1, "stat").is_none());
        let (io, cpu) = counters(&mut files, -1, &[-1]);

        assert_eq!(io, [0, 0]);
        assert_eq!(cpu, [0, 0]);
    }
}
