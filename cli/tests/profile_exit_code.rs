#![cfg(feature = "procfs")]

use std::{fs, process::Command};

use serde_json::Value;

#[test]
fn profile_returns_child_exit_code_and_writes_report() {
    let temp = tempfile::tempdir().unwrap();
    let report_path = temp.path().join("profile.json");

    for exit_code in [0, 42] {
        let output = Command::new(env!("CARGO_BIN_EXE_joule-profiler"))
            .args([
                "--sources",
                "procfs",
                "--output-format",
                "json",
                "--output-file",
            ])
            .arg(&report_path)
            .args(["profile", "--", "/bin/sh", "-c"])
            .arg(format!("exit {exit_code}"))
            .output()
            .unwrap();

        let report: Value = serde_json::from_slice(&fs::read(&report_path).unwrap())
            .unwrap_or_else(|err| {
                panic!(
                    "invalid profile report: {err}; stderr: {}",
                    String::from_utf8_lossy(&output.stderr)
                )
            });
        assert_eq!(report["exit_code"], exit_code);
        assert_eq!(output.status.code(), Some(exit_code));
    }
}
