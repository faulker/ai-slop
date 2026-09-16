use assert_cmd::Command;
use std::io::{Read, Write};

/// Builds a command with a fully cleared environment, so no test can pick up
/// real credentials from the developer's shell or saved config file. Wizard
/// is forced off so missing-credential cases stay non-interactive.
fn cmd() -> Command {
    let mut c = Command::cargo_bin("humble-game-check").unwrap();
    c.env_clear();
    c.env(
        "HUMBLE_GAME_CHECK_CONFIG",
        std::env::temp_dir().join("humble-game-check-tests-missing-config.json"),
    );
    c.env("HUMBLE_GAME_CHECK_WIZARD", "never");
    c
}

/// Starts a background thread that answers `connections` HTTP requests, each
/// with the body of the first route whose prefix matches the request path.
/// Good enough to stand in for the Humble/Steam APIs without a network call
/// or a mocking dependency.
fn start_stub_server(routes: Vec<(String, String)>, connections: usize) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for _ in 0..connections {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .nth(1)
                .unwrap_or("");
            let body = routes
                .iter()
                .find(|(prefix, _)| path.starts_with(prefix.as_str()))
                .map(|(_, b)| b.clone())
                .unwrap_or_else(|| "{}".to_string());
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

/// Starts a one-shot stub that answers the first request with a 302, the
/// shape of an expired Humble session cookie hitting the login redirect.
fn start_redirect_stub() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let response = "HTTP/1.1 302 Found\r\nLocation: /login\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}")
}

/// Reads the humble_orders.json fixture and re-keys the named order as the
/// `/api/v1/orders` endpoint would (an object keyed by gamekey).
fn orders_response_body(gamekey: &str) -> String {
    let raw = std::fs::read_to_string("tests/fixtures/dump/humble_orders.json").unwrap();
    let orders: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap();
    let order = orders
        .iter()
        .find(|o| o["gamekey"] == gamekey)
        .unwrap()
        .clone();
    serde_json::json!({ gamekey: order }).to_string()
}

/// Starts a Humble-base stub (order list + order detail for
/// "fixtureorderA1") and a Steam-base stub (vanity resolution + owned games
/// from the fixture), returning their base URLs.
fn start_stub_pair() -> (String, String) {
    let humble_routes = vec![
        (
            "/api/v1/user/order".to_string(),
            r#"[{"gamekey":"fixtureorderA1"}]"#.to_string(),
        ),
        (
            "/api/v1/orders".to_string(),
            orders_response_body("fixtureorderA1"),
        ),
    ];
    let humble_base = start_stub_server(humble_routes, 2);

    let steam_fixture = std::fs::read_to_string("tests/fixtures/dump/steam_owned.json").unwrap();
    let steam_routes = vec![
        (
            "/ISteamUser/ResolveVanityURL".to_string(),
            r#"{"response":{"success":1,"steamid":"7656119000000000"}}"#.to_string(),
        ),
        ("/IPlayerService/GetOwnedGames".to_string(), steam_fixture),
    ];
    let steam_base = start_stub_server(steam_routes, 2);

    (humble_base, steam_base)
}

#[test]
fn help_flag_shows_usage() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "List unrevealed Humble Bundle Steam keys",
        ));
}

#[test]
fn missing_humble_session_is_a_clear_error() {
    cmd()
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("HUMBLE_SESSION"))
        .stderr(predicates::str::contains("--humble-session"));
}

#[test]
fn missing_steam_key_is_a_clear_error() {
    cmd()
        .env("HUMBLE_SESSION", "fixture-session-cookie")
        .assert()
        .failure()
        .stderr(predicates::str::contains("STEAM_API_KEY"));
}

#[test]
fn missing_steam_id_is_a_clear_error() {
    cmd()
        .env("HUMBLE_SESSION", "fixture-session-cookie")
        .env("STEAM_API_KEY", "fixture-steam-key")
        .assert()
        .failure()
        .stderr(predicates::str::contains("No Steam account given"))
        .stderr(predicates::str::contains("--steam-vanity"));
}

