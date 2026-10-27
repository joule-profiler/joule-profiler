# Cgroup metric source

Cgroup v2 metric source for [Joule Profiler](https://github.com/joule-profiler/joule-profiler).

This crate builds a `Source` for Joule Profiler that collects **process-level** and, optionally, **system-wide** metrics using Linux **cgroup v2** interfaces.

Control groups (cgroups v2) are a Linux kernel feature that allows grouping processes and tracking/limiting their resource usage.
This source uses cgroup files exposed under `/sys/fs/cgroup`.

## Which cgroup is measured

### The cgroup made for the run

When `[sources.cgroup]` names no cgroup, Joule Profiler makes one for the run, moves the profiled program into it, and measures it. It is configured with the top-level `[cgroup]` table, whose keys are all optional:

```toml
[cgroup]
parent = "/sys/fs/cgroup"
name = "joule-profiler-run"           # default: joule-profiler-<pid>
controllers = ["cpu", "memory", "io"] # the default
attach = true                         # moves the program into it
freeze = false                        # frozen before the program starts, thawed once the sources are ready
```

The controllers are enabled on `parent` if they are not already. A controller of the default list that cannot be enabled is left out with a warning, while one listed in `controllers` stops the run. A cgroup Joule Profiler created is removed after the run, and the controllers it enabled are disabled again. A cgroup that already exists is used as is and kept.

Making it needs write access to the parent: root, or a delegated subtree (e.g. a systemd user session).

### An existing cgroup

To measure a cgroup that already exists instead, name it in the source table. No cgroup is then made for the run, unless a `[cgroup]` table is given.

```toml
[sources.cgroup]
name = "my.slice"       # relative to `root`, use "parent/child" for a nested cgroup
#root = "/sys/fs/cgroup" # the default
#poll_interval = "10ms"  # the default
#global = false          # also reports the root cgroup
```

The controllers `cpu`, `memory` and `io` must then be enabled on its parent, which Joule Profiler does **not** do for an existing cgroup. A metric whose controller is missing is not reported.

To verify which controllers are currently enabled:

```bash
cat /sys/fs/cgroup/cgroup.subtree_control
```

To enable the required controllers for the root cgroup:

```bash
echo "+cpu +memory +io" | sudo tee /sys/fs/cgroup/cgroup.subtree_control
```

If you are using a nested cgroup, each ancestor must propagate the controllers down. For example, if your cgroup lives under `/sys/fs/cgroup/nested_cgroup/mycgroup`:

```bash
echo "+cpu +memory +io" | sudo tee /sys/fs/cgroup/cgroup.subtree_control
echo "+cpu +memory +io" | sudo tee /sys/fs/cgroup/nested_cgroup/cgroup.subtree_control
```

> [!NOTE]
> On systemd-based systems, some controllers may already be enabled. Check `/sys/fs/cgroup/cgroup.subtree_control` first.

You can also create a cgroup slice using systemd to be able to spawn processes directly in it:
```bash
cat <<EOF | sudo tee /etc/systemd/system/{your_slice_name} >/dev/null
[Unit]
Description={description}

[Slice]
CPUWeight=100
IOWeight=100
MemoryMax=infinity
EOF
```

After that you can use systemd to spawn a process directly in your slice:

```bash
systemd-run --scope --slice={your_slice_name} {your_command}
```

The slice can then be named in `[sources.cgroup]`, and in `[sources.perf_event]` with `scope = "cgroup"`. See [perf_event](../perf_event/README.md) source documentation for further information.

## Implemented metrics

The metrics of the measured cgroup are prefixed with `proc`. With `global = true`, the same metrics are also reported for the root cgroup, prefixed with `global`.

The metrics reported depend on the controllers of the cgroup. Here is the list of the metrics implemented.

### CPU

| Metric | Description |
| - | - |
| `usage_usec` | CPU time consumed (user + kernel) during the phase |
| `user_usec` | CPU time spent in user space during the phase |
| `system_usec` | CPU time spent in kernel space during the phase |
| `nr_periods` | Number of scheduling periods |
| `nr_throttled` | Number of CPU throttling events |
| `throttled_usec` | Time spent throttled |
| `nr_bursts` | Number of burst events (burstable cgroups) |
| `burst_usec` | Time spent in burst mode |
| `cpu_usage` | CPU usage over the phase, 100% being one full core |

### Memory

Each value is polled every `poll_interval` and reported with a `_min`, `_max` and `_avg` suffix, in bytes.

| Metric | Description |
| - | - |
| `current` | Total current memory usage of the cgroup |
| `swap_current` | Current swap usage |
| `anon` | Anonymous memory (heap, stack, anonymous mmap) |
| `file` | File-backed memory (page cache) |
| `peak` | Peak memory usage observed for the cgroup |
| `shmem` | Shared memory usage (tmpfs, /dev/shm) |
| `kernel` | Kernel memory used by the cgroup |
| `kernel_stack` | Kernel stack memory usage |
| `slab` | Slab allocator memory usage |

### I/O

| Metric | Description |
| - | - |
| `read_bytes` | Bytes read by the cgroup during the phase, summed over the devices |
| `write_bytes` | Bytes written by the cgroup during the phase, summed over the devices |

## Requirements

| Requirement | Version |
| - | - |
| Linux kernel | cgroup v2 enabled (Linux 4.5) |
| Permissions | Write access to the cgroup hierarchy: root, or a delegated subtree (e.g., a systemd user session) with the required controllers enabled |
