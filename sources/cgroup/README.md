# Cgroup metric source

Cgroup v2 metric source for [joule-profiler](https://github.com/joule-profiler/joule-profiler).

This crate implements a `MetricReader` from `joule-profiler-core` and collects **process-level** and **system-wide** metrics using Linux **cgroup v2** interfaces.

Control groups (cgroups v2) are a Linux kernel feature that allows grouping processes and tracking/limiting their resource usage.
This source uses cgroup files exposed under `/sys/fs/cgroup`.

## Setup

Before using this source, the required cgroup controllers `cpu`, `memory`, `io` must be enabled on the cgroup hierarchy. This source does **not** enables them automatically, it is your responsibility to ensure controllers are active.

To enable them on the root cgroup:

```bash
echo "+cpu +memory +io" | sudo tee /sys/fs/cgroup/cgroup.subtree_control
```

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

> [!IMPORTANT]
> By default, a nested cgroup inherit its parent controllers files. Nonetheless, the parent of the nested cgroup must enable the controllers.

> [!NOTE]
> On systemd-based systems, some controllers may already be enabled. Check `/sys/fs/cgroup/cgroup.subtree_control` first.

### Manual cgroup creation / process attachment

Alternatively, you can configure the source to not create the cgroup or not attach the pid to the configured cgroup with the `create_cgroup` and `attach_pid` options in the config.

To create a cgroup, do:

```bash
sudo mkdir /sys/fs/cgroup/{your_cgroup_path}
```

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

By configuring the cgroup source with `create_cgroup` and `attach_pid` to false, Joule Profiler will be able to profile the process directly at spawn.
This configuration can also benefits to the `perf_event` source. See [perf_event](../perf_event/README.md) source documentation for further information.

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
| Permissions | Write access to the cgroup hierarchy: root, or a delegated subtree (e.g. a systemd user session) with the required controllers enabled |
