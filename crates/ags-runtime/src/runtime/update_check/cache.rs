//! On-disk cache for the update check.
//!
//! Records when we last hit GitHub (`checked_at`), the newest version we found
//! (`latest_version`), and which version we've already shown a hint for
//! (`notified_version`). The two version fields are decoupled on purpose:
//! `checked_at` throttles the *network call* (once per 24h); `notified_version`
//! throttles the *hint display* (once per release).
//!
//! Reads and writes are intentionally lock-free best-effort: two concurrent
//! `ags` runs could race the read-modify-write and lose one update, but the
//! worst case is a single redundant GitHub fetch or one extra hint — not worth
//! a cross-process lock for a passive nicety.

use std::path::{Path, PathBuf};

use super::github;
use crate::runtime::config;
use crate::support::file_system;

/// How long a cache entry stays fresh before another network check is allowed.
const STALE_AFTER_SECS: u64 = 24 * 60 * 60; // 24 hours

/// The JSON envelope persisted at `<cache_dir>/update_check.json`.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
struct CacheEnvelope {
    checked_at: u64,
    latest_version: Option<String>,
    notified_version: Option<String>,
}

impl CacheEnvelope {
    /// Whether this entry is old enough that a fresh check is due. `now` is
    /// injected so tests can pin time; clock skew (now < checked_at) reads as fresh.
    fn is_stale(&self, now: u64) -> bool {
        now.saturating_sub(self.checked_at) >= STALE_AFTER_SECS
    }

    /// The hint to show for `current`, or `None` if we've never checked, the
    /// latest isn't newer, or we've already notified this exact version.
    fn hint(&self, current: &str) -> Option<super::UpdateCheckResult> {
        let latest = self.latest_version.as_deref()?;
        let result = github::compare_versions(current, latest)?;
        if !result.is_newer {
            return None;
        }
        if self.notified_version.as_deref() == Some(latest) {
            return None;
        }
        Some(result)
    }
}

// ── Public API (module-internal) ──

/// The hint to show for the running version, reading the on-disk cache.
pub(super) fn cached_hint(current: &str) -> Option<super::UpdateCheckResult> {
    load()?.hint(current)
}

/// Record that a hint was shown for `version`, so it never repeats. Best-effort.
pub(super) fn mark_notified(version: &str) {
    let mut envelope = load().unwrap_or_default();
    envelope.notified_version = Some(version.to_string());
    if let Some(path) = cache_path() {
        let _ = save_to(&path, &envelope);
    }
}

/// Whether a fresh network check is due: no cache yet, or older than 24h.
pub(super) fn is_check_due(now: u64) -> bool {
    match load() {
        Some(envelope) => envelope.is_stale(now),
        None => true,
    }
}

/// Record a freshly-fetched latest version, preserving `notified_version` so a
/// hint we've already shown isn't re-triggered. Best-effort.
pub(super) fn record_latest(latest: &str, now: u64) {
    let mut envelope = load().unwrap_or_default();
    envelope.latest_version = Some(latest.to_string());
    envelope.checked_at = now;
    if let Some(path) = cache_path() {
        let _ = save_to(&path, &envelope);
    }
}

// ── Private disk helpers ──

/// Absolute path to the cache file, or `None` if the cache dir can't resolve.
fn cache_path() -> Option<PathBuf> {
    config::cache_dir()
        .ok()
        .map(|dir| dir.join("update_check.json"))
}

/// Read and parse the envelope from `path`; `None` on any miss. A corrupt file
/// is deleted and treated as a miss (mirrors `catalogue/cache.rs`).
fn load_from(path: &Path) -> Option<CacheEnvelope> {
    let data = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&data) {
        Ok(envelope) => Some(envelope),
        Err(_) => {
            let _ = std::fs::remove_file(path);
            None
        }
    }
}

/// Serialize `envelope` to JSON and write it through the restricted-write helper,
/// creating the parent directory if needed.
fn save_to(path: &Path, envelope: &CacheEnvelope) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        file_system::create_dir_restricted(dir)?;
    }
    let data = serde_json::to_string(envelope).map_err(std::io::Error::other)?;
    file_system::write_file_restricted(path, &data)
}

