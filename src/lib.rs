//! Fern CLI SDK
//!
//! A library for building CLIs from OpenAPI or GraphQL SDL schemas.
//! Uses `x-fern-sdk-group-name` and `x-fern-sdk-method-name` extensions
//! to build the command hierarchy.

// Public API — building blocks
pub mod app;
pub mod arg_source;
pub mod asyncapi;
pub mod auth;
pub mod binding;
pub mod cli_args;
pub mod completions;
pub(crate) mod custom_commands;
pub mod http;
pub mod error;
pub mod formatter;
pub mod graphql;
pub mod hooks;
pub mod man;
pub mod openapi;
pub mod pager;
pub mod profiles;
pub mod stability;
pub mod user_agent;
pub mod validate;
pub mod sdk_executor;
pub mod websocket;

// Convenience re-exports for auth types
pub use auth::{ApiKeyAuth, BasicAuth, BearerAuth, OAuth2Auth, OAuth2Grant, OAuth2TokenProvider, TokenCache};

// Re-exported for the generated wire-test harness so it derives multipart
// field flag names (`--<kebab>`) with the exact same rule the CLI registers
// them under, rather than reproducing the kebab-casing logic and risking drift.
pub use text::to_kebab_flag;

// Internal modules
pub(crate) mod debug;
pub(crate) mod early_intercept;
pub(crate) mod logging;
pub(crate) mod output;
pub(crate) mod text;

/// Initialize logging from environment variables. Call once at startup.
///
/// `cli_name` is the binary name (e.g. `"my-cli"`). The function reads
/// `<PREFIX>_LOG` and `<PREFIX>_LOG_FILE` where `<PREFIX>` is
/// `cli_name` uppercased with hyphens replaced by underscores.
pub fn init_logging(cli_name: &str) {
    logging::init_logging(cli_name);
}

/// Reset the `SIGPIPE` signal handler to its default disposition (`SIG_DFL`).
///
/// Rust's runtime sets `SIGPIPE` to `SIG_IGN`, which causes writes to a
/// broken pipe (e.g. `<cli> completion bash | head -5`) to return
/// `EPIPE` errors instead of terminating the process. For CLI tools that
/// produce large output this surfaces as a panic in `println!` or
/// `write_all`. Resetting to `SIG_DFL` lets the OS deliver the signal
/// and terminate the process cleanly — the standard behavior expected by
/// Unix pipelines.
///
/// This is the idiomatic fix used by `bat`, `ripgrep`, `fd`, `eza`, and
/// most other Rust CLI tools. Called at the very top of each binary's
/// `run()` method before any I/O.
///
/// On non-Unix platforms this is a no-op.
#[cfg(unix)]
pub fn reset_sigpipe() {
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

/// No-op on non-Unix platforms.
#[cfg(not(unix))]
pub fn reset_sigpipe() {}

/// Unscoped env vars a `.env` file may set. Everything else outside the CLI's
/// own `<PREFIX>_…` namespace is dropped: the CLI's process environment is
/// inherited by the programs it starts (the pager, the browser opener), and
/// far too many unscoped names make those programs run code — `LESSOPEN`,
/// `LESSKEY*`, `EDITOR`, `LD_PRELOAD`, `DYLD_*`, `XDG_*` — for a deny list to
/// stay complete. These two only switch off behavior.
const DOTENV_ALLOWED_BARE: &[&str] = &["NO_COLOR", "DO_NOT_TRACK"];

/// Unscoped env vars that are worth a warning when a `.env` tries to set them,
/// because each one redirects traffic, weakens transport security, or makes a
/// program the CLI starts execute something. Other unscoped keys (an app's
/// `DATABASE_URL`, say) are dropped silently — they never configured this CLI.
/// Matched ignoring case (see [`dotenv_key_is_denied`]).
const DOTENV_DENIED_BARE: &[&str] = &[
    // Names a program the CLI executes when paging output.
    "PAGER",
    // Names the program that opens the sign-in page.
    "BROWSER",
    // Make the default pager (`less`) run commands or read attacker files.
    "LESS",
    "LESSOPEN",
    "LESSCLOSE",
    "LESSKEY",
    "LESSKEYIN",
    "LESSKEY_SRC",
    "LESSKEY_SYSTEM",
    "LESSKEYIN_SYSTEM",
    "LESSKEY_SRC_SYSTEM",
    "LESSEDIT",
    "LESSSECURE",
    "LESSSECURE_ALLOW",
    "LESSHISTFILE",
    "EDITOR",
    "VISUAL",
    "SHELL",
    "MORE",
    // Choose which binaries and libraries child processes load.
    "PATH",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FRAMEWORK_PATH",
    // Redirect where credentials are stored and which `.desktop` handler
    // `xdg-open` runs for the sign-in page.
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "XDG_CONFIG_HOME",
    "XDG_CONFIG_DIRS",
    "XDG_DATA_HOME",
    "XDG_DATA_DIRS",
    // Route traffic through an attacker-chosen intermediary.
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    // Replace the trust store, enabling transparent interception.
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "CURL_CA_BUNDLE",
    "REQUESTS_CA_BUNDLE",
];

/// Suffixes of `<PREFIX>_…` env vars a `.env` file may not set. Same
/// reasoning as [`DOTENV_DENIED_BARE`], plus the two cross-host guards — a
/// `.env` that could disable them would undo the redirect and pagination
/// protections.
const DOTENV_DENIED_SUFFIXES: &[&str] = &[
    "_BASE_URL",
    "_PAGER",
    "_PROXY",
    "_NO_PROXY",
    "_INSECURE",
    "_INSECURE_SKIP_VERIFY",
    "_CA_BUNDLE",
    "_EXTRA_CA_CERTS",
    "_ALLOW_CROSS_HOST_REDIRECTS",
    "_ALLOW_CROSS_HOST_PAGINATION",
    // Turns every invocation into a shell-completion request (see `completions.rs`).
    "_COMPLETE",
];

/// The part of `key` after `<PREFIX>_`, with the leading underscore kept, or
/// `None` when `key` is outside this CLI's namespace. `key` must already be
/// uppercase.
fn dotenv_scoped_suffix<'a>(key: &'a str, prefix: &str) -> Option<&'a str> {
    key.strip_prefix(prefix)
        .filter(|rest| rest.len() > 1 && rest.starts_with('_'))
}

