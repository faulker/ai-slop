use assert_cmd::Command;
use std::io::{Read, Write};
use std::path::PathBuf;

/// A base URL whose port refuses connections, standing in for an unreachable API.
const UNREACHABLE_API: &str = "http://127.0.0.1:1";

/// Starts a background thread that answers exactly one HTTP request with
/// `body`, and returns its base URL. Good enough to stand in for the live API
/// without a network call or a mocking dependency.
fn start_stub_api_server(body: &'static str) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut discard = [0u8; 4096];
            let _ = stream.read(&mut discard);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}")
}

/// A token value that lets the API path reach the network without the Keychain.
const FAKE_TOKEN: &str = "sk-ant-oat-test";

/// Builds a command with an environment that can never reach the real Keychain
/// or a real token, so no test can pop a macOS password dialog.
fn cmd() -> Command {
    let mut cmd = Command::cargo_bin("claude-usage").unwrap();
    cmd.env("USER", "claude-usage-test-nonexistent")
        .env_remove("CLAUDE_USAGE_TOKEN")
        .env_remove("CLAUDE_USAGE_HISTORY_FILE");
    cmd
}

/// Writes a history fixture whose newest sample is `minutes_ago` old, preceded by
/// a zero sample 15 minutes earlier so a window boundary is visible.
fn write_history(name: &str, minutes_ago: i64) -> PathBuf {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let newest = now - minutes_ago * 60 * 1000;
    let previous = newest - 15 * 60 * 1000;
    write_fixture(
        name,
        &format!(
            r#"{{"version":2,"samples":[
                {{"t":{previous},"org":"test-org","u":{{"fh":0,"sd":18}}}},
                {{"t":{newest},"org":"test-org","u":{{"fh":42,"sd":18}}}}
            ]}}"#
        ),
    )
}

/// Writes arbitrary fixture content and returns its path.
fn write_fixture(name: &str, contents: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("claude-usage-{name}.json"));
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn help_flag_shows_usage() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains("Fetch Claude Code usage data"));
}

#[test]
fn help_shows_real_flags_and_hides_internal_ones() {
    let output = cmd().arg("--help").output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    for flag in ["--source", "--max-age", "--format"] {
        assert!(stdout.contains(flag), "{flag} should appear in help");
    }
    for flag in ["--history-file", "--api-base"] {
        assert!(!stdout.contains(flag), "{flag} should stay hidden");
    }
    // Guard against the deleted Chrome-cookie implementation coming back.
    assert!(!stdout.contains("--db"), "--db should not appear in help");
}

#[test]
fn local_fresh_json_matches_the_documented_contract() {
    let history = write_history("fresh-json", 4);
    let output = cmd()
        .args(["--source", "local", "--format", "json"])
        .arg("--history-file")
        .arg(&history)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&output).expect("stdout must be JSON");
    assert_eq!(parsed["source"], "local");
    assert_eq!(parsed["stale"], false);
    assert!(parsed["age_seconds"].as_i64().unwrap() < 60 * 10);
    assert_eq!(parsed["five_hour"]["used_percentage"], 42.0);
    assert_eq!(parsed["seven_day"]["used_percentage"], 18.0);
    assert!(
        parsed["five_hour"]["resets_at"].is_null(),
        "local must not claim an authoritative reset time"
    );
    assert!(
        parsed.as_object().unwrap().get("seven_day_opus").is_none(),
        "seven_day_opus is not part of the JSON contract"
    );
    assert!(
        parsed["extra_usage_dollars"].is_null(),
        "xu is absent from this fixture"
    );
}

#[test]
fn local_reports_extra_usage_when_present() {
    let history = write_fixture(
        "extra-usage",
        r#"{"version":2,"samples":[{"t":1783466131632,"u":{"fh":0,"sd":18,"xu":41.66}}]}"#,
    );
    cmd()
        .args([
            "--source",
            "local",
            "--format",
            "json",
            "--max-age",
            "999999999",
        ])
        .arg("--history-file")
        .arg(&history)
        .assert()
        .success()
        .stdout(predicates::str::contains("\"extra_usage_dollars\":41.66"));
}

#[test]
fn local_fresh_text_is_human_readable() {
    let history = write_history("fresh-text", 4);
    let output = cmd()
        .args(["--source", "local"])
        .arg("--history-file")
        .arg(&history)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&output);

    assert!(stdout.contains("5h window"), "got: {stdout}");
    assert!(stdout.contains("42% used"), "got: {stdout}");
    assert!(stdout.contains("7d window"), "got: {stdout}");
    assert!(stdout.contains("source: local"), "got: {stdout}");
    assert!(
        stdout.contains("resets in ~"),
        "estimate expected: {stdout}"
    );
    assert!(
        stdout.contains("(est)"),
        "estimate must be labeled: {stdout}"
    );
}

#[test]
fn local_missing_file_exits_nonzero() {
    cmd()
        .args(["--source", "local"])
        .arg("--history-file")
        .arg("/nonexistent/claude-usage/history.json")
        .assert()
        .failure()
        .stderr(predicates::str::contains("usage history"));
}

