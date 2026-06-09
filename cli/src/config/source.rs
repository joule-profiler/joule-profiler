use anyhow::Result;
use joule_profiler_core::{JouleProfiler, source::MetricReader};
use serde::Deserialize;

use crate::config::{cli_override::CliOverride, table::ConfigTable};

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
    R::Config: CliOverride,
{
    if let Some(reader) = config_table.build_source::<R>()? {
        profiler.add_source(reader);
    }

    Ok(())
}
