//! Individual diagnostic check functions for `ags doctor`.
//!
//! Each function returns a [`CheckResult`] (or `Vec<CheckResult>` for
//! checks that produce multiple entries). They are invoked by
//! [`super::runner::run_profile`] in tier order, with `TierRunner`
//! coordinating skip-on-failure logic.

use crate::runtime::auth::session;
use crate::runtime::auth::store;
use crate::runtime::config::{self, ProfileConfig};
use crate::support::unix_now;

use crate::protocol::diagnostics::{CheckResult, CheckStatus, CheckTier};

/// Identity of a single diagnostic check — tier, machine-readable name, and human-readable title.
///
/// Each check function declares one (or more) of these as a function-local
/// `const` and uses the `pass`/`fail`/`warning`/`skipped` builders so the
/// `tier`/`name`/`title` triple is written exactly once per identity.
struct CheckId {
    tier: CheckTier,
    name: &'static str,
    title: &'static str,
}

impl CheckId {
    /// Build a `Pass` result for this check identity.
    fn pass(&self, message: impl Into<String>) -> CheckResult {
        CheckResult {
            tier: self.tier,
            name: self.name,
            title: self.title,
            status: CheckStatus::Pass,
            message: message.into(),
            suggestion: None,
        }
    }

    /// Build a `Fail` result with a remediation suggestion.
    fn fail(&self, message: impl Into<String>, suggestion: impl Into<String>) -> CheckResult {
        CheckResult {
            tier: self.tier,
            name: self.name,
            title: self.title,
            status: CheckStatus::Fail,
            message: message.into(),
            suggestion: Some(suggestion.into()),
        }
    }

    /// Build a `Warning` result with no actionable suggestion attached.
    fn warning(&self, message: impl Into<String>) -> CheckResult {
        CheckResult {
            tier: self.tier,
            name: self.name,
            title: self.title,
            status: CheckStatus::Warning,
            message: message.into(),
            suggestion: None,
        }
    }

