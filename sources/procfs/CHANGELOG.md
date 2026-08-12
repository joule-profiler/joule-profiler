# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/joule-profiler/joule-profiler/releases/tag/joule-profiler-source-procfs-v0.1.0) - 2026-08-12

### Added

- *(config-file)* update all sources configuration and improved configuration file example
- *(procfs)* child processes detection put in a separate loop (less polling than measure), implemented a Backend trait for testing
- procfs global memory counters (mem usage, cache, anon, swap)
- *(procfs)* map child processes recursively and measure all
- *(procfs)* removed delta and first measure metrics and added per-phase minimum
- procfs source implementation, memory done and io operations unstable

### Fixed

- sources configurations serialization fixed
- *(config)* all sources configurations deserialization fixed
- *(procfs)* ignore measure when race condition between process exit and measure, also clippy warnings fix
- io counters fix (ignore permission denied error after process termination)

### Other

- fixed warnings and tests
- *(procfs)* use per-process spawn_blocking to read each smap_rollups instead of blocking runtime
- fix clippy --all-targets warnings
- *(procfs)* removed unused pid field in struct
- *(procfs)* documented backend functions
- *(procfs)* added more logging
- *(procfs)* added README.md for procfs source
- *(procfs)* procfs source tests implementation
- *(procfs)* improved procfs polling with cancellation token to gracefully stop the tokio task
- removed sentinel values and use option instead in a more idiomatic way
- move configuration directly into procfs struct
- *(procfs)* documented procfs source code
- *(procfs)* improved to_metrics clarity
- *(procfs)* improved procfs source code
- *(procfs)* fmt, fix clippy warnings and clear sentinel values if any in to_metrics
- added logging for procfs source