/// True when `key` is one a `.env` file must not be able to set. A `.env` may
/// only set this CLI's own `<PREFIX>_…` variables (minus
/// [`DOTENV_DENIED_SUFFIXES`]) and [`DOTENV_ALLOWED_BARE`]. Names are compared
/// ignoring case: the HTTP stack reads lowercase proxy names, and Windows
/// treats environment names as case-insensitive.
pub(crate) fn dotenv_key_is_denied(key: &str, prefix: &str) -> bool {
    let key = key.to_ascii_uppercase();
    if DOTENV_ALLOWED_BARE.contains(&key.as_str()) {
        return false;
    }
    match dotenv_scoped_suffix(&key, prefix) {
        Some(rest) => DOTENV_DENIED_SUFFIXES.contains(&rest),
        None => true,
    }
}

/// True when a denied `key` should be reported: it is in this CLI's namespace
/// (so the user likely meant it for us) or it is a known-dangerous name.
fn dotenv_denial_is_reported(key: &str, prefix: &str) -> bool {
    let key = key.to_ascii_uppercase();
    DOTENV_DENIED_BARE.contains(&key.as_str())
        || key.starts_with("LD_")
        || key.starts_with("DYLD_")
        || dotenv_scoped_suffix(&key, prefix).is_some()
}

/// Load `.env`, keeping only keys that configure this CLI.
///
/// A generated CLI is routinely run inside repositories the operator did not
/// write, and `dotenvy::dotenv()` searches the working directory and its
/// ancestors — so an attacker-authored `.env` would otherwise be able to point
/// the CLI at another host, disable certificate verification, route requests
/// through a proxy, turn off the cross-host redirect and pagination guards, or
/// make a program the CLI starts run code of their choosing: `<PREFIX>_PAGER`
/// is run as a command, and `LESSOPEN` makes the default `less` pager run one.
///
/// The legitimate use of `.env` — this CLI's credentials and output
/// preferences under `<PREFIX>_…` — is unaffected. Real process environment
/// always wins, matching `dotenvy::dotenv()`'s own precedence, so anything
/// genuinely exported by the operator's shell still applies.
/// Returns the reportable keys that were ignored, in file order, so the caller
/// can report them *after* logging is initialized. Warning from inside this
/// function would be silently dropped: it runs before `init_logging`, so no
/// subscriber exists yet and a legitimate `<PREFIX>_BASE_URL` in `.env` would
/// appear to be ignored for no reason.
pub fn load_dotenv_filtered(cli_name: &str) -> Vec<String> {
    let Ok(entries) = dotenvy::dotenv_iter() else {
        // No `.env` (the common case) or it is unreadable — nothing to do.
        return Vec::new();
    };
    apply_dotenv_entries(entries.flatten(), cli_name)
}

