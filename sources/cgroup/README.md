# Cgroup metric source

Cgroup v2 metric source for [joule-profiler](https://github.com/joule-profiler/joule-profiler).

This crate implements a `MetricReader` from `joule-profiler-core` and collects **process-level** and **system-wide** metrics using Linux **cgroup v2** interfaces.

Control groups (cgroups v2) are a Linux kernel feature that allows grouping processes and tracking/limiting their resource usage.
This source uses cgroup files exposed under `/sys/fs/cgroup`.

## Implemented metrics

All metrics are reported for both the process cgroup and the root cgroup, thus they are prefixed with a `proc` or a `global`.

Here is the list of the metrics implemented.

### CPU

| Metric | Description |
| - | - |
| `usage_usec` | Total CPU time consumed (user + kernel) |
| `user_usec` | CPU time spent in user space |
| `system_usec` | CPU time spent in kernel space |
| `nr_periods` | Number of scheduling periods |
| `nr_throttled` | Number of CPU throttling events |
| `throttled_usec` | Total time spent throttled |
| `nr_bursts` | Number of burst events (burstable cgroups) |
| `burst_usec` | Total time spent in burst mode |
| `cpu_usage` | CPU usage in percentage |

### Memory

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
| `read_bytes` | Total number of bytes read by the cgroup |
| `write_bytes` | Total number of bytes written by the cgroup |

## Requirements

| Requirement | Version |
| - | - |
| Linux kernel | cgroup v2 enabled (Linux 4.5) |
| Permissions | write access to cgroup filesystem (usually root privileges) |