/// Load the persisted cache from its resolved path, or `None` on any miss.
fn load() -> Option<CacheEnvelope> {
    load_from(&cache_path()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EnvGuard(&'static str);
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            std::env::remove_var(self.0);
        }
    }

    fn envelope_checked_at(ts: u64) -> CacheEnvelope {
        CacheEnvelope {
            checked_at: ts,
            latest_version: None,
            notified_version: None,
        }
    }

    fn envelope(latest: Option<&str>, notified: Option<&str>) -> CacheEnvelope {
        CacheEnvelope {
            checked_at: 0,
            latest_version: latest.map(|s| s.to_string()),
            notified_version: notified.map(|s| s.to_string()),
        }
    }

    #[test]
    fn fresh_entry_is_not_stale() {
        let envelope = envelope_checked_at(1_000_000);
        assert!(!envelope.is_stale(1_000_000 + 3_600));
    }

    #[test]
    fn entry_older_than_24h_is_stale() {
        let envelope = envelope_checked_at(1_000_000);
        assert!(envelope.is_stale(1_000_000 + 25 * 3_600));
    }

    #[test]
    fn exactly_24h_is_stale() {
        let envelope = envelope_checked_at(1_000_000);
        assert!(envelope.is_stale(1_000_000 + 24 * 3_600));
    }

    #[test]
    fn future_checked_at_is_not_stale() {
        let envelope = envelope_checked_at(1_000_000);
        assert!(!envelope.is_stale(999_000));
    }

    #[test]
    fn round_trip_write_then_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("update_check.json");
        let envelope = CacheEnvelope {
            checked_at: 1_732_300_800,
            latest_version: Some("0.5.0".to_string()),
            notified_version: None,
        };

        save_to(&path, &envelope).expect("save should succeed");
        let loaded = load_from(&path).expect("load should return the saved envelope");
        assert_eq!(loaded, envelope)
    }

    #[test]
    fn missing_file_is_a_miss() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("update_check.json");
        assert!(load_from(&path).is_none());
    }

    #[test]
    fn corrupt_file_is_deleted_and_missed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("update_check.json");
        std::fs::write(&path, "not valid json").unwrap();
        assert!(load_from(&path).is_none());
        assert!(!path.exists(), "a corrupt cache file must be deleted");
    }

    #[test]
    fn hint_shown_for_new_unnotified_version() {
        let env = envelope(Some("0.5.0"), None);
        let result = env.hint("0.4.0").expect("newer + un-notified should hint");
        assert_eq!(result.latest, "0.5.0");
        assert!(result.is_newer);
    }

    #[test]
    fn hint_suppressed_once_version_notified() {
        // Already showed 0.5.0 -> don't nag again.
        let env = envelope(Some("0.5.0"), Some("0.5.0"));
        assert!(env.hint("0.4.0").is_none());
    }

    #[test]
    fn hint_reappears_when_newer_version_arrives() {
        // We showed 0.5.0 before, but 0.6.0 is the latest now -> hint again.
        let env = envelope(Some("0.6.0"), Some("0.5.0"));
        let result = env.hint("0.4.0").expect("newer than notified should hint");
        assert_eq!(result.latest, "0.6.0");
    }

    #[test]
    fn hint_absent_when_not_newer() {
        let env = envelope(Some("0.4.0"), None);
        assert!(env.hint("0.4.0").is_none());
    }

    #[test]
    fn hint_absent_when_never_checked() {
        let env = envelope(None, None);
        assert!(env.hint("0.4.0").is_none());
    }

    #[test]
    fn hint_absent_when_notified_but_never_checked() {
        // latest = None (never checked) short-circuits before notified_version
        // is consulted, so this returns None rather than touching `notified`.
        let env = envelope(None, Some("0.5.0"));
        assert!(env.hint("0.4.0").is_none());
    }

    #[test]
    #[serial_test::serial]
    fn end_to_end_notify_then_dedupe() {
        let _guard = EnvGuard(crate::runtime::config::ENV_HOME);
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, tmp.path());

        let seeded = CacheEnvelope {
            checked_at: 0,
            latest_version: Some("0.5.0".to_string()),
            notified_version: None,
        };
        save_to(&cache_path().unwrap(), &seeded).unwrap();

        let hint = cached_hint("0.4.0").expect("should hint for 0.5.0");
        assert_eq!(hint.latest, "0.5.0");

        mark_notified("0.5.0");
        assert!(cached_hint("0.4.0").is_none());
    }

    #[test]
    #[serial_test::serial]
    fn record_latest_refreshes_cache_and_staleness() {
        let _guard = EnvGuard(crate::runtime::config::ENV_HOME);
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, tmp.path());

        // No cache yet -> a check is due.
        assert!(is_check_due(1_000));

        record_latest("0.5.0", 1_000);

        // Fresh -> not due 1h later; due again after 25h.
        assert!(!is_check_due(1_000 + 3_600));
        assert!(is_check_due(1_000 + 25 * 3_600));

        // The recorded version is what cached_hint reports.
        assert_eq!(cached_hint("0.4.0").unwrap().latest, "0.5.0");
    }
}