#[test]
fn local_unknown_version_exits_nonzero() {
    let history = write_fixture(
        "bad-version",
        r#"{"version":99,"samples":[{"t":1,"u":{"fh":0,"sd":0}}]}"#,
    );
    cmd()
        .args(["--source", "local"])
        .arg("--history-file")
        .arg(&history)
        .assert()
        .failure()
        .stderr(predicates::str::contains("99"));
}

#[test]
fn local_stale_data_warns_but_still_prints() {
    let history = write_history("stale-local", 200);
    let assert = cmd()
        .args(["--source", "local", "--max-age", "30"])
        .arg("--history-file")
        .arg(&history)
        .assert()
        .success();
    let output = assert.get_output();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("5h window"), "data still printed: {stdout}");
    assert!(stderr.contains("warning"), "got: {stderr}");
    assert!(stderr.contains("old"), "should mention the age: {stderr}");
}

#[test]
fn auto_uses_fresh_local_without_the_keychain_or_network() {
    let history = write_history("auto-fresh", 4);
    // No --source: auto is the default. An unreachable API and an unusable
    // Keychain: succeeding proves neither was used.
    cmd()
        .args(["--format", "json", "--api-base", UNREACHABLE_API])
        .arg("--history-file")
        .arg(&history)
        .assert()
        .success()
        .stdout(predicates::str::contains("\"source\":\"local\""));
}

#[test]
fn text_without_an_explicit_source_defaults_to_auto() {
    let history = write_history("text-default-source", 4);
    cmd()
        .args(["--api-base", UNREACHABLE_API])
        .arg("--history-file")
        .arg(&history)
        .assert()
        .success()
        .stdout(predicates::str::contains("source: local"));
}

#[test]
fn auto_falls_back_to_stale_local_when_the_api_fails() {
    let history = write_history("auto-stale", 200);
    let assert = cmd()
        .args([
            "--format",
            "json",
            "--max-age",
            "30",
            "--api-base",
            UNREACHABLE_API,
        ])
        .arg("--history-file")
        .arg(&history)
        // A token so the API path reaches the network rather than the Keychain.
        .env("CLAUDE_USAGE_TOKEN", FAKE_TOKEN)
        .assert()
        .success();
    let output = assert.get_output();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("\"source\":\"local\""), "got: {stdout}");
    assert!(stdout.contains("\"stale\":true"), "got: {stdout}");
    assert!(
        stderr.contains("old"),
        "should warn about staleness: {stderr}"
    );
    assert!(
        stderr.contains("API was unreachable"),
        "should warn about the API: {stderr}"
    );
}

#[test]
fn auto_fails_when_both_sources_fail() {
    cmd()
        .args(["--api-base", UNREACHABLE_API])
        .arg("--history-file")
        .arg("/nonexistent/claude-usage/history.json")
        .env("CLAUDE_USAGE_TOKEN", FAKE_TOKEN)
        .assert()
        .failure()
        .stderr(predicates::str::contains("no usage data"))
        .stderr(predicates::str::contains("local:"))
        .stderr(predicates::str::contains("api:"));
}

#[test]
fn raw_format_rejects_the_local_source() {
    let history = write_history("raw-local", 4);
    cmd()
        .args(["--source", "local", "--format", "raw"])
        .arg("--history-file")
        .arg(&history)
        .assert()
        .failure()
        .stderr(predicates::str::contains("invalid arguments"));
}

#[test]
fn api_source_enriches_the_real_reset_time_with_a_local_estimate() {
    // fh rises from 0 to 42 fifteen minutes before the newest sample, a
    // boundary the estimate can be computed from.
    let history = write_history("api-enrich", 4);
    let body = r#"{"five_hour":{"utilization":60.0,"resets_at":"2026-07-25T19:19:59Z"},
                    "seven_day":{"utilization":24.0,"resets_at":"2026-07-31T22:00:00Z"}}"#;
    let base = start_stub_api_server(body);

    let assert = cmd()
        .args(["--source", "api", "--format", "json"])
        .arg("--api-base")
        .arg(&base)
        .arg("--history-file")
        .arg(&history)
        .env("CLAUDE_USAGE_TOKEN", FAKE_TOKEN)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("stdout must be JSON");

    assert_eq!(parsed["source"], "api");
    assert_eq!(
        parsed["five_hour"]["resets_at"], "2026-07-25T19:19:59Z",
        "the API's own reset time must stay untouched: {stdout}"
    );
    assert!(
        parsed["five_hour"]["resets_at_estimated_ms"]
            .as_i64()
            .is_some(),
        "api results should also carry the local estimate when history is available: {stdout}"
    );
}

#[test]
fn api_source_leaves_the_estimate_null_without_local_history() {
    let body = r#"{"five_hour":{"utilization":60.0,"resets_at":"2026-07-25T19:19:59Z"}}"#;
    let base = start_stub_api_server(body);

    let assert = cmd()
        .args(["--source", "api", "--format", "json"])
        .arg("--api-base")
        .arg(&base)
        .arg("--history-file")
        .arg("/nonexistent/claude-usage/history.json")
        .env("CLAUDE_USAGE_TOKEN", FAKE_TOKEN)
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("stdout must be JSON");

    assert!(
        parsed["five_hour"]["resets_at_estimated_ms"].is_null(),
        "a missing history file must not fail the api result: {stdout}"
    );
}
