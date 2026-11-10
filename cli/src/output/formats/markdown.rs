use std::fs::File;
use std::io::{self, Write};

use crate::output::displayer::{Displayer, DisplayerError};
use joule_profiler_core::fs::{
    create_file_with_user_permissions, default_results_filename, get_absolute_path,
};
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::types::ProfilerResults;

type Result<T> = std::result::Result<T, DisplayerError>;

/// Markdown output writer, generic over the destination.
///
/// Writes to a file by default, or to stdout when the output file is `-`.
pub struct MarkdownOutput<W: Write> {
    writer: W,
    /// Absolute path of the output file, `None` when writing to stdout
    /// or to an arbitrary writer.
    path: Option<String>,
}

impl MarkdownOutput<Box<dyn Write>> {
    /// Create a Markdown output writer, optionally with a specific file.
    /// Passing `-` writes the report to stdout.
    pub fn new(output_file: Option<String>) -> Result<Self> {
        if output_file.as_deref() == Some("-") {
            return Ok(Self {
                writer: Box::new(io::stdout()),
                path: None,
            });
        }

        let filename = output_file.unwrap_or_else(|| default_results_filename("md"));
        let absolute_path = get_absolute_path(&filename)?;
        let file: File = create_file_with_user_permissions(&absolute_path)?;

        Ok(Self {
            writer: Box::new(file),
            path: Some(absolute_path),
        })
    }
}

impl<W: Write> MarkdownOutput<W> {
    /// Create a Markdown output writer on any `Write` (used by the tests).
    #[cfg(test)]
    pub fn from_writer(writer: W) -> Self {
        Self { writer, path: None }
    }

    /// Flush the writer and, for files only, tell the user where the report went.
    /// Nothing is printed on stdout when the report itself goes there.
    fn finalize(&mut self) -> Result<()> {
        self.writer.flush()?;
        if let Some(path) = &self.path {
            println!("Markdown written to: {path}");
        }
        Ok(())
    }
}

/// Escape a text for use inside a Markdown table cell.
fn escape_cell(s: &str) -> String {
    s.replace('|', "\\|").replace(['\n', '\r'], " ")
}

/// Wrap a text in an inline code span, using a backtick fence longer than
/// any backtick run found in the text.
fn inline_code(s: &str) -> String {
    let longest = s.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    let pad = if s.starts_with('`') || s.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{s}{pad}{fence}")
}

impl<W: Write> Displayer for MarkdownOutput<W> {
    fn display_results(
        &mut self,
        cmd: &[String],
        _token_pattern: &str,
        results: &ProfilerResults,
    ) -> Result<()> {
        writeln!(self.writer, "# ⚡ Joule Profiler Report\n")?;
        writeln!(self.writer, "## Command Execution")?;
        writeln!(
            self.writer,
            "- **Command:** {}",
            inline_code(&cmd.join(" "))
        )?;
        writeln!(
            self.writer,
            "- **Total Duration:** `{} ms`",
            results.duration_ms
        )?;
        writeln!(self.writer, "- **Exit Code:** `{}`\n", results.exit_code)?;

        if !results.phases.is_empty() {
            writeln!(self.writer, "## Profiled Phases\n")?;
            writeln!(
                self.writer,
                "| Phase | Duration (ms) | Start Token | End Token |"
            )?;
            writeln!(self.writer, "| :--- | ---: | :--- | :--- |")?;

            for phase in &results.phases {
                writeln!(
                    self.writer,
                    "| {} | {} | {} | {} |",
                    escape_cell(&phase.get_name()),
                    phase.duration_ms,
                    inline_code(&phase.start_token.to_string()),
                    inline_code(&phase.end_token.to_string()),
                )?;
            }
            writeln!(self.writer)?;

            writeln!(self.writer, "## Metrics\n")?;
            writeln!(self.writer, "| Phase | Metric | Value | Unit | Source |")?;
            writeln!(self.writer, "| :--- | :--- | ---: | :--- | :--- |")?;

            for phase in &results.phases {
                let phase_name = escape_cell(&phase.get_name());
                for metric in &phase.metrics {
                    writeln!(
                        self.writer,
                        "| {} | {} | {} | {} | {} |",
                        phase_name,
                        escape_cell(&metric.name),
                        metric.value,
                        escape_cell(&metric.unit.to_string()),
                        escape_cell(&metric.source),
                    )?;
                }
            }
            writeln!(self.writer)?;
        }

        self.finalize()
    }

