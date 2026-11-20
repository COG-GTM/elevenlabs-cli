//! A project's `.env` must not configure the programs the CLI starts.
//!
//! The CLI loads the nearest `.env` into its own environment, and it is
//! routinely run inside repositories someone else wrote. Loader and pager
//! variables (`LD_PRELOAD`, `LESSOPEN`, `NODE_OPTIONS`) would otherwise reach
//! the audio player, pager, browser opener and `npx`, and a `.env` `HOME`
//! would pick the settings file that names the player.

#![cfg(unix)]

use std::path::Path;
use std::process::{Command, Output};

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const AUDIO: &[u8] = b"RIFF\x24\x00\x00\x00WAVEfmt ELEVENLABS-DOTENV-TEST";
const DEFAULT_VOICE: &str = "JBFqnCBsd6RMkjVDRZzb";
const ATTACKER_VOICE: &str = "attackerVoiceFromDotenvHome";

fn cli(cwd: &Path, home: Option<&Path>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_elevenlabs"));
    cmd.args(args)
        .current_dir(cwd)
        .env("ELEVENLABS_API_KEY", "test-key")
        .env("NO_COLOR", "1")
        // Surfaces the warning for an ignored `.env` key on stderr.
        .env("ELEVENLABS_LOG", "warn")
        .env("FERN_CLI_CREDENTIAL_STORE", "file")
        .env_remove("ELEVENLABS_BASE_URL")
        .env_remove("ELEVENLABS_VIA");
    for key in [
        "LD_PRELOAD",
        "LESSOPEN",
        "LESS",
        "NODE_OPTIONS",
        "XDG_CONFIG_HOME",
        "SOME_PROJECT_SETTING",
    ] {
        cmd.env_remove(key);
    }
    match home {
        Some(home) => cmd.env("HOME", home),
        None => cmd.env_remove("HOME"),
    };
    cmd.output().expect("failed to spawn the elevenlabs binary")
}

/// A player that records the environment it was started with.
fn env_dumping_player(dir: &Path, dump: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("env-player");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ncat > /dev/null 2>&1\nenv > '{}'\n",
            dump.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

async fn tts_server(voice: &str) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/text-to-speech/{voice}/stream")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(AUDIO))
        .mount(&server)
        .await;
    server
}

#[tokio::test(flavor = "multi_thread")]
async fn dotenv_loader_and_pager_variables_never_reach_the_player() {
    let server = tts_server(DEFAULT_VOICE).await;
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let dump = home.path().join("player-env.txt");
    let player = env_dumping_player(home.path(), &dump);
    let evil = repo.path().join("evil.so");
    std::fs::write(
        repo.path().join(".env"),
        format!(
            "LD_PRELOAD={evil}\nLESSOPEN=|{evil} %s\nLESS=-+X\nNODE_OPTIONS=--require={evil}\n\
             XDG_CONFIG_HOME={repo}\nSOME_PROJECT_SETTING=from-dotenv\n",
            evil = evil.display(),
            repo = repo.path().display(),
        ),
    )
    .unwrap();

    let out = cli(
        repo.path(),
        Some(home.path()),
        &[
            "say",
            "hello",
            "--base-url",
            &server.uri(),
            "--player",
            player.to_str().unwrap(),
        ],
    );
    assert!(
        out.status.success(),
        "say should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let env = std::fs::read_to_string(&dump).expect("the player ran and dumped its env");
    for key in [
        "LD_PRELOAD=",
        "LESSOPEN=",
        "LESS=",
        "NODE_OPTIONS=",
        "XDG_CONFIG_HOME=",
        "SOME_PROJECT_SETTING=",
    ] {
        assert!(
            !env.lines().any(|l| l.starts_with(key)),
            "{key} from .env reached the player:\n{env}"
        );
    }
    assert!(
        env.lines().any(|l| l == "ELEVENLABS_API_KEY=test-key"),
        "the shell's own environment must still reach the player"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dotenv_home_cannot_choose_the_settings_file() {
    // The request only matches the default voice; a voice read from the
    // `.env`-chosen settings file would 404.
    let server = tts_server(DEFAULT_VOICE).await;
    let repo = tempfile::tempdir().unwrap();
    let fake_home = repo.path().join("fakehome");
    std::fs::create_dir_all(fake_home.join(".elevenlabs")).unwrap();
    std::fs::write(
        fake_home.join(".elevenlabs/config.json"),
        format!(r#"{{"say":{{"voice_id":"{ATTACKER_VOICE}","player":"/bin/false"}}}}"#),
    )
    .unwrap();
    std::fs::write(
        repo.path().join(".env"),
        format!("HOME={}\n", fake_home.display()),
    )
    .unwrap();
    let out_file = repo.path().join("out.wav");

    let out = cli(
        repo.path(),
        None,
        &[
            "say",
            "hello",
            "--base-url",
            &server.uri(),
            "--output",
            out_file.to_str().unwrap(),
        ],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "say should use the default voice, not one from a .env-chosen HOME: {stderr}"
    );
    assert!(stderr.contains("ignoring HOME from .env"), "{stderr}");
}
