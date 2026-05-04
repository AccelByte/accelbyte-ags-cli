use assert_cmd::Command;

fn ags_with_cache_dir(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("ags").unwrap();
    cmd.env("AGS_HOME", dir.to_str().unwrap())
        .env("AGS_NO_KEYCHAIN", "1");
    cmd
}

/// Cold run: no cache exists, must decompress + parse + write cache.
/// Debug builds are slow — 15s is generous but catches regressions.
#[test]
#[ignore]
fn test_iam_cold_load_under_15s() {
    let tmp = tempfile::tempdir().unwrap();

    let start = std::time::Instant::now();
    let output = ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .output()
        .unwrap();
    let elapsed = start.elapsed();

    assert!(output.status.success());
    assert!(
        elapsed < std::time::Duration::from_secs(15),
        "Cold ags iam --help took {elapsed:?}, expected < 15s"
    );
}

/// Warm run: cache exists, loads pre-parsed ServiceSchema.
/// Should be significantly faster than cold — no decompression or parsing.
#[test]
#[ignore]
fn test_iam_cached_load_under_5s() {
    let tmp = tempfile::tempdir().unwrap();

    // Populate cache
    ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .output()
        .unwrap();

    // Measure cached run
    let start = std::time::Instant::now();
    let output = ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .output()
        .unwrap();
    let elapsed = start.elapsed();

    assert!(output.status.success());
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "Cached ags iam --help took {elapsed:?}, expected < 5s"
    );
}