    fn list_sensors(&mut self, sensors: &[Sensor]) -> Result<()> {
        writeln!(self.writer, "# 🔌 Available Energy Sensors\n")?;
        writeln!(self.writer, "| Sensor Name | Source | Unit |")?;
        writeln!(self.writer, "| :--- | :--- | :--- |")?;

        for sensor in sensors {
            writeln!(
                self.writer,
                "| {} | {} | {} |",
                escape_cell(&sensor.name),
                escape_cell(&sensor.source),
                escape_cell(&sensor.unit.to_string()),
            )?;
        }
        writeln!(self.writer)?;

        self.finalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_cell_escapes_pipes_and_newlines() {
        assert_eq!(escape_cell("a|b"), "a\\|b");
        assert_eq!(escape_cell("a\nb"), "a b");
        assert_eq!(escape_cell("plain"), "plain");
    }

    #[test]
    fn inline_code_without_backticks() {
        assert_eq!(inline_code("sleep 2"), "`sleep 2`");
    }

    #[test]
    fn inline_code_uses_longer_fence_for_backticks() {
        assert_eq!(inline_code("a`b"), "``a`b``");
    }

    #[test]
    fn inline_code_pads_when_text_starts_or_ends_with_backtick() {
        assert_eq!(inline_code("`x`"), "`` `x` ``");
    }

    use joule_profiler_core::types::{Metric, Phase, PhaseToken, ProfilerResults};
    use joule_profiler_core::unit::{MetricUnit, Unit, UnitPrefix};

    fn metric(name: &str, value: u64) -> Metric {
        let unit = MetricUnit {
            unit: Unit::Joule,
            prefix: UnitPrefix::Micro,
        };
        Metric::new(name, value, unit, "rapl")
    }

    fn phase(index: usize, start: PhaseToken, end: PhaseToken, metrics: Vec<Metric>) -> Phase {
        Phase {
            index,
            start_token: start,
            end_token: end,
            duration_ms: 500,
            timestamp: 1000,
            start_token_line: None,
            end_token_line: None,
            metrics,
        }
    }

    fn results(phases: Vec<Phase>) -> ProfilerResults {
        ProfilerResults {
            timestamp: 0,
            duration_ms: 1234,
            exit_code: 0,
            phases,
        }
    }

    fn render(cmd: &[&str], res: &ProfilerResults) -> String {
        let cmd: Vec<String> = cmd.iter().map(|s| s.to_string()).collect();
        let mut out = MarkdownOutput::from_writer(Vec::new());
        out.display_results(&cmd, ".*", res).unwrap();
        String::from_utf8(out.writer).unwrap()
    }

    #[test]
    fn display_results_lists_metrics_of_every_phase() {
        let res = results(vec![
            phase(
                0,
                PhaseToken::Start,
                PhaseToken::Token("__A__".into()),
                vec![metric("PKG", 42), metric("DRAM", 7)],
            ),
            phase(
                1,
                PhaseToken::Token("__A__".into()),
                PhaseToken::End,
                vec![metric("PKG", 99)],
            ),
        ]);
        let md = render(&["echo"], &res);

        assert!(md.contains("## Metrics"));
        assert!(md.contains("| Phase | Metric | Value | Unit | Source |"));
        assert!(md.contains("| PKG | 42 |"));
        assert!(md.contains("| DRAM | 7 |"));
        assert!(md.contains("| PKG | 99 |"));
        assert!(md.contains("rapl"));
        assert!(md.contains("`1234 ms`"));
    }

    #[test]
    fn display_results_escapes_pipe_in_phase_name() {
        let res = results(vec![phase(
            0,
            PhaseToken::Token("a|b".into()),
            PhaseToken::End,
            vec![metric("PKG", 1)],
        )]);
        let md = render(&["echo"], &res);

        assert!(md.contains("a\\|b"));
    }

    #[test]
    fn display_results_wraps_command_with_backticks() {
        let res = results(vec![]);
        let md = render(&["echo", "`date`"], &res);

        assert!(md.contains("- **Command:** `` echo `date` ``"));
    }
}
