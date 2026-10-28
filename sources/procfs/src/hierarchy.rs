use std::collections::{HashSet, VecDeque};

use procfs::process::Process;

/// The process and its descendants, without threads.
pub fn get_process_descendants(root: i32) -> Vec<i32> {
    let mut found = HashSet::from([root]);
    let mut queue = VecDeque::from([root]);

    while let Some(pid) = queue.pop_front() {
        for child in get_process_children(pid) {
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
fn get_process_children(pid: i32) -> Vec<i32> {
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
