# Changelog

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
