//! Tier runner and per-profile orchestration for `ags doctor`.
//!
//! [`run_profile`] executes the three check tiers (Config, Auth, Network)
//! for a single profile, with skip-on-fail propagation between tiers and
//! optional `--offline` support. [`run_all`] iterates every known profile.

use std::time::Duration;

use crate::protocol::diagnostics::{
    CheckResult, CheckStatus, CheckTier, DoctorReport, DoctorResult,
};
use crate::protocol::error::RuntimeError;
use crate::runtime::config::{self, GlobalConfig, ProfileConfig};

use super::checks;

/// Skip reason used when a check is bypassed because an earlier check failed within its own tier.
const SKIP_REASON: &str = "skipped";

/// Tier 2 checks, in execution order. Used both to drive the runner and to enumerate skip stubs.
pub(crate) const AUTH_TIER_CHECKS: &[(&str, &str)] = &[
    ("keychain-access", "Keychain accessible"),
    ("credential-state", "Credentials"),
    ("token-state", "Access Token"),
];

/// Tier 3 checks, in execution order. Used both to drive the runner and to enumerate skip stubs.
pub(crate) const NETWORK_TIER_CHECKS: &[(&str, &str)] = &[
    ("base-url-reachable", "Base URL reachable"),
    ("server-auth", "Server auth"),
];

/// Run all three tiers of checks for a single profile.
pub async fn run_profile(profile: &str, is_active: bool, is_offline: bool) -> DoctorReport {
    let (mut checks, profile_config) = run_config_tier(profile);
    let has_tier1_failures = has_failure_in_tier(&checks, CheckTier::Config);

    if has_tier1_failures {
        checks.extend(skipped_tier(CheckTier::Auth, AUTH_TIER_CHECKS, SKIP_REASON));
    } else {
        checks.extend(run_auth_tier(profile));
    }

    let has_tier2_failures = has_tier1_failures || has_failure_in_tier(&checks, CheckTier::Auth);

    if has_tier2_failures || is_offline {
        let reason = if is_offline {
            "offline mode"
        } else {
            "earlier tier failed"
        };
        checks.extend(skipped_tier(
            CheckTier::Network,
            NETWORK_TIER_CHECKS,
            &format!("skipped ({reason})"),
        ));
    } else {
        let profile_config = profile_config
            .as_ref()
            .expect("tier 1 success guarantees profile_config is loaded");
        checks.extend(run_network_tier(profile, profile_config).await);
    }

    DoctorReport {
        profile: profile.to_string(),
        is_active,
        checks,
    }
}

/// Run diagnostics across all known profiles.
pub async fn run_all(is_offline: bool) -> Result<DoctorResult, RuntimeError> {
    let profiles = config::list_profiles()?;
    let global = GlobalConfig::load()?;
    let active = global.active_profile.as_deref();

    let mut reports = Vec::new();
    for name in &profiles {
        let is_active = active.is_some_and(|a| a == name);
        let report = run_profile(name, is_active, is_offline).await;
        reports.push(report);
    }

    if reports.is_empty() {
        reports.push(no_profile_report(
            "No profiles found",
            "Run 'ags profile create <name>' to get started",
            is_offline,
        ));
    }

    Ok(DoctorResult { reports })
}

/// Build a `DoctorReport` for the case where no profile context exists.
///
/// Emits a `Fail` profile-selection check carrying the supplied message and
/// suggestion, then skipped stubs for the entire Auth and Network tiers — the
/// same shape `run_profile` would produce if Tier 1 failed catastrophically.
pub(crate) fn no_profile_report(
    fail_message: impl Into<String>,
    fail_suggestion: impl Into<String>,
    is_offline: bool,
) -> DoctorReport {
    let mut checks = vec![CheckResult {
        tier: CheckTier::Config,
        name: "profile-selection",
        title: "Profile exists",
        status: CheckStatus::Fail,
        message: fail_message.into(),
        suggestion: Some(fail_suggestion.into()),
    }];
    checks.extend(skipped_tier(CheckTier::Auth, AUTH_TIER_CHECKS, SKIP_REASON));
    let reason = if is_offline {
        "offline mode"
    } else {
        "earlier tier failed"
    };
    checks.extend(skipped_tier(
        CheckTier::Network,
        NETWORK_TIER_CHECKS,
        &format!("skipped ({reason})"),
    ));

    DoctorReport {
        profile: "(none)".to_string(),
        is_active: false,
        checks,
    }
}

