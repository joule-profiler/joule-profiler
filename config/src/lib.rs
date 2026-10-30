//! Configuration of Joule Profiler.
//!
//! A [`ConfigTable`] is read from a file, then overridden key by key, before it is deserialized
//! into the different sections.
//!
//! The table is held as a [`serde_json::Value`] tree, which is only a representation: any format
//! serde supports could be loaded. The `toml` and `json` features choose the formats a
//! table can be read from.
//!
//! # Example
//!
//! ```no_run
//! use joule_profiler_config::{ConfigTable, ProfilerConfig, SourceName, configured_sources};
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct Config {
//!     #[serde(default)]
//!     profiler: ProfilerConfig,
//!
//!     #[serde(default)]
//!     sources: joule_profiler_config::Map<String, joule_profiler_config::Value>,
//! }
//!
//! let mut table = ConfigTable::read("config.toml".as_ref()).unwrap();
//! table.define("sources.procfs.global=true").unwrap();
//!
//! let config: Config = table.deserialize().unwrap();
//! for (name, table) in configured_sources(&[], &config.sources).unwrap() {
//!     let source = name.build(table).unwrap();
//! }
//! ```

#[cfg(not(any(
    feature = "_rapl",
    feature = "perf_event",
    feature = "procfs",
    feature = "cgroup",
    feature = "nvml",
    feature = "amdsmi",
)))]
compile_error!("at least one source feature must be enabled");

mod error;
mod section;
mod source;
mod table;

pub use serde_json::{Map, Value};

pub use error::ConfigError;
pub use section::ProfilerConfig;
pub use source::{SourceName, configured_sources};
pub use table::{ConfigTable, Format};
