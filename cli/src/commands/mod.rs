use crate::commands::profile::ProfileArgs;
use clap::Subcommand;
use clap_complete::Shell;

pub mod profile;

/// Subcommands of joule-profiler.
#[derive(Subcommand, Debug)]
pub enum ProfilerCommand {
    /// Profiling mode, executes a command and profiles it.
    Profile(ProfileArgs),

    /// List available sensors.
    ListSensors,

    /// Generate auto-completion file.
    #[command(name = "autocomplete", hide = true)]
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}
