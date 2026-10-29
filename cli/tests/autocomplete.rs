use std::process::Command;

fn joule_profiler_binary() -> std::path::PathBuf {
    std::env::var_os("CARGO_BIN_EXE_joule-profiler")
        .or_else(|| std::env::var_os("CARGO_BIN_EXE_joule_profiler"))
        .map(std::path::PathBuf::from)
        .expect("Cargo did not provide the joule-profiler test binary path")
}

#[test]
fn generates_completions_without_initializing_metric_sources() {
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let output = Command::new(joule_profiler_binary())
            .args(["autocomplete", shell])
            .output()
            .unwrap();

        assert!(output.status.success(), "{shell}: {output:?}");
        let script = String::from_utf8(output.stdout).unwrap();
        assert!(!script.is_empty(), "{shell}");
        assert!(script.contains("profile"), "{shell}");
        assert!(script.contains("list-sensors"), "{shell}");
        assert!(script.contains("output-format"), "{shell}");
    }
}

#[test]
fn hides_autocomplete_from_general_help() {
    let output = Command::new(joule_profiler_binary())
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(!help.contains("autocomplete"));
    assert!(help.contains("profile"));
    assert!(help.contains("list-sensors"));
}

#[test]
fn rejects_unsupported_completion_shell() {
    let output = Command::new(joule_profiler_binary())
        .args(["autocomplete", "elvish"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("invalid value"));
    assert!(error.contains("elvish"));
}
