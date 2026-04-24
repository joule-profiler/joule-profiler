# joule-profiler-source-nvml


NVIDIA GPU energy source for `joule-profiler` using the NVIDIA Management Library (NVML).

This crate implements MetricSource from joule-profiler-core by querying NVIDIA GPU energy counters through the nvml-wrapper Rust crate,

## Overview

NVML is the C-based API used internally by 'nvidia-smi'. It provides direct access to the NVIDIA GPU driver and exposes hardware energy counters, power draw, utilisation, clock speeds, memory usage, and more.
`joule-profiler` uses NVML exclusively for energy measurement: it reads the cumulative energy counter (nvmlDeviceGetTotalEnergyConsumption) at each phase boundary and reports the delta in millijoules (mJ)

---

## Metrics collected

| Metric | Unit | Description |
|---|---|---|
| Energy consumption | µJ | Cumulative energy delta per phase (from NVML energy counter) |

---
## Requirements

| Requirement | Details |
|---|---|
| OS | Linux |
| GPU | NVIDIA GPU |
| Driver | NVIDIA driver with `libnvidia-ml.so` (included in standard driver packages) |
| Permissions | Typically no extra permissions beyond driver access |


## See also

> Main project: [joule-profiler](https://github.com/joule-profiler/joule-profiler)