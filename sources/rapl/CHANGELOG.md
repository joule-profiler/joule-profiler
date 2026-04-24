# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.1](https://github.com/joule-profiler/joule-profiler/releases/tag/joule-profiler-source-rapl-v1.0.1) - 2026-04-24

### Added

- added MetricValue to support various metric types
- using /sys/devices/power/cpumask to discover socket primary cpu for perf
- perf_event hardware counters support
- implemented shared child process id (could be extended to a shared state if needed in the future) without modifying SourceEvent enum size (each source refresh state when the init event is sent): needed for perf_event counters pid filtering
- perf_event RAPL implementation with perf_event2 instead of perf_event_open_sys for maintainability and avoid unsafe code (perf_event2 uses unsage still)
- added new metrics unit fixed unit in Metric (was string)
- implemented sockets filtering for rapl, filtering cpus to get only online cores using sysfs: /sys/devices/system/cpu/online
- perf_events RAPL counters support implementation
- added nvml support, missing some doc
- added unit types
- added reset event to start measures from zero
- cleaned lib exposure from core, exposing only usefull traits

### Fixed

- doctest fix and cargo fmt
- command not executed with root rights if JouleProfiler is, added '--root' flag to bypass it
- try to open perf counters without cpu mask but online cpus
- perf_event_paranoid error detection without root user detection (removed libc dep), added logging to perf open domain function, simplified errors
- root privileges detection for perf_rapl (no error thrown if not handled), perf_event_paranoid reading and handling paranoid level for custom error messages

### Other

- removed workspace version to be able to release all crates separately and aligned all versions
- implemented new with generics for Sensor and Metric to provide better library and cleanup code
- clippy warnings and better naming
- remove iteration mode
- error types documentation
- sort MetricReader implementations functions order by the trait order, fix doctests
- replace exclude_hv(false) and kernel with include_hv and kernel in rapl perf backend
- centralized shared dependencies
- centralized clippy configuration in Cargo.toml
- cargo fmt
- updated orchestrator errors and powercap documentation
- added clippy pedantic warnings and fixed them
- CORE and CLI integration tests, unit testing of all parts of the project
- fix missing code documentation
- cargo fmt
- delayed snapshot differences computation on retrieval for perf and nvml
- put reset logic in reset instead of init, added nbody.py to example programs
- perf_event calculating snapshot difference instead of resetting counters to minimize introduced overhead, also added logging
- improved perf RAPL counters initialization and remove some unsafe code
- update perf doc
- simplify rapl domain index in Snapshot
- simplified CLI initialization, PSYS display without socket now
- rapl perf docs
- removed tokio dependency when not needed (nvml) and available features only when required
- put shared logic inside rapl module to reduce code duplication
- moved sockets spec str parsing in CLI, separate perf and powercap modules
- fix doctest, clippy warnings and rapl lib exposure cleaned
- cargo fmt and fix clippy warnings
- fix doctests and improved public documentation
- uncoupled rapl config from profiler config, added errors at initialization to avoid executing profiler when an error occured
- fix tests and doctests
- fix clippy warnings and cargo fmt
- delete SourceEventEmitter and refactor Rapl for all polling logic in sources
- replaced run method for init and join in MetricReader for more separation
- fix doctest and added documentation to MetricSourceRuntime
- rename some structs and changed select! in runtime in a more idiomatic way
- removed high coupling between accumulator and source, using events to poll from Rapl
- changed metric reader type bound of Into<Metrics> to to_metrics in MetricReader trait for more flexibility
- added method has_scheduler in MetricReader trait to efficiently know if metric reader needs to be scheduled or not (mainly for perf)
- reduced overhead of scheduling with biased tokio select (no random number generation), detect when there's no scheduling for a source to avoid more overhead. Facilitate sources scheduling implementation (see Rapl)
- removed unneeded constructors for datatypes, inline variables for perf and clarity
- removed coupling between CLI and core, displayer and core
- *(workspace)* add package metadata and remove unused dependencies
- *(workspace)* first workspace split (needs cleanup)
