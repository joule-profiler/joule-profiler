use std::collections::HashMap;
use std::io::{self, Write};

use joule_profiler_core::exporter::Exporter;
use joule_profiler_core::types::{AvailableSensor, Metric, Metrics, PhaseInfo};

const BORDER_DOUBLE: &str = "═";
const BORDER_SINGLE: &str = "─";
const BOX_WIDTH: usize = 50;

pub struct TerminalExporter;

impl TerminalExporter {
    fn print_header(out: &mut impl Write, title: &str) -> io::Result<()> {
        writeln!(out, "╔{}╗", BORDER_DOUBLE.repeat(BOX_WIDTH - 2))?;
        writeln!(out, "║  {:<width$} ║", title, width = BOX_WIDTH - 5)?;
        writeln!(out, "╚{}╝", BORDER_DOUBLE.repeat(BOX_WIDTH - 2))
    }

    fn print_subheader(out: &mut impl Write, title: &str) -> io::Result<()> {
        writeln!(out, "┌{}┐", BORDER_SINGLE.repeat(BOX_WIDTH - 2))?;
        writeln!(out, "│ {:<width$}│", title, width = BOX_WIDTH - 3)?;
        writeln!(out, "└{}┘", BORDER_SINGLE.repeat(BOX_WIDTH - 2))
    }

    fn display_phase_header(out: &mut impl Write, phase_info: &PhaseInfo) -> io::Result<()> {
        Self::print_header(
            out,
            &format!("{} -> {}", phase_info.start_token, phase_info.end_token),
        )?;
        writeln!(
            out,
            "  {:<20}: {:>10} μs",
            "Duration",
            phase_info.end_timestamp - phase_info.start_timestamp
        )?;
        writeln!(
            out,
            "  {:<20}: {:>10}",
            "Start token", phase_info.start_token
        )?;
        writeln!(out, "  {:<20}: {:>10}", "End token", phase_info.end_token)
    }

    fn display_metrics(
        out: &mut impl Write,
        metrics: &Metrics,
        phase_info: &PhaseInfo,
    ) -> io::Result<()> {
        writeln!(out)?;

        let mut metrics_per_source: HashMap<&str, Vec<&Metric>> = HashMap::new();
        for metric in metrics {
            metrics_per_source
                .entry(metric.source)
                .or_default()
                .push(metric);
        }

        let mut metrics_per_source: Vec<_> = metrics_per_source.into_iter().collect();
        metrics_per_source.sort_unstable_by_key(|(source, _)| *source);

        for (source, metrics) in metrics_per_source {
            Self::print_subheader(out, source)?;
            for metric in metrics {
                let value = format!("{:.6}", metric.value);
                writeln!(
                    out,
                    "  {:<20}: {:>20} (delay={}μs)",
                    metric.name,
                    value,
                    metric.timestamp - phase_info.start_timestamp,
                )?;
            }
        }
        Ok(())
    }

    pub fn print_sensors(out: &mut impl Write, sensors: &[AvailableSensor]) -> io::Result<()> {
        if sensors.is_empty() {
            return writeln!(out, "No sensors available.");
        }

        Self::print_header(out, "Available Sensors")?;

        let mut sensors_by_source: HashMap<&str, Vec<&AvailableSensor>> = HashMap::new();
        for sensor in sensors {
            sensors_by_source
                .entry(sensor.source)
                .or_default()
                .push(sensor);
        }

        let mut sources: Vec<&str> = sensors_by_source.keys().copied().collect();
        sources.sort_unstable();

        for source in sources {
            writeln!(out)?;
            Self::print_subheader(out, source)?;

            writeln!(out, "  {:<20} | {:<5}", "Name", "Unit")?;
            writeln!(out, " {}", BORDER_SINGLE.repeat(BOX_WIDTH - 2))?;

            for sensor in &sensors_by_source[source] {
                writeln!(out, "  {:<20} | {:<5}", sensor.name, sensor.unit)?;
            }
        }

        Ok(())
    }
}

impl Exporter for TerminalExporter {
    type Error = io::Error;

    async fn export(&mut self, phase_info: PhaseInfo, metrics: Metrics) -> io::Result<()> {
        let mut out = io::stdout().lock();
        Self::display_phase_header(&mut out, &phase_info)?;
        Self::display_metrics(&mut out, &metrics, &phase_info)
    }
}
