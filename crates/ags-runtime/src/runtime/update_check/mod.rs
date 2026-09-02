//! Passive update-check hint.
//!
//! Checks GitHub Releases for a newer `ags` version via a detached background
//! process and, at most once per new release, prints a one-line hint to stderr.
//! Never blocks a command, never fails one, never touches machine-readable output.
//!
//! Mirrors the shape of `diagnostics/`: pure/data logic lives in submodules,
//! the public API is exposed here at the module root.

mod cache;
mod github;

/// The outcome of comparing our version against the latest on GitHub.
///
/// This is plain data - no behavior. The `current` and `latest` strings are
/// the human-facing version numbers (e.g. "0.4.0"), and `is_newer` is the
/// already-computed answer to "should we consider hinting?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheckResult {
    pub current: String,
    pub latest: String,
    pub is_newer: bool,
}

/// A hint about a newer release, if the last cached check found one and we
/// haven't already shown it. Fast, synchronous, no network I/O.
pub fn cached_hint() -> Option<UpdateCheckResult> {
    cache::cached_hint(env!("CARGO_PKG_VERSION"))
}

/// Mark a release as "already shown" so its hint never repeats.
pub fn mark_notified(version: &str) {
    cache::mark_notified(version);
}

/// Whether a fresh network check is due (cache missing or >24h old). The CLI
/// checks this before launching the detached check child, so at most one child
/// is spawned per 24h.
pub fn is_check_due() -> bool {
    cache::is_check_due(crate::support::unix_now())
}

/// Run the GitHub check to completion and write the result to the cache.
///
/// Awaited by the detached `__update-check` child process launched from the CLI
/// layer — that detachment is what lets the fetch finish after the parent exits.
/// Silent on every failure; the sole writer of `latest_version`.
///
/// Respects `AGS_UPDATE_CHECK_URL` when set, so the functional test suite can
/// redirect the fetch to a mock server without hitting `api.github.com`.
pub async fn run_check(client: reqwest::Client) {
    let url = crate::runtime::config::update_check_url_override()
        .unwrap_or_else(|| github::GITHUB_LATEST_RELEASE_URL.to_string());
    run_check_with_url(&client, &url).await;
}

/// URL-injectable core of [`run_check`], so tests can target a mock server.
async fn run_check_with_url(client: &reqwest::Client, url: &str) {
    if let Some(latest) = github::fetch_latest_version(client, url).await {
        cache::record_latest(&latest, crate::support::unix_now());
    }
}

/// Build the dedicated update-check HTTP client (hard 3-second timeout).
pub fn build_client() -> Option<reqwest::Client> {
    github::build_client()
}

/// Whether the background check is suppressed entirely — the "don't call
/// GitHub" layer (env opt-out, `update-check=false` config, or CI).
pub fn is_check_suppressed() -> bool {
    use crate::runtime::config;
    if config::is_update_check_disabled() || config::is_ci() {
        return true;
    }
    matches!(
        config::GlobalConfig::load()
            .ok()
            .and_then(|c| c.update_check),
        Some(false)
    )
}

