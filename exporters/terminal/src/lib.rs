use std::fmt::Write as _;
use std::io::{self, BufWriter, Write};

use joule_profiler_core::exporter::Exporter;
use joule_profiler_core::info::Info;
use joule_profiler_core::metric::MetricValue;
use joule_profiler_core::phase::{PhaseInfo, SourceMetrics, Summary};
use joule_profiler_core::schema::Schema;
use joule_profiler_core::unit::MetricUnit;

const WIDTH: usize = 56;
const LABEL: usize = 24;
const VALUE: usize = 12;

/// Padding is written as a slice of this: `Formatter::pad` writes one character at a time.
const SPACES: &str = "                                                        ";

/// The top and bottom of a heavy box, built once.
struct Rules {
    heavy_top: String,
    heavy_bottom: String,
}

impl Default for Rules {
    fn default() -> Self {
        let heavy = "═".repeat(WIDTH - 2);

        Self {
            heavy_top: format!("╔{heavy}╗\n"),
            heavy_bottom: format!("╚{heavy}╝\n"),
        }
    }
}

/// The parts of a metric line that do not change between phases.
struct Line {
    /// The padded label, such as `"  proc_rss                : "`.
    before: Box<str>,

    /// The unit and newline. A count has no unit.
    after: Box<str>,

    digits: usize,
}

impl Line {
    fn new(name: &str, unit: MetricUnit) -> Self {
        let mut before = String::with_capacity(LABEL + 4);
        let _ = write!(before, "  {name}");
        pad_to(&mut before, LABEL, name);
        before.push_str(": ");

        let after = if unit == MetricUnit::COUNT {
            "\n".to_owned()
        } else {
            format!(" {unit}\n")
        };

        Self {
            before: before.into_boxed_str(),
            after: after.into_boxed_str(),
            digits: unit.precision(),
        }
    }
}

/// The box and lines of one source, built once in `begin`.
struct Reported {
    heading: Box<str>,
    lines: Box<[Line]>,
}

pub struct TerminalExporter {
    out: BufWriter<io::Stdout>,
    reported: Vec<Reported>,

    /// Reused to format the values.
    scratch: String,

    rules: Rules,
}

impl Default for TerminalExporter {
    fn default() -> Self {
        Self {
            out: BufWriter::new(io::stdout()),
            reported: Vec::new(),
            scratch: String::new(),
            rules: Rules::default(),
        }
    }
}

impl Exporter for TerminalExporter {
    type Error = io::Error;

    fn begin(&mut self, schema: &Schema) -> io::Result<()> {
        self.reported = schema
            .sources
            .iter()
            .map(|source| Reported {
                heading: light_box(&source.name).into_boxed_str(),
                lines: source
                    .metrics
                    .iter()
                    .map(|metric| Line::new(&metric.name, metric.unit))
                    .collect(),
            })
            .collect();

        Ok(())
    }

