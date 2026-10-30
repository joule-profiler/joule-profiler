use std::path::Path;

use anyhow::Result;
#[cfg(any(feature = "cgroup", feature = "perf_event"))]
use joule_profiler_config::Map;
use joule_profiler_config::{SourceName, Value};
use joule_profiler_core::JouleProfiler;

/// Builds the configured sources and adds them to `profiler`. The cgroup source, and perf on a
/// cgroup, measure `cgroup` unless their table names another.
#[cfg_attr(
    not(any(feature = "cgroup", feature = "perf_event")),
    allow(unused_variables, unused_mut)
)]
pub fn add(
    profiler: &mut JouleProfiler,
    sources: Vec<(SourceName, Option<Value>)>,
    cgroup: Option<&Path>,
) -> Result<()> {
    for (name, mut table) in sources {
        #[cfg(feature = "cgroup")]
        if name == SourceName::Cgroup
            && let Some(cgroup) = cgroup
        {
            set_default_cgroup_name(&mut table, cgroup);
        }

        #[cfg(feature = "perf_event")]
        if name == SourceName::Perf
            && let Some(cgroup) = cgroup
            && table
                .as_ref()
                .and_then(|table| table.get("scope"))
                .and_then(Value::as_str)
                == Some("cgroup")
        {
            set_default_cgroup_name(&mut table, cgroup);
        }

        log::debug!("building source {name}");

        if let Some(source) = name.build(table)? {
            profiler.add_source(source);
        }
    }

    Ok(())
}

/// Sets the `name` of the table to `cgroup`, unless it has one.
#[cfg(any(feature = "cgroup", feature = "perf_event"))]
fn set_default_cgroup_name(table: &mut Option<Value>, cgroup: &Path) {
    if let Some(table) = table
        .get_or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
    {
        table
            .entry("name")
            .or_insert_with(|| Value::String(cgroup.to_string_lossy().into_owned()));
    }
}
