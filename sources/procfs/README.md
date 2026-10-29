# Procfs metric source

Procfs metric source for [Joule Profiler](https://github.com/joule-profiler/joule-profiler).

This sources collects memory, I/O and CPU metrics at **process-level** and, optionally, **system-wide** using the Linux **procfs** interface.

The implementation relies on the `/proc` filesystem to read runtime information about the memory, I/O and CPU time of the profiled program and of every process it starts.

## Architecture

Three polling threads run during the profile, each at its own interval:
* one reads the memory of the process tree every `process_poll_interval`;
* one lists the process tree again from the profiled program PID every `hierarchy_poll_interval`, since that is the costly part;
* one reads the memory of the machine every `global_poll_interval`, only when `global` is set.

At each phase boundary, the I/O and CPU time of the process tree are read directly, and the memory is reported as the minimum, maximum and average of what was polled during the phase.

## Configuration

```toml
[sources.procfs]
process_poll_interval = "100ms"   # the memory of the process tree
hierarchy_poll_interval = "500ms" # how often the process tree is listed again
global = false                    # also reports the memory and CPU time of the machine
global_poll_interval = "10ms"     # the memory of the machine, when `global`
process_memory_unit = "byte"      # byte | kilobyte | megabyte | gigabyte
global_memory_unit = "byte"       # the memory of the machine
```

## Implemented metrics

Process metrics are prefixed with `proc`, and system-wide metrics, reported only with `global = true`, with `global`.

### Memory (process-level)

Each value is reported with a `_min`, `_max` and `_avg` suffix, in `process_memory_unit`. The multiples are decimal: a kilobyte is 1000 bytes.

| Metric | Description |
| - | - |
| `proc_vm_size` | Virtual memory size |
| `proc_rss` | Resident set size |
| `proc_pss` | Proportional set size |
| `proc_shared` | Shared memory usage |
| `proc_anon` | Anonymous memory usage |

### Memory (system-wide)

Each value is reported with a `_min`, `_max` and `_avg` suffix, in `global_memory_unit`.

| Metric | Description |
| - | - |
| `global_mem_used` | System memory usage |
| `global_cached` | Page cache usage |
| `global_anon` | Anonymous memory usage |
| `global_swap_free` | Available swap |

### I/O (process-level)

| Metric | Description |
| - | - |
| `proc_io_read_bytes` | Bytes read by the process tree during the phase |
| `proc_io_write_bytes` | Bytes written by the process tree during the phase |

### CPU (process-level)

| Metric | Description |
| - | - |
| `proc_cpu_user` | CPU time spent in user space during the phase, in ms |
| `proc_cpu_system` | CPU time spent in kernel space during the phase, in ms |

### CPU (system-wide)

| Metric | Description |
| - | - |
| `global_cpu_user` | CPU time the machine spent in user space during the phase, `nice` included, in ms |
| `global_cpu_system` | CPU time the machine spent in kernel space during the phase, interrupts included, in ms |
| `global_cpu_idle` | CPU time the machine spent idle during the phase, `iowait` included, in ms |

The times add up over every CPU: on a machine with 8 CPUs, a phase of 1 s accounts for 8000 ms.

### Design notes

* Memory metrics are tracked as extrema and averages over the phase rather than instantaneous snapshots
* System-wide metrics are derived from `/proc/meminfo` and `/proc/stat`
* I/O metrics come from `/proc/[pid]/io`, CPU time from `/proc/[pid]/stat`
* The process tree is listed again periodically to include the processes started during the run

## Requirements

| Requirement | Version |
| - | - |
| Linux kernel | Procfs support (all modern Linux kernels) |
| Permissions | Read access to `/proc` (typically available to all users) |

## Notes

> [!NOTE]
> Some metrics may be unavailable depending on kernel version, configuration or permissions
