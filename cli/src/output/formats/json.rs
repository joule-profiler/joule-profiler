use std::fs::File;
use std::io::Write;

use crate::output::displayer::error::IntoDisplayerError;
use crate::output::displayer::{Displayer, DisplayerError};
use joule_profiler_core::fs::{
    create_file_with_user_permissions, default_results_filename, get_absolute_path,
};
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::types::ProfilerResults;
use serde_json::json;

type Result<T> = std::result::Result<T, DisplayerError>;

/// JSON output writer to a file.
pub struct JsonOutput {
    /// File writer.
    writer: File,

    /// Output filename.
    filename: String,
}

impl JsonOutput {
    /// Create a JSON output writer, optionally with a specific file.
    pub fn new(output_file: Option<String>) -> Result<Self> {
        let filename = output_file.unwrap_or(default_results_filename("json"));

        let absolute_path = get_absolute_path(&filename)?;
        let file = create_file_with_user_permissions(&absolute_path)?;

        Ok(Self {
            writer: file,
            filename: absolute_path,
        })
    }

    /// Write a JSON value to the output file.
    fn write_json(&mut self, value: &serde_json::Value) -> Result<()> {
        let json_str = serde_json::to_string_pretty(value)
            .map_err(IntoDisplayerError::into_displayer_error)?;
        writeln!(self.writer, "{json_str}")?;
        println!("JSON written to: {}", self.filename);
        Ok(())
    }
}

impl Displayer for JsonOutput {
    fn display_results(
        &mut self,
        cmd: &[String],
        token_pattern: &str,
        results: &ProfilerResults,
    ) -> Result<()> {
        self.write_json(&json!({
            "command": cmd.join(" "),
            "token_pattern": token_pattern,
            "exit_code": results.exit_code,
            "overhead": results.overhead,
            "phases": results.phases,
        }))
    }

    fn list_sensors(&mut self, sensors: &[Sensor]) -> Result<()> {
        self.write_json(
            &serde_json::to_value(sensors).map_err(IntoDisplayerError::into_displayer_error)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use joule_profiler_core::{
        types::{Metric, Phase, PhaseToken, ProfilerOverhead},
        unit::{MetricUnit, Unit, UnitPrefix},
    };
    use std::fs;
    use tempfile::NamedTempFile;

    #[test]
    fn json_output_includes_overhead_estimation() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().to_str().unwrap().to_owned();
        let mut json_out = JsonOutput::new(Some(path)).unwrap();

        let unit = MetricUnit {
            prefix: UnitPrefix::Micro,
            unit: Unit::Joule,
        };
        let results = ProfilerResults {
            timestamp: 1_000,
            duration_ms: 50,
            exit_code: 0,
            phases: vec![Phase {
                index: 0,
                start_token: PhaseToken::Start,
                end_token: PhaseToken::End,
                timestamp: 1_000,
                duration_ms: 50,
                start_token_line: None,
                end_token_line: None,
                metrics: vec![Metric::new(
                    "PACKAGE-0",
                    10_000_u64,
                    unit,
                    "RAPL (perf_event)",
                )],
            }],
            overhead: ProfilerOverhead {
                cpu_time_us: 400,
                system_cpu_time_us: 40_000,
                cpu_share_percent: 1.0,
                estimated_energy_uj: Some(100),
                domains: vec![Metric::new("PACKAGE-0", 100_u64, unit, "RAPL (perf_event)")],
            },
        };

        json_out
            .display_results(&["echo".into(), "hi".into()], "__PHASE__", &results)
            .unwrap();

        let content = fs::read_to_string(tmp.path()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["overhead"]["cpu_time_us"], 400);
        assert_eq!(parsed["overhead"]["estimated_energy_uj"], 100);
        assert_eq!(parsed["overhead"]["domains"][0]["name"], "PACKAGE-0");
        assert_eq!(parsed["overhead"]["domains"][0]["value"], 100);
    }
}
