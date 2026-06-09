use joule_profiler_source_perf_event::config::PerfConfig;

use crate::CliArgs;

pub trait CliOverride: Sized {
    #[allow(unused_variables)]
    fn apply_override(self, cli: &CliArgs, config: &mut Self) {}
}

impl CliOverride for () {
    fn apply_override(self, _cli: &crate::CliArgs, _config: &mut Self) {}
}

impl CliOverride for PerfConfig {}
