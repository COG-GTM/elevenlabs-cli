//! A hostile id in `agents.json` / `tools.json` / `tests.json` must never pick
//! the endpoint of an authenticated request: each command below has to fail
//! before anything reaches the server.

use std::path::Path;
use std::process::Output;

use serde_json::{json, Value};
use wiremock::{MockServer, Request};

const KEY: &str = "sk_0123456789abcdef0123456789abcdef01234567";

const HOSTILE_IDS: &[&str] = &[
    "../agents/agent_prod",
    "%2e%2e/agents/agent_prod",
    "../outbound-campaigns/config#",
    "x?force=true",
    "a/b",
];

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

fn write(dir: &Path, file: &str, value: &Value) {
    let p = dir.join(file);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, value.to_string()).unwrap();
}

fn describe(out: &Output, requests: &[Request]) -> String {
    let sent: Vec<String> = requests
        .iter()
        .map(|r| format!("{} {}", r.method, r.url))
        .collect();
    format!(
        "requests: {sent:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Run `args` in a project prepared by `setup` and assert no request was sent.
async fn assert_no_request(
    id: &str,
    setup: impl Fn(&Path, &str),
    args: &[&str],
    expect_failure: bool,
) {
    let server = MockServer::start().await;
    let dir = tempfile::tempdir().unwrap();
    setup(dir.path(), id);

    let out = cli(dir.path(), &server.uri(), args);
    let requests = server.received_requests().await.unwrap();
    let detail = describe(&out, &requests);
    assert!(
        requests.is_empty(),
        "{args:?} with id {id:?} reached the server\n{detail}"
    );
    if expect_failure {
        assert!(
            !out.status.success(),
            "{args:?} with id {id:?} exited 0\n{detail}"
        );
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("ids may only contain"),
        "{args:?} with id {id:?}\n{detail}"
    );
}

fn agents_project(dir: &Path, id: &str) {
    write(
        dir,
        "agent_configs/support.json",
        &json!({
            "name": "Support",
            "conversation_config": {"agent": {"first_message": "Hello"}},
            "platform_settings": {"testing": {"attached_tests": [{"test_id": "test_1"}]}}
        }),
    );
    write(
        dir,
        "agents.json",
        &json!({"agents": [{"config": "agent_configs/support.json", "id": id}]}),
    );
}

fn tools_project(dir: &Path, id: &str) {
    write(
        dir,
        "tool_configs/lookup.json",
        &json!({
            "name": "lookup",
            "type": "webhook",
            "api_schema": {"url": "https://api.example.com/webhook", "method": "POST"}
        }),
    );
    write(
        dir,
        "tools.json",
        &json!({"tools": [{"type": "webhook", "config": "tool_configs/lookup.json", "id": id}]}),
    );
}

fn tests_project(dir: &Path, id: &str) {
    write(
        dir,
        "test_configs/greeting.json",
        &json!({"name": "greeting check", "success_condition": "agent greets the user"}),
    );
    write(
        dir,
        "tests.json",
        &json!({"tests": [{"config": "test_configs/greeting.json", "id": id}]}),
    );
}

#[tokio::test]
async fn agents_push_rejects_hostile_ids() {
    for id in HOSTILE_IDS {
        assert_no_request(id, agents_project, &["agents", "push"], true).await;
    }
}

#[tokio::test]
async fn agents_test_rejects_hostile_ids() {
    for id in HOSTILE_IDS {
        assert_no_request(id, agents_project, &["agents", "test", "Support"], true).await;
    }
}

#[tokio::test]
async fn tools_push_rejects_hostile_ids() {
    for id in HOSTILE_IDS {
        assert_no_request(id, tools_project, &["tools", "push"], true).await;
    }
}

#[tokio::test]
async fn tools_delete_all_rejects_hostile_ids() {
    for id in HOSTILE_IDS {
        assert_no_request(
            id,
            tools_project,
            &["tools", "delete", "--all", "--yes"],
            false,
        )
        .await;
    }
}

#[tokio::test]
async fn tests_push_rejects_hostile_ids() {
    for id in HOSTILE_IDS {
        assert_no_request(id, tests_project, &["tests", "push"], true).await;
    }
}

#[tokio::test]
async fn tests_delete_all_rejects_hostile_ids() {
    for id in HOSTILE_IDS {
        assert_no_request(
            id,
            tests_project,
            &["tests", "delete", "--all", "--yes"],
            false,
        )
        .await;
    }
}
