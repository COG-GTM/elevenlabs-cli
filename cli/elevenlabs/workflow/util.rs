//! Small helpers shared across the custom command handlers: downcasting
//! the type-erased binding context and reading framework-global / string
//! args from a clap match.

use fern_cli_sdk::error::CliError;
use fern_cli_sdk::openapi::AppContext;

/// Downcast the type-erased handler context to the OpenAPI `AppContext`.
pub fn downcast_ctx(ctx: &dyn std::any::Any) -> Result<&AppContext, CliError> {
    ctx.downcast_ref::<AppContext>()
        .ok_or_else(|| CliError::Validation("binding context type mismatch".to_string()))
}

/// Read the framework's global `--dry-run` flag (declared `global(true)`),
/// defaulting to false if absent.
pub fn dry_run_flag(matches: &clap::ArgMatches) -> bool {
    matches
        .try_get_one::<bool>("dry-run")
        .ok()
        .flatten()
        .copied()
        .unwrap_or(false)
}

/// Read an optional string argument.
pub fn opt_string(matches: &clap::ArgMatches, id: &str) -> Option<String> {
    matches.get_one::<String>(id).cloned()
}

/// What a `pull` should do with one remote entity.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PullAction {
    Create,
    Update,
    Skip,
}

/// Decide a pull action from whether the entity is already tracked locally and
/// the `--update` / `--all` flags. Shared by the agents, tools, and tests pull
/// commands so they can't drift apart.
///
/// Default (neither flag): create new, skip existing — so a plain `pull` never
/// clobbers local edits. `--update`: existing only. `--all`: both.
pub fn plan_pull_action(exists_locally: bool, update: bool, all: bool) -> PullAction {
    match exists_locally {
        true if update || all => PullAction::Update,
        true => PullAction::Skip,
        false if update => PullAction::Skip,
        false => PullAction::Create,
    }
}

/// `text` without control characters (an escape sequence in a tool name
/// could otherwise clear or rewrite the terminal, with or without colour)
/// and without the invisible or reordering Unicode that could disguise what
/// is printed: zero-width characters, bidi overrides and isolates, and line
/// separators. Newlines and tabs become spaces, since every caller prints
/// the value inside a single line. Use it on any name, ID, path or error
/// text that comes from a project file, an index file or an API response.
/// The same policy as the CLI's own output sanitizer.
pub fn terminal_safe(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            '\u{200B}'..='\u{200D}'
            | '\u{FEFF}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2028}'..='\u{2029}'
            | '\u{2066}'..='\u{2069}' => None,
            c => Some(c),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches_from(args: &[&str]) -> clap::ArgMatches {
        clap::Command::new("test")
            .arg(
                clap::Arg::new("dry-run")
                    .long("dry-run")
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(clap::Arg::new("agent").long("agent"))
            .get_matches_from(args)
    }

    #[test]
    fn dry_run_flag_reads_the_global_flag() {
        assert!(dry_run_flag(&matches_from(&["test", "--dry-run"])));
        assert!(!dry_run_flag(&matches_from(&["test"])));
    }

    #[test]
    fn dry_run_flag_defaults_false_when_the_arg_is_absent() {
        // Commands that never declare the flag must not panic.
        let m = clap::Command::new("test").get_matches_from(["test"]);
        assert!(!dry_run_flag(&m));
    }

    #[test]
    fn opt_string_reads_present_and_absent_values() {
        assert_eq!(
            opt_string(&matches_from(&["test", "--agent", "a_1"]), "agent"),
            Some("a_1".to_string())
        );
        assert_eq!(opt_string(&matches_from(&["test"]), "agent"), None);
    }

    #[test]
    fn default_pull_creates_new_and_skips_existing() {
        assert_eq!(plan_pull_action(false, false, false), PullAction::Create);
        assert_eq!(plan_pull_action(true, false, false), PullAction::Skip);
    }

    #[test]
    fn update_only_touches_existing() {
        assert_eq!(plan_pull_action(true, true, false), PullAction::Update);
        assert_eq!(plan_pull_action(false, true, false), PullAction::Skip);
    }

    #[test]
    fn all_covers_both() {
        assert_eq!(plan_pull_action(true, false, true), PullAction::Update);
        assert_eq!(plan_pull_action(false, false, true), PullAction::Create);
    }

    #[test]
    fn update_still_suppresses_new_items_when_combined_with_all() {
        // Matches v0: the new-item branch only checks `--update`, so passing
        // both flags still skips items that don't exist locally.
        assert_eq!(plan_pull_action(true, true, true), PullAction::Update);
        assert_eq!(plan_pull_action(false, true, true), PullAction::Skip);
    }

    #[test]
    fn terminal_safe_strips_escapes_and_invisible_unicode() {
        assert_eq!(terminal_safe("a\u{1b}[2Jb\u{7}c"), "a[2Jbc");
        assert_eq!(terminal_safe("x\u{202E}y\u{200B}z\u{2066}"), "xyz");
        assert_eq!(terminal_safe("one\r\ntwo\tthree"), "one  two three");
        assert_eq!(terminal_safe("Café 日本"), "Café 日本");
    }
}
