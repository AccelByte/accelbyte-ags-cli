//! One-time first-run onboarding hint.
//!
//! On the very first interactive human run the CLI emits a short welcome
//! message with next-step pointers, then persists a flag so the hint is
//! never shown again. The predicate, the emitter, and the flag read are
//! split into separate functions so the full truth table can be unit-tested
//! without a pseudo-terminal.
//!
//! The hint is emitted AFTER the surface is finalized: each route handler
//! calls `try_emit_first_run_hint` once the real surface is known. This
//! avoids predicting the surface and ensures a service command that
//! finalizes to `PlainTerminal` shows the hint safely.
//!
//! Because the emit site lives after surface finalization, the hint fires
//! only on the first qualifying run that also reaches that point. A first
//! invocation that errors during parse, prologue, or compile never reaches
//! the emit site, so the hint is deferred to the next successful qualifying
//! command. This is intentional: the finalized surface is a prerequisite
//! for TUI-safe emission, and the seen-flag is persisted only inside
//! `emit_first_run_hint`, so a failed early exit leaves it unset.

use super::context;
use super::context::PhaseBackend;

/// Check whether the `first_run_hint_seen` flag has already been persisted.
///
/// The read is intentionally best-effort: a config-load failure is silently
/// treated as "already seen" so the hint never blocks a real command.
pub(super) fn is_first_run_hint_seen() -> bool {
    ags_runtime::runtime::config::GlobalConfig::load()
        .map(|cfg| cfg.first_run_hint_seen.unwrap_or(false))
        .unwrap_or(true)
}

/// Decide whether the one-time first-run onboarding hint should be shown.
///
/// Single source of truth for the gate logic: both the production entry
/// point (`try_emit_first_run_hint`) and the truth-table unit tests exercise
/// this exact predicate.
///
/// Cheap in-memory checks (automation, interactive, meta, surface) run
/// first. The expensive `already_seen` closure — which reads the config
/// from disk in production — is evaluated only when all cheap checks pass.
///
/// The caller must invoke this AFTER the surface is finalized (or for routes
/// that do not call `finalize_surface`, when the effective surface is
/// known). The hint is suppressed on a TUI surface because it would be
/// scrolled over or corrupted by the terminal acquisition.
pub(super) fn should_show_first_run_hint(
    ctx: &context::FrontendContext,
    already_seen: impl FnOnce() -> bool,
    is_meta: bool,
) -> bool {
    // Automation consumers (--format=json, scripts) never see the hint.
    if ctx.is_automation() {
        return false;
    }
    // Non-interactive terminals (piped stdin or stderr) suppress the hint
    // so it never appears in redirected / CI output.
    if !ctx.terminal.allows_interactive_prompts() {
        return false;
    }
    // Meta-builtins (--version, --help) are not a "first real run".
    if is_meta {
        return false;
    }
    // Only emit on a PlainTerminal surface. A TUI surface (inline viewport
    // or fullscreen alt-screen) acquires the terminal after the hint would
    // have written; the resulting scroll corruption is exactly what this
    // gate prevents. When suppressed the seen-flag stays unset, so the hint
    // fires on the next plain-surface run.
    if !matches!(ctx.surface_backend(), PhaseBackend::PlainTerminal) {
        return false;
    }
    // Expensive disk check last — only runs when all cheap conditions pass.
    if already_seen() {
        return false;
    }
    true
}

/// Build the hint text lines for the one-time onboarding hint.
///
/// Separated from `emit_first_run_hint` so the copy content can be
/// unit-tested without capturing stderr.
///
/// `omit_next_line` drops the `Next: ags auth login` content line (and its
/// surrounding blank separator, collapsed to one) — used when the `Next:`
/// instruction would be wrong or redundant: the caller is already signed in,
/// or the command being run is itself an `ags auth` subcommand. The rest of
/// the banner (tagline, and the `ags --help` pointer) always stays, since it
/// remains useful regardless of auth state.
fn build_hint_lines(color: bool, omit_next_line: bool) -> Vec<String> {
    use crate::frontend::style::ansi;

    // Content lines without the box prefix — built first so each line's
    // styling is independent of the framing.
    let mut content = vec![
        ansi::bold(&ansi::cyan("\u{2726} AGS CLI", color), color),
        ansi::bold("Automate your AGS workflows with a unified CLI.", color),
        ansi::dim("Built for humans, scripts and AI agents.", color),
        String::new(),
    ];
    if !omit_next_line {
        content.push(format!(
            "{} Next: ags auth login   Log in and get set up in one step",
            ansi::fix_prefix(),
        ));
        content.push(String::new());
    }
    content.push(ansi::dim(
        "Explore anytime with  ags --help  or  ags <service> --help.",
        color,
    ));

    // Prefix every line with `* ` so the block reads as a visually bounded
    // box-out. Blank separator lines get a bare `*` (no trailing space).
    let mut lines: Vec<String> = content
        .into_iter()
        .map(|line| {
            if line.is_empty() {
                "*".to_string()
            } else {
                format!("* {line}")
            }
        })
        .collect();

    // Trailing blank (prefixed) line separates the banner from the
    // command's own output.
    lines.push("*".to_string());

    lines
}

