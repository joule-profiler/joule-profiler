use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use joule_profiler_core::util::cgroup::CgroupConfig;
use joule_profiler_injector_stdout::DEFAULT_PATTERN;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use toml::{Table, Value};

use crate::cli::{Cli, Command, SourceName};
use crate::exporter::ExporterConfig;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ConfigFile {
    profiler: ProfilerConfig,

    exporter: ExporterConfig,

    injector: InjectorConfig,

    cgroup: Option<CgroupConfig>,

    sources: Table,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ProfilerConfig {
    defer: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct InjectorConfig {
    token_pattern: Option<String>,

    /// Where the output of the program goes, instead of the terminal.
    stdout_file: Option<PathBuf>,

    /// Under `sudo`, whether the program keeps root.
    use_root: bool,
}

/// The configuration file and the flags, merged.
pub struct ConfigTable {
    pub sources: Vec<SourceName>,

    pub tables: Table,

    pub exporter: ExporterConfig,

    pub token_pattern: String,
    pub stdout_file: Option<PathBuf>,
    pub use_root: bool,

    pub defer: bool,

    pub cgroup: Option<CgroupConfig>,
}

impl ConfigTable {
    pub fn load_from_cli(cli: &Cli) -> Result<Self> {
        let config_file = read_file_and_override(cli)?;

        let (pattern, defer) = match &cli.command {
            Command::Profile(args) => (args.token_pattern.clone(), args.defer),
            Command::ListSensors | Command::Info => (None, false),
        };

        let sources = configured_sources(cli, &config_file)?;

        let cgroup = config_file.cgroup.or_else(|| {
            needs_run_cgroup(&sources, &config_file.sources).then(CgroupConfig::default)
        });

        Ok(Self {
            sources,

            exporter: config_file.exporter,

            // A flag overrides the file.
            token_pattern: pattern
                .or(config_file.injector.token_pattern)
                .unwrap_or_else(|| DEFAULT_PATTERN.to_owned()),

            stdout_file: config_file.injector.stdout_file,
            use_root: config_file.injector.use_root,

            defer: defer || config_file.profiler.defer,
            cgroup,

            tables: config_file.sources,
        })
    }

    pub fn source_table(&self, name: SourceName) -> Option<&Value> {
        self.tables.get(name.key())
    }
}

/// Reads the configuration file, then applies the `--define` overrides and the flags.
fn read_file_and_override(cli: &Cli) -> Result<ConfigFile> {
    let mut document = match &cli.config_file {
        Some(path) => {
            let content =
                fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

            toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?
        }
        None => Table::new(),
    };

    for assignment in &cli.set {
        set_assignment(&mut document, assignment)
            .with_context(|| format!("--define {assignment}"))?;
    }

    set_exporter_flags(&mut document, cli)?;

    document.try_into().context("reading the configuration")
}

/// Sets `section.key=value` in the document, creating the missing sections.
fn set_assignment(document: &mut Table, assignment: &str) -> Result<()> {
    let Some((path, raw)) = assignment.split_once('=') else {
        bail!("expected `key=value`");
    };

    let mut keys = path.split('.').peekable();
    let mut table = document;

    while let Some(key) = keys.next() {
        if keys.peek().is_none() {
            table.insert(key.to_owned(), parse_toml_value(raw));
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

/// Applies `--output-format` and `--output-file` over the `[exporter]` table.
fn set_exporter_flags(document: &mut Table, cli: &Cli) -> Result<()> {
    if cli.output_format.is_none() && cli.output_file.is_none() {
        return Ok(());
    }

    let exporter = document
        .entry("exporter")
        .or_insert_with(|| Value::Table(Table::new()))
        .as_table_mut()
        .context("`exporter` is a value, not a section")?;

    if let Some(format) = cli.output_format {
        let format = format.to_string();

        if exporter.get("format").and_then(Value::as_str) != Some(&format) {
            exporter.clear();
            exporter.insert("format".to_owned(), Value::String(format));
        }
    }

    if let Some(file) = &cli.output_file {
        if matches!(
            exporter.get("format").and_then(Value::as_str),
            None | Some("terminal")
        ) {
            bail!(
                "--output-file needs a format that writes a file, such as `--output-format json`"
            );
        }

        exporter.insert(
            "file".to_owned(),
            Value::String(file.to_string_lossy().into_owned()),
        );
    }

    Ok(())
}

/// Parses the value as TOML, or as a string if it is not valid TOML.
fn parse_toml_value(raw: &str) -> Value {
    toml::from_str::<Table>(&format!("value = {raw}"))
        .ok()
        .and_then(|mut table| table.remove("value"))
        .unwrap_or_else(|| Value::String(raw.to_owned()))
}

/// Returns the sources named by `--sources`, then those with a table in the file.
fn configured_sources(cli: &Cli, file: &ConfigFile) -> Result<Vec<SourceName>> {
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

#[cfg_attr(not(feature = "cgroup"), allow(unused_variables))]
fn needs_run_cgroup(sources: &[SourceName], tables: &Table) -> bool {
    #[cfg(feature = "cgroup")]
    {
        let names_one = tables
            .get(SourceName::Cgroup.key())
            .and_then(|table| table.get("name"))
            .is_some();

        sources.contains(&SourceName::Cgroup) && !names_one
    }

    #[cfg(not(feature = "cgroup"))]
    false
}

/// Reads a source configuration from its table. No table means all defaults.
pub fn read_source_config<C: DeserializeOwned>(
    name: SourceName,
    table: Option<&Value>,
) -> Result<C> {
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

        set_assignment(&mut table, "sources.perf_event.scope=cgroup").expect("sets");

        assert_eq!(
            table["sources"]["perf_event"]["scope"],
            Value::String("cgroup".to_owned()),
            "the sections were made on the way down"
        );
    }

    #[test]
    fn a_definition_reads_its_value_as_what_it_looks_like() {
        let mut table = document("");

        set_assignment(&mut table, "profiler.defer=true").expect("sets");
        set_assignment(&mut table, "sources.perf_event.cpus=[0, 1]").expect("sets");
        set_assignment(&mut table, "injector.token_pattern=__[A-Z]+__").expect("sets");

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
        let mut table = document("[exporter]\nformat = \"json\"\nlines = false");

        set_assignment(&mut table, "exporter.lines=true").expect("sets");

        assert_eq!(table["exporter"]["lines"], Value::Boolean(true));
    }

    #[test]
    fn a_definition_cannot_dig_through_a_value() {
        let mut table = document("[profiler]\ndefer = true");

        assert!(set_assignment(&mut table, "profiler.defer.deeper=1").is_err());
    }

    #[test]
    fn a_section_that_does_not_exist_is_refused() {
        let outcome: Result<ConfigFile, _> = document("[exportr]\nformat = 1").try_into();

        assert!(outcome.is_err());
    }

    fn exporter(toml: &str) -> Result<ExporterConfig, toml::de::Error> {
        document(toml)
            .try_into::<ConfigFile>()
            .map(|file| file.exporter)
    }

    #[test]
    fn the_format_picks_the_exporter_and_its_keys_sit_beside_it() {
        assert!(matches!(exporter(""), Ok(ExporterConfig::Terminal(_))));

        let Ok(ExporterConfig::Json(json)) =
            exporter("[exporter]\nformat = \"json\"\nlines = true\nfile = \"out.jsonl\"")
        else {
            panic!("reads as json");
        };
        assert!(json.lines);
        assert_eq!(json.file, Some(PathBuf::from("out.jsonl")));

        assert!(
            exporter("[exporter]\nformat = \"csv\"\nlines = true").is_err(),
            "a key of another format is refused"
        );
        assert!(
            exporter("[exporter]\nformat = \"terminal\"\nfile = \"out\"").is_err(),
            "the terminal writes no file"
        );
    }

    #[test]
    fn a_source_table_names_a_source_that_exists() {
        use clap::ValueEnum;

        for source in SourceName::value_variants() {
            let file: ConfigFile = document(&format!("[sources.{}]", source.key()))
                .try_into()
                .expect("reads");

            assert_eq!(
                SourceName::from_key(file.sources.keys().next().unwrap()),
                Some(*source)
            );
        }

        let file: ConfigFile = document("[sources.nothing]").try_into().expect("reads");
        assert!(SourceName::from_key(file.sources.keys().next().unwrap()).is_none());
    }

    #[test]
    fn a_cgroup_table_is_the_cgroup_made_for_the_run() {
        let file: ConfigFile =
            document("[cgroup]\nname = \"a-run\"\nattach = false\nfreeze = true")
                .try_into()
                .expect("reads");

        assert!(file.cgroup.is_some());
        assert!(
            document("[cgroup]\nnam = \"a-run\"")
                .try_into::<ConfigFile>()
                .is_err()
        );
    }

    #[cfg(feature = "cgroup")]
    #[test]
    fn the_cgroup_source_naming_no_cgroup_measures_one_made_for_the_run() {
        // The `[sources]` table, as the configuration holds it.
        let unnamed = document("[cgroup]\nglobal = false");
        let named = document("[cgroup]\nname = \"my.slice\"");

        assert!(needs_run_cgroup(&[SourceName::Cgroup], &unnamed));
        assert!(!needs_run_cgroup(&[SourceName::Cgroup], &named));
        assert!(!needs_run_cgroup(&[], &unnamed));
    }
}