    fn export(&mut self, phase: &PhaseInfo, sources: &[SourceMetrics<'_>]) -> io::Result<()> {
        // Borrows each field separately, so `reported` can be read while `out` is written.
        let Self {
            out,
            reported,
            scratch,
            rules,
        } = self;

        scratch.clear();
        let _ = write!(scratch, "{} -> {}", phase.start_token, phase.end_token);
        heavy_box(out, scratch, rules)?;

        label(out, "Duration")?;
        writeln!(out, "{:>VALUE$} ms", phase.duration_ms)?;

        if let Some(line) = phase.start_line {
            label(out, "Start token line")?;
            writeln!(out, "{line:>VALUE$}")?;
        }
        if let Some(line) = phase.end_line {
            label(out, "End token line")?;
            writeln!(out, "{line:>VALUE$}")?;
        }

        for (source, reported) in sources.iter().zip(reported.iter()) {
            out.write_all(b"\n")?;
            out.write_all(reported.heading.as_bytes())?;

            for (measured, line) in source.metrics.iter().zip(&reported.lines) {
                metric(out, scratch, line, measured.value)?;
            }
        }

        out.write_all(b"\n")
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    fn finish(&mut self, summary: &Summary) -> io::Result<()> {
        let out = &mut self.out;

        heavy_box(out, "Summary", &self.rules)?;

        label(out, "Phases")?;
        writeln!(out, "{:>VALUE$}", summary.phases)?;

        label(out, "Total duration")?;
        writeln!(out, "{:>VALUE$} ms", summary.duration_ms)?;

        if let Some(code) = summary.exit_code {
            label(out, "Exit code")?;
            writeln!(out, "{code:>VALUE$}")?;
        }

        out.flush()
    }
}

/// Prints the metrics of every source. Always to the terminal, whatever the exporter.
pub fn print_schema(title: &str, schema: &Schema) -> io::Result<()> {
    let mut out = BufWriter::new(io::stdout());
    let rules = Rules::default();

    heavy_box(&mut out, title, &rules)?;

    for source in &schema.sources {
        out.write_all(b"\n")?;
        out.write_all(light_box(&source.name).as_bytes())?;

        for metric in &source.metrics {
            label(&mut out, &metric.name)?;
            writeln!(out, "{}", metric.unit)?;
        }
    }

    out.write_all(b"\n")?;
    out.flush()
}

/// Always to the terminal, whatever the exporter.
pub fn print_info(title: &str, sections: &[(String, Info)]) -> io::Result<()> {
    let mut out = BufWriter::new(io::stdout());
    write_info(&mut out, title, sections)?;
    out.flush()
}

fn write_info(out: &mut impl Write, title: &str, sections: &[(String, Info)]) -> io::Result<()> {
    heavy_box(out, title, &Rules::default())?;

    for (name, info) in sections {
        out.write_all(b"\n")?;
        out.write_all(light_box(name).as_bytes())?;

        for (key, value) in info.entries() {
            label(out, key)?;
            writeln!(out, "{value}")?;
        }
    }

    out.write_all(b"\n")
}

/// Writes a metric line, its value right-aligned.
fn metric(
    out: &mut impl Write,
    scratch: &mut String,
    line: &Line,
    value: MetricValue,
) -> io::Result<()> {
    out.write_all(line.before.as_bytes())?;

    scratch.clear();
    let digits = line.digits;
    let _ = write!(scratch, "{value:.digits$}");

    out.write_all(room(VALUE, scratch).as_bytes())?;
    out.write_all(scratch.as_bytes())?;

    out.write_all(line.after.as_bytes())
}

/// Writes a padded label. A label longer than the column is not cut.
fn label(out: &mut impl Write, name: &str) -> io::Result<()> {
    out.write_all(b"  ")?;
    out.write_all(name.as_bytes())?;
    out.write_all(spaces(LABEL, name).as_bytes())?;
    out.write_all(b": ")
}

fn heavy_box(out: &mut impl Write, title: &str, rules: &Rules) -> io::Result<()> {
    out.write_all(rules.heavy_top.as_bytes())?;

    out.write_all("║ ".as_bytes())?;
    out.write_all(title.as_bytes())?;
    out.write_all(spaces(WIDTH - 4, title).as_bytes())?;
    out.write_all(" ║\n".as_bytes())?;

    out.write_all(rules.heavy_bottom.as_bytes())
}

fn light_box(title: &str) -> String {
    let light = "─".repeat(WIDTH - 2);
    let mut drawn = String::with_capacity(3 * (WIDTH * 3 + 1));

    let _ = write!(drawn, "┌{light}┐\n│ {title}");
    pad_to(&mut drawn, WIDTH - 4, title);
    let _ = write!(drawn, " │\n└{light}┘\n");

    drawn
}

/// The padding after `written` in a column of `width` characters.
fn spaces(width: usize, written: &str) -> &'static str {
    let filled = width.saturating_sub(written.chars().count());

    &SPACES[..filled.min(SPACES.len())]
}

/// The same for an ASCII number, whose length is its width.
fn room(width: usize, written: &str) -> &'static str {
    let filled = width.saturating_sub(written.len());

    &SPACES[..filled.min(SPACES.len())]
}

fn pad_to(out: &mut String, width: usize, written: &str) {
    out.push_str(spaces(width, written));
}

#[cfg(test)]
mod tests {
    use joule_profiler_core::info::InfoValue;

    use super::*;

    const J: MetricUnit = MetricUnit::JOULE;
    const B: MetricUnit = MetricUnit::BYTE;
    const COUNT: MetricUnit = MetricUnit::COUNT;

    fn written(name: &str, value: MetricValue, unit: MetricUnit) -> String {
        let mut out = Vec::new();
        let mut scratch = String::new();
        metric(&mut out, &mut scratch, &Line::new(name, unit), value).expect("a vec cannot fail");

        String::from_utf8(out).expect("a metric writes utf-8")
    }

