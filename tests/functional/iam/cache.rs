use crate::common::cli_helpers::ags;
use predicates::prelude::*;
use std::path::PathBuf;

fn ags_with_cache_dir(dir: &std::path::Path) -> assert_cmd::Command {
    let mut cmd = ags();
    cmd.env("AGS_HOME", dir.to_str().unwrap())
        .env("AGS_NO_KEYCHAIN", "1");
    cmd
}

fn cache_file(dir: &std::path::Path) -> PathBuf {
    dir.join("cache").join("iam.json")
}

// ── Cache lifecycle ──

/// The first invocation of a service decompresses the bundled spec and writes a cache file
#[test]
fn test_first_run_creates_cache_file() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = cache_file(tmp.path());

    assert!(!cache.exists(), "Cache should not exist before first run");

    ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .assert()
        .success();

    assert!(cache.exists(), "Cache should be created after first run");
}

/// Subsequent runs load from the cache and complete faster than the initial decompress+parse
#[test]
#[ignore]
fn test_cached_run_is_faster_than_first_run() {
    let tmp = tempfile::tempdir().unwrap();

    // First run — cold (decompress + parse + write cache)
    let start = std::time::Instant::now();
    ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .assert()
        .success();
    let cold = start.elapsed();

    // Second run — warm (load from cache)
    let start = std::time::Instant::now();
    ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .assert()
        .success();
    let warm = start.elapsed();

    // Use 90% threshold to avoid flakiness on loaded CI runners
    let threshold = cold.mul_f64(0.9);
    assert!(
        warm < threshold,
        "Cached run ({warm:?}) should be under 90% of cold run ({cold:?})"
    );
}

// ── Corrupted / empty cache ──

/// Invalid JSON in the cache file is auto-recovered by deleting and rebuilding from bundled specs
#[test]
fn test_corrupted_cache_auto_recovers() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = cache_file(tmp.path());

    // Write invalid JSON to the cache location
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::write(&cache, "{corrupt json").unwrap();

    ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("users"));

    // Cache should now contain valid JSON after auto-recovery
    let data = std::fs::read_to_string(&cache).unwrap();
    assert!(
        serde_json::from_str::<serde_json::Value>(&data).is_ok(),
        "Cache should contain valid JSON after auto-recovery"
    );
}

/// An empty cache file (e.g. from a truncated write) is auto-recovered
#[test]
fn test_empty_cache_file_auto_recovers() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = cache_file(tmp.path());

    // Write empty file to the cache location
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::write(&cache, "").unwrap();

    ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("users"));
}

/// A partially written cache (valid JSON prefix, truncated mid-object) is auto-recovered
#[test]
fn test_truncated_cache_file_auto_recovers() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = cache_file(tmp.path());

    // Simulate a partial write — valid JSON prefix, truncated mid-object
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::write(&cache, r#"{"service":"iam","resources":[{"name":"users"#).unwrap();

    ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("users"));
}

/// `ags refresh-specs iam` replaces a corrupted cache file with valid JSON.
#[test]
fn test_corrupted_cache_recovers_with_refresh_specs() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = cache_file(tmp.path());

    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::write(&cache, "not valid json").unwrap();

    ags_with_cache_dir(tmp.path())
        .args(["refresh-specs", "iam"])
        .assert()
        .success();

    let data = std::fs::read_to_string(&cache).unwrap();
    let _: serde_json::Value = serde_json::from_str(&data)
        .expect("Cache should contain valid JSON after refresh-specs iam");
}

// ── Flag combinations ──

// ── Permissions ──

/// Cache files are created with 0600 permissions to prevent other users reading spec data
#[cfg(unix)]
#[test]
fn test_cache_file_has_secure_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let cache = cache_file(tmp.path());

    ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .assert()
        .success();

    let mode = std::fs::metadata(&cache).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "Cache file should have 0600 permissions, got {mode:o}"
    );
}

/// The cache directory is created with 0700 permissions to prevent directory listing by others
#[cfg(unix)]
#[test]
fn test_cache_dir_has_secure_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let cache_dir = tmp.path().join("cache");

    ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .assert()
        .success();

    let mode = std::fs::metadata(&cache_dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o700,
        "Cache directory should have 0700 permissions, got {mode:o}"
    );
}

// ── Refresh-specs command ──

/// `ags refresh-specs iam` overwrites whatever is at the cache path with a real schema
/// and prints a one-line summary. Uses a content-based check rather than mtime so it's
/// robust on filesystems with second-resolution mtimes.
#[test]
fn test_refresh_specs_command_rebuilds_single_service() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = cache_file(tmp.path());

    // Seed the cache path with valid JSON that is NOT a ServiceSchema, so any
    // bug that made the command consult the existing cache instead of rebuilding
    // would leave this sentinel in place.
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    std::fs::write(&cache, r#"{"sentinel":"placeholder"}"#).unwrap();

    ags_with_cache_dir(tmp.path())
        .args(["refresh-specs", "iam"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Refreshed iam in"));

    let data = std::fs::read_to_string(&cache).unwrap();
    assert!(
        !data.contains("sentinel"),
        "refresh-specs did not overwrite the sentinel cache: {data}"
    );
    assert!(
        data.contains("\"resources\""),
        "rebuilt cache is not a ServiceSchema: {data}"
    );
}

/// `ags refresh-specs` (no arg) rebuilds cache files for every bundled service.
#[test]
fn test_refresh_specs_all_rebuilds_every_service() {
    let tmp = tempfile::tempdir().unwrap();
    let cache_dir = tmp.path().join("cache");

    ags_with_cache_dir(tmp.path())
        .args(["refresh-specs"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Refreshed").and(predicate::str::contains("services in")));

    // At minimum, expect cache files for iam and achievement.
    for svc in ["iam", "achievement"] {
        let path = cache_dir.join(format!("{svc}.json"));
        assert!(path.exists(), "missing cache file for {svc}");
        let data = std::fs::read_to_string(&path).unwrap();
        let _: serde_json::Value = serde_json::from_str(&data)
            .unwrap_or_else(|e| panic!("{svc} cache is not valid JSON: {e}"));
    }
}

/// `ags refresh-specs <unknown>` exits non-zero with a usage error.
#[test]
fn test_refresh_specs_unknown_service() {
    let tmp = tempfile::tempdir().unwrap();

    ags_with_cache_dir(tmp.path())
        .args(["refresh-specs", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown service"))
        .stderr(predicate::str::contains("Valid services"));
}
