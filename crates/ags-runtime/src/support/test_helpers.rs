//! Test-only helpers shared across in-source `#[cfg(test)]` modules.
//!
//! Integration tests under `tests/` cannot reach this module — they use the
//! parallel helpers in `tests/common/env_guard.rs` instead. Both copies
//! intentionally implement the same RAII semantics; keep them in sync when
//! either side changes.

use std::time::{SystemTime, UNIX_EPOCH};

/// RAII guard restoring an environment variable when dropped.
///
/// Captures the prior value on construction and restores it (or removes the
/// var if it was previously unset) when the guard goes out of scope.
pub struct TempEnvGuard {
    key: &'static str,
    original: Option<String>,
}

impl TempEnvGuard {
    /// Set the env var to `value`, capturing the prior value for restoration.
    pub fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, original }
    }

    /// Unset the env var, capturing the prior value for restoration.
    #[allow(dead_code)]
    pub fn remove(key: &'static str) -> Self {
        let original = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, original }
    }
}

impl Drop for TempEnvGuard {
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