    /// Build a `Warning` result paired with a remediation hint.
    fn warning_with_hint(
        &self,
        message: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> CheckResult {
        CheckResult {
            tier: self.tier,
            name: self.name,
            title: self.title,
            status: CheckStatus::Warning,
            message: message.into(),
            suggestion: Some(suggestion.into()),
        }
    }

    /// Build a `Skipped` result — used when a check cannot be run given current state.
    fn skipped(&self, message: impl Into<String>) -> CheckResult {
        CheckResult {
            tier: self.tier,
            name: self.name,
            title: self.title,
            status: CheckStatus::Skipped,
            message: message.into(),
            suggestion: None,
        }
    }
}

/// Truncate a client ID for display (first 8 + last 7 chars).
fn truncate_id(id: &str) -> String {
    if id.len() > 18 {
        format!("{}...{}", &id[..8], &id[id.len() - 7..])
    } else {
        id.to_string()
    }
}

// ── Tier 1: Config checks ──

/// Verify that the profile directory exists on disk (the precondition for all later checks).
pub(crate) fn assess_profile_selection(profile: &str) -> CheckResult {
    const ID: CheckId = CheckId {
        tier: CheckTier::Config,
        name: "profile-selection",
        title: "Profile exists",
    };

    let dir = match config::profile_dir(profile) {
        Ok(d) => d,
        Err(_) => {
            return ID.fail(
                "Cannot determine profile directory",
                "Check AGS_HOME or default config path",
            )
        }
    };

    if dir.is_dir() {
        ID.pass(format!("{} found", profile))
    } else {
        ID.fail(
            format!("{} not found", profile),
            format!("Run 'ags profile create {}'", profile),
        )
    }
}

/// Load and parse the profile's config file, returning the parsed value alongside the result.
///
/// The parsed `ProfileConfig` is returned to the caller so downstream checks
/// (base URL, client ID, namespace) can reuse it without re-reading the file.
pub(crate) fn assess_config_validity(profile: &str) -> (CheckResult, Option<ProfileConfig>) {
    const ID: CheckId = CheckId {
        tier: CheckTier::Config,
        name: "config-validity",
        title: "Config file valid",
    };

    match ProfileConfig::load(profile) {
        Ok(config) => (
            ID.pass(format!("{} config parses as JSON", profile)),
            Some(config),
        ),
        Err(e) => (
            ID.fail(
                format!("{} config is invalid: {e}", profile),
                "Edit or recreate the profile config file",
            ),
            None,
        ),
    }
}

/// Verify that base URL and client ID are present and well-formed, with env vars overriding config.
///
/// Returns a result for each. The client-ID check is skipped when the base URL
/// is missing or invalid, since fixing the URL is a prerequisite for using the ID.
pub(crate) fn assess_config_completeness(profile_config: &ProfileConfig) -> Vec<CheckResult> {
    const BASE_URL: CheckId = CheckId {
        tier: CheckTier::Config,
        name: "config-base-url",
        title: "Base URL",
    };
    const CLIENT_ID: CheckId = CheckId {
        tier: CheckTier::Config,
        name: "config-client-id",
        title: "Client ID",
    };

    let mut checks = Vec::new();

    let (base_url, base_url_source) = match std::env::var(config::ENV_BASE_URL)
        .ok()
        .filter(|v| !v.is_empty())
    {
        Some(v) => (Some(v), "from AGS_BASE_URL"),
        None => (profile_config.base_url.clone(), "from config"),
    };
    let (client_id, client_id_source) = match std::env::var(config::ENV_CLIENT_ID)
        .ok()
        .filter(|v| !v.is_empty())
    {
        Some(v) => (Some(v), "from AGS_CLIENT_ID"),
        None => (profile_config.client_id.clone(), "from config"),
    };

    let has_base_url_failed;
    checks.push(match &base_url {
        Some(url) => {
            if !config::is_valid_base_url(url) {
                has_base_url_failed = true;
                BASE_URL.fail(
                    format!("invalid URL: {url}"),
                    "Run 'ags config set base-url <url>' with a valid URL",
                )
            } else {
                has_base_url_failed = false;
                BASE_URL.pass(format!("{url} ({base_url_source})"))
            }
        }
        None => {
            has_base_url_failed = true;
            BASE_URL.fail("not set", "Run 'ags config set base-url <url>'")
        }
    });

    if has_base_url_failed {
        checks.push(CLIENT_ID.skipped("skipped"));
    } else {
        checks.push(match &client_id {
            Some(id) => {
                if !config::is_valid_client_id(id) {
                    CLIENT_ID.fail(
                        "invalid format (expected 32-char hex)",
                        "Run 'ags config set client-id <id>' with a valid client ID",
                    )
                } else {
                    let display_id = truncate_id(id);
                    CLIENT_ID.pass(format!("{display_id} ({client_id_source})"))
                }
            }
            None => CLIENT_ID.fail("not set", "Run 'ags config set client-id <id>'"),
        });
    }

    checks
}

/// Identity of the file-permissions check; shared by [`assess_file_permissions`] and its resolver helper.
const FILE_PERMISSIONS: CheckId = CheckId {
    tier: CheckTier::Config,
    name: "file-permissions",
    title: "Config file permissions",
};

/// Verify that the profile config file is not group- or world-readable (Unix) / lives under the user profile (Windows).
pub(crate) fn assess_file_permissions(profile: &str) -> CheckResult {
    let config_path = match resolve_existing_config_path(profile) {
        Ok(path) => path,
        Err(early) => return early,
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        match std::fs::metadata(&config_path) {
            Ok(meta) => {
                let mode = meta.mode() & 0o777;
                if mode & 0o077 == 0 {
                    FILE_PERMISSIONS.pass(format!("owner-only ({:04o})", mode))
                } else {
                    let group = mode & 0o070 != 0;
                    let world = mode & 0o007 != 0;
                    let access = match (group, world) {
                        (true, true) => "group- and world-readable",
                        (true, false) => "group-readable",
                        (false, true) => "world-readable",
                        (false, false) => unreachable!(),
                    };
                    FILE_PERMISSIONS.warning_with_hint(
                        format!("{access} ({:04o})", mode),
                        format!("Run 'chmod 600 {}'", config_path.display()),
                    )
                }
            }
            Err(e) => FILE_PERMISSIONS.warning(format!("Cannot read file permissions: {e}")),
        }
    }

    #[cfg(not(unix))]
    {
        let profile_root = match dirs::home_dir() {
            Some(p) => p,
            None => return FILE_PERMISSIONS.warning("Cannot determine user profile root"),
        };

        let canonical_path = match std::fs::canonicalize(&config_path) {
            Ok(p) => p,
            Err(e) => {
                return FILE_PERMISSIONS.warning(format!("Cannot canonicalize config path: {e}"))
            }
        };

        let canonical_root = match std::fs::canonicalize(&profile_root) {
            Ok(p) => p,
            Err(e) => {
                return FILE_PERMISSIONS
                    .warning(format!("Cannot canonicalize user profile root: {e}"))
            }
        };

        if is_user_profile_protected(&canonical_path, &canonical_root) {
            FILE_PERMISSIONS.pass("user-profile-protected")
        } else {
            FILE_PERMISSIONS.warning_with_hint(
                format!(
                    "file is outside the user profile: {}",
                    canonical_path.display()
                ),
                "Move AGS_HOME to a location under your user profile so credential files \
                 inherit user-only ACLs.",
            )
        }
    }
}

/// Resolve the credential file path for `assess_file_permissions`. Returns the
/// path on success, or a fully-formed `CheckResult` for the early-return cases
/// (path could not be derived, or the file does not exist).
fn resolve_existing_config_path(profile: &str) -> Result<std::path::PathBuf, CheckResult> {
    let path = config::profile_config_path(profile)
        .map_err(|_| FILE_PERMISSIONS.warning("Cannot determine config path"))?;

    if !path.exists() {
        return Err(FILE_PERMISSIONS.skipped("no config file found"));
    }

    Ok(path)
}

/// Surface any active `AGS_*` environment overrides as a warning so users notice them in local dev.
///
/// Returns an empty vector when no overrides are set — the check only emits a
/// result when there is something worth flagging.
pub(crate) fn assess_env_var_overrides() -> Vec<CheckResult> {
    const ID: CheckId = CheckId {
        tier: CheckTier::Config,
        name: "env-var-overrides",
        title: "Env var overrides",
    };

    let config_vars: &[(&str, &str)] = &[
        (config::ENV_BASE_URL, "AGS_BASE_URL"),
        (config::ENV_CLIENT_ID, "AGS_CLIENT_ID"),
        (config::ENV_NAMESPACE, "AGS_NAMESPACE"),
    ];
    let credential_vars: &[&str] = &[config::ENV_CLIENT_SECRET, config::ENV_ACCESS_TOKEN];

    let mut config_overrides: Vec<&str> = config_vars
        .iter()
        .filter(|(var, _)| config::is_env_var_set(var))
        .map(|(_, label)| *label)
        .collect();
    if config::is_keychain_disabled() {
        config_overrides.push("AGS_NO_KEYCHAIN");
    }
    let credential_count = credential_vars
        .iter()
        .filter(|var| config::is_env_var_set(var))
        .count();

    if config_overrides.is_empty() && credential_count == 0 {
        return Vec::new();
    }

    let mut parts = Vec::new();
    if !config_overrides.is_empty() {
        parts.push(config_overrides.join(", "));
    }
    if credential_count > 0 {
        parts.push(format!(
            "{credential_count} credential {}",
            if credential_count == 1 { "var" } else { "vars" }
        ));
    }

    vec![ID.warning_with_hint(
        format!("environment overrides active: {}", parts.join(", ")),
        "These override profile config values — expected in CI, check in local dev",
    )]
}

// ── Tier 2: Auth checks ──

/// Probe the OS keychain with a unique read-only entry to confirm credential storage is reachable.
pub(crate) fn assess_keychain_access() -> CheckResult {
    const ID: CheckId = CheckId {
        tier: CheckTier::Auth,
        name: "keychain-access",
        title: "Keychain accessible",
    };

    if config::is_keychain_disabled() {
        return ID.pass("Disabled (using file fallback)");
    }

    // Read-only probe with a unique key per invocation to avoid collisions
    let probe_id = format!("ags-doctor-probe-{}", std::process::id());
    match keyring::Entry::new(&probe_id, &probe_id) {
        Ok(entry) => {
            // Try a read — we expect NoEntry which proves the keychain is accessible
            match entry.get_password() {
                Ok(_) | Err(keyring::Error::NoEntry) => ID.pass("OS keychain is accessible"),
                Err(e) => ID.warning_with_hint(
                    format!("probe failed, using file fallback: {e}"),
                    "Set AGS_NO_KEYCHAIN=1 to suppress this warning",
                ),
            }
        }
        // No usable keychain backend at init: NoStorageAccess (e.g. WSL2
        // without D-Bus) or PlatformFailure (e.g. keyutils ENOSYS in a
        // container). File fallback is automatic — not an error condition.
        Err(e) if store::is_keychain_init_unavailable(&e) => {
            ID.pass("No keychain backend (using file fallback)")
        }
        Err(e) => ID.warning_with_hint(
            format!("unavailable, using file fallback: {e}"),
            "Set AGS_NO_KEYCHAIN=1 to suppress this warning",
        ),
    }
}

/// Resolve which credential source the profile will use, walking the same precedence chain as auth.
///
/// Order: env access token → env client secret → stored client secret → stored
/// browser tokens. Returns `Pass` for the first source found, `Fail` if none.
pub(crate) fn assess_credential_state(profile: &str) -> CheckResult {
    const ID: CheckId = CheckId {
        tier: CheckTier::Auth,
        name: "credential-state",
        title: "Credentials",
    };

    let has_env_secret = config::is_env_var_set(config::ENV_CLIENT_SECRET);
    let has_env_token = config::is_env_var_set(config::ENV_ACCESS_TOKEN);

    // 1. Environment token takes precedence
    if has_env_token {
        return ID.pass("via AGS_ACCESS_TOKEN");
    }

    // 2. Environment client secret
    if has_env_secret {
        return ID.pass("client credentials (from AGS_CLIENT_SECRET)");
    }

    // 3. Stored client secret (keychain or file fallback)
    match store::get_secret(profile) {
        Ok(Some(_)) => return ID.pass("client credentials (from keychain)"),
        Ok(None) => {
            // No secret stored — expected for authorization-code profiles. Fall through.
        }
        Err(e) => {
            return ID.fail(
                format!("cannot read client secret: {e}"),
                "Check keychain access or set AGS_NO_KEYCHAIN=1",
            );
        }
    }

    // 4. Stored browser login tokens
    let token_source = token_source_label();
    match store::get_token_data(profile) {
        Ok(Some(_)) => ID.pass(format!("browser login tokens ({token_source})")),
        Ok(None) => ID.fail(
            format!("no credentials found for {}", profile),
            "Run 'ags auth login' to authenticate",
        ),
        Err(e) => ID.fail(
            format!("cannot read token data: {e}"),
            "Run 'ags auth login' to re-authenticate",
        ),
    }
}

/// Label describing where stored tokens are read from, given the current keychain setting.
fn token_source_label() -> &'static str {
    if config::is_keychain_disabled() {
        "from file"
    } else {
        "from keychain"
    }
}

