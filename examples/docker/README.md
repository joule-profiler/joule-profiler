# Container Profiling with Joule Profiler (Cgroup v2)

This example demonstrates how to profile Docker and containerized workloads using Joule Profiler's Linux cgroup v2 source (`cgroup`).

## Overview

Modern cloud architectures and container runtimes (Docker, Podman, containerd, Kubernetes) rely on Linux **Control Groups v2 (cgroup v2)** to isolate and regulate resource consumption (CPU quotas, memory limits, and I/O throttling).

Joule Profiler provides native support for profiling containers:
- **Automatic Container Discovery**: Specify `--container <NAME_OR_ID>` to automatically locate the container's PID, inspect `/proc/<PID>/cgroup`, and target the container's cgroup slice in `/sys/fs/cgroup`.
- **Rootless Systemd Delegation**: Non-root users can profile workloads seamlessly through systemd user delegation (`user.slice/user-<UID>.slice/user@<UID>.service`).
- **Graceful Degradation**: If specific controllers (such as `io`) are not enabled in a user slice, Joule Profiler logs a debug note and continues capturing available controllers (CPU, memory) without failing.
- **Safety**: Joule Profiler detects existing container cgroups and avoids destroying them on teardown.

---

## Directory Structure

```text
examples/docker/
├── Dockerfile          # Python 3.12-slim container running workload
├── workload.py         # Multi-phase workload with CPU hashing & memory allocations
├── run.sh              # End-to-end automated demonstration script
└── README.md           # Documentation and quickstart
```

---

## Quickstart

### 1. Automated Run

Ensure Docker is running and your user has permissions to run Docker commands (or is part of the `docker` group):

```bash
./examples/docker/run.sh
```

### 2. Manual Step-by-Step

#### Build and start a background container:
```bash
docker build -t joule-profiler-demo:latest examples/docker/
docker run -d --name joule-demo-container joule-profiler-demo:latest sleep 300
```

#### Profile the container during a workload execution:
```bash
joule-profiler \
    --sources cgroup \
    --container joule-demo-container \
    profile -- docker exec joule-demo-container python3 /app/workload.py
```

#### Clean up:
```bash
docker rm -f joule-demo-container
```

---

## Configuration Options

Container targeting can be set via CLI flags, `-D` overrides, or `config.toml`:

### CLI Flag
```bash
joule-profiler --sources cgroup --container <NAME_OR_ID> profile -- <COMMAND>
```

### Dynamic Define (`-D`)
```bash
joule-profiler --sources cgroup -D sources.cgroup.container=<NAME_OR_ID> profile -- <COMMAND>
```

### Configuration File (`config.toml`)
```toml
[sources.cgroup]
container = "my-container-name"
poll_interval = "10ms"
```

---

## Understanding the Metrics

When profiling a container, Joule Profiler reports:
- **`proc_*` metrics**: Metrics isolated to the targeted container's cgroup (e.g. `proc_anon_max`, `proc_peak_max`, `proc_cpu_usage`).
- **`global_*` metrics**: Metrics measured at the root cgroup level for host context.
- **Phase Breakdown**: When workloads emit phase tokens (such as `__COMPUTE_START__`, `__COMPUTE_END__`), metrics are accurately attributed to each distinct phase of execution.
