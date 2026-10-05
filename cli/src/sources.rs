use std::path::Path;

use anyhow::Result;
use joule_profiler_core::profiler::JouleProfiler;
#[cfg(feature = "amdsmi")]
use joule_profiler_source_amdsmi::AmdSmi;
#[cfg(feature = "cgroup")]
use joule_profiler_source_cgroup::Cgroup;
#[cfg(feature = "nvml")]
use joule_profiler_source_nvml::Nvml;
#[cfg(feature = "perf_event")]
use joule_profiler_source_perf_event::PerfEvent;
#[cfg(feature = "procfs")]
use joule_profiler_source_procfs::Procfs;
#[cfg(feature = "rapl")]
use joule_profiler_source_rapl::Rapl;
use toml::Value;

use crate::cli::SourceName;
use crate::config::{Settings, configure};

/// The cgroup source measures `cgroup` unless its table names another.
#[cfg_attr(not(feature = "cgroup"), allow(unused_variables))]
pub fn add(profiler: &mut JouleProfiler, settings: &Settings, cgroup: Option<&Path>) -> Result<()> {
    for name in settings.sources.iter().copied() {
        let mut table = settings.table(name).cloned();
        let ignore = take_ignore(&mut table);

        #[cfg(feature = "cgroup")]
        if name == SourceName::Cgroup
            && let Some(cgroup) = cgroup
        {
            name_unless_named(&mut table, cgroup);
        }

        log::debug!("building source {name}");

        match build(profiler, name, table.as_ref()) {
            Err(error) if ignore => log::warn!("source `{name}` was left out: {error:#}"),
            outcome => outcome?,
        }
    }

    Ok(())
}

fn build(profiler: &mut JouleProfiler, name: SourceName, table: Option<&Value>) -> Result<()> {
    profiler.add_source(match name {
        #[cfg(feature = "rapl")]
        SourceName::Rapl => configure::<Rapl>(name, table)?.build()?,
        #[cfg(feature = "perf_event")]
        SourceName::Perf => configure::<PerfEvent>(name, table)?.build(),
        #[cfg(feature = "procfs")]
        SourceName::Procfs => configure::<Procfs>(name, table)?.build()?,
        #[cfg(feature = "cgroup")]
        SourceName::Cgroup => configure::<Cgroup>(name, table)?.build()?,
        #[cfg(feature = "nvml")]
        SourceName::Nvml => configure::<Nvml>(name, table)?.build()?,
        #[cfg(feature = "amdsmi")]
        SourceName::AmdSmi => configure::<AmdSmi>(name, table)?.build()?,
    });

    Ok(())
}

#[cfg(feature = "cgroup")]
fn name_unless_named(table: &mut Option<Value>, cgroup: &Path) {
    if let Some(table) = table
        .get_or_insert_with(|| Value::Table(toml::Table::new()))
        .as_table_mut()
    {
        table
            .entry("name")
            .or_insert_with(|| Value::String(cgroup.to_string_lossy().into_owned()));
    }
}

/// Removes `ignore_on_failure` from the table and returns it.
fn take_ignore(table: &mut Option<Value>) -> bool {
    table
        .as_mut()
        .and_then(Value::as_table_mut)
        .and_then(|table| table.remove("ignore_on_failure"))
        .as_ref()
        .and_then(Value::as_bool)
        .unwrap_or(false)
}
