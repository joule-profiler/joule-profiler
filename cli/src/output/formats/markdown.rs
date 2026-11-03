use std::fs::File;
use std::io::Write;

use crate::output::displayer::{Displayer, DisplayerError};
use joule_profiler_core::fs::{
    create_file_with_user_permissions, default_results_filename, get_absolute_path,
};
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::types::ProfilerResults;

type Result<T> = std::result::Result<T, DisplayerError>;

/// Markdown output writer to a file or stdout.
pub struct MarkdownOutput {
    writer: File,
    filename: String,
}

impl MarkdownOutput {
    /// Create a Markdown output writer, optionally with a specific file.
    pub fn new(output_file: Option<String>) -> Result<Self> {
        let filename = output_file.unwrap_or_else(|| default_results_filename("md"));

        let absolute_path = get_absolute_path(&filename)?;
        let file = create_file_with_user_permissions(&absolute_path)?;

        Ok(Self {
            writer: file,
            filename: absolute_path,
        })
    }
}

impl Displayer for MarkdownOutput {
    fn display_results(
        &mut self,
        cmd: &[String],
        _token_pattern: &str,
        results: &ProfilerResults,
    ) -> Result<()> {
        writeln!(self.writer, "# ⚡ Joule Profiler Report\n")?;
        writeln!(self.writer, "## Command Execution")?;
        writeln!(self.writer, "- **Command:** `{}`", cmd.join(" "))?;
        writeln!(self.writer, "- **Total Duration:** `{} ms`", results.duration_ms)?;
        writeln!(self.writer, "- **Exit Code:** `{}`\n", results.exit_code)?;

        if !results.phases.is_empty() {
            writeln!(self.writer, "## Profiled Phases\n")?;
            writeln!(self.writer, "| Phase | Duration (ms) | Start Token | End Token |")?;
            writeln!(self.writer, "| :--- | :--- | :--- | :--- |")?;

            for phase in &results.phases {
                writeln!(
                    self.writer,
                    "| {} | {} | `{}` | `{}` |",
                    phase.get_name(),
                    phase.duration_ms,
                    phase.start_token,
                    phase.end_token
                )?;
            }
            writeln!(self.writer)?;
        }

        println!("Markdown written to: {}", self.filename);
        Ok(())
    }

    fn list_sensors(&mut self, sensors: &[Sensor]) -> Result<()> {
        writeln!(self.writer, "# 🔌 Available Energy Sensors\n")?;
        writeln!(self.writer, "| Sensor Name | Source | Unit |")?;
        writeln!(self.writer, "| :--- | :--- | :--- |")?;

        for sensor in sensors {
            writeln!(
                self.writer,
                "| {} | {} | {} |",
                sensor.name, sensor.source, sensor.unit
            )?;
        }
        writeln!(self.writer)?;

        println!("Markdown written to: {}", self.filename);
        Ok(())
    }
}
