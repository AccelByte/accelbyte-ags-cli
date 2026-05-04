//! Cross-process token locking for auth state mutations and refreshes.

use base64::Engine as _;

use crate::protocol::error::{RuntimeError, RuntimeErrorKind};
use crate::runtime::config;
use crate::support::FileLock;

/// Lock path used to serialize token refresh and persistence.
///
/// When the keychain is in play the lock lives in the system-wide
/// `shared_lock_dir()` so concurrent processes with different `AGS_HOME`
/// overrides still serialise on the same shared keychain entry. When
/// `AGS_NO_KEYCHAIN=1` the keychain is never touched, all storage is
/// file-scoped to `AGS_HOME`, and the lock can scope there too — that
/// isolates concurrent processes that point at different `AGS_HOME`
/// values (notably the test suite, which sets a fresh `AGS_HOME` per
/// invocation but used to contend on one global lock file).
pub fn token_lock_path(profile: &str) -> Result<std::path::PathBuf, RuntimeError> {
    let encoded_profile = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(profile);
    let dir = if config::is_keychain_disabled() {
        config::config_dir()?.join("locks")
    } else {
        config::shared_lock_dir()?
    };
    Ok(dir.join(format!("{encoded_profile}.token.lock")))
}

/// Run one synchronous token mutation while holding the per-profile token lock.
pub fn with_token_lock<T>(
    profile: &str,
    f: impl FnOnce() -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    let _lock = acquire_token_lock(profile)?;
    f()
}

/// Run one synchronous token mutation while holding every listed profile token lock.
///
/// Lock paths are sorted and deduplicated before acquisition so multiple
/// callers always grab locks in the same global order, avoiding deadlocks
/// when two concurrent operations target overlapping profile sets. Any new
/// caller that acquires multiple token locks must use this helper rather
/// than acquiring `with_token_lock` calls in an arbitrary order.
pub fn with_token_locks<T>(
    profiles: &[&str],
    f: impl FnOnce() -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    let mut lock_paths = profiles
        .iter()
        .map(|profile| token_lock_path(profile))
        .collect::<Result<Vec<_>, _>>()?;
    lock_paths.sort();
    lock_paths.dedup();

    let mut locks = Vec::with_capacity(lock_paths.len());
    for path in lock_paths {
        locks.push(lock_from_path(&path)?);
    }

    let result = f();
    drop(locks);
    result
}

/// Acquire the per-profile token lock on a blocking thread so async callers do
/// not park Tokio worker threads while waiting for file-lock contention.
pub async fn acquire_async_token_lock(profile: &str) -> Result<FileLock, RuntimeError> {
    let profile = profile.to_string();
    tokio::task::spawn_blocking(move || acquire_token_lock(&profile))
        .await
        .map_err(|e| config::internal_error(format!("Failed to join token lock task: {e}")))?
}

/// Acquire the per-profile token lock for the duration of one synchronous operation.
fn acquire_token_lock(profile: &str) -> Result<FileLock, RuntimeError> {
    lock_from_path(&token_lock_path(profile)?)
}

/// Acquire one token lock from a fully resolved path.
fn lock_from_path(path: &std::path::Path) -> Result<FileLock, RuntimeError> {
    FileLock::acquire(path, "token store").map_err(|e| RuntimeError {
        kind: RuntimeErrorKind::Internal,
        message: e.to_string(),
        details: None,
        hint: None,
        trace: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The two tests below exercise the two branches of `token_lock_path`:
    // keychain-backed (lock pinned system-wide so concurrent processes
    // serialise on the same shared keychain entry) and file-backed (lock
    // scoped to AGS_HOME so concurrent processes with isolated AGS_HOME
    // values do not contend on a single global lock file).

    /// With keychain available, the token lock path is shared across `AGS_HOME` values so concurrent processes serialize on the same lock
    #[test]
    #[serial_test::serial]
    fn test_token_lock_path_is_stable_across_ags_home_when_keychain_backed() {
        let home_a = tempfile::tempdir().unwrap();
        let home_b = tempfile::tempdir().unwrap();

        std::env::remove_var(crate::runtime::config::ENV_NO_KEYCHAIN);
        std::env::set_var(crate::runtime::config::ENV_HOME, home_a.path());
        let path_a = token_lock_path("default").unwrap();

        std::env::set_var(crate::runtime::config::ENV_HOME, home_b.path());
        let path_b = token_lock_path("default").unwrap();

        assert_eq!(path_a, path_b);

        std::env::remove_var(crate::runtime::config::ENV_HOME);
    }

    /// Duplicate profile names must be deduplicated before lock acquisition;
    /// without dedup, the second `lock_from_path` call on the same path would
    /// deadlock the thread waiting for an exclusive flock it already holds.
    #[test]
    #[serial_test::serial]
    fn test_with_token_locks_deduplicates_identical_profiles() {
        let home = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, home.path());

        let result = with_token_locks(&["default", "default"], || Ok::<(), RuntimeError>(()));
        assert!(result.is_ok());

        std::env::remove_var(crate::runtime::config::ENV_HOME);
    }

    /// With the keychain disabled via `AGS_NO_KEYCHAIN`, the lock path scopes to `AGS_HOME` so isolated processes do not contend on one global lock
    #[test]
    #[serial_test::serial]
    fn test_token_lock_path_scopes_to_ags_home_when_keychain_disabled() {
        let home_a = tempfile::tempdir().unwrap();
        let home_b = tempfile::tempdir().unwrap();

        std::env::set_var(crate::runtime::config::ENV_NO_KEYCHAIN, "1");
        std::env::set_var(crate::runtime::config::ENV_HOME, home_a.path());
        let path_a = token_lock_path("default").unwrap();

        std::env::set_var(crate::runtime::config::ENV_HOME, home_b.path());
        let path_b = token_lock_path("default").unwrap();

        assert_ne!(path_a, path_b);
        assert!(path_a.starts_with(home_a.path()));
        assert!(path_b.starts_with(home_b.path()));

        std::env::remove_var(crate::runtime::config::ENV_HOME);
        std::env::remove_var(crate::runtime::config::ENV_NO_KEYCHAIN);
    }
}
