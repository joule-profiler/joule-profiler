use std::process::Command;

use tempfile::tempdir;

fn run_with_missing_command(
    output_format: &str,
    output_file: &std::path::Path,
) -> std::process::Output {
    Command::new(std::env::var("CARGO_BIN_EXE_joule-profiler").unwrap())
        .args([
            "--sources",
            "procfs",
            "--output-format",
            output_format,
            "--output-file",
            output_file.to_str().unwrap(),
            "profile",
            "--",
            "definitely-not-a-real-command",
        ])
        .output()
        .unwrap()
}

#[test]
fn missing_command_does_not_create_json_output() {
    let directory = tempdir().unwrap();
    let output_file = directory.path().join("failure.json");

    let output = run_with_missing_command("json", &output_file);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Command not found"));
    assert!(!output_file.exists());
}

#[test]
fn missing_command_does_not_create_csv_output() {
    let directory = tempdir().unwrap();
    let output_file = directory.path().join("failure.csv");

    let output = run_with_missing_command("csv", &output_file);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Command not found"));
    assert!(!output_file.exists());
}
