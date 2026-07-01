//! This file provides a simple example of the Joule Profiler library usage.

// use std::time::Duration;

use anyhow::Result;
// use joule_profiler_core::JouleProfiler;
// use joule_profiler_core::config::ProfileConfig;
// use joule_profiler_source_rapl::perf;

#[tokio::main]
async fn main() -> Result<()> {
    // // Instanciate Joule Profiler.
    // let mut profiler = JouleProfiler::new();

    // // Add sources.
    // let rapl = perf::Rapl::new(None)?;
    // profiler.add_source(rapl);
    // // ...

    // // Profile config.
    // let profile_config = ProfileConfig {
    //     stdout_file: None,
    //     cmd: vec!["sleep".into(), "1".into()],
    //     token_pattern: "__[A-Z0-9_]+__".into(),
    //     use_root: false,
    //     init_timeout: Duration::from_secs(1),
    // };

    // // Launch profiling.
    // let results = profiler.profile(&profile_config).await?;

    // println!("{results:#?}");

    Ok(())
}