/// Run the Config tier and return its results plus the parsed profile config (when load succeeded).
///
/// The parsed `ProfileConfig` is bubbled up so the Network tier can reuse it
/// without re-reading the file.
fn run_config_tier(profile: &str) -> (Vec<CheckResult>, Option<ProfileConfig>) {
    let mut tier = TierRunner::new(CheckTier::Config, SKIP_REASON);

    tier.push(checks::assess_profile_selection(profile));

    // Config validity also returns the loaded ProfileConfig for downstream use
    let profile_config = if !tier.has_failed() {
        let (result, config) = checks::assess_config_validity(profile);
        tier.push(result);
        config
    } else {
        tier.skip("config-validity", "Config file valid");
        None
    };

    tier.run("file-permissions", "Config file permissions", || {
        checks::assess_file_permissions(profile)
    });

    if let Some(ref profile_config) = profile_config {
        tier.run_many(
            &[
                ("config-base-url", "Base URL"),
                ("config-client-id", "Client ID"),
            ],
            || checks::assess_config_completeness(profile_config),
        );
        tier.run("namespace-valid", "Namespace", || {
            checks::assess_namespace(profile_config)
        });
    } else {
        tier.skip_many(&[
            ("config-base-url", "Base URL"),
            ("config-client-id", "Client ID"),
        ]);
        tier.skip("namespace-valid", "Namespace");
    }

    // env-var-overrides is informational — always run regardless of failures
    tier.checks.extend(checks::assess_env_var_overrides());

    (tier.finish(), profile_config)
}

/// Run the Auth tier (keychain, credentials, token state) for the given profile.
fn run_auth_tier(profile: &str) -> Vec<CheckResult> {
    let mut tier = TierRunner::new(CheckTier::Auth, SKIP_REASON);
    tier.push(checks::assess_keychain_access());
    tier.run("credential-state", "Credentials", || {
        checks::assess_credential_state(profile)
    });
    tier.run("token-state", "Access Token", || {
        checks::assess_token_state(profile)
    });
    tier.finish()
}

/// Run the Network tier (base-URL reachability, server auth) using a short-timeout client.
///
/// Caller must have confirmed Tier 1 succeeded so `profile_config` is loaded
/// and a base URL is resolvable.
async fn run_network_tier(profile: &str, profile_config: &ProfileConfig) -> Vec<CheckResult> {
    let client = match build_network_probe_client() {
        Ok(client) => client,
        Err(e) => {
            let mut tier = TierRunner::new(CheckTier::Network, SKIP_REASON);
            tier.push(CheckResult {
                tier: CheckTier::Network,
                name: "base-url-reachable",
                title: "Base URL reachable",
                status: CheckStatus::Fail,
                message: format!("cannot create HTTP client: {e}"),
                suggestion: None,
            });
            tier.skip("server-auth", "Server auth");
            return tier.finish();
        }
    };

    let base_url = std::env::var(config::ENV_BASE_URL)
        .ok()
        .or_else(|| profile_config.base_url.clone());

    let mut tier = TierRunner::new(CheckTier::Network, SKIP_REASON);
    let Some(url) = base_url else {
        // Defensive: Tier 1 should have caught a missing base URL
        tier.skip_many(NETWORK_TIER_CHECKS);
        return tier.finish();
    };

    tier.push(checks::assess_base_url_reachable(&url, &client).await);
    // token_refresh is async — the closure-based runner cannot await
    if !tier.has_failed() {
        tier.push(checks::assess_token_refresh(profile, &client).await);
    } else {
        tier.skip("server-auth", "Server auth");
    }
    tier.finish()
}

/// Build a reqwest client tuned for diagnostic probes.
///
/// Uses a short 10-second timeout so a hung environment fails fast rather
/// than blocking the doctor command. Differs intentionally from the default
/// dispatch client (`runtime::dispatch::http::build_http_client`, 60 s),
/// which serves real API calls and tolerates slower responses.
fn build_network_probe_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
}

