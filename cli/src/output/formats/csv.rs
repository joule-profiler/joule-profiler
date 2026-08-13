use std::fs::File;

use csv::{QuoteStyle, Writer};
use joule_profiler_core::fs::{
    create_file_with_user_permissions, default_results_filename, get_absolute_path,
};
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::types::{Phase, ProfilerResults};

use crate::output::displayer::{Displayer, DisplayerError};

type Result<T> = std::result::Result<T, DisplayerError>;

/// CSV output writer to a file.
pub struct CsvOutput {
    /// CSV writer wrapping the output file.
    writer: Writer<File>,

    /// Path to the output CSV file.
    filename: String,
}

impl CsvOutput {
    /// Create a CSV output writer to a file, optionally specifying the file path.
    pub fn try_new(output_file: Option<String>) -> Result<Self> {
        let filename = output_file.unwrap_or(default_results_filename("csv"));

        let absolute_path = get_absolute_path(&filename)?;
        let file = create_file_with_user_permissions(&absolute_path)?;

        let writer = csv::WriterBuilder::new()
            .delimiter(b';')
            .quote_style(QuoteStyle::Necessary)
            .from_writer(file);

        Ok(Self {
            writer,
            filename: absolute_path,
        })
    }

    /// Write CSV header row.
    fn write_header(&mut self) -> Result<()> {
        let mut record: Vec<&str> = Vec::new();

        record.extend_from_slice(&[
            "phase_id",
            "phase_name",
            "phase_duration_ms",
            "metric_name",
            "metric_value",
            "metric_unit",
            "metric_source",
            "start_token",
            "end_token",
            "start_token_line",
            "end_token_line",
            "timestamp",
            "command",
            "exit_code",
            "token_pattern",
        ]);

        self.writer
            .write_record(&record)
            .map_err(|err| DisplayerError::OutputFormatError(err.into()))?;
        Ok(())
    }

    /// Write a CSV row for a single phase.
    fn write_phase(
        &mut self,
        phase: &Phase,
        results: &ProfilerResults,
        cmd: &str,
        token_pattern: &str,
    ) -> Result<()> {
        let start_token_line = phase
            .start_token_line
            .map(|l| l.to_string())
            .unwrap_or_default();
        let end_token_line = phase
            .end_token_line
            .map(|l| l.to_string())
            .unwrap_or_default();

        let phase_id = phase.index.to_string();
        let phase_name = phase.get_name();
        let duration_ms = phase.duration_ms.to_string();
        let start_token = phase.start_token.to_string();
        let end_token = phase.end_token.to_string();
        let timestamp = phase.timestamp.to_string();
        let exit_code = results.exit_code.to_string();

        for metric in &phase.metrics {
            self.writer
                .write_record([
                    &phase_id,
                    &phase_name,
                    &duration_ms,
                    &metric.name,
                    &metric.value.to_string(),
                    &metric.unit.to_string(),
                    &metric.source,
                    &start_token,
                    &end_token,
                    &start_token_line,
                    &end_token_line,
                    &timestamp,
                    cmd,
                    &exit_code,
                    token_pattern,
                ])
                .map_err(|err| DisplayerError::OutputFormatError(err.into()))?;
        }

        Ok(())
    }

    /// Flush the writer and print a message indicating the CSV file has been written.
    fn finalize(&mut self) -> Result<()> {
        self.writer.flush()?;
        println!("CSV written to: {}", self.filename);
        Ok(())
    }
}

impl Displayer for CsvOutput {
    fn display_results(
        &mut self,
        cmd: &[String],
        token_pattern: &str,
        results: &ProfilerResults,
    ) -> Result<()> {
        if results.phases.is_empty() {
            return Ok(());
        }
        let command = cmd.join(" ");

        self.write_header()?;
        for phase in &results.phases {
            self.write_phase(phase, results, command.as_str(), token_pattern)?;
        }

        self.finalize()?;
        Ok(())
    }

