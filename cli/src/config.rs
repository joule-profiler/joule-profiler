use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use joule_profiler_core::util::cgroup::CgroupConfig;
use joule_profiler_injector_stdout::DEFAULT_PATTERN;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use toml::{Table, Value};

use crate::cli::{Cli, Command, OutputFormat, SourceName};

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct File {
    profiler: Profiler,

    exporter: Exporter,

    injector: Injector,

    /// The cgroup made for the run.
    cgroup: Option<CgroupConfig>,

    sources: Table,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Profiler {
    defer: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Exporter {
    format: Option<OutputFormat>,
    file: Option<PathBuf>,

    /// JSON only: one object per line.
    lines: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Injector {
    token_pattern: Option<String>,

    /// Where the output of the program goes, instead of the terminal.
    stdout_file: Option<PathBuf>,

    /// Under `sudo`, whether the program keeps root.
    use_root: bool,
}

/// The configuration file and the flags, merged.
pub struct ConfigTable {
    /// In the order they were named.
    pub sources: Vec<SourceName>,

    pub tables: Table,

    pub output_format: OutputFormat,
    pub output_file: Option<PathBuf>,

    /// JSON only: one object per line.
    pub lines: bool,

    pub token_pattern: String,
    pub stdout_file: Option<PathBuf>,
    pub use_root: bool,

    pub defer: bool,

    pub cgroup: Option<CgroupConfig>,
}

impl ConfigTable {
    pub fn resolve(cli: &Cli) -> Result<Self> {
        let file = read(cli)?;

        let (pattern, defer) = match &cli.command {
            Command::Profile(args) => (args.token_pattern.clone(), args.defer),
            Command::ListSensors | Command::Info => (None, false),
        };

        let sources = sources(cli, &file)?;

        // The cgroup source without a `name` measures a cgroup made for the run.
        let cgroup = file.cgroup.or_else(|| {
            measures_the_run_cgroup(&sources, &file.sources).then(CgroupConfig::default)
        });

        Ok(Self {
            sources,

            output_format: cli
                .output_format
                .or(file.exporter.format)
                .unwrap_or(OutputFormat::Terminal),
            output_file: cli.output_file.clone().or(file.exporter.file),
            lines: file.exporter.lines,

            // A flag overrides the file.
            token_pattern: pattern
                .or(file.injector.token_pattern)
                .unwrap_or_else(|| DEFAULT_PATTERN.to_owned()),

            stdout_file: file.injector.stdout_file,
            use_root: file.injector.use_root,

            defer: defer || file.profiler.defer,
            cgroup,

            tables: file.sources,
        })
    }

    pub fn table(&self, name: SourceName) -> Option<&Value> {
        self.tables.get(name.key())
    }
}

/// The configuration file, with the `--define` overrides applied.
fn read(cli: &Cli) -> Result<File> {
    let mut document = match &cli.config_file {
        Some(path) => {
            let content =
                fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

            toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?
        }
        None => Table::new(),
    };

    for assignment in &cli.set {
        set(&mut document, assignment).with_context(|| format!("--define {assignment}"))?;
    }

    document.try_into().context("reading the configuration")
}

/// Sets `section.key=value` in the document, creating the missing sections.
fn set(document: &mut Table, assignment: &str) -> Result<()> {
    let Some((path, raw)) = assignment.split_once('=') else {
        bail!("expected `key=value`");
    };

    let mut keys = path.split('.').peekable();
    let mut table = document;

    while let Some(key) = keys.next() {
        if keys.peek().is_none() {
            table.insert(key.to_owned(), value(raw));
            return Ok(());
        }

        table = table
            .entry(key)
            .or_insert_with(|| Value::Table(Table::new()))
            .as_table_mut()
            .with_context(|| format!("`{key}` is a value, not a section"))?;
    }

    bail!("expected a key to the left of `=`")
}

/// The value as TOML, or as a string if it is not valid TOML.
fn value(raw: &str) -> Value {
    toml::from_str::<Table>(&format!("value = {raw}"))
        .ok()
        .and_then(|mut table| table.remove("value"))
        .unwrap_or_else(|| Value::String(raw.to_owned()))
}

/// The sources named by `--sources`, then those with a table in the file.
fn sources(cli: &Cli, file: &File) -> Result<Vec<SourceName>> {
    let mut sources = cli.sources.clone();

    for key in file.sources.keys() {
        let Some(name) = SourceName::from_key(key) else {
            bail!("the configuration names a source that does not exist: `{key}`");
        };

        if !sources.contains(&name) {
            sources.push(name);
        }
    }

    Ok(sources)
}

fn measures_the_run_cgroup(sources: &[SourceName], tables: &Table) -> bool {
    #[cfg(feature = "cgroup")]
    {
        let names_one = tables
            .get(SourceName::Cgroup.key())
            .and_then(|table| table.get("name"))
            .is_some();

        sources.contains(&SourceName::Cgroup) && !names_one
    }

    #[cfg(not(feature = "cgroup"))]
    {
        let _ = (sources, tables);
        false
    }
}

/// Reads a source configuration from its table. No table means all defaults.
pub fn configure<C: DeserializeOwned>(name: SourceName, table: Option<&Value>) -> Result<C> {
    table
        .cloned()
        .unwrap_or_else(|| Value::Table(Table::new()))
        .try_into()
        .with_context(|| format!("reading the `{}` table", name.key()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(toml: &str) -> Table {
        toml::from_str(toml).expect("parses")
    }

    #[test]
    fn a_definition_digs_as_deep_as_the_dots_go() {
        let mut table = document("");

        set(&mut table, "sources.perf_event.scope=cgroup").expect("sets");

        assert_eq!(
            table["sources"]["perf_event"]["scope"],
            Value::String("cgroup".to_owned()),
            "the sections were made on the way down"
        );
    }

    #[test]
    fn a_definition_reads_its_value_as_what_it_looks_like() {
        let mut table = document("");

        set(&mut table, "profiler.defer=true").expect("sets");
        set(&mut table, "sources.perf_event.cpus=[0, 1]").expect("sets");
        set(&mut table, "injector.token_pattern=__[A-Z]+__").expect("sets");

        assert_eq!(table["profiler"]["defer"], Value::Boolean(true));
        assert_eq!(table["sources"]["perf_event"]["cpus"][1], Value::Integer(1));
        assert_eq!(
            table["injector"]["token_pattern"],
            Value::String("__[A-Z]+__".to_owned()),
            "what is not a toml value of its own is left a string"
        );
    }

    #[test]
    fn a_definition_outranks_the_file() {
        let mut table = document("[exporter]\nlines = false");

        set(&mut table, "exporter.lines=true").expect("sets");

        assert_eq!(table["exporter"]["lines"], Value::Boolean(true));
    }

    #[test]
    fn a_definition_cannot_dig_through_a_value() {
        let mut table = document("[profiler]\ndefer = true");

        assert!(set(&mut table, "profiler.defer.deeper=1").is_err());
    }

    #[test]
    fn a_section_that_does_not_exist_is_refused() {
        let outcome: Result<File, _> = document("[exportr]\nformat = 1").try_into();

        assert!(outcome.is_err());
    }

    #[test]
    fn a_source_table_names_a_source_that_exists() {
        use clap::ValueEnum;

        for source in SourceName::value_variants() {
            let file: File = document(&format!("[sources.{}]", source.key()))
                .try_into()
                .expect("reads");

            assert_eq!(
                SourceName::from_key(file.sources.keys().next().unwrap()),
                Some(*source)
            );
        }

        let file: File = document("[sources.nothing]").try_into().expect("reads");
        assert!(SourceName::from_key(file.sources.keys().next().unwrap()).is_none());
    }

    #[test]
    fn a_cgroup_table_is_the_cgroup_made_for_the_run() {
        let file: File = document("[cgroup]\nname = \"a-run\"\nattach = false")
            .try_into()
            .expect("reads");

        assert!(file.cgroup.is_some());
        assert!(
            document("[cgroup]\nnam = \"a-run\"")
                .try_into::<File>()
                .is_err()
        );
    }

    #[cfg(feature = "cgroup")]
    #[test]
    fn the_cgroup_source_naming_no_cgroup_measures_one_made_for_the_run() {
        // The `[sources]` table, as the configuration holds it.
        let unnamed = document("[cgroup]\nglobal = false");
        let named = document("[cgroup]\nname = \"my.slice\"");

        assert!(measures_the_run_cgroup(&[SourceName::Cgroup], &unnamed));
        assert!(!measures_the_run_cgroup(&[SourceName::Cgroup], &named));
        assert!(!measures_the_run_cgroup(&[], &unnamed));
    }
}
