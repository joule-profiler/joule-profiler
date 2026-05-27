use joule_profiler_core::{
    types::MetricValue,
    unit::{MetricUnit, Unit, UnitPrefix},
};
use procfs::process::Process;
use std::collections::VecDeque;

/// Reads direct child of pid (threads excluded).
///
/// Reads `/proc/{pid}/task/{tid}/children` for each thread of the process,
/// then filters out threads by keeping only processes.
fn read_child_processes(pid: i32) -> Vec<i32> {
    let Ok(process) = Process::new(pid) else {
        return vec![];
    };
    let Ok(tasks) = process.tasks() else {
        return vec![];
    };

    tasks
        .flatten()
        .flat_map(|task| {
            let path = format!("/proc/{}/task/{}/children", pid, task.tid);
            std::fs::read_to_string(&path)
                .unwrap_or_default()
                .split_whitespace()
                .filter_map(|s| s.parse::<i32>().ok())
                .collect::<Vec<_>>()
        })
        .filter(|&child_pid| is_process(child_pid))
        .collect()
}

/// Returns `true` if `pid` is a process leader (not a thread).
///
/// A thread has `pid != tgid` in `/proc/{pid}/status`.
/// The process leader has `pid == tgid`.
fn is_process(pid: i32) -> bool {
    Process::new(pid)
        .and_then(|p| p.status())
        .is_ok_and(|s| s.pid == s.tgid)
}

/// Collects `root_pid` and all its descendant recursively.
/// Threads are excluded, only process leaders are returned.
pub fn collect_all_children(root_pid: i32) -> Vec<i32> {
    let mut pids: Vec<i32> = vec![root_pid];
    let mut queue: VecDeque<i32> = VecDeque::from([root_pid]);

    while let Some(pid) = queue.pop_front() {
        for child_pid in read_child_processes(pid) {
            if !pids.contains(&child_pid) {
                pids.push(child_pid);
                queue.push_back(child_pid);
            }
        }
    }
    pids
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MemoryUnit {
    Bytes,
    Kilo,
    #[default]
    Mega,
    Giga,
}

impl From<MemoryUnit> for MetricUnit {
    fn from(unit: MemoryUnit) -> Self {
        match unit {
            MemoryUnit::Bytes => MetricUnit {
                prefix: UnitPrefix::None,
                unit: Unit::Byte,
            },
            MemoryUnit::Kilo => MetricUnit {
                prefix: UnitPrefix::Kilo,
                unit: Unit::Byte,
            },
            MemoryUnit::Mega => MetricUnit {
                prefix: UnitPrefix::Mega,
                unit: Unit::Byte,
            },
            MemoryUnit::Giga => MetricUnit {
                prefix: UnitPrefix::Giga,
                unit: Unit::Byte,
            },
        }
    }
}

/// Convert bytes to the provided unit.
/// If the unit is bigger than Kilo, then it becomes a float and is rounded to 2 decimals.
pub fn make_conversion(unit: MemoryUnit, value: u64) -> MetricValue {
    #[allow(clippy::cast_precision_loss)]
    (match unit {
        MemoryUnit::Bytes => |b| MetricValue::UnsignedInteger(b),
        MemoryUnit::Kilo => |b| MetricValue::UnsignedInteger(b / 1024),
        MemoryUnit::Mega => |b| MetricValue::Float(b as f64 / f64::from(1024 * 1024), Some(2)),
        MemoryUnit::Giga => {
            |b| MetricValue::Float(b as f64 / f64::from(1024 * 1024 * 1024), Some(2))
        }
    })(value)
}
