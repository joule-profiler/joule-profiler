# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/joule-profiler/joule-profiler/releases/tag/joule-profiler-source-cgroup-v0.1.0) - 2026-08-12

### Added

- *(cgroups)* added tests to cgroup source and improve error handling
- *(perf)* added cgroup_root to configure the cgroup base path
- *(perf)* added cgroup scoped counters for perf_event source, optimized reading with spawn blocking and added pre_init function to do some sources work before spawning process
- *(config-file)* update all sources configuration and improved configuration file example
- *(cgroup)* added attach_pid and create_cgroup to cgroup config for already created cgroup or already attached pid
- *(cgroup)* added cpu usage metrics, computed with usage_usec and phase time
- *(cgroup)* polling implementation for non monotonic counters
- *(sources)* add cgroup v2 source base implementation

### Fixed

- sources configurations serialization fixed
- *(procfs)* ignore measure when race condition between process exit and measure, also clippy warnings fix

### Other

- update cgroup error doc
- add warning if the cgroup is already created and the configuration specify to create it
- update readmes, cli and cgroup doc
- some minor fixes in RAPL and cgroup
- changed all tokio::sync::Mutex to std::sync::Mutex because there's no contention on Mutexes
- fix clippy --all-targets warnings
- *(cgroup)* put default polling interval to 1ms
- *(cgroup)* improved error handling, readme documentation, improved configuration and removed controller creation (delegated to user for performance)
- constrained root and child cgroup types for safety
- added test for cgroup, mocked cgroup backend to abstract from sysfs
- *(cgroup)* rename IO metrics in README to match the exported one
- update README of cgroup source, also fixed other sources README
- *(cgroup)* tested cgroup source
- *(cgroup)* documented cgroup source
- split root cgroup and child cgroups in two separate structs
- *(cgroup)* source refactoring, separation into modules and improved logic
