use procfs::process::Process;
use std::collections::VecDeque;

fn read_task_children(pid: i32) -> Vec<i32> {
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
        .filter(|&child_pid| {
            Process::new(child_pid)
                .and_then(|p| p.status())
                .is_ok_and(|s| s.pid == s.tgid)
        })
        .collect()
}

pub fn collect_all_children(root_pid: i32) -> Vec<i32> {
    let mut pids: Vec<i32> = vec![root_pid];
    let mut queue: VecDeque<i32> = VecDeque::from([root_pid]);

    while let Some(pid) = queue.pop_front() {
        for child_pid in read_task_children(pid) {
            if !pids.contains(&child_pid) {
                pids.push(child_pid);
                queue.push_back(child_pid);
            }
        }
    }
    pids
}