/// The hint to render this run — the "don't print right now" layer. Pure
/// function of the display gates plus whether a hint exists: shown only on a
/// real TTY, in a human format, and not for a suppressed command.
pub fn footer_to_show(
    hint: Option<UpdateCheckResult>,
    stderr_is_tty: bool,
    is_automation: bool,
    is_suppressed_command: bool,
) -> Option<UpdateCheckResult> {
    if stderr_is_tty && !is_automation && !is_suppressed_command {
        hint
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_hint() -> UpdateCheckResult {
        UpdateCheckResult {
            current: "0.4.0".to_string(),
            latest: "0.5.0".to_string(),
            is_newer: true,
        }
    }

    // ── The pure display gate: no TTY, no env, fully deterministic ──

    #[test]
    fn footer_shown_when_all_gates_pass() {
        let shown = footer_to_show(Some(sample_hint()), true, false, false);
        assert_eq!(shown.map(|h| h.latest), Some("0.5.0".to_string()));
    }

    #[test]
    fn footer_hidden_when_stderr_not_tty() {
        assert!(footer_to_show(Some(sample_hint()), false, false, false).is_none());
    }

    #[test]
    fn footer_hidden_under_automation() {
        assert!(footer_to_show(Some(sample_hint()), true, true, false).is_none());
    }

    #[test]
    fn footer_hidden_for_suppressed_command() {
        assert!(footer_to_show(Some(sample_hint()), true, false, true).is_none());
    }

    #[test]
    fn footer_hidden_when_no_hint() {
        assert!(footer_to_show(None, true, false, false).is_none());
    }

    // ── The network gate: env + CI, isolated via a temp AGS_HOME ──

    use crate::support::test_helpers::TempEnvGuard;

    #[test]
    #[serial_test::serial]
    fn check_suppressed_by_env_or_ci() {
        // Isolate config to an empty temp dir so GlobalConfig::load() sees no key.
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        let _nc = TempEnvGuard::remove("AGS_NO_UPDATE_CHECK");
        let _ci = TempEnvGuard::remove("CI");
        assert!(!is_check_suppressed()); // nothing set -> not suppressed

        std::env::set_var("AGS_NO_UPDATE_CHECK", "1");
        assert!(is_check_suppressed()); // env opt-out
        std::env::remove_var("AGS_NO_UPDATE_CHECK");

        std::env::set_var("CI", "true");
        assert!(is_check_suppressed()); // CI detected
    }

    /// `update_check: Some(false)` in GlobalConfig is the documented user-facing
    /// off-switch (`ags config set update-check false`). With env and CI cleared,
    /// the config value must be the sole cause of suppression.
    #[test]
    #[serial_test::serial]
    fn check_suppressed_by_config_false() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        // One guard per mutation — clear the two env-based suppressors so the
        // config value is unambiguously the only active suppression source.
        let _nc = TempEnvGuard::remove("AGS_NO_UPDATE_CHECK");
        let _ci = TempEnvGuard::remove("CI");

        // Write via the real persistence path so the serde round-trip is covered.
        let config = crate::runtime::config::GlobalConfig {
            update_check: Some(false),
            ..Default::default()
        };
        config.save().unwrap();

        assert!(
            is_check_suppressed(),
            "update_check=Some(false) must suppress the check"
        );
    }

    /// `update_check: Some(true)` and `None` must NOT suppress the check — they
    /// are the "keep checking" states. Without this companion the `Some(false)`
    /// test cannot distinguish "the config branch works" from "something else
    /// suppressed".
    #[test]
    #[serial_test::serial]
    fn check_not_suppressed_by_config_true_or_none() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _nc = TempEnvGuard::remove("AGS_NO_UPDATE_CHECK");
        let _ci = TempEnvGuard::remove("CI");

        // Some(true) — explicitly enabled, must not suppress.
        let enabled = crate::runtime::config::GlobalConfig {
            update_check: Some(true),
            ..Default::default()
        };
        enabled.save().unwrap();
        assert!(
            !is_check_suppressed(),
            "update_check=Some(true) must not suppress the check"
        );

        // None — field absent from config (fresh install), must not suppress.
        let default = crate::runtime::config::GlobalConfig::default();
        default.save().unwrap();
        assert!(
            !is_check_suppressed(),
            "update_check=None must not suppress the check"
        );
    }

    // ── The full fetch → cache → read path against a mock server ──

    #[tokio::test]
    #[serial_test::serial]
    async fn run_check_writes_latest_to_cache() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "tag_name": "v999.0.0" })),
            )
            .mount(&server)
            .await;

        let url = format!("{}/releases/latest", server.uri());
        run_check_with_url(&reqwest::Client::new(), &url).await;

        // The check wrote 999.0.0 to disk; cached_hint() (current version) sees it.
        assert_eq!(cached_hint().unwrap().latest, "999.0.0");
    }
}
