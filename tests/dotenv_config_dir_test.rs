//! A `.env` file must not be able to move the config directory.
//!
//! `profiles.toml` is read from the config directory, an active profile
//! outranks env vars, and a profile can set `base_url` and the transport
//! knobs (`proxy`, `insecure`, `ca_bundle`). So a repository whose `.env`
//! points `XDG_CONFIG_HOME` (or `APPDATA`, `HOME`, …) at a planted profile
//! would otherwise send the developer's API key to a host of its choosing,
//! in any CLI that opts into profiles.
//!
//! Its own test binary: it changes the process's working directory and
//! environment, which would race with any other test in the same process.

use std::path::Path;

const CLI: &str = "dotenv-test-cli";
const PLANTED_URL: &str = "https://evil.example";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn dotenv_cannot_load_a_planted_profile_by_moving_the_config_dir() {
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    let planted_cfg = repo.path().join(".cfg");
    let planted_home = repo.path().join("h");
    let profile = format!(
        "version = 1\nactive = \"x\"\n\n[profiles.x]\nbase_url = \"{PLANTED_URL}\"\n\n\
         [profiles.x.transport]\ninsecure = true\n"
    );
    // Wherever the platform would look: XDG_CONFIG_HOME / APPDATA, and
    // HOME / USERPROFILE (`.config`, `Library/Application Support`,
    // `AppData/Roaming`).
    for dir in [
        planted_cfg.clone(),
        planted_home.join(".config"),
        planted_home.join("Library").join("Application Support"),
        planted_home.join("AppData").join("Roaming"),
    ] {
        write(&dir.join(CLI).join("profiles.toml"), &profile);
    }
    write(
        &repo.path().join(".env"),
        &format!(
            "DOTENV_TEST_CLI_API_KEY=allowed\n\
             XDG_CONFIG_HOME={cfg}\nAPPDATA={cfg}\nLOCALAPPDATA={cfg}\n\
             HOME={h}\nUSERPROFILE={h}\n",
            cfg = planted_cfg.display(),
            h = planted_home.display(),
        ),
    );

    // The developer's real environment: a home directory, and none of the
    // overrides set — the case in which `.env` would otherwise fill them in.
    std::env::set_var("HOME", home.path());
    std::env::set_var("USERPROFILE", home.path());
    for var in [
        "XDG_CONFIG_HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "DOTENV_TEST_CLI_PROFILE",
    ] {
        std::env::remove_var(var);
    }
    std::env::set_current_dir(repo.path()).unwrap();

    let ignored = fern_cli_sdk::load_dotenv_filtered(CLI);

    for var in [
        "XDG_CONFIG_HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "HOME",
        "USERPROFILE",
    ] {
        assert!(
            ignored.iter().any(|k| k == var),
            "{var} not ignored: {ignored:?}"
        );
    }
    assert_eq!(std::env::var_os("XDG_CONFIG_HOME"), None);
    assert_eq!(std::env::var_os("APPDATA"), None);
    assert_eq!(
        std::env::var_os("HOME").as_deref(),
        Some(home.path().as_os_str())
    );
    // Credentials are still loaded from `.env`.
    assert_eq!(
        std::env::var("DOTENV_TEST_CLI_API_KEY").as_deref(),
        Ok("allowed")
    );

    let store = fern_cli_sdk::profiles::ProfileStore::for_cli(CLI).expect("home is set");
    assert!(
        !store.path().starts_with(repo.path()),
        "profiles.toml read from the repository: {}",
        store.path().display()
    );
    let selection = fern_cli_sdk::profiles::resolve_selection(CLI, &[CLI.to_string()]).unwrap();
    assert!(
        selection.is_none(),
        "planted profile selected: {selection:?}"
    );
}
