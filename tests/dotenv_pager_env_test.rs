//! A `.env` in an untrusted checkout must not be able to make the pager run
//! commands. `less` (the default pager) runs `LESSOPEN`/`LESSCLOSE` as shell
//! commands, `|-cmd` even when it reads a pipe.
#![cfg(unix)]

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

use fern_cli_sdk::pager::{pager_command, PagerConfig};

const DENIED: &[&str] = &[
    "LESSOPEN",
    "LESSCLOSE",
    "LESSKEYIN",
    "LESSEDIT",
    "LESS",
    "EDITOR",
];
const ALLOWED: &str = "ELEVENLABS_DOTENV_PAGER_TEST";

fn run_pager(program: &str, capture_stdout: bool) -> String {
    let config = PagerConfig {
        program: program.to_string(),
        disabled: false,
    };
    let mut cmd = pager_command(&config, "dotenv-test").expect("pager command");
    if capture_stdout {
        cmd.stdout(Stdio::piped());
    }
    let mut child = cmd.spawn().expect("spawn pager");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"hello from the cli\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

// One test function: it changes the working directory and process
// environment, which every test in this binary would otherwise share.
#[test]
fn dotenv_cannot_make_the_pager_run_commands() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("dotenv-command-ran");
    let env_dump = dir.path().join("pager-env");
    let fake_pager = dir.path().join("fake-pager");
    std::fs::write(
        &fake_pager,
        format!(
            "#!/bin/sh\nenv > '{}'\ncat > /dev/null\n",
            env_dump.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake_pager, std::fs::Permissions::from_mode(0o755)).unwrap();
    let m = marker.display();
    std::fs::write(
        dir.path().join(".env"),
        format!(
            "LESSOPEN=\"|-touch {m} %s\"\n\
             LESSCLOSE=\"touch {m} %s %s\"\n\
             LESSKEYIN=/dev/null\n\
             LESSEDIT=\"touch {m}\"\n\
             LESS=\"+!touch {m}\"\n\
             EDITOR=\"touch {m}\"\n\
             {ALLOWED}=from-dotenv\n"
        ),
    )
    .unwrap();

    for key in DENIED.iter().chain([&ALLOWED]) {
        std::env::remove_var(key);
    }
    std::env::set_current_dir(dir.path()).unwrap();

    let ignored = fern_cli_sdk::load_dotenv_filtered("elevenlabs");
    for key in DENIED {
        assert!(
            ignored.iter().any(|k| k == key),
            "{key} should be ignored, got {ignored:?}"
        );
        assert!(
            std::env::var_os(key).is_none(),
            "{key} leaked into the environment"
        );
    }
    assert_eq!(std::env::var(ALLOWED).as_deref(), Ok("from-dotenv"));
    assert!(fern_cli_sdk::dotenv_loaded_keys()
        .iter()
        .any(|k| k == ALLOWED));

    // The pager child gets nothing that came from `.env`.
    run_pager(fake_pager.to_str().unwrap(), false);
    let env = std::fs::read_to_string(&env_dump).unwrap();
    for key in DENIED.iter().filter(|k| **k != "LESS").chain([&ALLOWED]) {
        assert!(
            !env.lines().any(|l| l.starts_with(&format!("{key}="))),
            "{key} reached the pager:\n{env}"
        );
    }
    assert!(
        env.lines().any(|l| l == "LESS=FRX"),
        "LESS default missing:\n{env}"
    );

    // And the real default pager runs no command.
    if Command::new("less")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        let shown = run_pager("less", true);
        assert!(
            shown.contains("hello from the cli"),
            "less output: {shown:?}"
        );
    }
    assert!(!marker.exists(), "a command from .env ran");
}
