//! Filesystem path derivation for runtime config, profiles, cache, and locks.

use std::path::PathBuf;

use crate::protocol::error::RuntimeError;
use crate::runtime::config::{internal_error, ENV_HOME};

// ── Directory helpers ──

/// Platform-specific app directory suffix
fn app_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("com.accelbyte.ags")
    } else if cfg!(target_os = "windows") {
        PathBuf::from("AccelByte").join("AGS")
    } else {
        PathBuf::from("ags")
    }
}

/// Base directory override — when set, all state goes here
fn home_override() -> Option<PathBuf> {
    std::env::var(ENV_HOME).ok().map(PathBuf::from)
}

/// Resolve the platform-specific AGS state root, ignoring any `AGS_HOME` override.
fn platform_config_root() -> Result<PathBuf, RuntimeError> {
    resolve_platform_config_root(dirs::config_dir(), dirs::home_dir())
}

/// Resolve the platform-specific root for AGS state, with a USERPROFILE-style
/// fallback when `dirs::config_dir()` cannot determine a location.
///
/// On Windows, `dirs::config_dir()` resolves to `%APPDATA%`. If that is
/// unavailable (rare; happens on unusual installations), this function falls
/// back to `<home>/<app_path>` — the same `app_path()` suffix used under the
/// normal lookup, so a user whose `%APPDATA%` later becomes available finds
/// their existing config at the equivalent absolute location. Returning
/// `Err` requires that both lookups fail.
fn resolve_platform_config_root(
    config_dir: Option<PathBuf>,
    home_dir: Option<PathBuf>,
) -> Result<PathBuf, RuntimeError> {
    if let Some(d) = config_dir {
        return Ok(d.join(app_path()));
    }
    if let Some(h) = home_dir {
        return Ok(h.join(app_path()));
    }
    Err(internal_error("Cannot determine config directory."))
}

/// Resolve the configuration directory, respecting AGS_HOME override
pub fn config_dir() -> Result<PathBuf, RuntimeError> {
    if let Some(home) = home_override() {
        return Ok(home);
    }
    platform_config_root()
}

/// Resolve the cache directory, respecting AGS_HOME override
pub fn cache_dir() -> Result<PathBuf, RuntimeError> {
    if let Some(home) = home_override() {
        return Ok(home.join("cache"));
    }
    dirs::cache_dir()
        .ok_or_else(|| internal_error("Cannot determine cache directory."))
        .map(|d| d.join(app_path()))
}

/// Directory for process-wide lock files that must remain stable even when
/// `AGS_HOME` overrides the normal config/data directories.
pub(crate) fn shared_lock_dir() -> Result<PathBuf, RuntimeError> {
    Ok(platform_config_root()?.join("locks"))
}

/// Path to the advisory lock guarding writes to the global config file.
pub(crate) fn global_config_lock_path() -> Result<PathBuf, RuntimeError> {
    Ok(config_dir()?.join(".config.lock"))
}

/// Path to the advisory lock guarding writes to a profile's config file.
pub(crate) fn profile_config_lock_path(profile: &str) -> Result<PathBuf, RuntimeError> {
    Ok(profile_dir(profile)?.join(".config.lock"))
}

/// Directory containing all profile subdirectories
pub fn profiles_dir() -> Result<PathBuf, RuntimeError> {
    Ok(config_dir()?.join("profiles"))
}

/// Directory for a specific profile
pub fn profile_dir(name: &str) -> Result<PathBuf, RuntimeError> {
    Ok(profiles_dir()?.join(name))
}

/// Path to a profile's config.json
pub fn profile_config_path(name: &str) -> Result<PathBuf, RuntimeError> {
    Ok(profile_dir(name)?.join("config.json"))
}

#[cfg(test)]
mod tests {
    use super::resolve_platform_config_root;
    use std::path::PathBuf;

    /// When the platform exposes a config directory, the resolver uses it as the root
    #[test]
    fn test_resolve_uses_config_dir_when_present() {
        let config = Some(PathBuf::from("/some/config"));
        let home = Some(PathBuf::from("/some/home"));
        let resolved = resolve_platform_config_root(config, home).unwrap();
        let expected = PathBuf::from("/some/config").join(super::app_path());
        assert_eq!(resolved, expected);
    }

    /// Missing config dir falls back to `<home>/.ags` so Windows installs without `%APPDATA%` still work
    #[test]
    fn test_resolve_falls_back_to_home_dir_when_config_dir_missing() {
        let resolved =
            resolve_platform_config_root(None, Some(PathBuf::from("/some/home"))).unwrap();
        let expected = PathBuf::from("/some/home").join(super::app_path());
        assert_eq!(resolved, expected);
    }

    /// Both config and home dir missing surfaces an internal error rather than silently picking a default
    #[test]
    fn test_resolve_errors_when_both_missing() {
        let err = resolve_platform_config_root(None, None).unwrap_err();
        assert!(err.message.contains("Cannot determine config directory"));
    }
}