#[test]
fn from_dump_runs_offline() {
    let output = cmd()
        .args(["--from-dump", "tests/fixtures/dump"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&output);

    assert!(stdout.contains("NOT OWNED (1)"), "got: {stdout}");
    assert!(stdout.contains("Fixture Quest 3"), "got: {stdout}");
    assert!(
        stdout.contains("https://www.humblebundle.com/downloads?key="),
        "got: {stdout}"
    );
    assert!(
        !stdout.contains("XXXXX-XXXXX"),
        "revealed key value leaked: {stdout}"
    );
    assert!(
        !stdout.contains("\nOWNED ("),
        "owned section must be hidden: {stdout}"
    );
    assert!(stdout.contains("LIKELY OWNED"), "got: {stdout}");
    assert!(stdout.contains("Fixture Racer Deluxe"), "got: {stdout}");
}

#[test]
fn from_dump_json() {
    let output = cmd()
        .args(["--from-dump", "tests/fixtures/dump", "--format", "json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(parsed["not_owned"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["owned"].as_array().unwrap().len(), 0);

    let output = cmd()
        .args([
            "--from-dump",
            "tests/fixtures/dump",
            "--format",
            "json",
            "--show-owned",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(parsed["owned"].as_array().unwrap().len(), 1);
}

#[test]
fn humble_auth_failure_message() {
    let base = start_redirect_stub();
    cmd()
        .env("HUMBLE_SESSION", "fixture-session-cookie")
        .env("STEAM_API_KEY", "fixture-steam-key")
        .args(["--steam-id", "7656119000000000"])
        .args(["--humble-base", &base])
        .assert()
        .failure()
        .stderr(predicates::str::contains("session cookie"));
}

#[test]
fn vanity_is_resolved_via_stub() {
    let (humble_base, steam_base) = start_stub_pair();

    cmd()
        .env("HUMBLE_SESSION", "fixture-session-cookie")
        .env("STEAM_API_KEY", "fixture-steam-key")
        .env("STEAM_VANITY", "yourvanityname")
        .args(["--humble-base", &humble_base])
        .args(["--steam-base", &steam_base])
        .assert()
        .success()
        .stdout(predicates::str::contains("NOT OWNED"));
}

#[test]
fn dump_dir_writes_both_files() {
    let (humble_base, steam_base) = start_stub_pair();
    let dir =
        std::env::temp_dir().join(format!("humble-game-check-cli-dump-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    cmd()
        .env("HUMBLE_SESSION", "fixture-session-cookie")
        .env("STEAM_API_KEY", "fixture-steam-key")
        .env("STEAM_VANITY", "yourvanityname")
        .args(["--humble-base", &humble_base])
        .args(["--steam-base", &steam_base])
        .arg("--dump-dir")
        .arg(&dir)
        .assert()
        .success();

    assert!(dir.join("humble_orders.json").exists());
    assert!(dir.join("steam_owned.json").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Unique temp path for a saved-credentials file used by one CLI test.
fn temp_config_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "humble-game-check-cli-{label}-{}.json",
        std::process::id()
    ))
}

#[test]
fn stored_config_fills_missing_credentials() {
    let (humble_base, steam_base) = start_stub_pair();
    let path = temp_config_path("stored");
    let cfg = serde_json::json!({
        "humble_session": "fixture-session-cookie",
        "steam_key": "fixture-steam-key",
        "steam_vanity": "yourvanityname"
    });
    std::fs::write(&path, serde_json::to_string(&cfg).unwrap()).unwrap();

    cmd()
        .env("HUMBLE_GAME_CHECK_CONFIG", &path)
        .args(["--humble-base", &humble_base])
        .args(["--steam-base", &steam_base])
        .assert()
        .success()
        .stdout(predicates::str::contains("NOT OWNED"));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn wizard_collects_credentials_and_saves_when_asked() {
    let (humble_base, steam_base) = start_stub_pair();
    let path = temp_config_path("wizard-save");
    let _ = std::fs::remove_file(&path);

    cmd()
        .env("HUMBLE_GAME_CHECK_WIZARD", "1")
        .env("HUMBLE_GAME_CHECK_CONFIG", &path)
        .args(["--humble-base", &humble_base])
        .args(["--steam-base", &steam_base])
        .write_stdin("fixture-session-cookie\nfixture-steam-key\n2\nyourvanityname\ny\n")
        .assert()
        .success()
        .stdout(predicates::str::contains("NOT OWNED"))
        .stderr(predicates::str::contains("Saved credentials"));

    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["humble_session"], "fixture-session-cookie");
    assert_eq!(saved["steam_key"], "fixture-steam-key");
    assert_eq!(saved["steam_vanity"], "yourvanityname");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn wizard_skips_save_when_declined() {
    let (humble_base, steam_base) = start_stub_pair();
    let path = temp_config_path("wizard-nosave");
    let _ = std::fs::remove_file(&path);

    cmd()
        .env("HUMBLE_GAME_CHECK_WIZARD", "1")
        .env("HUMBLE_GAME_CHECK_CONFIG", &path)
        .args(["--humble-base", &humble_base])
        .args(["--steam-base", &steam_base])
        .write_stdin("fixture-session-cookie\nfixture-steam-key\n1\n7656119000000000\nn\n")
        .assert()
        .success()
        .stdout(predicates::str::contains("NOT OWNED"));

    assert!(
        !path.exists(),
        "config should not be written when save is declined"
    );
}

#[test]
fn wizard_eof_is_cancelled() {
    cmd()
        .env("HUMBLE_GAME_CHECK_WIZARD", "1")
        .write_stdin("")
        .assert()
        .failure()
        .stderr(predicates::str::contains("setup cancelled"));
}

#[test]
fn wizard_only_asks_for_values_still_missing() {
    let (humble_base, steam_base) = start_stub_pair();
    let path = temp_config_path("wizard-partial");
    let _ = std::fs::remove_file(&path);

    cmd()
        .env("HUMBLE_GAME_CHECK_WIZARD", "1")
        .env("HUMBLE_GAME_CHECK_CONFIG", &path)
        .env("HUMBLE_SESSION", "fixture-session-cookie")
        .env("STEAM_VANITY", "yourvanityname")
        .args(["--humble-base", &humble_base])
        .args(["--steam-base", &steam_base])
        .write_stdin("fixture-steam-key\ny\n")
        .assert()
        .success()
        .stdout(predicates::str::contains("NOT OWNED"));

    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["steam_key"], "fixture-steam-key");
    assert!(saved.get("humble_session").is_none());
    let _ = std::fs::remove_file(&path);
}