/// Inspect the cached access token's expiry and report time remaining, expiry, or absence.
///
/// Missing tokens warn rather than fail because client-credential profiles
/// obtain a token lazily on the first API call.
pub(crate) fn assess_token_state(profile: &str) -> CheckResult {
    const ID: CheckId = CheckId {
        tier: CheckTier::Auth,
        name: "token-state",
        title: "Access Token",
    };

    if config::is_env_var_set(config::ENV_ACCESS_TOKEN) {
        return ID.pass("present (from AGS_ACCESS_TOKEN)");
    }

    let token_source = token_source_label();

    match store::get_token_data(profile) {
        Ok(Some(token_data)) => {
            let now = unix_now();
            if now < token_data.expires_at {
                let remaining = token_data.expires_at.saturating_sub(now);
                ID.pass(format!(
                    "valid, expires in {} ({token_source})",
                    crate::support::format_duration(remaining)
                ))
            } else {
                let suggestion = if token_data.refresh_token.is_some() {
                    "Token will be refreshed automatically on next API call"
                } else {
                    "Run 'ags auth login' to re-authenticate"
                };
                ID.warning_with_hint("expired", suggestion)
            }
        }
        // Warning, not Fail: client-credential profiles do not pre-store a token;
        // one will be obtained on the first API call.
        Ok(None) => ID.warning_with_hint(
            "not found",
            "Run 'ags auth login' or make an API call to obtain a token",
        ),
        Err(e) => ID.fail(
            format!("cannot read token data: {e}"),
            "Run 'ags auth login' to re-authenticate",
        ),
    }
}

