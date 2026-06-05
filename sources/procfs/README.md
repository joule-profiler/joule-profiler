# Procfs metric source

Procfs metric source for [joule-profiler](https://github.com/joule-profiler/joule-profiler).

This crate implements the `MetricReader` trait from `joule-profiler-core` and collects **process-level** and **system-wide** metrics using Linux **procfs** interface.

The implementation relies on `/proc` filesystem to read runtime information about memory and I/O activity at process and system level.

To do so, it uses two Tokio asynchronous background tasks:
* one continuously polls and aggregates performance metrics.
* the other continuously polls and rebuilds the process hierarchy.

This separation allows efficient metric sampling while maintaining the overhead introduce at the lowest and rebuilding the process hierarchy efficiently.

## Architecture

### Asynchronous polling

* **Metrics polling task:**
  * Periodically reads the procfs
  * Aggregates memory and I/O metrics
  * Maintains snapshots per process and for the whole system

* **Process hierarchy task:**
  * Periodically scans `/proc`
  * Builds a process tree from the profiled program PID.

## Implemented metrics

All metrics are reported for both:
- individual processes
- system-wide

Metrics are prefixed with `proc` or `global`.

### Memory (process-level)

| Metric | Description |
| - | - |
| `proc_vm_size_min` | Minimum virtual memory size observed |
| `proc_vm_size_max` | Maximum virtual memory size observed |
| `proc_rss_min` | Minimum resident set size |
| `proc_rss_max` | Maximum resident set size |
| `proc_pss_min` | Minimum proportional set size |
| `proc_pss_max` | Maximum proportional set size |
| `proc_shared_min` | Minimum shared memory usage |
| `proc_shared_max` | Maximum shared memory usage |
| `proc_anon_min` | Minimum anonymous memory usage |
| `proc_anon_max` | Maximum anonymous memory usage |

### Memory (system-wide)

| Metric | Description |
| - | - |
| `global_mem_used_min` | Minimum system memory usage observed |
| `global_mem_used_max` | Maximum system memory usage observed |
| `global_cached_min` | Minimum page cache usage |
| `global_cached_max` | Maximum page cache usage |
| `global_anon_min` | Minimum anonymous memory usage (system-wide) |
| `global_anon_max` | Maximum anonymous memory usage (system-wide) |
| `global_swap_free_min` | Minimum available swap observed |
| `global_swap_free_max` | Maximum available swap observed |

### I/O (process-level)

| Metric | Description |
| - | - |
| `proc_io_read_bytes` | Total bytes read by the process |
| `proc_io_write_bytes` | Total bytes written by the process |

### Design notes

* Memory metrics are tracked as time-series extrema (min/max) rather than instantaneous snapshots
* System-wide metrics are derived from `/proc/meminfo` and aggregated process data
* I/O metrics come from `/proc/[pid]/io`
* Process hierarchy is rebuilt continuously to detect the most accurate bounds for each metrics

## Requirements

| Requirement | Version |
| - | - |
| Linux kernel | Procfs support (all modern Linux kernels) |
| Permissions | Read access to `/proc` (typically available to all users, some fields may require elevated privileges) |

## Notes

> [!NOTE]
> Some metrics may be unavailable depending on kernel configuration or permissions
