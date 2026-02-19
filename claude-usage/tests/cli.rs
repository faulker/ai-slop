use assert_cmd::Command;

#[test]
fn help_flag_shows_usage() {
    let mut cmd = Command::cargo_bin("claude-usage").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains("Fetch Claude.ai usage data"));
}

#[test]
fn help_shows_db_option() {
    let mut cmd = Command::cargo_bin("claude-usage").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains("--db"));
}

#[test]
fn help_does_not_show_removed_flags() {
    let mut cmd = Command::cargo_bin("claude-usage").unwrap();
    let output = cmd.arg("--help").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("--format"),
        "--format should not appear in help"
    );
    assert!(
        !stdout.contains("--session-only"),
        "--session-only should not appear in help"
    );
}
