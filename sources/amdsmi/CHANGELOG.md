# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/joule-profiler/joule-profiler/releases/tag/joule-profiler-source-amdsmi-v0.1.0) - 2026-08-12

### Added

- *(config-file)* update all sources configuration and improved configuration file example
- *(amdsmi)* added GPU utilization metrics
- *(amdsmi)* gpu devices filtering with spec implementation
- *(amdsmi)* add gpu specification for device filtering
- amdsmi source implementation

### Other

- Fix amdsmi dep
- changed all tokio::sync::Mutex to std::sync::Mutex because there's no contention on Mutexes
- fix clippy --all-targets warnings
- *(amdsmi)* typo in README.md
- *(amdsmi)* changed default polling interval to 20 milliseconds
- *(amdsmi)* parallelized AMD SMI measurements into blocking thread pool
- *(amdsmi)* added source README.md
- *(amdsmi)* replaced local amdsmi path to github repository
- *(amdsmi)* test AMD SMI source
- *(amdsmi)* document AMD SMI hardware
- *(amdsmi)* rename source and error better handling
- *(amdsmi)* fix clippy warnings
- *(amdsmi)* AMD SMI source documentation
- *(amdsmi)* source refactoring, lock mutex once for counter update
- *(amdsmi)* ignore not supported devices at the initialization
