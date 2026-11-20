//! `agents status` and `agents test` must read the config paths named in a
//! committed `agents.json` with the same containment and symlink checks as the
//! other agent commands, and must not echo the target file's contents.

use std::path::Path;
use std::process::Output;

const KEY: &str = "sk_0123456789abcdef0123456789abcdef01234567";
const SECRET: &str = "LEAKED_SECRET_VALUE=do-not-print";

fn cli(dir: &Path, args: &[&str]) -> Output {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_elevenlabs"));
    // Nothing listens on port 9: these commands must fail before any request.
    cmd.args(["--base-url", "http://127.0.0.1:9"])
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

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// A project dir plus a sibling "outside" dir holding a file that is not
/// valid TOML, so a TOML parse error would quote it.
struct Fixture {
    _tmp: tempfile::TempDir,
    project: std::path::PathBuf,
    outside_file: std::path::PathBuf,
}

fn fixture(config: &str) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("project");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(project.join("agent_configs")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let outside_file = outside.join("secrets.env");
    std::fs::write(
        &outside_file,
        format!("{SECRET}\nname = \"Outside\"\n[platform_settings.testing]\nattached_tests = [{{test_id = \"t\"}}]\n"),
    )
    .unwrap();
    let index = serde_json::json!({
        "agents": [{"config": config, "id": "agent_1"}]
    });
    std::fs::write(project.join("agents.json"), index.to_string()).unwrap();
    Fixture {
        _tmp: tmp,
        project,
        outside_file,
    }
}

#[cfg(unix)]
fn symlinked_fixture() -> Fixture {
    let f = fixture("agent_configs/a.toml");
    std::os::unix::fs::symlink(&f.outside_file, f.project.join("agent_configs/a.toml")).unwrap();
    f
}

#[cfg(unix)]
#[test]
fn status_refuses_a_symlinked_config() {
    let f = symlinked_fixture();
    let out = cli(&f.project, &["agents", "status"]);
    let text = text(&out);
    assert!(text.contains("Refusing to read through the symlink"), "{text}");
    assert!(!text.contains(SECRET), "leaked target contents:\n{text}");
}

#[test]
fn status_refuses_a_config_outside_the_project() {
    let f = fixture("../outside/secrets.env.toml");
    std::fs::copy(&f.outside_file, f.project.join("../outside/secrets.env.toml")).unwrap();
    let out = cli(&f.project, &["agents", "status"]);
    let text = text(&out);
    assert!(text.contains("resolves outside the project directory"), "{text}");
    assert!(!text.contains(SECRET), "leaked target contents:\n{text}");
}

#[cfg(unix)]
#[test]
fn test_refuses_a_symlinked_config() {
    let f = symlinked_fixture();
    let out = cli(&f.project, &["agents", "test", "agent_1"]);
    let text = text(&out);
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("Refusing to read through the symlink"), "{text}");
    assert!(!text.contains(SECRET), "leaked target contents:\n{text}");
    assert!(!text.contains("Outside"), "leaked target name:\n{text}");
}

#[test]
fn test_refuses_a_config_outside_the_project() {
    let f = fixture("../outside/secrets.env.toml");
    std::fs::copy(&f.outside_file, f.project.join("../outside/secrets.env.toml")).unwrap();
    let out = cli(&f.project, &["agents", "test", "agent_1"]);
    let text = text(&out);
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("resolves outside the project directory"), "{text}");
    assert!(!text.contains(SECRET), "leaked target contents:\n{text}");
}

#[cfg(unix)]
#[test]
fn status_refuses_a_dangling_symlink() {
    let f = fixture("agent_configs/a.toml");
    std::os::unix::fs::symlink(
        f.project.join("../outside/missing.toml"),
        f.project.join("agent_configs/a.toml"),
    )
    .unwrap();
    let out = cli(&f.project, &["agents", "status"]);
    let text = text(&out);
    assert!(text.contains("Refusing to read through the symlink"), "{text}");
}

#[test]
fn status_still_reports_regular_configs() {
    let f = fixture("agent_configs/a.toml");
    std::fs::write(f.project.join("agent_configs/a.toml"), "name = \"Support\"\n").unwrap();
    let out = cli(&f.project, &["agents", "status"]);
    let text = text(&out);
    assert!(out.status.success(), "{text}");
    assert!(text.contains("Status: Created (use push to update)"), "{text}");
}

#[test]
fn status_reports_a_missing_config() {
    let f = fixture("agent_configs/missing.toml");
    let out = cli(&f.project, &["agents", "status"]);
    let text = text(&out);
    assert!(text.contains("Status: Config file not found"), "{text}");
}

#[test]
fn test_reports_a_missing_config() {
    let f = fixture("agent_configs/missing.toml");
    let out = cli(&f.project, &["agents", "test", "agent_1"]);
    let text = text(&out);
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("Config file not found for agent"), "{text}");
}