/// Decide whether the hint drops its `Next: ags auth login` line: for an
/// `ags auth` subcommand, or when the caller is already signed in.
///
/// `signed_in` is a lazy closure (`is_signed_in` in production, which reads
/// config and token storage) so an auth command skips the disk read.
pub(super) fn should_omit_next_line(
    is_auth_command: bool,
    signed_in: impl FnOnce() -> bool,
) -> bool {
    is_auth_command || signed_in()
}

/// Emit the one-time first-run onboarding hint to stderr using the standard
/// info/next-step styling, then persist the `first_run_hint_seen` flag so the
/// hint is never shown again. The flag is persisted regardless of whether the
/// command itself succeeds — "first interactive run" means shown once.
///
/// `omit_next_line` is forwarded to `build_hint_lines` — see there.
pub(super) fn emit_first_run_hint(omit_next_line: bool) {
    let color = crate::frontend::style::ansi::is_stderr_enabled();
    for line in build_hint_lines(color, omit_next_line) {
        crate::frontend::write_stderr_line(&line);
    }

    // Persist the flag so the hint never appears again. Unconditional: the
    // hint was shown (banner + help pointer, at minimum) regardless of
    // whether the `Next:` line was included, so this is still the user's
    // one first-run moment.
    let _ = ags_runtime::runtime::config::GlobalConfig::update(|cfg| {
        cfg.first_run_hint_seen = Some(true);
        Ok(())
    });
}

/// Report whether the active profile is currently signed in, the same way
/// `ags auth status` does, without requiring a constructed `Runtime` and
/// without any network access.
///
/// `profile_flag` is the raw `--profile` value (`None` when not passed),
/// resolved with `resolve_profile_name_read_only` so this check never creates
/// the `default` profile or writes the global config: the hint runs on the
/// user's first command, before they may have chosen a profile. Signed in
/// means an environment token, environment client credentials, a stored
/// session with a still-valid token, or an expired stored token whose refresh
/// token `ags auth status` reports will auto-refresh
/// (`RefreshTokenState::is_usable`). Any other outcome — including no
/// resolvable profile or a config-read failure — is treated as "not signed
/// in".
pub(super) fn is_signed_in(profile_flag: Option<&str>) -> bool {
    use ags_runtime::runtime::auth::operations::{auth_snapshot, AuthSnapshot, StoredTokenState};
    use ags_runtime::runtime::config::resolve_profile_name_read_only;

    let Ok(Some(profile)) = resolve_profile_name_read_only(profile_flag) else {
        return false;
    };
    match auth_snapshot(&profile) {
        Ok(AuthSnapshot::EnvironmentToken)
        | Ok(AuthSnapshot::EnvironmentCredentials { .. })
        | Ok(AuthSnapshot::Stored {
            token_state: StoredTokenState::Valid { .. },
            ..
        }) => true,
        Ok(AuthSnapshot::Stored {
            token_state: StoredTokenState::Expired { refresh_token, .. },
            ..
        }) => refresh_token.is_usable(),
        _ => false,
    }
}

#[cfg(test)]
mod first_run_hint_tests {
    use super::{build_hint_lines, should_show_first_run_hint};
    use crate::invocation::context::{
        ConsumerKind, FrontendContext, InteractionPolicy, TerminalCapabilities,
    };
    use crate::invocation::flags::UiFlag;

