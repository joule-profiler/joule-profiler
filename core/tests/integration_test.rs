//! Runs the profiler end to end, with an injector, a sensor and an exporter of its own: the
//! target is a script of tokens, and every step of the run is written down where the tests can
//! read it back.

use std::{
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use joule_profiler_core::{
    Error, Exporter, Info, Injector, JouleProfiler, MetricInfo, MetricUnit, MetricValue, PhaseInfo,
    PhaseToken, Processor, Schema, Sensor, Source, SourceMetrics, StopHandle, Summary, Target,
};

#[derive(Debug, thiserror::Error)]
#[error("fake")]
struct Fake;

#[derive(Clone, Default)]
struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    fn push(&self, event: impl Into<String>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.into());
    }

    fn has(&self, event: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|seen| seen == event)
    }

    fn count(&self, event: &str) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|seen| *seen == event)
            .count()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fail {
    Nothing,
    Init,
    Attach,
    /// The nth measure, the first being that of the start of the run.
    Measure(usize),
    Panic(usize),
    Begin,
    NextPhase,
}

struct ScriptInjector {
    tokens: Vec<&'static str>,
    exit_code: i32,
    fail: Fail,
    stopped: Arc<AtomicBool>,
    events: Events,
}

impl Injector for ScriptInjector {
    type Error = Fake;

    fn start(&mut self) -> Result<Target, Fake> {
        self.events.push("started");
        Ok(Target { pid: 1 })
    }

    fn stop_handle(&self) -> StopHandle {
        let (stopped, events) = (Arc::clone(&self.stopped), self.events.clone());

        Box::new(move || {
            stopped.store(true, Ordering::SeqCst);
            events.push("stopped");
            Ok(())
        })
    }

    fn resume(&mut self) -> Result<(), Fake> {
        self.events.push("resumed");
        Ok(())
    }

    fn next_phase(&mut self) -> Result<Option<PhaseToken>, Fake> {
        thread::sleep(Duration::from_millis(2));

        if self.fail == Fail::NextPhase {
            return Err(Fake);
        }
        if self.stopped.load(Ordering::SeqCst) || self.tokens.is_empty() {
            return Ok(None);
        }

        Ok(Some(PhaseToken {
            text: self.tokens.remove(0).to_owned(),
            line: None,
        }))
    }

    fn wait(&mut self) -> Result<Option<i32>, Fake> {
        self.events.push("waited");
        Ok(Some(self.exit_code))
    }
}

struct CountingSensor {
    measures: usize,
    fail: Fail,
    events: Events,
}

impl Sensor for CountingSensor {
    type Snapshot = usize;
    type Error = Fake;

    fn name(&self) -> &'static str {
        "counting"
    }

    fn init(&mut self) -> Result<(), Fake> {
        self.events.push("init");
        if self.fail == Fail::Init {
            Err(Fake)
        } else {
            Ok(())
        }
    }

    fn attach(&mut self, _target: Target) -> Result<(), Fake> {
        self.events.push("attach");
        if self.fail == Fail::Attach {
            Err(Fake)
        } else {
            Ok(())
        }
    }

    fn measure(&mut self) -> Result<usize, Fake> {
        self.measures += 1;

        match self.fail {
            Fail::Measure(at) if at == self.measures => Err(Fake),
            Fail::Panic(at) if at == self.measures => panic!("the sensor panicked"),
            _ => Ok(self.measures),
        }
    }

    fn close(&mut self) -> Result<(), Fake> {
        self.events.push("close");
        Ok(())
    }
}

struct CountingProcessor;

impl Processor<CountingSensor> for CountingProcessor {
    fn metrics(&self) -> Vec<MetricInfo> {
        vec![MetricInfo::new("measures", MetricUnit::COUNT)]
    }

