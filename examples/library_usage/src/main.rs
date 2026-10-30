//! This file provides a simple example of the Joule Profiler library usage.

use anyhow::Result;
use joule_profiler_core::JouleProfiler;
use joule_profiler_exporter_terminal::TerminalExporter;
use joule_profiler_injector_stdout::StdoutInjector;
use joule_profiler_source_rapl::Rapl;

fn main() -> Result<()> {
    // Instantiate Joule Profiler.
    let mut profiler = JouleProfiler::new();

    // Add sources.
    profiler.add_source(Rapl::new().build()?);
    // ...

    // Run the command, and start a new phase each time it prints a token.
    let command = "echo __WORK_START__; sleep 1; echo __WORK_END__; sleep 0.5";
    let injector = StdoutInjector::new(
        vec!["sh".into(), "-c".into(), command.into()],
        "__[A-Z0-9_]+__",
    )?;
    profiler.set_injector(injector);

    // Print each phase in the terminal.
    profiler.set_exporter(TerminalExporter::default());

    // Launch profiling.
    let summary = profiler.profile()?;

    println!("{summary:#?}");

    Ok(())
}