// ── Tier 3: Network checks ──

/// Probe `<base_url>/iam/healthz` to confirm the AccelByte environment is reachable.
///
/// Any non-5xx response (including 403) counts as reachable — only connection
/// errors prove the URL or network is wrong. 5xx is a warning, not a failure,
/// since the environment may simply be in maintenance.
pub(crate) async fn assess_base_url_reachable(
    base_url: &str,
    client: &reqwest::Client,
) -> CheckResult {
    const ID: CheckId = CheckId {
        tier: CheckTier::Network,
        name: "base-url-reachable",
        title: "Base URL reachable",
    };

    // IAM healthz is available on all supported AccelByte environments.
    // Any HTTP response (including 403) proves reachability — only connection
    // failures indicate the URL is wrong or the network is down.
    let probe_url = format!("{}/iam/healthz", base_url.trim_end_matches('/'));

    match client.get(&probe_url).send().await {
        Ok(resp) => {
            let status_code = resp.status();
            if status_code.is_server_error() {
                ID.warning_with_hint(
                    format!("reachable but server returned {}", status_code.as_u16()),
                    "The environment may be degraded or in maintenance",
                )
            } else {
                // Any non-5xx response (including 403) proves the server is reachable and healthy
                ID.pass("connected")
            }
        }
        Err(e) => {
            let msg = if e.is_timeout() {
                "Connection timed out (10s)".to_string()
            } else if e.is_connect() {
                format!("Connection refused: {e}")
            } else {
                format!("Request failed: {e}")
            };
            ID.fail(msg, "Check your network connection and base URL")
        }
    }
}

