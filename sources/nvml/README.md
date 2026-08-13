# NVML metric source

NVIDIA GPU energy source for `joule-profiler` using the NVIDIA Management Library (NVML).

## Overview

NVML is the C-based API used internally by 'nvidia-smi'. It provides direct access to the NVIDIA GPU driver and exposes hardware energy counters, power draw, utilization and memory usage.
This source relies on the [nvml-wrapper](https://github.com/rust-nvml/nvml-wrapper) crate to query the NVML driver.

### Metrics

| Metric | Unit | Description |
|---|---|---|
| `GPU-{id}-energy` | µJ | Energy consumed between two measurements |
| `GPU-{id}-vram_min` | Bytes | Minimum VRAM usage observed during the interval |
| `GPU-{id}-vram_max` | Bytes | Maximum VRAM usage observed during the interval |
| `GPU-{id}-utilization_min` | % | Minimum GPU utilization observed during the interval |
| `GPU-{id}-utilization_max` | % | Maximum GPU utilization observed during the interval |

> Available metrics depend on device support. VRAM and utilization require a polling interval to be configured.

## Requirements

| Requirement | Details |
|---|---|
| Hardware | NVIDIA GPU |
| Driver | NVIDIA driver with `libnvidia-ml.so` (included in standard driver packages) |
