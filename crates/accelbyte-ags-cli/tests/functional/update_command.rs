//! Functional tests for `ags update`.

use crate::common::cli_helpers::{ags, ags_with_update_check_enabled};
use predicates::prelude::*;

/// A newer release is reported with the release URL and exit 0.
#[tokio::test]
async fn test_update_reports_newer_release() {
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

    ags()
        .arg("update")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .assert()
        .success()
        .stdout(predicate::str::contains("is available (current:"))
        .stdout(predicate::str::contains(
            "https://github.com/AccelByte/accelbyte-ags-cli/releases/tag/v99.0.0",
        ));
}

/// When the latest version equals the current version, report "is the latest
/// release" and exit 0.
#[tokio::test]
async fn test_update_reports_current_is_latest() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let current = env!("CARGO_PKG_VERSION");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "tag_name": format!("v{current}") })),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let url = format!("{}/releases/latest", server.uri());

    ags()
        .arg("update")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .assert()
        .success()
        .stdout(predicate::str::contains("is the latest release"));
}

/// JSON output contains exactly the eight documented fields.
#[tokio::test]
async fn test_update_json_has_the_documented_fields() {
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

    let output = ags()
        .args(["update", "--format", "json"])
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .output()
        .unwrap();

    assert!(output.status.success(), "exit code must be 0");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON");

    // Exactly the eight documented keys.
    let obj = json.as_object().expect("top level must be an object");
    let keys: Vec<&String> = obj.keys().collect();
    assert_eq!(
        keys.len(),
        8,
        "expected exactly 8 keys, got {}: {keys:?}",
        keys.len()
    );
    assert!(obj.contains_key("current"), "missing 'current'");
    assert!(obj.contains_key("latest"), "missing 'latest'");
    assert!(
        obj.contains_key("update_available"),
        "missing 'update_available'"
    );
    assert!(
        obj.contains_key("install_method"),
        "missing 'install_method'"
    );
    assert!(obj.contains_key("binary_path"), "missing 'binary_path'");
    assert!(
        obj.contains_key("upgrade_command"),
        "missing 'upgrade_command'"
    );
    assert!(
        obj.contains_key("download_archive"),
        "missing 'download_archive'"
    );
    assert!(obj.contains_key("release_url"), "missing 'release_url'");

    assert_eq!(json["update_available"], true);
    let method = json["install_method"].as_str().unwrap();
    assert!(
        ["installer", "homebrew", "manual"].contains(&method),
        "install_method must be one of the three values, got: {method}"
    );
}

/// Exit code 4 when GitHub is unreachable.
#[tokio::test]
async fn test_update_exits_4_when_github_is_unreachable() {
    let tmp = tempfile::tempdir().unwrap();

    // Bind a socket and immediately drop it so the port is known-closed.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let url = format!("http://127.0.0.1:{port}/releases/latest");

    let output = ags()
        .arg("update")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(4),
        "exit code must be 4 (network error), got {:?}",
        output.status.code()
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.is_empty(), "stdout must be empty on error");
}

/// Exit code 4 when GitHub answers with an error HTTP status (e.g. 403).
#[tokio::test]
async fn test_update_exits_4_when_github_answers_with_error_status() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let url = format!("{}/releases/latest", server.uri());

    let output = ags()
        .arg("update")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(4),
        "exit code must be 4 (network error for HTTP 403), got {:?}",
        output.status.code()
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.is_empty(), "stdout must be empty on error");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("403"),
        "stderr must mention the HTTP status; got: {stderr}"
    );
}

/// `AGS_NO_UPDATE_CHECK=1` and `CI=true` do NOT suppress `ags update`.
#[tokio::test]
async fn test_update_ignores_passive_check_opt_out() {
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
        .arg("update")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .env("AGS_NO_UPDATE_CHECK", "1")
        .env("CI", "true")
        .assert()
        .success();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        1,
        "exactly one request must reach the mock (the explicit check), got {}",
        requests.len()
    );
}

/// `--dry-run` makes no request and writes no cache.
#[tokio::test]
async fn test_update_dry_run_makes_no_request_and_writes_no_cache() {
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

    ags()
        .args(["update", "--dry-run"])
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .assert()
        .success();

    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        0,
        "zero requests must reach the mock under --dry-run, got {}",
        requests.len()
    );

    let cache_path = tmp.path().join("cache").join("update_check.json");
    assert!(
        !cache_path.exists(),
        "cache/update_check.json must not exist under --dry-run"
    );
}

/// Exit code 4 when the release tag is not a valid version (e.g. "nightly-latest").
/// The behaviour already exists — this test pins it so regressions are caught.
#[tokio::test]
async fn test_update_exits_4_when_release_tag_is_not_a_version() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "tag_name": "nightly-latest" })),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let url = format!("{}/releases/latest", server.uri());

    let output = ags()
        .arg("update")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(4),
        "exit code must be 4 (network error for invalid tag), got {:?}",
        output.status.code()
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.is_empty(), "stdout must be empty on error");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nightly-latest"),
        "stderr must name the invalid tag; got: {stderr}"
    );
}

/// When current == latest, the JSON `release_url` must point to `/releases/latest`
/// (the stable page), not a tag page that may not exist.
#[tokio::test]
async fn test_update_current_release_url_points_to_latest_page() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let current = env!("CARGO_PKG_VERSION");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "tag_name": format!("v{current}") })),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let url = format!("{}/releases/latest", server.uri());

    let output = ags()
        .args(["update", "--format", "json"])
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .output()
        .unwrap();

    assert!(output.status.success(), "exit code must be 0");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON");

    let release_url = json["release_url"]
        .as_str()
        .expect("release_url must be a string");
    assert!(
        release_url.ends_with("/releases/latest"),
        "release_url for current version must end with /releases/latest; got: {release_url}"
    );
}

/// When a newer release exists, the JSON `release_url` must point to the
/// specific tag page for that version.
#[tokio::test]
async fn test_update_newer_release_url_points_to_tagged_release() {
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

    let output = ags()
        .args(["update", "--format", "json"])
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .output()
        .unwrap();

    assert!(output.status.success(), "exit code must be 0");

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON");

    let release_url = json["release_url"]
        .as_str()
        .expect("release_url must be a string");
    assert!(
        release_url.ends_with("/releases/tag/v99.0.0"),
        "release_url for newer version must end with /releases/tag/v99.0.0; got: {release_url}"
    );
}

/// After a newer result, the cache has `notified_version` equal to the mock tag.
#[tokio::test]
async fn test_update_marks_newer_release_notified() {
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

    ags()
        .arg("update")
        .env("AGS_HOME", tmp.path())
        .env("AGS_NO_KEYCHAIN", "1")
        .env("AGS_UPDATE_CHECK_URL", &url)
        .assert()
        .success();

    let cache_path = tmp.path().join("cache").join("update_check.json");
    let cache: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cache_path).expect("cache file must exist"))
            .expect("cache must be valid JSON");

    assert_eq!(
        cache["notified_version"].as_str(),
        Some("99.0.0"),
        "notified_version must equal the mock tag after a newer result"
    );
}