    #[test]
    fn a_value_stops_growing_the_buffer_it_is_written_through() {
        let mut out = Vec::new();
        let mut scratch = String::new();
        let line = Line::new("core", J);

        for _ in 0..2 {
            metric(&mut out, &mut scratch, &line, MetricValue::F64(123.456)).unwrap();
        }
        let settled = scratch.capacity();

        for value in 0..1_000 {
            metric(&mut out, &mut scratch, &line, MetricValue::U64(value)).unwrap();
        }

        assert_eq!(
            scratch.capacity(),
            settled,
            "the buffer grew, so it allocated"
        );
    }

    #[test]
    fn a_value_is_right_aligned_in_its_column() {
        assert_eq!(
            written("core", MetricValue::F64(1.5), J),
            "  core                    :     1.500000 J\n"
        );
        assert_eq!(
            written("proc_rss", MetricValue::U64(4096), B),
            "  proc_rss                :         4096 B\n"
        );
    }

    #[test]
    fn values_of_different_lengths_end_at_the_same_column() {
        let short = written("a", MetricValue::U64(1), B);
        let long = written("b", MetricValue::U64(123_456_789), B);

        assert_eq!(short.trim_end().len(), long.trim_end().len());
    }

    #[test]
    fn a_unit_that_prints_as_nothing_leaves_no_space_behind() {
        let line = written("nr_periods", MetricValue::U64(17), COUNT);

        assert!(!line.trim_end_matches('\n').ends_with(' '), "{line:?}");
        assert!(line.contains("17"));
    }

    #[test]
    fn a_name_past_its_column_takes_the_room_it_needs() {
        let name = "a_metric_name_far_longer_than_the_column_allows_for";
        let line = written(name, MetricValue::U64(1), B);

        assert!(line.contains(name), "a name must never be cut: {line:?}");
        assert!(line.trim_end().ends_with("1 B"));
    }

    #[test]
    fn a_value_is_shown_to_as_many_places_as_its_unit_asks() {
        let micro = MetricUnit::MICROSECOND;

        assert!(written("core", MetricValue::F64(1.5), J).contains("1.500000"));
        assert!(written("usage", MetricValue::F64(1.5), micro).contains("2 \u{b5}s"));
    }

    #[test]
    fn an_integer_is_not_given_decimals_it_does_not_have() {
        assert_eq!(
            written("nr_periods", MetricValue::U64(17), COUNT).trim_end(),
            "  nr_periods              :           17"
        );
    }

    #[test]
    fn a_label_written_now_sits_where_a_laid_out_one_does() {
        let mut now = Vec::new();
        label(&mut now, "proc_rss").unwrap();

        assert_eq!(
            String::from_utf8(now).unwrap(),
            Line::new("proc_rss", B).before.as_ref()
        );
    }

    #[test]
    fn info_is_a_box_per_section_and_a_line_per_value() {
        let sections = [(
            "rapl".to_owned(),
            Info::new()
                .with("backend", "perf")
                .with("domains", InfoValue::list(["PACKAGE-0", "DRAM-0"])),
        )];
        let mut out = Vec::new();

        write_info(&mut out, "Information", &sections).unwrap();

        let written = String::from_utf8(out).unwrap();
        assert_eq!(
            written.lines().skip(4).collect::<Vec<_>>(),
            [
                light_box("rapl").lines().collect::<Vec<_>>().as_slice(),
                &[
                    "  backend                 : perf",
                    "  domains                 : PACKAGE-0, DRAM-0",
                    "",
                ],
            ]
            .concat()
        );
    }

    #[test]
    fn a_box_is_drawn_to_the_width_it_says() {
        let rules = Rules::default();
        let mut out = Vec::new();

        heavy_box(&mut out, "Summary", &rules).unwrap();
        out.write_all(light_box("rapl").as_bytes()).unwrap();

        let drawn = String::from_utf8(out).unwrap();
        assert_eq!(drawn.lines().count(), 6);

        for line in drawn.lines() {
            assert_eq!(line.chars().count(), WIDTH, "{line:?}");
        }
    }

    #[test]
    fn a_title_past_its_box_is_not_padded_into_nonsense() {
        let rules = Rules::default();
        let long = "a phase name far longer than the box was ever drawn to hold";
        let mut out = Vec::new();

        heavy_box(&mut out, long, &rules).unwrap();
        let drawn = String::from_utf8(out).unwrap();

        assert!(drawn.contains(long));
        assert!(drawn.lines().nth(1).unwrap().ends_with(" ║"));
    }
}
