//! Release-build performance tests.
//!
//! These tests are `#[ignore]` by default and should be run with:
//!   cargo test --release --test performance -- --ignored
//!
//! Thresholds include subprocess spawn overhead (~350ms on macOS).
//! The intent is to catch material regressions, not micro-benchmark.

use assert_cmd::Command;

fn ags() -> Command {
    Command::cargo_bin("ags").unwrap()
}

// ── Startup latency ──
// Baseline: ~360ms (subprocess spawn) + minimal work.

#[test]
#[ignore]
fn test_release_version_under_500ms() {
    let start = std::time::Instant::now();
    let output = ags().arg("--version").output().unwrap();
    let elapsed = start.elapsed();

    assert!(output.status.success());
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "ags --version took {elapsed:?}, expected < 500ms (release)"
    );
}

#[test]
#[ignore]
fn test_release_help_under_500ms() {
    let start = std::time::Instant::now();
    let output = ags().arg("--help").output().unwrap();
    let elapsed = start.elapsed();

    assert!(output.status.success());
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "ags --help took {elapsed:?}, expected < 500ms (release)"
    );
}

// ── Spec loading + Clap tree build ──

fn ags_with_cache_dir(dir: &std::path::Path) -> Command {
    let mut cmd = ags();
    cmd.env("AGS_HOME", dir.to_str().unwrap())
        .env("AGS_NO_KEYCHAIN", "1");
    cmd
}

// Cold: decompress + parse + write cache + Clap tree build
#[test]
#[ignore]
fn test_release_iam_help_cold_under_3s() {
    let tmp = tempfile::tempdir().unwrap();

    let start = std::time::Instant::now();
    let output = ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .output()
        .unwrap();
    let elapsed = start.elapsed();

    assert!(output.status.success());
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "Cold ags iam --help took {elapsed:?}, expected < 3s (release)"
    );
}

// Warm: load cached ServiceSchema + Clap tree build
#[test]
#[ignore]
fn test_release_iam_help_cached_under_1s() {
    let tmp = tempfile::tempdir().unwrap();

    // Populate cache
    ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .output()
        .unwrap();

    let start = std::time::Instant::now();
    let output = ags_with_cache_dir(tmp.path())
        .args(["iam", "--help"])
        .output()
        .unwrap();
    let elapsed = start.elapsed();

    assert!(output.status.success());
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "Cached ags iam --help took {elapsed:?}, expected < 1s (release)"
    );
}

// Cold: includes spec loading for nested command
#[test]
#[ignore]
fn test_release_iam_users_help_cold_under_3s() {
    let tmp = tempfile::tempdir().unwrap();

    let start = std::time::Instant::now();
    let output = ags_with_cache_dir(tmp.path())
        .args(["iam", "users", "--help"])
        .output()
        .unwrap();
    let elapsed = start.elapsed();

    assert!(output.status.success());
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "Cold ags iam users --help took {elapsed:?}, expected < 3s (release)"
    );
}

// Warm: cached path for nested command
#[test]
#[ignore]
fn test_release_iam_users_help_cached_under_1s() {
    let tmp = tempfile::tempdir().unwrap();

    // Populate cache
    ags_with_cache_dir(tmp.path())
        .args(["iam", "users", "--help"])
        .output()
        .unwrap();

    let start = std::time::Instant::now();
    let output = ags_with_cache_dir(tmp.path())
        .args(["iam", "users", "--help"])
        .output()
        .unwrap();
    let elapsed = start.elapsed();

    assert!(output.status.success());
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "Cached ags iam users --help took {elapsed:?}, expected < 1s (release)"
    );
}

// ── Rendering throughput ──

#[test]
#[ignore]
fn test_release_render_list_100_items_under_50ms() {
    use ags::catalogue::Catalogue;
    use ags::frontend::human::render;
    use ags::frontend::RenderOptions;
    use ags::protocol::output::{ApiBody, ApiOutput, CommandOutput};
    use ags::runtime::dispatch::shape::shape_response;

    let service = Catalogue::load_bundled("iam").expect("load IAM spec");
    let resource = service
        .resources
        .into_iter()
        .find(|resource| resource.name == "users")
        .unwrap();
    let op = resource
        .operations()
        .find(|operation| operation.name == "list")
        .cloned()
        .unwrap();

    // Build a 100-item paginated list
    let items: Vec<serde_json::Value> = (0..100)
        .map(|i| {
            serde_json::json!({
                "userId": format!("user-{i:04}"),
                "displayName": format!("User {i}"),
                "emailAddress": format!("user{i}@example.com"),
                "namespace": "test"
            })
        })
        .collect();
    let body = serde_json::json!({"data": items, "paging": {}});

    let options = RenderOptions::default();

    let start = std::time::Instant::now();
    let shaped = shape_response(&body, &op, "users", false);
    let output = CommandOutput::Service(Box::new(ApiOutput {
        operation: op.clone(),
        resource_name: "users".to_string(),
        body: ApiBody::Shaped(Box::new(shaped)),
        success: None,
        trace: None,
    }));
    let result = render(&output, &options).unwrap();
    let elapsed = start.elapsed();

    assert!(!result.stdout.as_deref().unwrap_or("").is_empty());
    assert!(
        elapsed < std::time::Duration::from_millis(50),
        "Rendering 100-item list took {elapsed:?}, expected < 50ms (release)"
    );
}

#[test]
#[ignore]
fn test_release_render_list_1000_items_under_200ms() {
    use ags::catalogue::Catalogue;
    use ags::frontend::human::render;
    use ags::frontend::RenderOptions;
    use ags::protocol::output::{ApiBody, ApiOutput, CommandOutput};
    use ags::runtime::dispatch::shape::shape_response;

    let service = Catalogue::load_bundled("iam").expect("load IAM spec");
    let resource = service
        .resources
        .into_iter()
        .find(|resource| resource.name == "users")
        .unwrap();
    let op = resource
        .operations()
        .find(|operation| operation.name == "list")
        .cloned()
        .unwrap();

    let items: Vec<serde_json::Value> = (0..1000)
        .map(|i| {
            serde_json::json!({
                "userId": format!("user-{i:04}"),
                "displayName": format!("User {i}"),
                "emailAddress": format!("user{i}@example.com"),
                "namespace": "test"
            })
        })
        .collect();
    let body = serde_json::json!({"data": items, "paging": {}});

    let options = RenderOptions::default();

    let start = std::time::Instant::now();
    let shaped = shape_response(&body, &op, "users", false);
    let output = CommandOutput::Service(Box::new(ApiOutput {
        operation: op.clone(),
        resource_name: "users".to_string(),
        body: ApiBody::Shaped(Box::new(shaped)),
        success: None,
        trace: None,
    }));
    let result = render(&output, &options).unwrap();
    let elapsed = start.elapsed();

    assert!(!result.stdout.as_deref().unwrap_or("").is_empty());
    assert!(
        elapsed < std::time::Duration::from_millis(200),
        "Rendering 1000-item list took {elapsed:?}, expected < 200ms (release)"
    );
}
