use crate::commands::profile::ProfileArgs;
use clap::{Args, Subcommand, ValueEnum};

pub mod profile;

/// Shells supported for completion script generation.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    #[value(name = "powershell")]
    PowerShell,
}

/// Arguments for shell completion generation.
#[derive(Args, Debug)]
pub struct AutocompleteArgs {
    /// Shell for which to generate a completion script.
    #[arg(value_enum)]
    pub shell: CompletionShell,
}

/// Subcommands of joule-profiler.
#[derive(Subcommand, Debug)]
pub enum ProfilerCommand {
    /// Profiling mode, executes a command and profiles it.
    Profile(ProfileArgs),

    /// List available sensors.
    ListSensors,

    /// Generate shell completion scripts.
    #[command(hide = true)]
    Autocomplete(AutocompleteArgs),
}
