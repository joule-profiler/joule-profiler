use anyhow::Result;
use joule_profiler_core::{JouleProfiler, source::MetricReader};
use serde::Deserialize;

use crate::config::table::ConfigTable;

#[derive(Debug, Default, Deserialize)]
pub struct MetricSourceConfig<T> {
    #[serde(flatten)]
    pub inner: T,
    #[serde(default)]
    pub ignore_on_failure: bool,
}

#[macro_export]
macro_rules! register_sources {
    ($profiler:expr, $configs:expr, [$($source:ty),* $(,)?]) => {
        $(register_source::<$source>($profiler, $configs)?;)*
    };
}

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

pub fn register_source_override<R, O>(
    profiler: &mut JouleProfiler,
    config_table: &mut ConfigTable,
    config_override: &mut O,
    config_override_fn: impl FnOnce(&mut O, &mut R::Config),
) -> Result<()>
where
    R: MetricReader,
{
    if let Some(reader) =
        config_table.build_source_override::<R, O>(config_override, config_override_fn)?
    {
        profiler.add_source(reader);
    }

    Ok(())
}