/// Set the permitted `entries` in the process environment; see
/// [`load_dotenv_filtered`].
pub(crate) fn apply_dotenv_entries(
    entries: impl IntoIterator<Item = (String, String)>,
    cli_name: &str,
) -> Vec<String> {
    let prefix = cli_name.to_uppercase().replace('-', "_");
    let mut ignored = Vec::new();
    for (key, value) in entries {
        if dotenv_key_is_denied(&key, &prefix) {
            if dotenv_denial_is_reported(&key, &prefix) {
                ignored.push(key);
            }
            continue;
        }
        // Real environment wins, as with `dotenvy::dotenv()`.
        if std::env::var_os(&key).is_none() {
            std::env::set_var(&key, &value);
        }
    }
    ignored
}

/// Report the keys [`load_dotenv_filtered`] dropped. Call after
/// `init_logging`, or the messages go nowhere.
pub fn warn_ignored_dotenv_keys(ignored: &[String]) {
    for key in ignored {
        tracing::warn!(
            key = %key,
            "ignoring {key} from .env: it controls transport or process execution, and a \
             .env file is not a trusted source for it. Export it from your shell instead."
        );
    }
}

#[cfg(test)]
mod dotenv_filter_tests {
    use super::dotenv_key_is_denied;

    #[test]
    fn credentials_and_preferences_are_allowed_from_dotenv() {
        // The legitimate use of `.env`: secrets and output preferences.
        for key in [
            "ELEVENLABS_API_KEY",
            "ELEVENLABS_TOKEN",
            "ELEVENLABS_CLIENT_ID",
            "ELEVENLABS_CLIENT_SECRET",
            "ELEVENLABS_OUTPUT",
            "ELEVENLABS_VIA",
            "ELEVENLABS_TIMEOUT_SECS",
            "ELEVENLABS_CONNECT_TIMEOUT_SECS",
            "NO_COLOR",
            "DO_NOT_TRACK",
            "no_color",
        ] {
            assert!(
                !dotenv_key_is_denied(key, "ELEVENLABS"),
                "{key} should be loadable from .env"
            );
        }
    }

    #[test]
    fn transport_and_execution_keys_are_denied_from_dotenv() {
        // Each of these lets an attacker-authored `.env` redirect traffic,
        // weaken TLS, or run a program of their choosing.
        for key in [
            "ELEVENLABS_BASE_URL",
            "ELEVENLABS_PAGER",
            "ELEVENLABS_PROXY",
            "ELEVENLABS_NO_PROXY",
            "ELEVENLABS_INSECURE",
            "ELEVENLABS_INSECURE_SKIP_VERIFY",
            "ELEVENLABS_CA_BUNDLE",
            "ELEVENLABS_EXTRA_CA_CERTS",
            "PAGER",
            "BROWSER",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "CURL_CA_BUNDLE",
            "REQUESTS_CA_BUNDLE",
            // The HTTP stack reads the lowercase names too, and Windows any case.
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
            "elevenlabs_base_url",
            "Elevenlabs_Insecure",
            "browser",
            // Make the default `less` pager run commands.
            "LESSOPEN",
            "LESSCLOSE",
            "lessopen",
            "LESS",
            "LESSKEY",
            "LESSKEYIN",
            "LESSEDIT",
            "LESSSECURE",
            "EDITOR",
            "VISUAL",
            "SHELL",
            // Pick the binaries and libraries child processes load.
            "PATH",
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            // Redirect credential storage and `xdg-open` handlers.
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "HOME",
        ] {
            assert!(
                dotenv_key_is_denied(key, "ELEVENLABS"),
                "{key} must not be settable from .env"
            );
        }
    }

