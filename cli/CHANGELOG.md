# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [3.0.0](https://github.com/joule-profiler/joule-profiler/compare/joule-profiler-cli-v2.1.1...joule-profiler-cli-v3.0.0) - 2026-08-12

### Added

- *(cli)* override configuration with -D flag
- *(config-file)* update all sources configuration and improved configuration file example
- *(config)* RAPL config fix, removed CLI override
- *(config)* replace CLI override with function to abstract override logic
- *(config)* Config deserialization using toml crate, override with CLI using CliOverride trait
- amdsmi source implementation
- *(nvml)* added power, vram and GPU usage
- *(cgroup)* polling implementation for non monotonic counters
- *(sources)* add cgroup v2 source base implementation
- *(procfs)* added config struct
- procfs source implementation, memory done and io operations unstable
- added default value in CLI for token-pattern and init-timeout for CLI documentation
- *(cli)* removed per-source cli option and replaced it with comma-separated list

### Fixed

- sources configurations serialization fixed
- config file with empty sources serialized as empty
- *(config)* all sources configurations deserialization fixed
- escape quoted fields and semicolons
- added CLI flag --init-timeout to be able to configure it on slow devices

### Other

- add features flags to enable or disable features
- add examples directory link into readme
- *(cli)* remove rapl polling in profiler config because now it can be set with -D
- update readme for -D CLI flag
- update readme with examples directory doc
- update readme examples
- update readmes, cli and cgroup doc
- update config table documentation
- update RAPL new functions doc and cli doc
- fix clippy --all-targets warnings
- changed output format from --json --csv to --output-format and fixed file creation when no parent path is provided.
- *(config)* fix RAPL sockets spec using config, clippy warnings, cargo fmt
- *(config)* fix former documentation
- *(config)* documented all config functions
- *(amdsmi)* changed default polling interval to 20 milliseconds
- cargo fmt
- *(amdsmi)* rename source and error better handling
- update amdsmi config with default in cli
- *(nvml)* changed polling interval to 20 hz because the majority of counters does not have a higher frequency
- *(cgroup)* put default polling interval to 1ms
- cargo fmt
- *(procfs)* procfs source tests implementation
- move configuration directly into procfs struct
- *(procfs)* improved procfs source code
- improved ProfileConfig builder usage
- forbid source duplication in cli --sources

## [2.1.1](https://github.com/joule-profiler/joule-profiler/compare/joule-profiler-cli-v2.1.0...joule-profiler-cli-v2.1.1) - 2026-04-29

### Other

- Update README link to doc and update CONTRIBUTING.md link
- *(README)* added cargo installation method to README
- Update command example (phase -> profile)
- Update docs link in readme
- *(CARGO)* Add readme link for cli crate

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
