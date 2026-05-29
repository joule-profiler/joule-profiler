# perf_event metric source

Performance counter source for `joule-profiler`. using the Linux perf_event subsystem.
This crate implements `MetricSource` from `joule-profiler-core` and collects hardware and software performance counters (CPU cycles, instructions, cache misses, branch mispredictions…) via the perf_event_open(2) syscall, per phase.

## Overview

`perf_event` is the Linux kernel's performance monitoring API, available since kernel 2.6.31. It provides access to a wide range of hardware PMU counters, software counters, and kernel tracepoints. In the context of `joule-profiler`, these counters complement energy measurements by revealing the execution characteristics of each phase allowing you to correlate energy with IPC, cache efficiency, etc.

---

## Requirements

| Requirement | Details |
|---|---|
| OS | Linux kernel 2.6.31+ |
| CPU | Any architecture with PMU support (x86, ARM, RISC-V…) |
| Permissions | Root, or `kernel.perf_event_paranoid ≤ 1` |

### Adjusting perf_event_paranoid

```bash
# Check current value
cat /proc/sys/kernel/perf_event_paranoid
 
# Allow per-process counters for unprivileged users (temporary)
sudo sysctl -w kernel.perf_event_paranoid=1
 
# Persistent
echo 'kernel.perf_event_paranoid=1' | sudo tee /etc/sysctl.d/99-perf.conf
sudo sysctl --system
```
---

## Scope

At the moment, Joule Profiler attaches `perf_event` counters to the **monitored process** only (per-process mode).

---

## See also

> Main project: [joule-profiler](https://github.com/joule-profiler/joule-profiler)