    fn process(
        &mut self,
        previous: &usize,
        current: &usize,
        values: &mut Vec<MetricValue>,
    ) -> Result<(), Fake> {
        values.push(MetricValue::U64((current - previous) as u64));
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Exported {
    index: usize,
    start: String,
    end: String,
    values: Vec<MetricValue>,
}

#[derive(Clone, Default)]
struct Collected {
    phases: Arc<Mutex<Vec<Exported>>>,
    summary: Arc<Mutex<Option<Summary>>>,
}

impl Collected {
    fn phases(&self) -> Vec<Exported> {
        self.phases.lock().unwrap().clone()
    }

    fn tokens(&self) -> Vec<(String, String)> {
        self.phases()
            .into_iter()
            .map(|phase| (phase.start, phase.end))
            .collect()
    }
}

struct CollectingExporter {
    collected: Collected,
    fail: Fail,
}

impl Exporter for CollectingExporter {
    type Error = Fake;

    fn begin(&mut self, _schema: &Schema) -> Result<(), Fake> {
        if self.fail == Fail::Begin {
            Err(Fake)
        } else {
            Ok(())
        }
    }

    fn export(&mut self, phase: &PhaseInfo, sources: &[SourceMetrics<'_>]) -> Result<(), Fake> {
        self.collected.phases.lock().unwrap().push(Exported {
            index: phase.index,
            start: phase.start_token.clone(),
            end: phase.end_token.clone(),
            values: sources
                .iter()
                .flat_map(|source| source.metrics.iter().map(|metric| metric.value))
                .collect(),
        });
        Ok(())
    }

    fn finish(&mut self, summary: &Summary) -> Result<(), Fake> {
        *self.collected.summary.lock().unwrap() = Some(*summary);
        Ok(())
    }
}

struct Run {
    tokens: Vec<&'static str>,
    exit_code: i32,
    fail: Fail,
    sources: usize,
    defer: bool,
}

impl Run {
    fn printing(tokens: &[&'static str]) -> Self {
        Self {
            tokens: tokens.to_vec(),
            exit_code: 0,
            fail: Fail::Nothing,
            sources: 2,
            defer: false,
        }
    }

    fn failing(mut self, fail: Fail) -> Self {
        self.fail = fail;
        self
    }

    /// Runs on its own thread so that a hung run fails the test instead of blocking it.
    fn profile(self) -> (Result<Summary, Error>, Events, Collected) {
        let events = Events::default();
        let collected = Collected::default();

        let mut profiler = JouleProfiler::new();
        for _ in 0..self.sources {
            profiler.add_source(Source::new(
                CountingSensor {
                    measures: 0,
                    fail: self.fail,
                    events: events.clone(),
                },
                CountingProcessor,
            ));
        }
        profiler.set_injector(ScriptInjector {
            tokens: self.tokens,
            exit_code: self.exit_code,
            fail: self.fail,
            stopped: Arc::default(),
            events: events.clone(),
        });
        profiler.set_exporter(CollectingExporter {
            collected: collected.clone(),
            fail: self.fail,
        });
        profiler.set_defer(self.defer);

        let (done, outcome) = mpsc::channel();
        thread::spawn(move || done.send(profiler.profile()));
        let summary = outcome.recv_timeout(Duration::from_secs(10)).unwrap();

        (summary, events, collected)
    }
}

#[test]
fn info_lists_machine_then_sources() {
    let events = Events::default();
    let mut profiler = JouleProfiler::new();
    for _ in 0..2 {
        profiler.add_source(
            Source::new(
                CountingSensor {
                    measures: 0,
                    fail: Fail::Nothing,
                    events: events.clone(),
                },
                CountingProcessor,
            )
            .with_info(Info::new().with("counts", "measures")),
        );
    }

    let info = profiler.info();

    let names: Vec<&str> = info.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["machine", "counting", "counting"]);
    assert_eq!(info[1].1, Info::new().with("counts", "measures"));
    assert!(!events.has("init"));
}

fn assert_cleaned_up(events: &Events, sources: usize) {
    assert!(events.has("waited"));
    assert_eq!(events.count("close"), sources);
}

#[test]
fn no_tokens_one_phase() {
    let (summary, events, collected) = Run::printing(&[]).profile();
    let summary = summary.unwrap();

    assert_eq!(collected.tokens(), [("START".into(), "END".into())]);
    assert_eq!(summary.phases, 1);
    assert_eq!(summary.exit_code, Some(0));
    assert!(!events.has("stopped"));
    assert_cleaned_up(&events, 2);
}

#[test]
fn tokens_split_phases() {
    let (summary, _, collected) = Run::printing(&["__A__", "__B__", "__C__"]).profile();
    summary.unwrap();

    assert_eq!(
        collected.tokens(),
        [
            ("START".into(), "__A__".into()),
            ("__A__".into(), "__B__".into()),
            ("__B__".into(), "__C__".into()),
            ("__C__".into(), "END".into()),
        ]
    );
}

#[test]
fn phases_in_order() {
    let (summary, _, collected) = Run::printing(&["__A__", "__B__"]).profile();
    summary.unwrap();

    for (index, phase) in collected.phases().into_iter().enumerate() {
        assert_eq!(phase.index, index);
        assert_eq!(phase.values, [MetricValue::U64(1), MetricValue::U64(1)]);
    }
}

#[test]
fn exit_code() {
    let mut run = Run::printing(&[]);
    run.exit_code = 42;

    let (summary, _, collected) = run.profile();

    assert_eq!(summary.unwrap().exit_code, Some(42));
    assert_eq!(
        collected.summary.lock().unwrap().unwrap().exit_code,
        Some(42)
    );
}

#[test]
fn deferred_run() {
    let mut run = Run::printing(&["__A__", "__B__"]);
    run.defer = true;

    let (summary, _, collected) = run.profile();
    summary.unwrap();

    assert_eq!(collected.phases().len(), 3);
    assert!(
        collected
            .phases()
            .iter()
            .all(|phase| phase.values == [MetricValue::U64(1), MetricValue::U64(1)])
    );
}

#[test]
fn init_failure_stops_before_start() {
    let (summary, events, _) = Run::printing(&["__A__"]).failing(Fail::Init).profile();

    assert!(matches!(summary, Err(Error::Source { .. })));
    assert!(!events.has("started"));
    assert_eq!(events.count("close"), 2);
}

#[test]
fn attach_failure_stops_target() {
    let (summary, events, _) = Run::printing(&["__A__"]).failing(Fail::Attach).profile();

    assert!(matches!(summary, Err(Error::Source { .. })));
    assert!(events.has("stopped"));
    assert!(!events.has("resumed"));
    assert_cleaned_up(&events, 2);
}

#[test]
fn measure_failure_stops_target() {
    for at in [1, 3] {
        let (summary, events, _) = Run::printing(&["__A__", "__B__", "__C__"])
            .failing(Fail::Measure(at))
            .profile();

        assert!(matches!(summary, Err(Error::Source { .. })), "measure {at}");
        assert!(events.has("stopped"), "measure {at}");
        assert_cleaned_up(&events, 2);
    }
}

#[test]
fn sensor_panic_is_reported() {
    let (summary, events, _) = Run::printing(&["__A__", "__B__"])
        .failing(Fail::Panic(2))
        .profile();

    assert!(matches!(summary, Err(Error::Panicked(_))));
    assert!(events.has("stopped"));
    assert_cleaned_up(&events, 2);
}

#[test]
fn exporter_failure_never_resumes_target() {
    let (summary, events, _) = Run::printing(&["__A__"]).failing(Fail::Begin).profile();

    assert!(matches!(summary, Err(Error::Exporter(_))));
    assert!(!events.has("resumed"));
    assert_cleaned_up(&events, 2);
}

#[test]
fn injector_failure_stops_target() {
    let (summary, events, _) = Run::printing(&["__A__"]).failing(Fail::NextPhase).profile();

    assert!(matches!(summary, Err(Error::Injector(_))));
    assert!(events.has("stopped"));
    assert_cleaned_up(&events, 2);
}

#[test]
fn no_source_refused() {
    let mut run = Run::printing(&["__A__"]);
    run.sources = 0;

    let (summary, events, _) = run.profile();

    assert!(matches!(summary, Err(Error::NoSource)));
    assert!(!events.has("started"));
}

#[test]
fn injector_and_exporter_required() {
    let mut profiler = JouleProfiler::new();
    profiler.add_source(Source::new(
        CountingSensor {
            measures: 0,
            fail: Fail::Nothing,
            events: Events::default(),
        },
        CountingProcessor,
    ));
    assert!(matches!(profiler.profile(), Err(Error::NoInjector)));

    profiler.set_injector(ScriptInjector {
        tokens: Vec::new(),
        exit_code: 0,
        fail: Fail::Nothing,
        stopped: Arc::default(),
        events: Events::default(),
    });
    assert!(matches!(profiler.profile(), Err(Error::NoExporter)));
}
