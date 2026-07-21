//! Shared test helpers for scoped env-var manipulation and clock readings.

use std::time::{SystemTime, UNIX_EPOCH};

/// Scoped guard that sets an env var on construction and restores its prior
/// value on drop. Keeps the test process (and any spawned subprocess) pointed
/// at the same `AGS_HOME` / `AGS_NO_KEYCHAIN` for the duration of one test.
pub struct TempEnvGuard {
    /// The env var name being managed.
    key: &'static str,
    /// The original value present before this guard was constructed, or
    /// `None` if the variable was unset.
    original: Option<String>,
}

impl TempEnvGuard {
    /// Set the env var to `value`, capturing the prior value for restoration
    /// when the guard is dropped.
    pub fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, original }
    }

    /// Unset the env var, capturing the prior value for restoration when the
    /// guard is dropped.
    pub fn remove(key: &'static str) -> Self {
        let original = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, original }
    }
}

impl Drop for TempEnvGuard {
    /// Restore the env var to its prior state when the guard goes out of scope.
    fn drop(&mut self) {
        match &self.original {
            Some(val) => std::env::set_var(self.key, val),
            None => std::env::remove_var(self.key),
        }
    }
}

/// Current wall-clock seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
