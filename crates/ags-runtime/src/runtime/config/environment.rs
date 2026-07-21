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
