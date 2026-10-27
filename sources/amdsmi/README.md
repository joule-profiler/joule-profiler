# AMD SMI metric source

AMD SMI metric source for [Joule Profiler](https://github.com/joule-profiler/joule-profiler).

This crate builds a `Source` for Joule Profiler that measures AMD GPU energy counters via the **AMD System Management Interface** library. It also tracks VRAM usage and GPU utilization at a configurable polling interval.

## What is AMD SMI?

AMD SMI (System Management Interface) is the AMD library for monitoring and managing AMD GPU devices. It exposes some energy accumulation counters, instantaneous power draw, VRAM usage, and GPU utilization for the GPUs on the system.

This source relies on the [amd-smi-wrapper](https://github.com/joule-profiler/amd-smi-wrapper) crate, a safe Rust wrapper around the AMD SMI C library with pre-generated bindings for the latest supported version.

When a device supports direct energy accumulation registers, the source reads them directly. Otherwise it falls back to integrating instantaneous power samples using the trapezoidal rule.

### Metrics

| Metric | Unit | Description |
|---|---|---|
| `GPU-{uuid}-energy` | µJ | Energy consumed between two measurements |
| `GPU-{uuid}-vram_min` | Bytes | Minimum VRAM usage observed during the interval |
| `GPU-{uuid}-vram_max` | Bytes | Maximum VRAM usage observed during the interval |
| `GPU-{uuid}-vram_avg` | Bytes | Average VRAM usage observed during the interval |
| `GPU-{uuid}-utilization_min` | % | Minimum GPU utilization observed during the interval |
| `GPU-{uuid}-utilization_max` | % | Maximum GPU utilization observed during the interval |
| `GPU-{uuid}-utilization_avg` | % | Average GPU utilization observed during the interval |

> Available metrics depend on device support. By default (`minimal = true`) only the energy is reported, set `minimal = false` to also poll VRAM and utilization.

## Configuration

```toml
[sources.amdsmi]
gpus = ["GPU-00000000-0000-0000-0000-000000000000"] # device UUIDs, omit for every GPU
minimal = true         # the default, energy only, without VRAM and utilization
poll_interval = "20ms" # the default
```

## Requirements

| | |
|---|---|
| Hardware | AMD GPU |
| Library | `amd-smi-lib` |

## Bindings

The AMD SMI C library bindings are pre-generated for the latest supported version and distributed via [amd-smi-wrapper](https://github.com/joule-profiler/amd-smi-wrapper). If you need to target a different version of `amd-smi-lib`, you can regenerate them locally, see the wrapper repository for instructions.