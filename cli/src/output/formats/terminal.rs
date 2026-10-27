use std::collections::HashMap;

use joule_profiler_core::{
    sensor::Sensor,
    types::{Metric, Phase, ProfilerOverhead, ProfilerResults},
};

use crate::output::displayer::{Displayer, DisplayerError};

/// Constants for formatting
const BORDER_DOUBLE: &str = "═";
const BORDER_SINGLE: &str = "─";
const BOX_WIDTH: usize = 50;

type Result<T> = std::result::Result<T, DisplayerError>;

#[derive(Debug, Clone, Default)]
pub struct TerminalOutput;

impl TerminalOutput {
    /// Display command header
    fn display_command(command: &[String]) {
        if !command.is_empty() {
            Self::print_header("Command");
            println!("  {}", command.join(" "));
        }
    }

    /// Print a formatted header
    fn print_header(title: &str) {
        println!("╔{}╗", BORDER_DOUBLE.repeat(BOX_WIDTH - 2));
        println!("║  {:<width$} ║", title, width = BOX_WIDTH - 5);
        println!("╚{}╝", BORDER_DOUBLE.repeat(BOX_WIDTH - 2));
    }

    /// Print a formatted sub-header
    fn print_subheader(title: &str, prefix: &str) {
        println!(
            "{}┌{}┐",
            prefix,
            BORDER_SINGLE.repeat(BOX_WIDTH - prefix.len() - 2)
        );
        println!(
            "{}│ {:<width$}│",
            prefix,
            title,
            width = BOX_WIDTH - prefix.len() - 3
        );
        println!(
            "{}└{}┘",
            prefix,
            BORDER_SINGLE.repeat(BOX_WIDTH - prefix.len() - 2)
        );
    }

    /// Display a single measurement result
    fn display_phase(phase: &Phase, prefix: &str) {
        println!();

        let mut keys: Vec<_> = phase
            .metrics
            .iter()
            .map(|metric| &metric.name)
            .cloned()
            .collect();
        keys.sort_unstable();

        let mut metrics_per_source: HashMap<&String, Vec<&Metric>> = HashMap::new();
        for metric in &phase.metrics {
            metrics_per_source
                .entry(&metric.source)
                .or_default()
                .push(metric);
        }

        let mut metrics_per_source: Vec<(&String, Vec<&Metric>)> =
            metrics_per_source.into_iter().collect();
        metrics_per_source.sort_by_key(|(source, _)| *source);

        for (source, metrics) in metrics_per_source {
            Self::print_subheader(source, prefix);

            for metric in metrics {
                println!(
                    "{}  {:<20}: {:10.6} {}",
                    prefix, metric.name, metric.value, metric.unit
                );
            }
        }
    }

    /// Display phase header with token information
    fn display_phase_header(phase: &Phase, prefix: &str) {
        let phase_name = phase.get_name();
        println!();
        if prefix.is_empty() {
            println!("╔{}╗", BORDER_DOUBLE.repeat(BOX_WIDTH - 2));
            println!("║  Phase: {:<width$} ║", phase_name, width = BOX_WIDTH - 12);
            println!("╚{}╝", BORDER_DOUBLE.repeat(BOX_WIDTH - 2));
        } else {
            Self::print_subheader(&format!("Phase: {phase_name}"), prefix);
        }

        // Display token information
        let start_info = if let Some(line) = phase.start_token_line {
            format!("{} (line {})", phase.start_token, line)
        } else {
            phase.start_token.to_string()
        };

        let end_info = if let Some(line) = phase.end_token_line {
            format!("{} (line {})", phase.end_token, line)
        } else {
            phase.end_token.to_string()
        };

        println!(
            "{}  {:<20}: {:>10} ms",
            prefix, "Duration", phase.duration_ms
        );

        println!("{}  {:<20}: {:>10}", prefix, "Start token", start_info);

        println!("{}  {:<20}: {:>10}", prefix, "End token", end_info);
    }

    /// Display estimated profiler overhead summary.
    fn display_overhead(overhead: &ProfilerOverhead, prefix: &str) {
        println!();
        Self::print_header("Profiler Overhead (estimated)");
        println!(
            "{}  {:<20}: {:>10} µs",
            prefix, "Profiler CPU time", overhead.cpu_time_us
        );
        println!(
            "{}  {:<20}: {:>10.2} %",
            prefix, "Profiler CPU share", overhead.cpu_share_percent
        );
        if let Some(energy_uj) = overhead.estimated_energy_uj {
            println!(
                "{}  {:<20}: {:>10} µJ",
                prefix, "Estimated energy", energy_uj
            );
        }

        if !overhead.domains.is_empty() {
            println!();
            Self::print_subheader("Overhead by RAPL domain", prefix);
            for metric in &overhead.domains {
                println!(
                    "{}  {:<20}: {:10.6} {}",
                    prefix, metric.name, metric.value, metric.unit
                );
            }
        }
    }
}

impl Displayer for TerminalOutput {
    fn display_results(
        &mut self,
        cmd: &[String],
        _token_pattern: &str,
        results: &ProfilerResults,
    ) -> Result<()> {
        Self::display_command(cmd);
        println!(" {}", BORDER_SINGLE.repeat(BOX_WIDTH - 2));

        let prefix = "";

        println!(
            "{}  {:<20}: {:>10} ms",
            prefix, "Duration", results.duration_ms
        );
        println!("{}  {:<20}: {:>10}", prefix, "Exit code", results.exit_code);

        for phase in &results.phases {
            Self::display_phase_header(phase, prefix);
            Self::display_phase(phase, prefix);
        }

        Self::display_overhead(&results.overhead, prefix);

        Ok(())
    }

    fn list_sensors(&mut self, sensors: &[Sensor]) -> Result<()> {
        if sensors.is_empty() {
            println!("No sensors available.");
            return Ok(());
        }

        Self::print_header("Available Sensors");

        let mut sensors_by_source: HashMap<&String, Vec<&Sensor>> = HashMap::new();
        for sensor in sensors {
            sensors_by_source
                .entry(&sensor.source)
                .or_default()
                .push(sensor);
        }

        let mut sources: Vec<_> = sensors_by_source.keys().collect();
        sources.sort_unstable();

        for source in sources {
            let source_sensors = &sensors_by_source[source];

            println!();
            Self::print_subheader(source, "");

            println!("  {:<20} | {:<5}", "Name", "Unit");
            println!(" {}", BORDER_SINGLE.repeat(BOX_WIDTH - 2));

            for sensor in source_sensors {
                println!("  {:<20} | {:<5}", sensor.name, sensor.unit);
            }
        }

        Ok(())
    }
}
