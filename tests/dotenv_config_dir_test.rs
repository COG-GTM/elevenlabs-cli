//! A project's `.env` must not be able to move the config directory. If it
//! could, a planted `profiles.toml` with an active profile would set
//! `base_url` (which outranks `ELEVENLABS_BASE_URL`) and the operator's API
//! key would be sent to whatever host that profile names.
#![cfg(unix)]

use std::path::Path;
use std::process::Output;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk_0123456789abcdef0123456789abcdef01234567";

fn write(file: &Path, text: &str) {
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

/// A `profiles.toml` whose active profile points at `base_url`.
fn plant_profile(config_dir: &Path, base_url: &str) {
    write(
        &config_dir.join("elevenlabs").join("profiles.toml"),
        &format!("version = 1\nactive = \"x\"\n\n[profiles.x]\nbase_url = \"{base_url}\"\n"),
    );
}

/// Run `elevenlabs user get` in `project` with the operator's key and base
/// URL exported in the real environment. `home` of `None` leaves `HOME`
/// unset, so only a `.env` could supply it.
fn run_user_get(project: &Path, home: Option<&Path>, tmp: &Path, base_url: &str) -> Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_elevenlabs"));
    cmd.args(["user", "get"])
        .current_dir(project)
        .env("TMPDIR", tmp)
        .env("NO_COLOR", "1")
        .env("FERN_CLI_CREDENTIAL_STORE", "file")
        .env("ELEVENLABS_API_KEY", KEY)
        .env("ELEVENLABS_BASE_URL", base_url)
        .env("ELEVENLABS_TELEMETRY", "0");
    for var in [
        "XDG_CONFIG_HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "USERPROFILE",
        "ELEVENLABS_PROFILE",
        "ELEVENLABS_VIA",
        "ELEVENLABS_OUTPUT",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        cmd.env_remove(var);
    }
    match home {
        Some(h) => cmd.env("HOME", h),
        None => cmd.env_remove("HOME"),
    };
    cmd.output().expect("failed to spawn the elevenlabs binary")
}

async fn servers() -> (MockServer, MockServer) {
    let legit = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&legit)
        .await;
    let evil = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&evil)
        .await;
    (legit, evil)
}

async fn assert_key_stayed_home(out: &Output, legit: &MockServer, evil: &MockServer) {
    let diag = format!(
        "status: {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let to_evil = evil.received_requests().await.unwrap_or_default();
    assert!(
        to_evil.is_empty(),
        "the planted profile's base_url received {} request(s):\n{diag}",
        to_evil.len()
    );
    let to_legit = legit.received_requests().await.unwrap_or_default();
    assert_eq!(
        to_legit.len(),
        1,
        "expected one request to ELEVENLABS_BASE_URL:\n{diag}"
    );
    assert!(
        to_legit[0]
            .headers
            .values()
            .any(|v| v.to_str().is_ok_and(|v| v.contains(KEY))),
        "the request to ELEVENLABS_BASE_URL should carry the key:\n{diag}"
    );
}

#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn dotenv_xdg_config_home_cannot_load_a_planted_profile() {
    let (legit, evil) = servers().await;
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let project = root.path().join("repo");
    std::fs::create_dir_all(&home).unwrap();
    plant_profile(&project.join(".cfg"), &evil.uri());
    write(
        &project.join(".env"),
        &format!("XDG_CONFIG_HOME={}\n", project.join(".cfg").display()),
    );

    let out = run_user_get(&project, Some(&home), root.path(), &legit.uri());
    assert_key_stayed_home(&out, &legit, &evil).await;
}

#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn dotenv_relative_xdg_config_home_cannot_load_a_planted_profile() {
    let (legit, evil) = servers().await;
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let project = root.path().join("repo");
    std::fs::create_dir_all(&home).unwrap();
    plant_profile(&project.join(".cfg"), &evil.uri());
    write(&project.join(".env"), "XDG_CONFIG_HOME=./.cfg\n");

    let out = run_user_get(&project, Some(&home), root.path(), &legit.uri());
    assert_key_stayed_home(&out, &legit, &evil).await;
}

#[tokio::test]
async fn dotenv_home_cannot_load_a_planted_profile() {
    let (legit, evil) = servers().await;
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("repo");
    let planted_home = project.join("h");
    #[cfg(target_os = "macos")]
    let planted_config = planted_home.join("Library").join("Application Support");
    #[cfg(not(target_os = "macos"))]
    let planted_config = planted_home.join(".config");
    plant_profile(&planted_config, &evil.uri());
    write(
        &project.join(".env"),
        &format!("HOME={}\n", planted_home.display()),
    );

    let out = run_user_get(&project, None, root.path(), &legit.uri());
    assert_key_stayed_home(&out, &legit, &evil).await;
}
