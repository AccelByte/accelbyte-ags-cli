//! Update-check module: passive hint and explicit check.
//!
//! Checks GitHub Releases for a newer `ags` version. The **passive** path
//! runs via a detached background process, at most once per 24h, and prints a
//! one-line hint to stderr. The **explicit** path (`check_now`) is the
//! synchronous, error-reporting entry point used by `ags update`.
//!
//! Mirrors the shape of `diagnostics/`: pure/data logic lives in submodules,
//! the public API is exposed here at the module root.

mod cache;
mod github;
pub mod install_method;

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

/// The effective API URL for the update check: `AGS_UPDATE_CHECK_URL` when
/// set, otherwise the real GitHub latest-release endpoint. Shared by
/// `check_now`, `run_check`, and the CLI handler's dry-run preview.
pub fn api_url() -> String {
    crate::runtime::config::update_check_url_override()
        .unwrap_or_else(|| github::GITHUB_LATEST_RELEASE_URL.to_string())
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
    let url = api_url();
    run_check_with_url(&client, &url).await;
}

/// URL-injectable core of [`run_check`], so tests can target a mock server.
async fn run_check_with_url(client: &reqwest::Client, url: &str) {
    if let Ok(latest) = github::fetch_latest_version(client, url).await {
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

// ── Explicit check (ags update) ──

/// Error from an explicit update check, carrying the cause so the CLI layer
/// can compose the user-facing message with the URL and fallback link.
#[derive(Debug)]
pub enum UpdateCheckError {
    /// Network transport failed (unreachable, timeout, DNS error).
    Transport(String),
    /// GitHub responded with a non-success HTTP status.
    HttpStatus(u16),
    /// The release tag is not a valid semantic version.
    InvalidTag(String),
}

impl std::fmt::Display for UpdateCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(msg) => write!(f, "network error: {msg}"),
            Self::HttpStatus(code) => write!(f, "HTTP {code}"),
            Self::InvalidTag(tag) => write!(f, "tag is not a version: {tag}"),
        }
    }
}

/// Perform an explicit update check. Fetches the latest release from GitHub,
/// compares against the running version, records the result in the cache, and
/// marks the version as notified when it is newer.
///
/// Does NOT consult `is_check_suppressed`; the opt-outs govern the passive
/// check only. An explicit `ags update` is the user asking.
pub async fn check_now(client: &reqwest::Client) -> Result<UpdateCheckResult, UpdateCheckError> {
    let url = api_url();

    let latest = github::fetch_latest_version(client, &url).await?;

    let result = github::try_compare_versions(env!("CARGO_PKG_VERSION"), &latest)
        .map_err(UpdateCheckError::InvalidTag)?;

    cache::record_latest(&latest, crate::support::unix_now());

    if result.is_newer {
        cache::mark_notified(&latest);
    }

    Ok(result)
}

/// The GitHub releases page URL for the latest release (the fallback link
/// shown in error messages, not the API endpoint).
pub fn latest_release_url() -> &'static str {
    "https://github.com/AccelByte/accelbyte-ags-cli/releases/latest"
}

/// The releases page URL for a specific version, with a `v` prefix on the tag.
pub fn release_url(version: &str) -> String {
    format!("https://github.com/AccelByte/accelbyte-ags-cli/releases/tag/v{version}")
}

/// Build an HTTP client with a caller-chosen timeout. The CLI layer uses this
/// with its `--timeout` value; the passive check uses the hard 3-second client.
pub fn build_client_with_timeout(timeout: std::time::Duration) -> Option<reqwest::Client> {
    github::build_client_with_timeout(timeout)
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

    // ── release_url ──

    #[test]
    fn release_url_uses_v_prefixed_tag() {
        assert_eq!(
            release_url("0.5.2"),
            "https://github.com/AccelByte/accelbyte-ags-cli/releases/tag/v0.5.2"
        );
    }

    // ── Explicit check (ags update) against a mock server ──

    #[tokio::test]
    #[serial_test::serial]
    async fn check_now_reports_newer_and_records_cache() {
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
                    .set_body_json(serde_json::json!({ "tag_name": "v99.0.0" })),
            )
            .mount(&server)
            .await;

        let _url_guard = TempEnvGuard::set(
            crate::runtime::config::ENV_UPDATE_CHECK_URL,
            &format!("{}/releases/latest", server.uri()),
        );

        let result = check_now(&reqwest::Client::new()).await.unwrap();
        assert!(result.is_newer);
        assert_eq!(result.latest, "99.0.0");

        // When is_newer, check_now marks the version notified. cached_hint()
        // returns None because notified_version == latest_version.
        assert!(
            cached_hint().is_none(),
            "notified_version must equal latest_version after check_now reports newer"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn check_now_reports_current_without_marking_notified() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        let current = env!("CARGO_PKG_VERSION");

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "tag_name": format!("v{current}") })),
            )
            .mount(&server)
            .await;

        let _url_guard = TempEnvGuard::set(
            crate::runtime::config::ENV_UPDATE_CHECK_URL,
            &format!("{}/releases/latest", server.uri()),
        );

        let result = check_now(&reqwest::Client::new()).await.unwrap();
        assert!(!result.is_newer);
        assert_eq!(result.current, current);

        // notified_version must remain unset when the version is not newer.
        let cache_path = tmp.path().join("cache").join("update_check.json");
        let cache: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&cache_path).unwrap()).unwrap();
        assert!(
            cache["notified_version"].is_null(),
            "notified_version must remain unchanged when not newer"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn check_now_returns_error_on_http_403() {
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
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let _url_guard = TempEnvGuard::set(
            crate::runtime::config::ENV_UPDATE_CHECK_URL,
            &format!("{}/releases/latest", server.uri()),
        );

        let err = check_now(&reqwest::Client::new()).await.unwrap_err();
        assert!(
            matches!(err, UpdateCheckError::HttpStatus(403)),
            "expected HttpStatus(403), got: {err}"
        );
    }

    /// The error from an invalid latest tag must name the tag that failed to
    /// parse, not a generic message — the user needs to see the offending value.
    #[tokio::test]
    #[serial_test::serial]
    async fn check_now_error_names_the_value_that_failed_to_parse() {
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
                    .set_body_json(serde_json::json!({ "tag_name": "nightly-latest" })),
            )
            .mount(&server)
            .await;

        let _url_guard = TempEnvGuard::set(
            crate::runtime::config::ENV_UPDATE_CHECK_URL,
            &format!("{}/releases/latest", server.uri()),
        );

        let err = check_now(&reqwest::Client::new()).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("nightly-latest"),
            "error must name the invalid tag 'nightly-latest'; got: {msg}"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn check_now_returns_error_on_non_version_tag() {
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
                    .set_body_json(serde_json::json!({ "tag_name": "nightly-latest" })),
            )
            .mount(&server)
            .await;

        let _url_guard = TempEnvGuard::set(
            crate::runtime::config::ENV_UPDATE_CHECK_URL,
            &format!("{}/releases/latest", server.uri()),
        );

        let err = check_now(&reqwest::Client::new()).await.unwrap_err();
        assert!(
            matches!(err, UpdateCheckError::InvalidTag(_)),
            "expected InvalidTag, got: {err}"
        );
    }
}
