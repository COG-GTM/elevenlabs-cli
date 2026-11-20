//! Integration tests: `tools` commands strip terminal control characters from
//! names, IDs, paths and errors that come from project files or the API.

use std::path::Path;
use std::process::Output;

use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk_0123456789abcdef0123456789abcdef01234567";
const HOSTILE: &str = "evil\u{1b}[2J\u{1b}]0;title\u{7}\u{202E}name\r\nfake";

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

/// Neither stream carries a control character (other than the newlines that
/// end lines) or a bidi override, and the visible text is still printed.
fn assert_terminal_safe(out: &Output, expect_visible: &str) {
    for (label, bytes) in [("stdout", &out.stdout), ("stderr", &out.stderr)] {
        let text = String::from_utf8_lossy(bytes);
        assert!(!bytes.contains(&0x1b), "{label} contains ESC: {text:?}");
        assert!(!bytes.contains(&0x07), "{label} contains BEL: {text:?}");
        assert!(!bytes.contains(&b'\r'), "{label} contains CR: {text:?}");
        assert!(!text.contains('\u{202E}'), "{label} contains RLO: {text:?}");
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(expect_visible),
        "expected {expect_visible:?} in stdout:\n{stdout}"
    );
    assert!(
        !stdout.lines().any(|l| l.trim_start().starts_with("fake")),
        "a newline in the name started a forged line:\n{stdout}"
    );
}

fn hostile_tool() -> Value {
    json!({
        "name": HOSTILE,
        "description": "Lookup tool",
        "type": "webhook",
        "api_schema": {"url": "https://api.example.com/webhook", "method": "POST"}
    })
}

fn setup_project(dir: &Path, id: Option<&str>) {
    write(dir, "tool_configs/lookup.json", &hostile_tool().to_string());
    let mut entry = json!({"type": "webhook", "config": "tool_configs/lookup.json"});
    if let Some(id) = id {
        entry["id"] = json!(id);
    }
    write(dir, "tools.json", &json!({"tools": [entry]}).to_string());
}

#[tokio::test]
async fn push_dry_run_strips_escapes_from_config_name() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), Some("tool_1"));

    let out = cli(dir.path(), &server.uri(), &["tools", "push", "--dry-run"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_terminal_safe(
        &out,
        "[DRY RUN] Would update tool: evil[2J]0;titlename  fake",
    );
}

#[tokio::test]
async fn push_strips_escapes_from_name_and_api_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/convai/tools"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": "tool\u{1b}[31m_9", "tool_config": {"name": "x"}})),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), None);

    let out = cli(dir.path(), &server.uri(), &["tools", "push"]);
    assert_terminal_safe(
        &out,
        "Created tool evil[2J]0;titlename  fake (ID: tool[31m_9)",
    );
}

#[tokio::test]
async fn push_strips_escapes_from_missing_config_path() {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "tools.json",
        &json!({"tools": [{"type": "webhook", "config": "gone\u{1b}[2K.json", "id": "t"}]})
            .to_string(),
    );

    let out = cli(dir.path(), &server.uri(), &["tools", "push"]);
    assert_terminal_safe(&out, "Warning: Config file not found: gone[2K.json");
}

#[tokio::test]
async fn delete_all_list_strips_escapes_from_name_and_id() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    setup_project(dir.path(), Some("tool\u{1b}[8m_1"));

    let out = cli(
        dir.path(),
        &server.uri(),
        &["tools", "delete", "--all", "--yes"],
    );
    assert_terminal_safe(&out, "1. evil[2J]0;titlename  fake (tool[8m_1)");
}

#[tokio::test]
async fn pull_dry_run_strips_escapes_from_api_names_and_ids() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/convai/tools"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "tools": [{"id": "tool\u{1b}[8m_2", "tool_config": hostile_tool()}]
        })))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();

    let out = cli(dir.path(), &server.uri(), &["tools", "pull", "--dry-run"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_terminal_safe(
        &out,
        "[DRY RUN] Would pull tool: evil[2J]0;titlename  fake (ID: tool[8m_2)",
    );
}