/// Returns true if any check in the given tier has failed.
fn has_failure_in_tier(checks: &[CheckResult], tier: CheckTier) -> bool {
    checks
        .iter()
        .any(|c| c.tier == tier && c.status == CheckStatus::Fail)
}

/// Runs checks in order within a tier, auto-skipping after the first failure.
struct TierRunner {
    tier: CheckTier,
    checks: Vec<CheckResult>,
    skip_reason: String,
}

impl TierRunner {
    /// Create a new runner for `tier`, using `skip_reason` as the message on auto-skipped results.
    fn new(tier: CheckTier, skip_reason: &str) -> Self {
        Self {
            tier,
            checks: Vec::new(),
            skip_reason: skip_reason.to_string(),
        }
    }

    /// Returns true when at least one collected check has status `Fail`.
    fn has_failed(&self) -> bool {
        self.checks.iter().any(|c| c.status == CheckStatus::Fail)
    }

    /// Run a single check, or emit a skipped result if the tier has already failed.
    fn run(&mut self, name: &'static str, title: &'static str, f: impl FnOnce() -> CheckResult) {
        if self.has_failed() {
            self.skip(name, title);
        } else {
            self.checks.push(f());
        }
    }

    /// Run a check that produces multiple results, or emit skipped results for each.
    fn run_many(
        &mut self,
        skip_entries: &[(&'static str, &'static str)],
        f: impl FnOnce() -> Vec<CheckResult>,
    ) {
        if self.has_failed() {
            self.skip_many(skip_entries);
        } else {
            self.checks.extend(f());
        }
    }

    /// Emit a skipped result unconditionally — used when the caller already knows the check cannot run.
    fn skip(&mut self, name: &'static str, title: &'static str) {
        self.checks
            .push(skipped_check(self.tier, name, title, &self.skip_reason));
    }

    /// Emit skipped results for several checks at once.
    fn skip_many(&mut self, entries: &[(&'static str, &'static str)]) {
        self.checks
            .extend(skipped_tier(self.tier, entries, &self.skip_reason));
    }

    /// Push a result directly (for checks that need special handling).
    fn push(&mut self, result: CheckResult) {
        self.checks.push(result);
    }

    /// Consume and return all collected checks.
    fn finish(self) -> Vec<CheckResult> {
        self.checks
    }
}

/// Generate a single Skipped result.
pub(crate) fn skipped_check(
    tier: CheckTier,
    name: &'static str,
    title: &'static str,
    reason: &str,
) -> CheckResult {
    CheckResult {
        tier,
        name,
        title,
        status: CheckStatus::Skipped,
        message: reason.to_string(),
        suggestion: None,
    }
}

