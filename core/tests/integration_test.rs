use std::time::Duration;

use joule_profiler_core::{
    JouleProfiler,
    config::ProfileConfig,
    sensor::Sensors,
    source::MetricReader,
    types::{Metrics, PhaseToken},
};
use mockall::mock;

#[derive(Debug)]
pub struct MockError;

impl std::fmt::Display for MockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "mock error")
    }
}
impl std::error::Error for MockError {}

mock! {
    pub MetricReader {}

    impl MetricReader for MetricReader {
        type Type = ();
        type Error = MockError;

        async fn init(&mut self, pid: i32) -> Result<(), MockError>;
        async fn join(&mut self) -> Result<(), MockError>;
        async fn measure(&mut self) -> Result<(), MockError>;
        async fn retrieve(&mut self) -> Result<(), MockError>;
        fn get_sensors(&self) -> Result<Sensors, MockError>;
        fn to_metrics(&self, v: ()) -> Result<Metrics, MockError>;
        fn get_name() -> &'static str;
    }
}

fn mock_reader() -> MockMetricReader {
    let mut mock = MockMetricReader::new();
    mock.expect_init().returning(|_| Ok(()));
    mock.expect_join().returning(|| Ok(()));
    mock.expect_measure().returning(|| Ok(()));
    mock.expect_retrieve().returning(|| Ok(()));
    mock.expect_get_sensors()
        .returning(|| Ok(Sensors::default()));
    mock.expect_to_metrics()
        .returning(|_| Ok(Metrics::default()));
    mock
}

fn config(cmd: Vec<String>, pattern: &str) -> ProfileConfig {
    ProfileConfig {
        cmd,
        token_pattern: pattern.to_string(),
        stdout_file: None,
        use_root: false,
        init_timeout: Duration::from_secs(1),
    }
}

#[tokio::test]
async fn profile_no_phases() {
    let mut profiler = JouleProfiler::new();
    profiler.add_source(mock_reader());
    let config = config(vec!["echo".into(), "hello world".into()], "__PHASE__");
    let results = profiler.profile(&config).await.unwrap();
    assert_eq!(results.phases.len(), 1);
    assert_eq!(results.exit_code, 0);
    assert_eq!(results.phases.len(), 1);
    assert_eq!(results.phases[0].start_token, PhaseToken::Start);
    assert_eq!(results.phases[0].end_token, PhaseToken::End);
}

#[tokio::test]
async fn profile_single_phase_token() {
    let mut profiler = JouleProfiler::new();
    profiler.add_source(mock_reader());
    let config = config(
        vec!["echo".into(), "__PHASE_1__".into()],
        "__PHASE_[0-9]+__",
    );
    let results = profiler.profile(&config).await.unwrap();
    assert_eq!(results.exit_code, 0);
    assert_eq!(results.phases.len(), 2);
    assert_eq!(
        results.phases[0].end_token,
        PhaseToken::Token("__PHASE_1__".into())
    );
    assert_eq!(
        results.phases[1].start_token,
        PhaseToken::Token("__PHASE_1__".into())
    );
    assert_eq!(results.phases[1].end_token, PhaseToken::End);
}

#[tokio::test]
async fn profile_multiple_phase_tokens() {
    let mut profiler = JouleProfiler::new();
    profiler.add_source(mock_reader());
    let config = config(
        vec![
            "sh".into(),
            "-c".into(),
            "printf '__PHASE_1__\\n__PHASE_2__\\n__PHASE_3__\\n'".into(),
        ],
        "__PHASE_[0-9]+__",
    );
    let results = profiler.profile(&config).await.unwrap();
    let phases = &results.phases;
    assert_eq!(phases.len(), 4);
    assert_eq!(phases[0].start_token, PhaseToken::Start);
    assert_eq!(phases[0].end_token, PhaseToken::Token("__PHASE_1__".into()));
    assert_eq!(
        phases[1].start_token,
        PhaseToken::Token("__PHASE_1__".into())
    );
    assert_eq!(phases[1].end_token, PhaseToken::Token("__PHASE_2__".into()));
    assert_eq!(phases[2].end_token, PhaseToken::Token("__PHASE_3__".into()));
    assert_eq!(
        phases[3].start_token,
        PhaseToken::Token("__PHASE_3__".into())
    );
    assert_eq!(phases[3].end_token, PhaseToken::End);
}

#[tokio::test]
async fn profile_multiple_phases() {
    let mut profiler = JouleProfiler::new();
    profiler.add_source(mock_reader());
    let config = config(
        vec!["echo".into(), "__PHASE_1__".into()],
        "__PHASE_[0-9]+__",
    );

    let results = profiler.profile(&config).await.unwrap();
    assert_eq!(results.exit_code, 0);
    assert_eq!(results.phases.len(), 2);

    for (i, phase) in results.phases.iter().enumerate() {
        assert_eq!(phase.index, i);
    }
}

#[tokio::test]
async fn profile_nonzero_exit_code_is_reported() {
    let mut profiler = JouleProfiler::new();
    profiler.add_source(mock_reader());
    let config = config(
        vec!["sh".into(), "-c".into(), "exit 42".into()],
        "__PHASE__",
    );
    let result = profiler.profile(&config).await.unwrap();
    assert_eq!(result.exit_code, 42);
}
