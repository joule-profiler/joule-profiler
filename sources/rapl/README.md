# RAPL metric source

RAPL energy source for [joule-profiler](https://github.com/joule-profiler/joule-profiler).

This crate implements `MetricSource` from `joule-profiler-core` and measures Intel RAPL energy counters via two interchangeable backends: **powercap** (sysfs) and **perf_event** (syscall). The CLI selects the backend automatically or on demand via `--rapl-backend`.

## What is RAPL?

RAPL (Running Average Power Limit) is an Intel processor feature available since the **Sandy Bridge** generation. It exposes accumulated energy counters for different hardware domains, accessible through model-specific registers (MSRs). Linux makes these counters available through two kernel interfaces: the **powercap** sysfs framework and the **perf_event** subsystem.

### Domains

| Domain | Description |
|---|---|
| **Package / PKG** | Entire CPU socket (cores + uncore) |
| **Core / PP0** | CPU cores only |
| **Uncore / PP1** | Integrated GPU (desktop CPUs) |
| **DRAM** | Memory subsystem |
| **PSYS** | Full SoC (Skylake+, laptops only) |

> Available domains depend on the processor model. Both backends auto-discover domains at startup.

---
## Requirements

| | powercap | perf_event |
|---|---|---|
| OS | Linux 3.13+ | Linux 3.14+ |
| CPU | Intel Sandy Bridge+ | Intel Sandy Bridge+ |
| Permissions | Root (kernel ≥ 5.10) | Root or `paranoid ≤ 0` |
---
