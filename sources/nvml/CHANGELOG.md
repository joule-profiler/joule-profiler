# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.1](https://github.com/joule-profiler/joule-profiler/releases/tag/joule-profiler-source-nvml-v1.0.1) - 2026-04-24

### Added

- added MetricValue to support various metric types
- added new metrics unit fixed unit in Metric (was string)
- added cli --gpu option to activate NVML and logging
- nvml sensor name is now of format GPU-{index} instead of the device name
- added nvml support, missing some doc

### Other

- removed workspace version to be able to release all crates separately and aligned all versions
- implemented new with generics for Sensor and Metric to provide better library and cleanup code
- cargo fmt
- remove iteration mode
- error types documentation
- sort MetricReader implementations functions order by the trait order, fix doctests
- replace manual mockall mock with automock macro in NVML source
- test NVML source using generics (doesn't need to change the public API with default generics), and mock with mockall
- centralized shared dependencies
- centralized clippy configuration in Cargo.toml
- added clippy pedantic warnings and fixed them
- CORE and CLI integration tests, unit testing of all parts of the project
- fix missing code documentation
- delayed snapshot differences computation on retrieval for perf and nvml
- put reset logic in reset instead of init, added nbody.py to example programs
- removed tokio dependency when not needed (nvml) and available features only when required
- cargo fmt
- display better warning when NVML driver cannot be loaded
- cargo fmt and fix clippy warnings
- update nvml and unit doc
- nvml source doc added