/// Generate Skipped results for an entire tier.
pub(crate) fn skipped_tier(
    tier: CheckTier,
    entries: &[(&'static str, &'static str)],
    reason: &str,
) -> Vec<CheckResult> {
    entries
        .iter()
        .map(|(name, title)| CheckResult {
            tier,
            name,
            title,
            status: CheckStatus::Skipped,
            message: reason.to_string(),
            suggestion: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a Pass result for use in tests
    fn pass(tier: CheckTier, name: &'static str) -> CheckResult {
        CheckResult {
            tier,
            name,
            title: name,
            status: CheckStatus::Pass,
            message: "ok".to_string(),
            suggestion: None,
        }
    }

    /// Helper: build a Fail result for use in tests
    fn fail(tier: CheckTier, name: &'static str) -> CheckResult {
        CheckResult {
            tier,
            name,
            title: name,
            status: CheckStatus::Fail,
            message: "bad".to_string(),
            suggestion: None,
        }
    }

    /// `skipped_tier` produces one Skipped result per entry, all on the requested tier
    #[test]
    fn test_skipped_tier_generates_correct_count() {
        let results = skipped_tier(
            CheckTier::Auth,
            &[("a", "A title"), ("b", "B title"), ("c", "C title")],
            "skipped",
        );
        assert_eq!(results.len(), 3);
        for r in &results {
            assert_eq!(r.tier, CheckTier::Auth);
            assert_eq!(r.status, CheckStatus::Skipped);
        }
    }

    /// `has_failure_in_tier` is scoped — a failure in one tier does not register against another
    #[test]
    fn test_has_failure_in_tier_detects_failed_checks() {
        let checks = vec![pass(CheckTier::Config, "test")];
        assert!(!has_failure_in_tier(&checks, CheckTier::Config));

        let checks = vec![fail(CheckTier::Config, "test")];
        assert!(has_failure_in_tier(&checks, CheckTier::Config));
        assert!(!has_failure_in_tier(&checks, CheckTier::Auth));
    }

    /// A new runner starts with no collected results and reports `has_failed = false`
    #[test]
    fn test_tier_runner_starts_empty() {
        let runner = TierRunner::new(CheckTier::Config, "skipped");
        assert!(!runner.has_failed());
        assert!(runner.finish().is_empty());
    }

    /// `push` collects results in order and a pushed Fail flips `has_failed` to true
    #[test]
    fn test_tier_runner_push_tracks_failure_state() {
        let mut runner = TierRunner::new(CheckTier::Config, "skipped");
        runner.push(pass(CheckTier::Config, "first"));
        assert!(!runner.has_failed());
        runner.push(fail(CheckTier::Config, "second"));
        assert!(runner.has_failed());
        assert_eq!(runner.finish().len(), 2);
    }

    /// `run` executes the closure when no prior failure has occurred
    #[test]
    fn test_tier_runner_run_executes_when_clean() {
        let mut runner = TierRunner::new(CheckTier::Config, "skipped");
        runner.run("name", "Title", || pass(CheckTier::Config, "name"));
        let results = runner.finish();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, CheckStatus::Pass);
    }

    /// `run` emits a Skipped without invoking the closure once the tier has a failure
    #[test]
    fn test_tier_runner_run_skips_after_failure() {
        let mut runner = TierRunner::new(CheckTier::Config, "skipped");
        runner.push(fail(CheckTier::Config, "boom"));
        runner.run("never", "Never", || {
            panic!("closure must not run after a failure")
        });
        let results = runner.finish();
        assert_eq!(results.len(), 2);
        assert_eq!(results[1].status, CheckStatus::Skipped);
        assert_eq!(results[1].name, "never");
        assert_eq!(results[1].message, "skipped");
    }

    /// `run_many` runs the closure exactly once, extending the collected results
    #[test]
    fn test_tier_runner_run_many_executes_when_clean() {
        let mut runner = TierRunner::new(CheckTier::Config, "skipped");
        runner.run_many(&[("a", "A"), ("b", "B")], || {
            vec![pass(CheckTier::Config, "a"), pass(CheckTier::Config, "b")]
        });
        assert_eq!(runner.finish().len(), 2);
    }

    /// `run_many` skips every entry without invoking the closure once the tier has a failure
    #[test]
    fn test_tier_runner_run_many_skips_after_failure() {
        let mut runner = TierRunner::new(CheckTier::Config, "skipped");
        runner.push(fail(CheckTier::Config, "boom"));
        runner.run_many(&[("a", "A"), ("b", "B")], || {
            panic!("closure must not run after a failure")
        });
        let results = runner.finish();
        assert_eq!(results.len(), 3);
        assert_eq!(results[1].status, CheckStatus::Skipped);
        assert_eq!(results[2].status, CheckStatus::Skipped);
    }

    /// `skip` and `skip_many` emit Skipped results regardless of prior failure state
    #[test]
    fn test_tier_runner_skip_emits_unconditionally() {
        let mut runner = TierRunner::new(CheckTier::Auth, "offline mode");
        runner.skip("a", "A");
        runner.skip_many(&[("b", "B"), ("c", "C")]);
        let results = runner.finish();
        assert_eq!(results.len(), 3);
        for r in &results {
            assert_eq!(r.status, CheckStatus::Skipped);
            assert_eq!(r.tier, CheckTier::Auth);
            assert_eq!(r.message, "offline mode");
        }
    }
}
