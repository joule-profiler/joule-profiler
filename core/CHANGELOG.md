# Changelog

## [2.0.0](https://github.com/joule-profiler/joule-profiler/compare/joule-profiler-core-v1.0.2...joule-profiler-core-v2.0.0) - 2026-08-12

### Added

- *(perf)* added cgroup scoped counters for perf_event source, optimized reading with spawn blocking and added pre_init function to do some sources work before spawning process
- *(config)* Config deserialization using toml crate, override with CLI using CliOverride trait
- *(cgroup)* added cpu usage metrics, computed with usage_usec and phase time
- *(cgroup)* polling implementation for non monotonic counters
- procfs source implementation, memory done and io operations unstable
- added init validation in orchestrator and refactored orchestrator

### Fixed

- *(config)* all sources configurations deserialization fixed
- check command not empty before spawning process to avoid panic
- added CLI flag --init-timeout to be able to configure it on slow devices
- skip empty sources results and throw an error only when merging different phases counts

### Other

- moved new types into types.rs
- removed spawn blocking for waiting and put oneshot sender
- profiler.rs improvement
- spawn reader task in a separate thread to not block the async runtime
- spawn phase detection mechanism in its own thread instead of blocking async runtime
- update RAPL new functions doc and cli doc
- some minor fixes in RAPL and cgroup
- *(nvml)* use spawn blocking for each gpu to avoid blocking the async runtime
- fix clippy --all-targets warnings
- changed output format from --json --csv to --output-format and fixed file creation when no parent path is provided.
- *(nvml)* fix tests after updating nvml hardware
- update doc for metric reader type bound
- *(cgroup)* improved error handling, readme documentation, improved configuration and removed controller creation (delegated to user for performance)
- *(procfs)* fmt, fix clippy warnings and clear sentinel values if any in to_metrics
- orchestrator refactoring
- orchestrator refactoring for source initialization
- update orchestrator documentation
- documented orchestrator measure method to indicate that it does not ensure measurement completion
- removed former default ProfileConfig values in core
- improved ProfileConfig builder usage
- fix doctests with new CLI flag

## [1.0.2](https://github.com/joule-profiler/joule-profiler/compare/joule-profiler-core-v1.0.1...joule-profiler-core-v1.0.2) - 2026-04-29

### Fixed

- MetricValue serialization implementation to inline values in json results

## [1.0.1](https://github.com/jwoirhaye/joule-profiler/compare/v1.0.0...v1.0.1) (2026-01-08)


### Bug Fixes

* skip non-UTF-8 stdout lines and propagate other I/O errors ([e34fcfc](https://github.com/jwoirhaye/joule-profiler/commit/e34fcfc059577a4f7739061dc177caaad1bb90ad))

## [1.0.0](https://github.com/jwoirhaye/joule-profiler/compare/v0.2.0...v1.0.0) (2025-11-23)


### ⚠ BREAKING CHANGES

* - Update Config struct: token_pattern replaces token_start/token_end

### Features

* display token metadata in terminal phase output ([1917f64](https://github.com/jwoirhaye/joule-profiler/commit/1917f6447def21bbb10c70403add094cc8847b8e))


### Bug Fixes

* resolve clippy warnings in output/csv.rs ([44be94a](https://github.com/jwoirhaye/joule-profiler/commit/44be94a081cc14c146c04ef438e440fa38437fe8))


### Code Refactoring

* replace --token-start/--token-end with regex-based --token-pattern ([87dc619](https://github.com/jwoirhaye/joule-profiler/commit/87dc619d84fd0e7543b47116a9b6362fac4fee8a))

## [0.2.0](https://github.com/jwoirhaye/joule-profiler/compare/v0.1.0...v0.2.0) (2025-11-23)


### Features

* add example data files and test program ([7ac087f](https://github.com/jwoirhaye/joule-profiler/commit/7ac087fda8a44cd63ce64a11f59c65c6b92e9875))
* add executed command to all output formats ([9291031](https://github.com/jwoirhaye/joule-profiler/commit/92910313ef00dec638e35090f3884b7833abb2b1))
* add installer script ([9177a6b](https://github.com/jwoirhaye/joule-profiler/commit/9177a6bb20198e687c586fe310c1432421bca317))
* add phases analysis notebook ([7e4a1be](https://github.com/jwoirhaye/joule-profiler/commit/7e4a1bedb61526b0d7d3dd23a4464b01c04236f0))
* add quickstart notebook for simple mode visualization ([884cf40](https://github.com/jwoirhaye/joule-profiler/commit/884cf406a2610a9a399aafb330203acb7223751a))
* add uninstaller script ([1c7a8f0](https://github.com/jwoirhaye/joule-profiler/commit/1c7a8f0be45d9fc85ee99fd1569606a3e7cd30ac))

## 0.1.0 (2025-11-22)


### Features

* initial implementation of joule-profiler energy measurement tool ([c4483b3](https://github.com/jwoirhaye/joule-profiler/commit/c4483b3df369924b15c25dece5fa37ea7b65d413))