    /// Restores an environment variable when dropped. Tests using it must be
    /// `#[serial_test::serial]`.
    struct TempEnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl TempEnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, original }
        }

        fn clear(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, original }
        }
    }

    impl Drop for TempEnvGuard {
        fn drop(&mut self) {
            match &self.original {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    /// Build a `FrontendContext` for an interactive human at a real terminal
    /// with a PlainTerminal surface (the default before finalization, and the
    /// result after finalization for routes that stay plain).
    fn human_interactive_plain() -> FrontendContext {
        FrontendContext {
            consumer: ConsumerKind::Human,
            interaction: InteractionPolicy {
                allow_input: true,
                prefer_rich_ui: false,
                prefer_fullscreen: false,
            },
            terminal: TerminalCapabilities {
                stdin_is_tty: true,
                stdout_is_tty: true,
                stderr_is_tty: true,
                color_force_off: false,
            },
            ui_intent: UiFlag::Auto,
        }
    }

    /// Build a `FrontendContext` for an automation consumer (--format=json).
    fn automation() -> FrontendContext {
        FrontendContext {
            consumer: ConsumerKind::Automation,
            interaction: InteractionPolicy {
                allow_input: false,
                prefer_rich_ui: false,
                prefer_fullscreen: false,
            },
            terminal: TerminalCapabilities {
                stdin_is_tty: false,
                stdout_is_tty: false,
                stderr_is_tty: false,
                color_force_off: false,
            },
            ui_intent: UiFlag::Auto,
        }
    }

    /// Build a `FrontendContext` for a human with a non-interactive terminal
    /// (e.g. piped stderr).
    fn human_non_interactive() -> FrontendContext {
        FrontendContext {
            consumer: ConsumerKind::Human,
            interaction: InteractionPolicy {
                allow_input: false,
                prefer_rich_ui: false,
                prefer_fullscreen: false,
            },
            terminal: TerminalCapabilities {
                stdin_is_tty: false,
                stdout_is_tty: false,
                stderr_is_tty: false,
                color_force_off: false,
            },
            ui_intent: UiFlag::Auto,
        }
    }

    /// Build a `FrontendContext` whose finalized surface is InlineTerminalUi.
    fn human_interactive_inline() -> FrontendContext {
        FrontendContext {
            consumer: ConsumerKind::Human,
            interaction: InteractionPolicy {
                allow_input: true,
                prefer_rich_ui: true,
                prefer_fullscreen: false,
            },
            terminal: TerminalCapabilities {
                stdin_is_tty: true,
                stdout_is_tty: true,
                stderr_is_tty: true,
                color_force_off: false,
            },
            ui_intent: UiFlag::Auto,
        }
    }

    /// Build a `FrontendContext` whose finalized surface is FullscreenTerminalUi.
    fn human_interactive_fullscreen() -> FrontendContext {
        FrontendContext {
            consumer: ConsumerKind::Human,
            interaction: InteractionPolicy {
                allow_input: true,
                prefer_rich_ui: false,
                prefer_fullscreen: true,
            },
            terminal: TerminalCapabilities {
                stdin_is_tty: true,
                stdout_is_tty: true,
                stderr_is_tty: true,
                color_force_off: false,
            },
            ui_intent: UiFlag::Auto,
        }
    }

    // ── Predicate truth-table tests ──

    /// The hint fires on the first interactive human run that is not a
    /// meta-builtin AND the finalized surface is PlainTerminal.
    #[test]
    fn test_shows_for_human_interactive_unseen_non_meta_plain_surface() {
        assert!(should_show_first_run_hint(
            &human_interactive_plain(),
            || false,
            false,
        ));
    }

    /// Automation consumers (--format=json, scripts) never see the hint.
    #[test]
    fn test_suppressed_for_automation_consumer() {
        assert!(!should_show_first_run_hint(&automation(), || false, false));
    }

    /// Non-interactive terminals (piped/redirected) suppress the hint.
    #[test]
    fn test_suppressed_for_non_interactive_terminal() {
        assert!(!should_show_first_run_hint(
            &human_non_interactive(),
            || false,
            false,
        ));
    }

    /// Once the hint has been shown (flag persisted), it never appears again.
    #[test]
    fn test_suppressed_when_already_seen() {
        assert!(!should_show_first_run_hint(
            &human_interactive_plain(),
            || true,
            false,
        ));
    }

    /// Meta-builtins (--version, --help) are not considered a "first real run".
    #[test]
    fn test_suppressed_for_meta_builtin() {
        assert!(!should_show_first_run_hint(
            &human_interactive_plain(),
            || false,
            true,
        ));
    }

    // ── TUI-safety (post-finalize surface checks) ──

    /// An inline TUI surface suppresses the hint even when every other
    /// condition (human, interactive, unseen, non-meta) would allow it.
    /// The seen-flag stays unset so the hint fires on the next plain run.
    #[test]
    fn test_suppressed_for_inline_surface() {
        assert!(!should_show_first_run_hint(
            &human_interactive_inline(),
            || false,
            false,
        ));
    }

    /// A fullscreen alt-screen surface suppresses the hint.
    #[test]
    fn test_suppressed_for_fullscreen_surface() {
        assert!(!should_show_first_run_hint(
            &human_interactive_fullscreen(),
            || false,
            false,
        ));
    }

    /// PlainTerminal surface with all other conditions met fires the hint.
    #[test]
    fn test_shows_on_plain_surface() {
        assert!(should_show_first_run_hint(
            &human_interactive_plain(),
            || false,
            false,
        ));
    }

    // ── Post-finalize: service commands ──

    /// A service command that finalizes to PlainTerminal (e.g. Service +
    /// Zero shape) qualifies for the hint. This is the over-suppression
    /// fix: the old predicate-based approach suppressed ALL service
    /// commands because the route CAN produce a TUI for some shapes.
    #[test]
    fn test_shows_for_service_command_finalized_to_plain() {
        use crate::invocation::shape::{RouteKind, Shape};
        let ctx = human_interactive_plain().finalize_surface(RouteKind::Service, Shape::Zero);
        assert!(should_show_first_run_hint(&ctx, || false, false));
    }

    /// A service command that finalizes to InlineTerminalUi (Service + Form)
    /// suppresses the hint, protecting TUI safety.
    #[test]
    fn test_suppressed_for_service_command_finalized_to_inline() {
        use crate::invocation::shape::{RouteKind, Shape};
        let ctx = human_interactive_plain().finalize_surface(RouteKind::Service, Shape::Form);
        assert!(!should_show_first_run_hint(&ctx, || false, false));
    }

    // ── Trailing --help ──

    /// A command with a trailing `--help` (e.g. `ags iam users create --help`)
    /// sets `is_meta = true` because `is_meta_builtin` matches `--help`
    /// anywhere in argv. The hint is suppressed for the help request, and the
    /// seen-flag stays unset so the hint fires on the next flag-free run.
    #[test]
    fn test_trailing_help_suppresses_hint_via_is_meta() {
        assert!(
            !should_show_first_run_hint(
                &human_interactive_plain(),
                || false, // not yet seen
                true,     // is_meta — trailing --help matched
            ),
            "trailing --help must suppress the hint; the seen-flag stays unset \
             so the hint fires on the next flag-free run"
        );
    }

    // ── Production entry-point tests ──

    /// The production entry point (`try_emit_first_run_hint`) suppresses
    /// the hint for automation consumers and does not persist the flag.
    // Env-mutating test: sets process-wide AGS_HOME and AGS_NO_KEYCHAIN.
    #[test]
    #[serial_test::serial]
    fn test_try_emit_suppressed_for_automation() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var(ags_runtime::runtime::config::ENV_HOME).ok();
        let prev_kc = std::env::var(ags_runtime::runtime::config::ENV_NO_KEYCHAIN).ok();
        std::env::set_var(
            ags_runtime::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        std::env::set_var(ags_runtime::runtime::config::ENV_NO_KEYCHAIN, "1");

        crate::invocation::try_emit_first_run_hint(
            &automation(),
            crate::invocation::HintCommand::Other,
            None,
        );

        // The flag must NOT be persisted — the hint was suppressed.
        let config_path = tmp.path().join("config.json");
        if config_path.exists() {
            let contents = std::fs::read_to_string(&config_path).unwrap();
            assert!(
                !contents.contains("first_run_hint_seen"),
                "first_run_hint_seen must not be set when the hint is suppressed"
            );
        }

        // Restore environment.
        match prev_home {
            Some(v) => std::env::set_var(ags_runtime::runtime::config::ENV_HOME, v),
            None => std::env::remove_var(ags_runtime::runtime::config::ENV_HOME),
        }
        match prev_kc {
            Some(v) => std::env::set_var(ags_runtime::runtime::config::ENV_NO_KEYCHAIN, v),
            None => std::env::remove_var(ags_runtime::runtime::config::ENV_NO_KEYCHAIN),
        }
    }

    /// The production entry point (`try_emit_first_run_hint`) emits the
    /// hint and persists the flag for a plain interactive first run.
    // Env-mutating test: sets process-wide AGS_HOME and AGS_NO_KEYCHAIN.
    #[test]
    #[serial_test::serial]
    fn test_try_emit_shows_for_plain_interactive() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var(ags_runtime::runtime::config::ENV_HOME).ok();
        let prev_kc = std::env::var(ags_runtime::runtime::config::ENV_NO_KEYCHAIN).ok();
        std::env::set_var(
            ags_runtime::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        std::env::set_var(ags_runtime::runtime::config::ENV_NO_KEYCHAIN, "1");

        crate::invocation::try_emit_first_run_hint(
            &human_interactive_plain(),
            crate::invocation::HintCommand::Other,
            None,
        );

        // The flag must be persisted — the hint was shown.
        let cfg = ags_runtime::runtime::config::GlobalConfig::load().unwrap();
        assert_eq!(
            cfg.first_run_hint_seen,
            Some(true),
            "try_emit_first_run_hint must persist the flag for a plain interactive run"
        );

        // Restore environment.
        match prev_home {
            Some(v) => std::env::set_var(ags_runtime::runtime::config::ENV_HOME, v),
            None => std::env::remove_var(ags_runtime::runtime::config::ENV_HOME),
        }
        match prev_kc {
            Some(v) => std::env::set_var(ags_runtime::runtime::config::ENV_NO_KEYCHAIN, v),
            None => std::env::remove_var(ags_runtime::runtime::config::ENV_NO_KEYCHAIN),
        }
    }

    /// A signed-in caller (`AGS_ACCESS_TOKEN` set) still gets the hint shown
    /// (banner + help pointer) and the flag persisted — only the `Next:`
    /// line is omitted from what's printed. The flag is unconditional on
    /// whether the hint text included the `Next:` line: the user still had
    /// their one first-run moment.
    // Env-mutating test: sets process-wide AGS_ACCESS_TOKEN, AGS_HOME and AGS_NO_KEYCHAIN.
    #[test]
    #[serial_test::serial]
    fn test_try_emit_persists_flag_for_signed_in_user() {
        use ags_runtime::runtime::config::{ENV_ACCESS_TOKEN, ENV_HOME, ENV_NO_KEYCHAIN};
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(ENV_HOME, tmp.path().to_str().unwrap());
        let _kc = TempEnvGuard::set(ENV_NO_KEYCHAIN, "1");
        let _token = TempEnvGuard::set(ENV_ACCESS_TOKEN, "a-token");

        crate::invocation::try_emit_first_run_hint(
            &human_interactive_plain(),
            crate::invocation::HintCommand::Other,
            None,
        );

        let cfg = ags_runtime::runtime::config::GlobalConfig::load().unwrap();
        assert_eq!(
            cfg.first_run_hint_seen,
            Some(true),
            "the flag must still be persisted for a signed-in user; only the \
             Next: line content changes, not whether the hint (and thus the \
             first-run moment) happened"
        );
    }

    // ── `is_signed_in` (no network access) ──

    /// With `AGS_ACCESS_TOKEN` set, `is_signed_in` reports `true` without
    /// any network access.
    // Env-mutating test: sets process-wide AGS_ACCESS_TOKEN and AGS_HOME.
    #[test]
    #[serial_test::serial]
    fn test_is_signed_in_true_for_env_access_token() {
        use ags_runtime::runtime::config::{ENV_ACCESS_TOKEN, ENV_HOME};
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(ENV_HOME, tmp.path().to_str().unwrap());
        let _token = TempEnvGuard::set(ENV_ACCESS_TOKEN, "a-token");

        assert!(
            super::is_signed_in(None),
            "AGS_ACCESS_TOKEN must be enough to report signed in"
        );
    }

    /// With `AGS_CLIENT_ID` and `AGS_CLIENT_SECRET` both set, `is_signed_in`
    /// reports `true` without any network access or stored token.
    // Env-mutating test: sets AGS_HOME, AGS_NO_KEYCHAIN, AGS_CLIENT_ID, AGS_CLIENT_SECRET.
    #[test]
    #[serial_test::serial]
    fn test_is_signed_in_true_for_env_client_credentials() {
        use ags_runtime::runtime::config::{
            ENV_ACCESS_TOKEN, ENV_CLIENT_ID, ENV_CLIENT_SECRET, ENV_HOME, ENV_NO_KEYCHAIN,
            ENV_PROFILE,
        };
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(ENV_HOME, tmp.path().to_str().unwrap());
        let _kc = TempEnvGuard::set(ENV_NO_KEYCHAIN, "1");
        let _id = TempEnvGuard::set(ENV_CLIENT_ID, "a-client");
        let _secret = TempEnvGuard::set(ENV_CLIENT_SECRET, "a-secret");
        let _token = TempEnvGuard::clear(ENV_ACCESS_TOKEN);
        let _profile = TempEnvGuard::clear(ENV_PROFILE);

        assert!(
            super::is_signed_in(None),
            "AGS_CLIENT_ID and AGS_CLIENT_SECRET must be enough to report signed in"
        );
    }

    /// On a fresh `AGS_HOME`, `is_signed_in` must not set anything up: no
    /// `profiles` directory and no `active_profile`. The hint runs on the
    /// user's first command, so creating `default` here would make it the
    /// active profile before the user chose one.
    // Env-mutating test: sets process-wide AGS_HOME and AGS_NO_KEYCHAIN.
    #[test]
    #[serial_test::serial]
    fn test_is_signed_in_does_not_create_profile_on_fresh_home() {
        use ags_runtime::runtime::config::{
            ENV_ACCESS_TOKEN, ENV_CLIENT_ID, ENV_CLIENT_SECRET, ENV_HOME, ENV_NO_KEYCHAIN,
            ENV_PROFILE,
        };
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(ENV_HOME, tmp.path().to_str().unwrap());
        let _kc = TempEnvGuard::set(ENV_NO_KEYCHAIN, "1");
        let _token = TempEnvGuard::clear(ENV_ACCESS_TOKEN);
        let _id = TempEnvGuard::clear(ENV_CLIENT_ID);
        let _secret = TempEnvGuard::clear(ENV_CLIENT_SECRET);
        let _profile = TempEnvGuard::clear(ENV_PROFILE);

        assert!(!super::is_signed_in(None));

        assert!(
            !tmp.path().join("profiles").exists(),
            "is_signed_in must not create the profiles directory"
        );
        let cfg = ags_runtime::runtime::config::GlobalConfig::load().unwrap();
        assert_eq!(
            cfg.active_profile, None,
            "is_signed_in must not set an active profile"
        );
    }

    /// Point `AGS_HOME` at `home` with a profile that has stored-session
    /// credentials (base URL + client ID, no env secret), and clear every
    /// environment variable that would make `auth_snapshot` short-circuit.
    fn stored_session_env(home: &std::path::Path) -> Vec<TempEnvGuard> {
        use ags_runtime::runtime::config::{
            ENV_ACCESS_TOKEN, ENV_BASE_URL, ENV_CLIENT_ID, ENV_CLIENT_SECRET, ENV_HOME,
            ENV_NO_KEYCHAIN, ENV_PROFILE,
        };
        vec![
            TempEnvGuard::set(ENV_HOME, home.to_str().unwrap()),
            TempEnvGuard::set(ENV_NO_KEYCHAIN, "1"),
            TempEnvGuard::set(ENV_BASE_URL, "https://example.invalid"),
            TempEnvGuard::set(ENV_CLIENT_ID, "a-client"),
            TempEnvGuard::clear(ENV_CLIENT_SECRET),
            TempEnvGuard::clear(ENV_ACCESS_TOKEN),
            TempEnvGuard::clear(ENV_PROFILE),
        ]
    }

    /// Store a token for `profile` whose access token expires `access_ttl`
    /// seconds from now (negative = already expired), with an optional refresh
    /// token valid for a day.
    fn seed_token(profile: &str, access_ttl: i64, refresh_token: Option<&str>) {
        use ags_runtime::runtime::auth::store::{store_token_data, TokenData};
        let now = ags_runtime::support::unix_now();
        store_token_data(
            profile,
            &TokenData {
                access_token: "an-access-token".to_string(),
                expires_at: now.saturating_add_signed(access_ttl),
                refresh_token: refresh_token.map(str::to_string),
                refresh_expires_at: refresh_token.map(|_| now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        )
        .unwrap();
    }

    /// An expired stored token with a valid refresh token counts as signed
    /// in: `ags auth status` says it will auto-refresh on the next API call,
    /// so the hint must not send the user to `ags auth login`.
    // Env-mutating test: sets AGS_HOME, AGS_NO_KEYCHAIN, AGS_BASE_URL, AGS_CLIENT_ID.
    #[test]
    #[serial_test::serial]
    fn test_is_signed_in_true_for_expired_token_with_valid_refresh() {
        let tmp = tempfile::tempdir().unwrap();
        let _env = stored_session_env(tmp.path());
        seed_token("dev", -300, Some("a-refresh-token"));

        assert!(super::is_signed_in(Some("dev")));
    }

    /// An expired stored token with no refresh token is not signed in: the
    /// user really does need `ags auth login`.
    // Env-mutating test: sets AGS_HOME, AGS_NO_KEYCHAIN, AGS_BASE_URL, AGS_CLIENT_ID.
    #[test]
    #[serial_test::serial]
    fn test_is_signed_in_false_for_expired_token_without_refresh() {
        let tmp = tempfile::tempdir().unwrap();
        let _env = stored_session_env(tmp.path());
        seed_token("dev", -300, None);

        assert!(!super::is_signed_in(Some("dev")));
    }

    /// A stored token that has not expired yet counts as signed in.
    // Env-mutating test: sets AGS_HOME, AGS_NO_KEYCHAIN, AGS_BASE_URL, AGS_CLIENT_ID.
    #[test]
    #[serial_test::serial]
    fn test_is_signed_in_true_for_valid_stored_token() {
        let tmp = tempfile::tempdir().unwrap();
        let _env = stored_session_env(tmp.path());
        seed_token("dev", 3_600, None);

        assert!(super::is_signed_in(Some("dev")));
    }

    // ── `should_omit_next_line` decision ──

    /// An `ags auth` subcommand omits the `Next:` line without checking
    /// sign-in state at all.
    #[test]
    fn test_omit_next_line_for_auth_command() {
        assert!(super::should_omit_next_line(true, || panic!(
            "sign-in state must not be read for an auth command"
        )));
    }

    /// A signed-in caller running any other command omits the `Next:` line.
    #[test]
    fn test_omit_next_line_when_signed_in() {
        assert!(super::should_omit_next_line(false, || true));
    }

    /// A caller who is not signed in and not running `ags auth` keeps the
    /// `Next: ags auth login` line.
    #[test]
    fn test_keep_next_line_when_neither() {
        assert!(!super::should_omit_next_line(false, || false));
    }

    // ── Emit infallibility + flag persistence ──

    /// The emit path must be infallible and must persist the
    /// `first_run_hint_seen` flag so the hint is never shown again.
    // Env-mutating test: sets process-wide AGS_HOME and AGS_NO_KEYCHAIN.
    #[test]
    #[serial_test::serial]
    fn test_emit_first_run_hint_is_infallible_and_persists_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_home = std::env::var(ags_runtime::runtime::config::ENV_HOME).ok();
        let prev_kc = std::env::var(ags_runtime::runtime::config::ENV_NO_KEYCHAIN).ok();
        std::env::set_var(
            ags_runtime::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        std::env::set_var(ags_runtime::runtime::config::ENV_NO_KEYCHAIN, "1");

        // Call the emit path — must not panic.
        super::emit_first_run_hint(false);

        // The flag must now be persisted as true.
        let cfg = ags_runtime::runtime::config::GlobalConfig::load().unwrap();
        assert_eq!(
            cfg.first_run_hint_seen,
            Some(true),
            "emit_first_run_hint must persist first_run_hint_seen = true"
        );

        // Restore environment.
        match prev_home {
            Some(v) => std::env::set_var(ags_runtime::runtime::config::ENV_HOME, v),
            None => std::env::remove_var(ags_runtime::runtime::config::ENV_HOME),
        }
        match prev_kc {
            Some(v) => std::env::set_var(ags_runtime::runtime::config::ENV_NO_KEYCHAIN, v),
            None => std::env::remove_var(ags_runtime::runtime::config::ENV_NO_KEYCHAIN),
        }
    }

    // ── Copy content assertions ──

    /// The hint must contain the product tagline and the single next-step
    /// command (`ags auth login`), and must NOT reference `profile create`.
    #[test]
    fn test_hint_copy_contains_tagline_and_auth_login() {
        let lines = build_hint_lines(false, false);
        let full_text = lines.join("\n");
        assert!(
            full_text.contains("Automate your AGS workflows with a unified CLI"),
            "hint must contain the product tagline"
        );
        assert!(
            full_text.contains("ags auth login"),
            "hint must contain the single next-step command"
        );
    }

    #[test]
    fn test_hint_copy_does_not_contain_profile_create() {
        let lines = build_hint_lines(false, false);
        let full_text = lines.join("\n");
        assert!(
            !full_text.contains("profile create"),
            "hint must not lead with profile creation"
        );
    }

    /// The next-step line must follow the shared `→ Next:` convention used
    /// across the CLI (via `fix_prefix()`), not a bespoke dimmed form.
    #[test]
    fn test_hint_copy_uses_fix_prefix_next_convention() {
        let lines = build_hint_lines(false, false);
        let full_text = lines.join("\n");
        assert!(
            full_text.contains("Next: ags auth login"),
            "hint must use the shared 'fix_prefix() Next:' convention"
        );
    }

    /// `omit_next_line = true` drops the `Next: ags auth login` line
    /// entirely (used for an already signed-in caller, or an `ags auth`
    /// subcommand) but keeps the rest of the banner: tagline, subtitle, and
    /// the `ags --help` pointer.
    #[test]
    fn test_hint_omits_next_line_when_requested() {
        let lines = build_hint_lines(false, true);
        let full_text = lines.join("\n");
        assert!(
            !full_text.contains("ags auth login"),
            "Next: ags auth login must not appear when omit_next_line is true"
        );
        assert!(
            full_text.contains("Automate your AGS workflows with a unified CLI"),
            "the tagline must still appear when the Next: line is omitted"
        );
        assert!(
            full_text.contains("Explore anytime with"),
            "the help pointer must still appear when the Next: line is omitted"
        );
    }

    // ── Box-out prefix and trailing blank assertions ──

    /// Every line returned by `build_hint_lines` must start with `*` so the
    /// onboarding block reads as a visually bounded box-out. This includes
    /// blank separator lines, which carry a bare `*` prefix.
    #[test]
    fn test_hint_every_line_has_star_prefix() {
        let lines = build_hint_lines(false, false);
        for (i, line) in lines.iter().enumerate() {
            assert!(
                line.starts_with('*'),
                "line {i} must start with '*', got: {line:?}"
            );
        }
    }

    /// The box-out prefix invariant holds with the `Next:` line omitted too.
    #[test]
    fn test_hint_every_line_has_star_prefix_with_next_line_omitted() {
        let lines = build_hint_lines(false, true);
        for (i, line) in lines.iter().enumerate() {
            assert!(
                line.starts_with('*'),
                "line {i} must start with '*', got: {line:?}"
            );
        }
    }

    /// The hint vector must end with a blank (prefixed) line so the banner
    /// is visually separated from whatever the command prints next.
    #[test]
    fn test_hint_ends_with_trailing_blank() {
        let lines = build_hint_lines(false, false);
        let last = lines.last().expect("hint must have at least one line");
        assert_eq!(
            last.trim(),
            "*",
            "last line must be a blank separator (just the prefix); got: {last:?}"
        );
    }

    // ── Banner styling assertions (color ON) ──

    /// With color enabled the onboarding hint must render a distinct bold
    /// banner heading — not the same `info()` styling used for routine
    /// status lines. This test guards against regression to the old
    /// `info()`-styled tagline that was visually indistinguishable from
    /// ordinary status output.
    #[test]
    fn test_hint_banner_is_bold_not_info_styled() {
        let lines = build_hint_lines(true, false);

        // Heading: bold + cyan banner containing the star glyph and product name.
        let heading = lines
            .iter()
            .find(|l| l.contains("\u{2726} AGS CLI"))
            .expect("hint must contain the ✦ AGS CLI heading line");
        assert!(heading.contains("\x1b[1m"), "heading must be bold (SGR 1)");
        assert!(
            heading.contains("\x1b[36m"),
            "heading must be cyan (SGR 36)"
        );

        // Tagline: bold default text, NOT an info-prefixed line.
        let tagline = lines
            .iter()
            .find(|l| l.contains("Automate your AGS workflows"))
            .expect("hint must contain the tagline");
        assert!(tagline.contains("\x1b[1m"), "tagline must be bold (SGR 1)");
        assert!(
            !tagline.contains('\u{203a}'),
            "tagline must NOT use the info symbol \u{203a} — it is not a status line"
        );

        // Subtitle: dimmed text.
        let subtitle = lines
            .iter()
            .find(|l| l.contains("Built for humans"))
            .expect("hint must contain the subtitle");
        assert!(subtitle.contains("\x1b[2m"), "subtitle must be dim (SGR 2)");
    }
}