    fn list_sensors(&mut self, sensors: &[Sensor]) -> Result<()> {
        self.writer
            .write_record(["sensor", "unit", "source"])
            .map_err(|err| DisplayerError::OutputFormatError(err.into()))?;

        for sensor in sensors {
            self.writer
                .write_record([
                    sensor.name.as_str(),
                    sensor.unit.to_string().as_str(),
                    sensor.source.as_str(),
                ])
                .map_err(|err| DisplayerError::OutputFormatError(err.into()))?;
        }

        self.finalize()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use joule_profiler_core::{
        types::{Metric, Phase, PhaseToken, ProfilerResults},
        unit::{MetricUnit, Unit, UnitPrefix},
    };
    use std::fs;
    use tempfile::NamedTempFile;

    fn unit() -> MetricUnit {
        MetricUnit {
            unit: Unit::Joule,
            prefix: UnitPrefix::Micro,
        }
    }

    fn metric(name: &str, value: u64) -> Metric {
        Metric::new(name, value, unit(), "rapl")
    }

    #[allow(clippy::too_many_arguments)]
    fn phase(
        index: usize,
        start: PhaseToken,
        end: PhaseToken,
        duration_ms: u128,
        timestamp: u128,
        start_line: Option<usize>,
        end_line: Option<usize>,
        metrics: Vec<Metric>,
    ) -> Phase {
        Phase {
            index,
            start_token: start,
            end_token: end,
            duration_ms,
            timestamp,
            start_token_line: start_line,
            end_token_line: end_line,
            metrics,
        }
    }

    fn simple_phase(metrics: Vec<Metric>) -> Phase {
        phase(
            0,
            PhaseToken::Start,
            PhaseToken::End,
            500,
            1000,
            None,
            None,
            metrics,
        )
    }

    fn results(exit_code: i32, phases: Vec<Phase>) -> ProfilerResults {
        ProfilerResults {
            timestamp: 0,
            duration_ms: 0,
            exit_code,
            phases,
        }
    }

    fn csv_to_tempfile() -> (CsvOutput, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_owned();
        (CsvOutput::try_new(Some(path)).unwrap(), tmp)
    }

    fn read(tmp: &NamedTempFile) -> String {
        fs::read_to_string(tmp.path()).unwrap()
    }

    #[test]
    fn phases_single_empty_phases_writes_nothing() {
        let (mut csv, tmp) = csv_to_tempfile();
        csv.display_results(&["echo".into()], ".*", &results(0, vec![]))
            .unwrap();
        assert!(read(&tmp).is_empty());
    }

    #[test]
    fn phases_single_writes_metric_values() {
        let (mut csv, tmp) = csv_to_tempfile();
        let iter = results(0, vec![simple_phase(vec![metric("PKG", 42)])]);
        csv.display_results(&["echo".into()], ".*", &iter).unwrap();
        let content = read(&tmp);
        assert!(content.contains("PKG"));
        assert!(content.contains("42"));
        assert!(content.contains("rapl"));
    }

    #[test]
    fn phases_single_writes_phase_metadata() {
        let (mut csv, tmp) = csv_to_tempfile();
        let iter = results(3, vec![simple_phase(vec![metric("PKG", 1)])]);
        csv.display_results(&["my_cmd".into(), "--flag".into()], "MY_PATTERN", &iter)
            .unwrap();
        let content = read(&tmp);
        assert!(content.contains("500")); // duration_ms
        assert!(content.contains("MY_PATTERN"));
        assert!(content.contains("my_cmd --flag"));
        assert!(content.contains('3')); // exit_code
    }

    #[test]
    fn phases_single_writes_token_info() {
        let (mut csv, tmp) = csv_to_tempfile();
        let iter = results(
            0,
            vec![phase(
                0,
                PhaseToken::Token("__A__".into()),
                PhaseToken::Token("__B__".into()),
                200,
                0,
                Some(3),
                Some(7),
                vec![metric("PKG", 1)],
            )],
        );
        csv.display_results(&["cmd".into()], ".*", &iter).unwrap();
        let content = read(&tmp);
        assert!(content.contains("__A__"));
        assert!(content.contains("__B__"));
        assert!(content.contains('3'));
        assert!(content.contains('7'));
    }

    #[test]
    fn phases_single_one_row_per_metric() {
        let (mut csv, tmp) = csv_to_tempfile();
        let iter = results(
            0,
            vec![simple_phase(vec![
                metric("PKG", 1),
                metric("DRAM", 2),
                metric("CORE", 3),
            ])],
        );
        csv.display_results(&["cmd".into()], ".*", &iter).unwrap();
        let content = read(&tmp);

        assert_eq!(content.lines().count(), 4);
    }

    #[test]
    fn phases_single_right_phase_name() {
        let (mut csv, tmp) = csv_to_tempfile();
        let iter = results(
            0,
            vec![phase(
                0,
                PhaseToken::Token("__START__".into()),
                PhaseToken::Token("__END__".into()),
                100,
                0,
                None,
                None,
                vec![metric("PKG", 1)],
            )],
        );
        csv.display_results(&["cmd".into()], ".*", &iter).unwrap();

        assert!(read(&tmp).contains("__START__ -> __END__"));
    }

    #[test]
    fn list_sensors_writes_header_and_one_row_per_sensor() {
        let (mut csv, tmp) = csv_to_tempfile();
        let sensors = vec![
            Sensor::new("PKG", unit(), "rapl"),
            Sensor::new("DRAM", unit(), "rapl"),
        ];
        csv.list_sensors(&sensors).unwrap();
        let content = read(&tmp);
        assert!(content.contains("sensor;unit;source"));
        assert!(content.contains("PKG"));
        assert!(content.contains("DRAM"));
        assert_eq!(content.lines().count(), 3);
    }

    #[test]
    fn phases_semicolon_in_field_does_not_break_columns() {
        let (mut csv, tmp) = csv_to_tempfile();
        let iter = results(0, vec![simple_phase(vec![metric("PKG", 1)])]);
        csv.display_results(&["echo".into(), "\"a;b\"".into()], "pat;tern", &iter)
            .unwrap();
        let content = read(&tmp);
        let header_cols = content.lines().next().unwrap().split(';').count();
        assert!(content.contains("a;b") || content.contains("\"a;b\""));
        assert!(content.contains("pat;tern") || content.contains("\"pat;tern\""));
        assert!(header_cols > 0);
    }

    #[test]
    fn quotes_are_escaped() {
        let (mut csv, tmp) = csv_to_tempfile();
        let iter = results(0, vec![simple_phase(vec![metric("PKG", 1)])]);

        csv.display_results(&["echo".into(), r#"hello "world""#.into()], ".*", &iter)
            .unwrap();

        let content = read(&tmp);

        assert!(content.contains(r#"""world""""#));
    }
}
