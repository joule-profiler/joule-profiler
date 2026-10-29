use std::fs::File;
use std::io::Write;

use crate::output::displayer::error::IntoDisplayerError;
use crate::output::displayer::{Displayer, DisplayerError};
use joule_profiler_core::fs::{
    create_new_file_with_user_permissions, default_results_filename, get_absolute_path,
};
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::types::ProfilerResults;
use serde_json::json;

type Result<T> = std::result::Result<T, DisplayerError>;

/// JSON output writer to a file.
pub struct JsonOutput {
    /// File writer.
    writer: Option<File>,

    /// Output filename.
    filename: String,
}

impl JsonOutput {
    /// Create a JSON output writer, optionally with a specific file.
    pub fn new(output_file: Option<String>) -> Result<Self> {
        let filename = output_file.unwrap_or(default_results_filename("json"));

        let absolute_path = get_absolute_path(&filename)?;

        Ok(Self {
            writer: None,
            filename: absolute_path,
        })
    }

    fn writer(&mut self) -> Result<&mut File> {
        if self.writer.is_none() {
            self.writer = Some(create_new_file_with_user_permissions(&self.filename)?);
        }

        Ok(self.writer.as_mut().expect("writer was initialized"))
    }

    /// Write a JSON value to the output file.
    fn write_json(&mut self, value: &serde_json::Value) -> Result<()> {
        let json_str = serde_json::to_string_pretty(value)
            .map_err(IntoDisplayerError::into_displayer_error)?;
        writeln!(self.writer()?, "{json_str}")?;
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
    use joule_profiler_core::types::ProfilerResults;
    use tempfile::tempdir;

    #[test]
    fn new_does_not_create_output_file() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("results.json");
        let output = JsonOutput::new(Some(path.to_str().unwrap().to_owned())).unwrap();

        assert!(!path.exists());
        drop(output);
    }

    #[test]
    fn display_results_creates_output_file() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("results.json");
        let mut output = JsonOutput::new(Some(path.to_str().unwrap().to_owned())).unwrap();

        output
            .display_results(
                &["echo".into()],
                ".*",
                &ProfilerResults {
                    timestamp: 0,
                    duration_ms: 0,
                    exit_code: 0,
                    phases: vec![],
                },
            )
            .unwrap();

        assert!(path.exists());
    }

    #[test]
    fn display_results_does_not_overwrite_existing_file() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("results.json");
        std::fs::write(&path, "keep this").unwrap();
        let mut output = JsonOutput::new(Some(path.to_str().unwrap().to_owned())).unwrap();

        assert!(
            output
                .display_results(
                    &["echo".into()],
                    ".*",
                    &ProfilerResults {
                        timestamp: 0,
                        duration_ms: 0,
                        exit_code: 0,
                        phases: vec![],
                    }
                )
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), "keep this");
    }
}
