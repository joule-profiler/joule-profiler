//! Source registration helpers for Joule Profiler.

use anyhow::Result;
use joule_profiler_core::{JouleProfiler, source::MetricReader};
use serde::Deserialize;

use crate::config::table::ConfigTable;

/// Metric source configuration wrapper.
#[derive(Debug, Default, Deserialize)]
pub struct MetricSourceConfig<T> {
    /// The source configuration, deserialized from a TOML table.
    #[serde(flatten)]
    pub inner: T,

    ///  When true, a failure to initialize this source is logged as a warning and silently skipped rather than propagating an error.
    #[serde(default)]
    pub ignore_on_failure: bool,
}

/// Builds a metric source reader of type `R` from `config_table` and, if
/// successful, attaches it to `profiler`.
///
/// If the source is disabled or its initialization fails with
/// `ignore_on_failure` set, no source is added and `Ok(())` is returned
/// silently.
///
/// Returns an error if source initialization fails and `ignore_on_failure` is
/// not set in the source's config.
pub fn register_source<R>(
    profiler: &mut JouleProfiler,
    config_table: &mut ConfigTable,
) -> Result<()>
where
    R: MetricReader,
{
    if let Some(reader) = config_table.build_source::<R>()? {
        profiler.add_source(reader);
    }
    Ok(())
}

/// Builds a metric source reader with an external config override,
/// then attaches it to `profiler` if initialization succeeds.
///
/// This is the override variant of [`register_source`], used when another
/// subsystem needs to inject values into the source's config before it is
/// constructed. The caller supplies an override object `O` and a closure the source's config.
///
/// If the source is disabled or its initialization fails with
/// `ignore_on_failure` set, no source is added and `Ok(())` is returned.
/// Returns an error if source initialization fails and `ignore_on_failure` is
/// not set in the source's config.
pub fn register_source_override<R, O>(
    profiler: &mut JouleProfiler,
    config_table: &mut ConfigTable,
    config_override_fn: impl FnOnce(&mut R::Config),
) -> Result<()>
where
    R: MetricReader,
{
    if let Some(reader) = config_table.build_source_override::<R>(config_override_fn)? {
        profiler.add_source(reader);
    }
    Ok(())
}
