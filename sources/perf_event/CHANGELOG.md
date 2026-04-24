# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.1](https://github.com/joule-profiler/joule-profiler/releases/tag/joule-profiler-source-perf_event-v1.0.1) - 2026-04-24

### Added

- added MetricValue to support various metric types
- perf_event hardware counters support

### Fixed

- try to open perf counters without cpu mask but online cpus
- inherited perf counters cannot be grouped
- *(perf_counters)* inherit counters by subprocess and threads

### Other

- removed workspace version to be able to release all crates separately and aligned all versions
- implemented new with generics for Sensor and Metric to provide better library and cleanup code
- remove iteration mode
- error types documentation
- sort MetricReader implementations functions order by the trait order, fix doctests
- test perf_event source using generics and mockall, also include kernel and hv code in counters
- centralized shared dependencies
- centralized clippy configuration in Cargo.toml
- removed unrelevant tests in json format and unit function in perf_event
- updated perf_event source description in Cargo.toml
- added clippy pedantic warnings and fixed them
- CORE and CLI integration tests, unit testing of all parts of the project
- delayed snapshot differences computation on retrieval for perf and nvml
- put reset logic in reset instead of init, added nbody.py to example programs
- perf_event calculating snapshot difference instead of resetting counters to minimize introduced overhead, also added logging
- update perf_event tags in Cargo.toml
