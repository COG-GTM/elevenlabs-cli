//! `tests` commands print names, IDs and paths taken from project files and
//! API responses; none of them may reach the terminal with control
//! characters in them.

use std::path::Path;
use std::process::Output;

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk_0123456789abcdef0123456789abcdef01234567";

fn cli(dir: &Path, server: &str, args: &[&str]) -> Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_elevenlabs"));
    cmd.args(["--base-url", server])
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .env("TMPDIR", dir)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("APPDATA")
        .env("NO_COLOR", "1")
        .env("FERN_CLI_CREDENTIAL_STORE", "file")
        .env("ELEVENLABS_API_KEY", KEY)
        .env("ELEVENLABS_TELEMETRY", "0");
    for var in ["ELEVENLABS_BASE_URL", "ELEVENLABS_VIA", "ELEVENLABS_OUTPUT"] {
        cmd.env_remove(var);
    }
    cmd.output().expect("failed to spawn the elevenlabs binary")
}

fn write(dir: &Path, file: &str, text: &str) {
    let p = dir.join(file);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, text).unwrap();
}

fn assert_no_control_bytes(out: &Output) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for (stream, text) in [("stdout", &stdout), ("stderr", &stderr)] {
        assert!(
            !text.chars().any(|c| c.is_control() && c != '\n'),
            "{stream} contains control characters: {text:?}"
        );
    }
}

fn test_config(name: &str) -> String {
    json!({
        "name": name,
        "chat_history": [{"role": "user", "time_in_call_secs": 1, "message": "Hi"}],
        "success_condition": "ok",
        "type": "llm"
    })
    .to_string()
}

#[tokio::test]
async fn push_dry_run_strips_escapes_from_names_paths_and_ids() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    write(dir, "test_configs/evil.json", &test_config("\u{1b}[2Jx"));
    write(dir, "test_configs/bad\u{1b}[2J.json", "{ not json");
    write(
        dir,
        "tests.json",
        &json!({"tests": [
            {"config": "test_configs/evil.json", "id": "t\u{1b}[8m1"},
            {"config": "missing\u{1b}]0;pwned\u{7}.json", "id": "t2"}
        ]})
        .to_string(),
    );

    let out = cli(dir, &server.uri(), &["tests", "push", "--dry-run"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_no_control_bytes(&out);
    assert!(
        stdout.contains("[DRY RUN] Would update test: [2Jx"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Config file not found: missing]0;pwned.json"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Skipping unreadable test config"),
        "{stdout}"
    );
}

#[tokio::test]
async fn pull_dry_run_strips_escapes_from_api_names_and_ids() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/convai/agent-testing"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "tests": [{"id": "t\u{1b}]52;c;aGk=\u{7}1", "name": "\u{1b}[2J\u{1b}[1Aevil\u{202E}"}],
            "has_more": false
        })))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    write(dir, "tests.json", &json!({"tests": []}).to_string());

    let out = cli(dir, &server.uri(), &["tests", "pull", "--dry-run"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_no_control_bytes(&out);
    assert!(!stdout.contains('\u{202E}'), "{stdout}");
    assert!(
        stdout.contains("[DRY RUN] Would pull test: [2J[1Aevil (ID: t]52;c;aGk=1)"),
        "{stdout}"
    );
}

#[tokio::test]
async fn delete_all_list_strips_escapes_from_names_and_ids() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    write(
        dir,
        "test_configs/a.json",
        &test_config("Real\u{1b}[2K\rFake"),
    );
    write(
        dir,
        "tests.json",
        &json!({"tests": [{"config": "test_configs/a.json", "id": "t\u{1b}[8mhidden"}]})
            .to_string(),
    );

    let out = cli(dir, &server.uri(), &["tests", "delete", "--all", "--yes"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_no_control_bytes(&out);
    assert!(stdout.contains("1. Real[2K Fake (t[8mhidden)"), "{stdout}");
}