    #[test]
    fn dotenv_cannot_disable_the_cross_host_guards() {
        // Otherwise a `.env` could undo the redirect and pagination
        // protections, which is the whole point of having them.
        assert!(dotenv_key_is_denied(
            "ELEVENLABS_ALLOW_CROSS_HOST_REDIRECTS",
            "ELEVENLABS"
        ));
        assert!(dotenv_key_is_denied(
            "ELEVENLABS_ALLOW_CROSS_HOST_PAGINATION",
            "ELEVENLABS"
        ));
    }

    #[test]
    fn only_this_binary_s_namespace_is_loadable() {
        // Unscoped and foreign-prefixed keys never configured this CLI, and
        // reach the programs it starts (pager, browser) if loaded.
        for key in [
            "SOME_UNRELATED_APP_KEY",
            "OTHERCLI_API_KEY",
            "OTHERCLI_BASE_URL",
            "BASE_URL",
            "ELEVENLABS",
            "ELEVENLABS_",
            "ELEVENLABSX_API_KEY",
        ] {
            assert!(
                dotenv_key_is_denied(key, "ELEVENLABS"),
                "{key} must not be settable from .env"
            );
        }
        // Kebab-cased binary names map to underscored prefixes.
        assert!(!dotenv_key_is_denied("MY_CLI_API_KEY", "MY_CLI"));
        assert!(dotenv_key_is_denied("MY_CLI_BASE_URL", "MY_CLI"));
    }

    #[test]
    fn unrelated_keys_are_dropped_without_a_warning() {
        use super::dotenv_denial_is_reported;
        assert!(!dotenv_denial_is_reported("DATABASE_URL", "ELEVENLABS"));
        assert!(!dotenv_denial_is_reported(
            "OTHERCLI_BASE_URL",
            "ELEVENLABS"
        ));
        assert!(dotenv_denial_is_reported("LESSOPEN", "ELEVENLABS"));
        assert!(dotenv_denial_is_reported("LD_BIND_NOW", "ELEVENLABS"));
        assert!(dotenv_denial_is_reported(
            "ELEVENLABS_BASE_URL",
            "ELEVENLABS"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn dotenv_lessopen_never_reaches_the_pager_process() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(
            &path,
            "LESSOPEN=\"|-sh -c 'touch /tmp/pwned' %s\"\n\
             LESSCLOSE=\"sh -c 'touch /tmp/pwned' %s %s\"\n\
             LD_PRELOAD=/tmp/evil.so\n\
             UNRELATED_DOTENV_FILTER_TEST_KEY=1\n\
             FERN_DOTENV_FILTER_TEST_MARKER=loaded\n",
        )
        .unwrap();
        let real: Vec<_> = ["LESSOPEN", "LESSCLOSE", "LD_PRELOAD"]
            .iter()
            .map(|k| std::env::var_os(k))
            .collect();

        let entries = dotenvy::from_path_iter(&path).unwrap().flatten();
        let ignored = super::apply_dotenv_entries(entries, "fern-dotenv-filter-test");

        assert_eq!(ignored, ["LESSOPEN", "LESSCLOSE", "LD_PRELOAD"]);
        assert_eq!(
            std::env::var("FERN_DOTENV_FILTER_TEST_MARKER").as_deref(),
            Ok("loaded")
        );
        assert!(std::env::var_os("UNRELATED_DOTENV_FILTER_TEST_KEY").is_none());

        // What a spawned pager inherits: only the operator's real values, if any.
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg("printf '%s|%s|%s' \"${LESSOPEN-unset}\" \"${LESSCLOSE-unset}\" \"${LD_PRELOAD-unset}\"")
            .output()
            .unwrap();
        let expected = real
            .iter()
            .map(|v| {
                v.as_ref()
                    .map_or("unset".to_string(), |v| v.to_string_lossy().into_owned())
            })
            .collect::<Vec<_>>()
            .join("|");
        assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    }

    #[test]
    fn dotenv_cannot_enable_dynamic_completion() {
        assert!(super::dotenv_key_is_denied("MY_CLI_COMPLETE", "MY_CLI"));
        assert_eq!(
            crate::completions::complete_env_var("my-cli"),
            "MY_CLI_COMPLETE"
        );
    }

}
