# perf_event metric source

Performance counter source for Joule Profiler, using the Linux perf_event subsystem.
This crate builds a `Source` for Joule Profiler that collects hardware and software performance counters (CPU cycles, instructions, cache misses, branch mispredictions...) via the perf_event_open(2) syscall, per phase.

## Requirements

| Requirement | Details |
|---|---|
| OS | Linux kernel 2.6.31+ |
| CPU | Any architecture with PMU support (x86, ARM, RISC-V...) |
| Permissions | Root, or `kernel.perf_event_paranoid <= 1` |

### Adjusting perf_event_paranoid

```bash
# Check current value
cat /proc/sys/kernel/perf_event_paranoid
 
# Allow per-process counters for unprivileged users (temporary)
sudo sysctl kernel.perf_event_paranoid=1
```

## Scope

By default, Joule Profiler attaches `perf_event` counters to the **monitored process** only (per-process mode, via the pid), following it and any children it spawns across every CPU it runs on.

```toml
[sources.perf_event]
scope = "pid"   # the default
events = ["cpu_cycles", "instructions", "cache_misses", "branch_misses"] # the default
```

The available hardware events are `cpu_cycles`, `instructions`, `cache_references`, `cache_misses`, `branch_instructions`, `branch_misses`, `bus_cycles` and `ref_cpu_cycles`.

The available software events, counted by the kernel, are `cpu_clock`, `task_clock`, `page_faults`, `context_switches`, `cpu_migrations`, `minor_faults`, `major_faults`, `alignment_faults`, `emulation_faults` and `cgroup_switches` (Linux 5.13+). The perf names (`cycles`, `branches`, `cs`, `minor-faults`...) are accepted too.

Alternatively, counters can be scoped to a **cgroup v2** instead of the process's pid, with `scope = "cgroup"`:

```toml
[sources.perf_event]
scope = "cgroup"
name = "my-cgroup"        # relative to `root`, use "parent/child" for a nested cgroup
#root = "/sys/fs/cgroup"  # the default
#cpus = [0, 1]            # default: every online CPU
```

Without `name`, the counters are opened on the cgroup Joule Profiler makes for the run, when there is one (see the [cgroup](../cgroup/README.md) source).

This tracks every process inside the cgroup rather than a single pid and its children, it can be useful for containers profiling, and lets counters be opened *before* the profiled process is spawned (the cgroup must already exist).

## Grouping

By default the counters are opened as one perf group: the kernel schedules them together, so they are all measured over the same instants. A group must fit in the hardware counters of the CPU: when it does not, opening it fails, or it is accepted but never scheduled and its counters are reported as 0 (the NMI watchdog, when enabled, keeps a counter for itself).

With `group = false`, each counter is scheduled on its own: when there are more events than hardware counters, the kernel has them take turns, and each count is scaled by the time it was running.

```toml
[sources.perf_event]
group = false   # default: true
```
