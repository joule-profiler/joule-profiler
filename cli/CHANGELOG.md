# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [2.1.0](https://github.com/joule-profiler/joule-profiler/compare/joule-profiler-cli-v2.0.0...joule-profiler-cli-v2.1.0) - 2026-04-24

### Added

- force cli release

## [2.0.0](https://github.com/joule-profiler/joule-profiler/compare/joule-profiler-cli-v1.0.1...joule-profiler-cli-v2.0.0) - 2026-04-24

### Added

- [**breaking**] force cli release

## [1.0.1](https://github.com/joule-profiler/joule-profiler/releases/tag/joule-profiler-cli-v1.0.1) - 2026-04-24

### Added

- added MetricValue to support various metric types
- changed milliseconds timestamps to microseconds
- perf_event hardware counters support
- added new metrics unit fixed unit in Metric (was string)
- perf_events RAPL counters support implementation
- added cli --gpu option to activate NVML and logging
- added nvml support, missing some doc
- cleaned lib exposure from core, exposing only usefull traits

### Fixed

- doctest fix and cargo fmt
- command not executed with root rights if JouleProfiler is, added '--root' flag to bypass it
- removed rapl path in perf backend
- CLI --version was considered an error

### Other

- removed workspace version to be able to release all crates separately and aligned all versions
- implemented new with generics for Sensor and Metric to provide better library and cleanup code
- rename config param 'with_root' to 'use_root'
- update 'with_root' config option
- update phases args with profile args
- rename phase command to profile command
- removed all iterations mentions in doc
- remove all remaining iterations code, replace pid atomic i32 by one shot channel to initialize sources once
- cargo fmt
- remove iteration mode
- centralized shared dependencies
- centralized clippy configuration in Cargo.toml
- removed unrelevant tests in json format and unit function in perf_event
- remove unrelevant CSV output format test
- remove unrelevant test in json output format
- removed must_use compilation flags added by clippy
- removed CLI integration test because it requires RAPL counters
- added clippy pedantic warnings and fixed them
- CORE and CLI integration tests, unit testing of all parts of the project
- removed unused imports
- simplify rapl domain index in Snapshot
- simplified CLI initialization, PSYS display without socket now
- removed tokio dependency when not needed (nvml) and available features only when required
- moved sockets spec str parsing in CLI, separate perf and powercap modules
- fix doctest, clippy warnings and rapl lib exposure cleaned
- display better warning when NVML driver cannot be loaded
- cargo fmt and fix clippy warnings
- nvml source doc added
- rename profiler accessible functions for readability, displayer implement profile method and handle iterations instead of having logic in cli
- rename start_line and end_line with token prefix for phases
- fix doctests and improved public documentation
- uncoupled rapl config from profiler config, added errors at initialization to avoid executing profiler when an error occured
- fix doctest and added documentation to MetricSourceRuntime
- removed high coupling between accumulator and source, using events to poll from Rapl
- move displayer and outputs from core to CLI
- removed coupling between CLI and core, displayer and core
- put displayers implementation into their own crates in outputs
- *(workspace)* add package metadata and remove unused dependencies
- *(workspace)* first workspace split (needs cleanup)