/// Verify end-to-end auth by resolving an access token, refreshing it if needed.
///
/// This check has a deliberate side effect: when the stored token is expired
/// it triggers a live refresh and persists the new token. That proves the
/// auth chain works and leaves the user's credentials in a ready state for
/// subsequent commands.
pub(crate) async fn assess_token_refresh(profile: &str, client: &reqwest::Client) -> CheckResult {
    const ID: CheckId = CheckId {
        tier: CheckTier::Network,
        name: "server-auth",
        title: "Server auth",
    };

    match session::resolve_access_token(client, profile).await {
        Ok(resolution) => {
            let source_label = match resolution.source {
                session::TokenSource::Environment => "authenticated via environment token",
                session::TokenSource::Stored => "authenticated via stored token",
                session::TokenSource::Refreshed => {
                    "authenticated via token refresh (token updated)"
                }
                session::TokenSource::ClientCredentials => "authenticated via client credentials",
            };
            if !resolution.warnings.is_empty() {
                return ID.warning(format!(
                    "{source_label}; {}",
                    resolution.warnings.join("; ")
                ));
            }
            ID.pass(source_label)
        }
        Err(e) => ID.fail(
            format!("cannot obtain access token: {e}"),
            "Run 'ags auth login' to re-authenticate",
        ),
    }
}

/// Verify the configured namespace satisfies AccelByte's format rules (lowercase alphanumeric, ≤ 48 chars).
pub(crate) fn assess_namespace(profile_config: &ProfileConfig) -> CheckResult {
    const ID: CheckId = CheckId {
        tier: CheckTier::Config,
        name: "namespace-valid",
        title: "Namespace",
    };

    let namespace = std::env::var(config::ENV_NAMESPACE)
        .ok()
        .or(profile_config.namespace.clone());

    match namespace {
        Some(ns) if !ns.is_empty() => {
            if config::is_valid_namespace(&ns) {
                ID.pass(ns.to_string())
            } else {
                ID.fail(
                    format!("invalid format: {ns}"),
                    "Run 'ags config set namespace <ns>' (lowercase alphanumeric, max 48 chars)",
                )
            }
        }
        _ => ID.warning_with_hint(
            "not set",
            "Run 'ags config set namespace <ns>' if commands require it",
        ),
    }
}

/// Return whether `canonical_path` lies under `canonical_profile_root`.
///
/// Both inputs must already be canonical (`std::fs::canonicalize`); this
/// function performs only path-component containment, no I/O. Used by the
/// Windows branch of `assess_file_permissions` to decide whether a credential
/// file inherits user-only ACLs from the user profile.
#[cfg(any(not(unix), test))]
fn is_user_profile_protected(
    canonical_path: &std::path::Path,
    canonical_profile_root: &std::path::Path,
) -> bool {
    canonical_path.starts_with(canonical_profile_root)
}

#[cfg(test)]
mod tests {
    use super::is_user_profile_protected;
    use std::path::Path;

    /// A path nested below the user-profile root counts as protected
    #[test]
    fn test_path_strictly_under_root_is_protected() {
        let root = Path::new("/users/alice");
        let path = Path::new("/users/alice/.config/ags/config.json");
        assert!(is_user_profile_protected(path, root));
    }

    /// The user-profile root itself is treated as protected (boundary case)
    #[test]
    fn test_path_equal_to_root_is_protected() {
        let root = Path::new("/users/alice");
        assert!(is_user_profile_protected(root, root));
    }

    /// Containment is component-wise — a sibling that shares a string prefix is not protected
    #[test]
    fn test_path_with_sibling_prefix_is_not_protected() {
        // Component-aware containment: "/users/alice2" must not be considered
        // under "/users/alice" just because the string prefix matches.
        let root = Path::new("/users/alice");
        let path = Path::new("/users/alice2/.config/ags/config.json");
        assert!(!is_user_profile_protected(path, root));
    }

    /// A path under an entirely different root is not protected
    #[test]
    fn test_unrelated_path_is_not_protected() {
        let root = Path::new("/users/alice");
        let path = Path::new("/var/lib/ags/config.json");
        assert!(!is_user_profile_protected(path, root));
    }
}
