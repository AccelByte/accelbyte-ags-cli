use crate::common::cli_helpers::ags_with_update_check_enabled;
use predicates::prelude::*;

/// Polling window for detached-child cache writes. The update-check child
/// process starts up, makes an HTTP request to a localhost wiremock server, and
/// writes a file — typically completing within 1 second. Five seconds provides
/// wide margin for slow CI runners while keeping the suite responsive. The
/// positive control (`test_non_dry_run_spawns_update_check`) calibrates this
/// window: it proves the cache appears within this duration, which makes the
/// sibling absence assertion (`test_dry_run_does_not_spawn_update_check`)
/// meaningful.
const DETACHED_CHILD_POLL_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Poll for a file to appear at `path`, returning `true` if it materializes
/// within `timeout`. Checks once per [`POLL_INTERVAL`].
fn poll_for_file(path: &std::path::Path, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    path.exists()
}

/// The `__update-check` hidden argument runs the fetch-and-cache cycle and
/// exits silently. With `AGS_UPDATE_CHECK_URL` pointing at a wiremock server,
/// the child fetches the mock release, writes the result to the on-disk cache,
/// and exits 0 with no output — verifying the full fetch→cache→child-process
/// path under process isolation without hitting `api.github.com`.
#[tokio::test]
async fn test_update_check_child_writes_cache_via_mock() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "tag_name": "v99.0.0" })),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let url = format!("{}/releases/latest", server.uri());

    ags_with_update_check_enabled()
        .arg("__update-check")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        // Override the update-check endpoint so the child hits the mock server
        // instead of api.github.com.
        .env("AGS_UPDATE_CHECK_URL", &url)
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());

    // The child wrote the fetched version to the on-disk cache.
    let cache_path = tmp.path().join("cache").join("update_check.json");
    let contents = std::fs::read_to_string(&cache_path)
        .expect("update_check.json should exist after a successful check");
    assert!(
        contents.contains("99.0.0"),
        "cache should contain the fetched version 99.0.0, got: {contents}"
    );
}

/// The `is_check_suppressed()` guard in the `__update-check` child arm must
/// prevent the fetch-and-cache cycle when `AGS_NO_UPDATE_CHECK=1` is set.
/// The env var is the sole suppressor under test: `CI` is explicitly removed,
/// and a live wiremock server would return a valid tag if the guard were absent.
/// The existing `test_update_check_child_writes_cache_via_mock` is the positive
/// control — identical setup without suppression, asserting the cache IS written.
#[tokio::test]
async fn test_update_check_child_suppressed_by_env() {
    use assert_cmd::Command;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "tag_name": "v99.0.0" })),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let url = format!("{}/releases/latest", server.uri());

    // Suppression active: AGS_NO_UPDATE_CHECK=1. CI is removed so the env var
    // is unambiguously the guard under test.
    Command::cargo_bin("ags")
        .unwrap()
        .arg("__update-check")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .env("AGS_NO_UPDATE_CHECK", "1")
        .env_remove("CI")
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());

    // The guard prevented the fetch — no cache file written.
    let cache_path = tmp.path().join("cache").join("update_check.json");
    assert!(
        !cache_path.exists(),
        "update_check.json must not exist when AGS_NO_UPDATE_CHECK=1 suppresses the check"
    );
}

/// Positive control for the `--dry-run` guard: without `--dry-run`, a command
/// with update check fully enabled spawns the detached child, which writes the
/// cache file within [`DETACHED_CHILD_POLL_WINDOW`]. This calibrates the window
/// so the sibling absence assertion (`test_dry_run_does_not_spawn_update_check`)
/// is meaningful — the window is proven long enough for the child to complete.
#[tokio::test]
async fn test_non_dry_run_spawns_update_check() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "tag_name": "v99.0.0" })),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let url = format!("{}/releases/latest", server.uri());

    // No suppression: ags_with_update_check_enabled() clears AGS_NO_UPDATE_CHECK
    // and CI. No --dry-run. The detached child should be spawned and write cache.
    ags_with_update_check_enabled()
        .arg("--help")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .assert()
        .success();

    let cache_path = tmp.path().join("cache").join("update_check.json");
    assert!(
        poll_for_file(&cache_path, DETACHED_CHILD_POLL_WINDOW),
        "update_check.json should appear within the polling window — the \
         detached child should have written it (window: {DETACHED_CHILD_POLL_WINDOW:?})",
    );

    let contents = std::fs::read_to_string(&cache_path)
        .expect("update_check.json should be readable after the child writes it");
    assert!(
        contents.contains("99.0.0"),
        "cache should contain the fetched version 99.0.0, got: {contents}"
    );
}

/// `--dry-run` must be fully side-effect-free: no detached update-check child
/// is spawned and no cache file is written. Both `AGS_NO_UPDATE_CHECK` and `CI`
/// are cleared (via `ags_with_update_check_enabled`) and a live wiremock server
/// would succeed if a child were spawned — so `--dry-run` is the SOLE mechanism
/// preventing the cache write. The positive control
/// (`test_non_dry_run_spawns_update_check`) calibrates the wait window: it
/// proves the cache appears within [`DETACHED_CHILD_POLL_WINDOW`] when no
/// `--dry-run` is active.
#[tokio::test]
async fn test_dry_run_does_not_spawn_update_check() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "tag_name": "v99.0.0" })),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let url = format!("{}/releases/latest", server.uri());

    // --dry-run is the sole guard: update check enabled, mock server would
    // succeed, so only --dry-run prevents the spawn.
    ags_with_update_check_enabled()
        .args(["--dry-run", "--help"])
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .assert()
        .success();

    // Wait the same window that the positive control proves is sufficient.
    let cache_path = tmp.path().join("cache").join("update_check.json");
    assert!(
        !poll_for_file(&cache_path, DETACHED_CHILD_POLL_WINDOW),
        "update_check.json must not exist after a --dry-run invocation — the \
         detached child should never have been spawned",
    );
}
