//! Integration tests for the global `--dry-run` flag on the typed workflow
//! commands that change state (`tools add/delete`, `tests add/delete`,
//! `agents init/add/test`, `components add`). A dry run must send no request and
//! leave the project files untouched.

use std::path::Path;
use std::process::Output;

use serde_json::{json, Value};
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

fn output(out: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn write(dir: &Path, file: &str, text: &str) {
    let p = dir.join(file);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, text).unwrap();
}

fn read(dir: &Path, file: &str) -> String {
    std::fs::read_to_string(dir.join(file)).unwrap()
}

/// Every file under `dir` (relative path, contents), sorted.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let rel = p.strip_prefix(root).unwrap().display().to_string();
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

async fn assert_no_requests(server: &MockServer, out: &Output) {
    let requests = server.received_requests().await.unwrap();
    let sent: Vec<String> = requests
        .iter()
        .map(|r| format!("{} {}", r.method, r.url.path()))
        .collect();
    assert!(
        sent.is_empty(),
        "dry run sent requests {sent:?}\n{}",
        output(out)
    );
}

/// Run `args` in a dry run and check it succeeds, sends nothing, prints a
/// `[DRY RUN]` preview, and leaves every file in `dir` byte-for-byte unchanged.
async fn assert_dry_run_is_inert(dir: &Path, args: &[&str]) -> Output {
    let server = MockServer::start().await;
    let before = snapshot(dir);
    let out = cli(dir, &server.uri(), args);
    assert!(out.status.success(), "expected exit 0\n{}", output(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("[DRY RUN]"),
        "expected a dry-run preview\n{}",
        output(&out)
    );
    assert_no_requests(&server, &out).await;
    assert_eq!(
        snapshot(dir),
        before,
        "dry run changed files\n{}",
        output(&out)
    );
    out
}

// ── tools ─────────────────────────────────────────────────────────

fn setup_tools_project(dir: &Path) {
    for (file, name) in [
        ("tool_configs/lookup.json", "lookup"),
        ("tool_configs/notify.json", "notify"),
    ] {
        write(
            dir,
            file,
            &json!({"name": name, "type": "webhook"}).to_string(),
        );
    }
    write(
        dir,
        "tools.json",
        &json!({"tools": [
            {"type": "webhook", "config": "tool_configs/lookup.json", "id": "tool_1"},
            {"type": "webhook", "config": "tool_configs/notify.json", "id": "tool_2"}
        ]})
        .to_string(),
    );
}

#[tokio::test]
async fn tools_delete_all_dry_run_deletes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    setup_tools_project(dir.path());
    let out = assert_dry_run_is_inert(
        dir.path(),
        &["tools", "delete", "--all", "--yes", "--dry-run"],
    )
    .await;
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tool_1") && stdout.contains("tool_2"),
        "{stdout}"
    );
}

#[tokio::test]
async fn tools_delete_all_dry_run_skips_the_prompt() {
    // Without --yes a dry run must not ask for (or act on) confirmation.
    let dir = tempfile::tempdir().unwrap();
    setup_tools_project(dir.path());
    assert_dry_run_is_inert(dir.path(), &["tools", "delete", "--all", "--dry-run"]).await;
}

#[tokio::test]
async fn tools_delete_one_dry_run_deletes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    setup_tools_project(dir.path());
    assert_dry_run_is_inert(dir.path(), &["tools", "delete", "tool_1", "--dry-run"]).await;
}

#[tokio::test]
async fn tools_add_dry_run_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    setup_tools_project(dir.path());
    assert_dry_run_is_inert(dir.path(), &["tools", "add", "weather", "--dry-run"]).await;
}

#[tokio::test]
async fn tools_add_dry_run_does_not_create_tools_json() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "keep.txt", "x");
    assert_dry_run_is_inert(dir.path(), &["tools", "add", "weather", "--dry-run"]).await;
    assert!(!dir.path().join("tools.json").exists());
}

#[tokio::test]
async fn tools_delete_all_without_dry_run_still_deletes() {
    // Control: the same command without --dry-run reaches the API and
    // empties the registry, so the tests above are not passing vacuously.
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/convai/tools/tool_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/convai/tools/tool_2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    setup_tools_project(dir.path());

    let out = cli(
        dir.path(),
        &server.uri(),
        &["tools", "delete", "--all", "--yes"],
    );
    assert!(out.status.success(), "{}", output(&out));
    let deletes = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "DELETE")
        .count();
    assert_eq!(deletes, 2, "{}", output(&out));
    let tools: Value = serde_json::from_str(&read(dir.path(), "tools.json")).unwrap();
    assert_eq!(tools["tools"], json!([]));
    assert!(!dir.path().join("tool_configs/lookup.json").exists());
}

// ── tests ─────────────────────────────────────────────────────────

fn setup_tests_project(dir: &Path) {
    write(
        dir,
        "test_configs/greeting.json",
        &json!({"name": "greeting check", "type": "llm"}).to_string(),
    );
    write(
        dir,
        "tests.json",
        &json!({"tests": [{"config": "test_configs/greeting.json", "id": "test_1"}]}).to_string(),
    );
}

#[tokio::test]
async fn tests_delete_all_dry_run_deletes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    setup_tests_project(dir.path());
    assert_dry_run_is_inert(
        dir.path(),
        &["tests", "delete", "--all", "--yes", "--dry-run"],
    )
    .await;
}

#[tokio::test]
async fn tests_delete_one_dry_run_deletes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    setup_tests_project(dir.path());
    assert_dry_run_is_inert(dir.path(), &["tests", "delete", "test_1", "--dry-run"]).await;
}

#[tokio::test]
async fn tests_add_dry_run_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    setup_tests_project(dir.path());
    assert_dry_run_is_inert(dir.path(), &["tests", "add", "farewell", "--dry-run"]).await;
}

// ── agents ────────────────────────────────────────────────────────

fn setup_agents_project(dir: &Path) {
    write(
        dir,
        "agent_configs/support.json",
        &json!({
            "name": "Support",
            "platform_settings": {"testing": {"attached_tests": [{"test_id": "test_1"}]}}
        })
        .to_string(),
    );
    write(
        dir,
        "agents.json",
        &json!({"agents": [{"config": "agent_configs/support.json", "id": "agent_1"}]}).to_string(),
    );
}

#[tokio::test]
async fn agents_add_dry_run_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    setup_agents_project(dir.path());
    assert_dry_run_is_inert(dir.path(), &["agents", "add", "Billing", "--dry-run"]).await;
}

#[tokio::test]
async fn agents_test_dry_run_runs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    setup_agents_project(dir.path());
    assert_dry_run_is_inert(dir.path(), &["agents", "test", "agent_1", "--dry-run"]).await;
}

#[tokio::test]
async fn agents_init_override_dry_run_deletes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    setup_agents_project(dir.path());
    assert_dry_run_is_inert(
        dir.path(),
        &["agents", "init", "--override", "--yes", "--dry-run"],
    )
    .await;
}

// ── components ────────────────────────────────────────────────────

#[tokio::test]
async fn components_add_dry_run_does_not_run_npx() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "package.json", "{}");
    let out = assert_dry_run_is_inert(dir.path(), &["components", "add", "orb", "--dry-run"]).await;
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("https://ui.elevenlabs.io/r/orb.json"),
        "{stdout}"
    );
}
