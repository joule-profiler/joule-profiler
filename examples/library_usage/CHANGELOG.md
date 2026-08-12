# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/joule-profiler/joule-profiler/releases/tag/library_usage_example-v0.1.0) - 2026-08-12

### Added

- *(cli)* override configuration with -D flag
- update example and put it in it's own crate, also upgrade tokio version to 1.52.3
- initial implementation of joule-profiler energy measurement tool

### Other

- add examples directory link into readme
- update readme for -D CLI flag
- update readme with examples directory doc
- update readme examples
- update readmes, cli and cgroup doc
- changed all tokio::sync::Mutex to std::sync::Mutex because there's no contention on Mutexes
- Update README link to doc and update CONTRIBUTING.md link
- *(README)* added cargo installation method to README
- Update command example (phase -> profile)
- Update docs link in readme
- *(readme)* [**breaking**] Update link in the readme for doc and repo
- Update docs link in readme
- update readme
- update README for regex-based phase detection
- update installation section with installer and uninstaller script
- *(install)* add system-wide installation to /usr/local/bin
- Initial commit
