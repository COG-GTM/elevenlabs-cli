//! `agents init` must not write `.gitignore` or `.env.example` through a
//! symlink planted in the project: the write would land on the link's target,
//! outside the project, and create it when the link dangles.
#![cfg(unix)]

use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Output;

fn init(dir: &Path, args: &[&str]) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_elevenlabs"))
        .args(["agents", "init"])
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .env("TMPDIR", dir)
        .env_remove("XDG_CONFIG_HOME")
        .env("NO_COLOR", "1")
        .env("ELEVENLABS_TELEMETRY", "0")
        .output()
        .expect("failed to spawn the elevenlabs binary")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

const OUTSIDE_BODY: &str = "ssh-ed25519 AAAA keep-me\n";

/// A project whose `.gitignore` links to an existing file outside it and whose
/// `.env.example` links to a path outside it that does not exist yet.
fn hostile_project() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let victim = outside.join("authorized_keys");
    std::fs::write(&victim, OUTSIDE_BODY).unwrap();
    let missing = outside.join("created_by_init");
    let project = tmp.path().join("project");
    std::fs::create_dir(&project).unwrap();
    symlink(&victim, project.join(".gitignore")).unwrap();
    symlink(&missing, project.join(".env.example")).unwrap();
    (tmp, project, victim, missing)
}

#[test]
fn init_override_refuses_to_write_through_symlinks() {
    let (_tmp, project, victim, missing) = hostile_project();

    let out = init(&project, &["--override", "--yes"]);
    let output = text(&out);

    assert!(!out.status.success(), "init should fail:\n{output}");
    assert!(
        output.contains("Refusing to write through the symlink"),
        "{output}"
    );
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), OUTSIDE_BODY);
    assert!(
        !missing.exists(),
        "init created the dangling symlink's target"
    );

    // Remove the .gitignore link: the .env.example link must be refused too.
    std::fs::remove_file(project.join(".gitignore")).unwrap();
    let out = init(&project, &["--override", "--yes"]);
    let output = text(&out);
    assert!(!out.status.success(), "init should fail:\n{output}");
    assert!(
        output.contains("Refusing to write through the symlink"),
        "{output}"
    );
    assert!(
        !missing.exists(),
        "init created the dangling symlink's target"
    );
}

#[test]
fn init_without_override_skips_symlinks_including_dangling_ones() {
    let (_tmp, project, victim, missing) = hostile_project();

    let out = init(&project, &[]);
    let output = text(&out);

    assert!(out.status.success(), "init failed:\n{output}");
    assert!(
        output.contains(".gitignore already exists (skipped)"),
        "{output}"
    );
    assert!(
        output.contains(".env.example already exists (skipped)"),
        "{output}"
    );
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), OUTSIDE_BODY);
    assert!(
        !missing.exists(),
        "init created the dangling symlink's target"
    );
}

#[test]
fn init_still_writes_regular_files() {
    let tmp = tempfile::tempdir().unwrap();
    let out = init(tmp.path(), &[]);
    assert!(out.status.success(), "init failed:\n{}", text(&out));
    let gitignore = std::fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
    assert!(gitignore.contains(".env"), "{gitignore}");
    let env = std::fs::read_to_string(tmp.path().join(".env.example")).unwrap();
    assert!(env.contains("ELEVENLABS_API_KEY="), "{env}");

    // --override rewrites a regular file in place.
    std::fs::write(tmp.path().join(".env.example"), "stale\n").unwrap();
    let out = init(tmp.path(), &["--override", "--yes"]);
    assert!(
        out.status.success(),
        "init --override failed:\n{}",
        text(&out)
    );
    let env = std::fs::read_to_string(tmp.path().join(".env.example")).unwrap();
    assert!(env.contains("ELEVENLABS_API_KEY="), "{env}");
}
