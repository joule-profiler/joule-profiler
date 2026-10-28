# RAPL metric source

RAPL source for [Joule Profiler](https://github.com/joule-profiler/joule-profiler).

This sources measures the energy consumed by the processor and the memory over each phase, through one of two backends: **perf_event** (the `power` PMU, default backend) or **powercap** (the `/sys/class/powercap` sysfs). The backend can be selected with the `backend` key of the `[sources.rapl]` table, from a configuration file or with `-D sources.rapl.backend=powercap` on the command line.

## What is RAPL?

RAPL (Running Average Power Limit) is a feature of Intel processors since **Sandy Bridge**, and of AMD processors since **Zen**. It exposes accumulated energy counters for several hardware domains. Linux makes them available through two interfaces: the **perf_event** subsystem and the **powercap** sysfs framework.

### Domains

| Domain | Description |
|---|---|
| **Package / PKG** | Entire CPU socket (cores + uncore) |
| **Core / PP0** | CPU cores only |
| **Uncore / PP1** | Integrated GPU |
| **DRAM** | Memory attached to the socket |
| **PSYS** | Whole platform (Skylake+, mostly laptops) |

The domains available depend on the processor. Both backends find them at startup.

### Metrics

Each domain of each socket is reported as one metric, such as `PACKAGE-0` or `DRAM-1`. PSYS has no socket and is reported once, as `PSYS`.

On a processor with several dies per socket (Linux 5.3+), each die has its own counters and is reported on its own, such as `PACKAGE-0-DIE-1`. The die is only named when the socket has several.

## Requirements

| | perf_event | powercap |
|---|---|---|
| OS | Linux 3.14+ | Linux 3.13+ |
| CPU | Intel Sandy Bridge+, AMD Zen+ | Intel Sandy Bridge+, AMD Zen+ |
| Permissions | Root, `CAP_PERFMON`, or `kernel.perf_event_paranoid <= 0` | Root (Linux 5.10+) |

## Configuration

```toml
[sources.rapl]
backend = "perf"       # perf | powercap
unit = "microjoule"    # joule | millijoule | microjoule
sockets = [0]          # omit for every socket, PSYS is always measured
domains = ["package", "core", "uncore", "dram", "psys"] # omit for every domain found
```

The powercap backend takes two more keys:

```toml
[sources.rapl]
backend = "powercap"
root = "/sys/class/powercap" # the default
poll_interval = "1s"         # the default
```

The powercap counters wrap after a few hundred kilojoules. They are read every `poll_interval`, so that no wrap is missed during a long phase.
