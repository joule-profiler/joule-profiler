//! Intel RAPL metric source for Joule Profiler.
//!
//! This module provides several implementations of [`MetricReader`] for
//! collecting energy metrics from Intel RAPL (Running Average Power Limit) domains.
//!
//! # Backends
//!
//! This module supports **two backends** for reading energy metrics:
//! - [`powercap`] - uses the Linux `powercap` interface for energy readings.
//! - [`perf`] - uses `perf_event` counters (`perf_event_open`) for RAPL domains.

use joule_profiler_core::unit::{MetricUnit, Unit, UnitPrefix};

mod domain_type;
mod error;

#[cfg(not(any(feature = "backend-perf", feature = "backend-powercap")))]
compile_error!("Enable at least one backend: `backend-perf` or `backend-powercap`.");

#[cfg(feature = "backend-perf")]
pub mod perf;

#[cfg(feature = "backend-powercap")]
pub mod powercap;

mod snapshot;
mod util;

pub use error::RaplError;

/// Custom result type for Rapl
type Result<T> = std::result::Result<T, RaplError>;

const MICRO_JOULE_UNIT: MetricUnit = MetricUnit {
    prefix: UnitPrefix::Micro,
    unit: Unit::Joule,
};

const RAPL_SOURCE_ID: &str = "rapl";
