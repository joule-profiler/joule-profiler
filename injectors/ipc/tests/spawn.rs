//! Spawns this test binary as the profiler of a session. It runs without the test harness, so
//! that its process has a single thread, as `serve_spawned` requires.

use std::convert::Infallible;
use std::process::Command;

use joule_profiler_core::metric::{MetricInfo, MetricValue};
use joule_profiler_core::processor::Processor;
use joule_profiler_core::profiler::JouleProfiler;
use joule_profiler_core::sensor::Sensor;
use joule_profiler_core::source::Source;
use joule_profiler_core::unit::MetricUnit;
use joule_profiler_injector_ipc::{IpcError, Received, Session, serve_spawned};

const PROFILER: &str = "JOULE_PROFILER_TEST_PROFILER";

#[derive(Default)]
struct Ticks(u64);

impl Sensor for Ticks {
    type Snapshot = u64;
    type Error = Infallible;

    fn name(&self) -> &'static str {
        "ticks"
    }

    fn measure(&mut self) -> Result<u64, Infallible> {
        self.0 += 1;
        Ok(self.0)
    }
}

struct Elapsed;

impl Processor<Ticks> for Elapsed {
    fn metrics(&self) -> Vec<MetricInfo> {
        vec![MetricInfo::new("ticks", MetricUnit::COUNT)]
    }

    fn process(
        &mut self,
        previous: &u64,
        current: &u64,
        values: &mut Vec<MetricValue>,
    ) -> Result<(), Infallible> {
        values.push(MetricValue::U64(current - previous));
        Ok(())
    }
}

fn main() {
    if std::env::var_os(PROFILER).is_some() {
        serve_spawned(|config| match config {
            "ticks" => {
                let mut profiler = JouleProfiler::new();
                profiler.add_source(Source::new(Ticks::default(), Elapsed));
                Ok(profiler)
            }
            _ => Err("no source available".into()),
        })
        .expect("the profiler detaches");
        return;
    }

    a_spawned_profiler_measures_this_process();
    a_spawned_profiler_that_cannot_measure_says_why();
}

fn profiler() -> Command {
    let mut command = Command::new(std::env::current_exe().expect("this test binary"));
    command.env(PROFILER, "1");
    command
}

fn a_spawned_profiler_measures_this_process() {
    let (mut session, results) = Session::spawn(profiler(), "ticks").unwrap();
    session.phase("load").unwrap();
    session.finish(0).unwrap();

    let mut names = Vec::new();
    let mut exit_code = None;
    for received in results {
        match received.unwrap() {
            Received::Schema(_) => {}
            Received::Phase(phase) => names.push(phase.info.name()),
            Received::Summary(summary) => exit_code = summary.exit_code,
        }
    }
    assert_eq!(names, ["START -> load", "load -> END"]);
    assert_eq!(exit_code, Some(0));
}

fn a_spawned_profiler_that_cannot_measure_says_why() {
    let outcome = Session::spawn(profiler(), "nothing");

    assert!(
        matches!(&outcome, Err(IpcError::Profiler(message)) if message == "no source available"),
        "{:?}",
        outcome.err()
    );
}
