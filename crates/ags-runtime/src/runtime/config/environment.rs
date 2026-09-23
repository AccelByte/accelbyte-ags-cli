//! Environment variables and default runtime configuration values.

/// Env var: bearer token that bypasses the normal auth flow
pub const ENV_ACCESS_TOKEN: &str = "AGS_ACCESS_TOKEN";
/// Env var: AccelByte base URL (e.g. `https://demo.accelbyte.io`)
pub const ENV_BASE_URL: &str = "AGS_BASE_URL";
/// Env var: OAuth2 client ID for authentication
pub const ENV_CLIENT_ID: &str = "AGS_CLIENT_ID";
/// Env var: OAuth2 client secret for client-credentials flow
pub const ENV_CLIENT_SECRET: &str = "AGS_CLIENT_SECRET";
/// Env var: override directory for all config, data, and cache state
pub const ENV_HOME: &str = "AGS_HOME";
/// Env var: default namespace sent with API requests
pub const ENV_NAMESPACE: &str = "AGS_NAMESPACE";
/// Env var: timeout in seconds for the browser-based auth flow
pub const ENV_AUTH_TIMEOUT: &str = "AGS_AUTH_TIMEOUT";
/// Env var: when set, disables OS keychain and falls back to file-based token storage
pub const ENV_NO_KEYCHAIN: &str = "AGS_NO_KEYCHAIN";
/// Env var: select active profile without modifying global config
pub const ENV_PROFILE: &str = "AGS_PROFILE";

/// Env var: when set to `"1"`, disables the background update check entirely
pub const ENV_NO_UPDATE_CHECK: &str = "AGS_NO_UPDATE_CHECK";

/// Env var: override the update-check endpoint URL (test hook — not for
/// end-user use). When set, the background `__update-check` child targets
/// this URL instead of the real GitHub releases endpoint.
pub const ENV_UPDATE_CHECK_URL: &str = "AGS_UPDATE_CHECK_URL";

/// Env var: override the installer base URL (test hook — not for end-user
/// use). When set, `ags update --install` downloads the installer script
/// from this URL instead of the real GitHub releases endpoint.
pub const ENV_UPDATE_INSTALLER_URL: &str = "AGS_UPDATE_INSTALLER_URL";

/// Env var: override the health-check timeout in seconds (test hook — not
/// for end-user use). Shortens the 15-second limit the install step gives
/// the new binary to answer `version`.
pub const ENV_UPDATE_HEALTH_TIMEOUT_SECS: &str = "AGS_UPDATE_HEALTH_TIMEOUT_SECS";

/// Built-in profile name used when no profile is explicitly configured
pub const DEFAULT_PROFILE: &str = "default";

/// Returns true when `AGS_NO_KEYCHAIN=1` is set, disabling OS keychain use.
///
/// Only the literal string `"1"` activates the override — any other value
/// (including `"true"`, `"yes"`, or unset) leaves the keychain enabled.
pub fn is_keychain_disabled() -> bool {
    std::env::var(ENV_NO_KEYCHAIN).is_ok_and(|value| value == "1")
}

/// Returns true when the named environment variable is set to a non-empty string.
///
/// Treats unset and empty-string values identically — callers generally want
/// "user supplied a value", and an explicit empty value is indistinguishable
/// from omission for our purposes.
pub fn is_env_var_set(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| !value.is_empty())
}

/// Returns true when `AGS_NO_UPDATE_CHECK=1` is set, disabling the update check.
///
/// Only the literal string `"1"` activates it — mirrors [`is_keychain_disabled`].
pub fn is_update_check_disabled() -> bool {
    std::env::var(ENV_NO_UPDATE_CHECK).is_ok_and(|value| value == "1")
}

/// Returns the override URL for the update-check endpoint, if set.
///
/// This is a test hook that redirects the background `__update-check` child
/// to a mock server, so the functional test suite can verify the
/// fetch→cache→child-process path without hitting `api.github.com`.
pub fn update_check_url_override() -> Option<String> {
    std::env::var(ENV_UPDATE_CHECK_URL)
        .ok()
        .filter(|s| !s.is_empty())
}

/// Returns the override installer base URL, if set.
///
/// This is a test hook that redirects `ags update --install` to a mock
/// server, so the functional test suite can verify the download and install
/// path without hitting GitHub.
pub fn update_installer_url_override() -> Option<String> {
    std::env::var(ENV_UPDATE_INSTALLER_URL)
        .ok()
        .filter(|s| !s.is_empty())
}

/// Returns the override health-check timeout in seconds, if set and valid.
///
/// Parses the value as `u64`; returns `None` when unset, empty, or not a
/// number.
pub fn update_health_timeout_override() -> Option<u64> {
    std::env::var(ENV_UPDATE_HEALTH_TIMEOUT_SECS)
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
}

pub fn is_ci() -> bool {
    is_env_var_set("CI")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    use crate::support::test_helpers::TempEnvGuard;

    #[test]
    #[serial]
    fn update_check_disabled_only_on_literal_1() {
        let _g = TempEnvGuard::set(ENV_NO_UPDATE_CHECK, "1");
        assert!(is_update_check_disabled());
        std::env::set_var(ENV_NO_UPDATE_CHECK, "true"); // NOT "1" -> ignored
        assert!(!is_update_check_disabled());
        std::env::remove_var(ENV_NO_UPDATE_CHECK);
        assert!(!is_update_check_disabled());
    }

    #[test]
    #[serial]
    fn ci_detected_only_when_set_nonempty() {
        let _g = TempEnvGuard::set("CI", "true");
        assert!(is_ci());
        std::env::set_var("CI", ""); // empty == unset, per is_env_var_set
        assert!(!is_ci());
        std::env::remove_var("CI");
        assert!(!is_ci());
    }
}
