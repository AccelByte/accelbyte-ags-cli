//! Functional tests for `ags update --install`.

use crate::common::cli_helpers::ags_copied_to;

// ── Shared helpers ──

/// Script name for the platform's installer.
fn installer_script_name() -> &'static str {
    if std::env::consts::OS == "windows" {
        "accelbyte-ags-cli-installer.ps1"
    } else {
        "accelbyte-ags-cli-installer.sh"
    }
}

/// Mock path for the installer script download.
fn installer_mock_path() -> String {
    format!("/{}", installer_script_name())
}

/// Mount a release mock that returns the given tag (e.g. "v99.0.0").
async fn mount_release_mock(server: &wiremock::MockServer, tag: &str) {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    Mock::given(method("GET"))
        .and(path("/releases/latest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "tag_name": tag })),
        )
        .mount(server)
        .await;
}

/// Mount an installer mock that serves `body` at the platform script path.
async fn mount_installer_mock(server: &wiremock::MockServer, body: &str) {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    Mock::given(method("GET"))
        .and(path(installer_mock_path()))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
}

/// A shell script body that exits with the given code (Unix failure body).
fn failure_installer_body() -> String {
    if std::env::consts::OS == "windows" {
        "exit 3".to_string()
    } else {
        "#!/bin/sh\nexit 3".to_string()
    }
}

/// A shell script body that writes the env marker and a fake ags binary
/// printing the given version. Unix only — Windows cannot produce a
/// runnable `ags.exe` from a script.
#[cfg(unix)]
fn success_installer_body(version: &str) -> String {
    format!(
        r#"#!/bin/sh
# Determine target directory
if [ -n "$CARGO_DIST_FORCE_INSTALL_DIR" ]; then
  TARGET_DIR="$CARGO_DIST_FORCE_INSTALL_DIR/bin"
elif [ -n "$ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL" ]; then
  TARGET_DIR="$ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL"
else
  exit 1
fi
mkdir -p "$TARGET_DIR"
env | grep -E '^(ACCELBYTE_AGS_CLI_|CARGO_DIST_)' | sort > "$TARGET_DIR/installer-env.txt"
printf '#!/bin/sh\necho "ags {version}"\n' > "$TARGET_DIR/ags"
chmod +x "$TARGET_DIR/ags"
"#,
        version = version
    )
}

/// Write a receipt file pointing at the given install prefix.
///
/// Placed at `<config_home>/accelbyte-ags-cli/accelbyte-ags-cli-receipt.json`,
/// where `config_home` is set via `XDG_CONFIG_HOME` on Unix or `LOCALAPPDATA`
/// on Windows.
#[cfg(unix)]
fn write_receipt(config_home: &std::path::Path, prefix: &std::path::Path) {
    let receipt_dir = config_home.join("accelbyte-ags-cli");
    std::fs::create_dir_all(&receipt_dir).unwrap();
    let receipt_path = receipt_dir.join("accelbyte-ags-cli-receipt.json");
    let content = serde_json::json!({ "install_prefix": prefix.display().to_string() });
    std::fs::write(receipt_path, content.to_string()).unwrap();
}

/// Count requests received by a mock server on the installer path.
async fn installer_request_count(server: &wiremock::MockServer) -> usize {
    let requests = server.received_requests().await.unwrap_or_default();
    let installer_path = installer_mock_path();
    requests
        .iter()
        .filter(|r| r.url.path() == installer_path)
        .count()
}

// ── Both-platform tests ──

/// When the latest version equals the current version, report "Nothing to
/// install" and exit 0; the installer host receives zero requests.
#[tokio::test]
async fn test_update_install_already_current_installs_nothing() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    let current = env!("CARGO_PKG_VERSION");
    mount_release_mock(&release_server, &format!("v{current}")).await;
    mount_installer_mock(&installer_server, &failure_installer_body()).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());

    let output = cmd
        .args(["update", "--install"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "exit code must be 0, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Nothing to install"),
        "stdout must contain 'Nothing to install'; got: {stdout}"
    );
    assert_eq!(
        installer_request_count(&installer_server).await,
        0,
        "zero installer requests expected"
    );

    // The binary is unchanged.
    let _ = binary_path;
}

/// A binary under a Cellar path is detected as Homebrew; the command
/// refuses with the `brew upgrade` message and exit 1.
#[tokio::test]
async fn test_update_install_refuses_homebrew_copy() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    mount_installer_mock(&installer_server, &failure_installer_body()).await;

    let tmp = tempfile::tempdir().unwrap();
    let cellar_bin = tmp
        .path()
        .join("Cellar")
        .join("ags-cli")
        .join("0.5.0")
        .join("bin");
    std::fs::create_dir_all(&cellar_bin).unwrap();

    let ags_home = tempfile::tempdir().unwrap();
    let (_, mut cmd) = ags_copied_to(&cellar_bin);

    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "exit code must be 1 for Homebrew refusal, got {:?}",
        output.status.code()
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("brew upgrade accelbyte/tap/ags-cli"),
        "stderr must name the brew upgrade command; got: {stderr}"
    );
    assert_eq!(
        installer_request_count(&installer_server).await,
        0,
        "zero installer requests expected for Homebrew"
    );
}

/// `--no-input` without `--yes` exits 1 with the `--yes` suggestion and
/// never reads stdin.
#[tokio::test]
async fn test_update_install_no_input_without_yes_is_usage_error() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    mount_installer_mock(&installer_server, &failure_installer_body()).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (_, mut cmd) = ags_copied_to(tmp.path());

    let output = cmd
        .args(["update", "--install", "--no-input"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "exit code must be 1 for no-input without --yes, got {:?}",
        output.status.code()
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--yes"),
        "stderr must name --yes; got: {stderr}"
    );
    assert_eq!(
        installer_request_count(&installer_server).await,
        0,
        "zero installer requests expected for no-input"
    );
}

/// `--dry-run` exits 0, names the installer URL and binary path, sends
/// zero requests to either mock, and leaves the binary unchanged.
#[tokio::test]
async fn test_update_install_dry_run_makes_no_request() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    mount_installer_mock(&installer_server, &failure_installer_body()).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());
    let original_bytes = std::fs::read(&binary_path).unwrap();

    let output = cmd
        .args(["update", "--install", "--dry-run"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "exit code must be 0 for dry-run, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(installer_script_name()),
        "stdout must name the installer script; got: {stdout}"
    );
    assert!(
        stdout.contains(&binary_path.display().to_string()),
        "stdout must name the binary path; got: {stdout}"
    );

    // Zero requests to either mock.
    let release_requests = release_server.received_requests().await.unwrap_or_default();
    assert_eq!(
        release_requests.len(),
        0,
        "zero release requests expected under --dry-run"
    );
    assert_eq!(
        installer_request_count(&installer_server).await,
        0,
        "zero installer requests expected under --dry-run"
    );

    // Binary unchanged.
    assert_eq!(
        std::fs::read(&binary_path).unwrap(),
        original_bytes,
        "binary must be unchanged under --dry-run"
    );

    // No .old file.
    let old_path = binary_path.with_extension(if cfg!(windows) { "exe.old" } else { "old" });
    assert!(!old_path.exists(), ".old must not exist under --dry-run");
}

/// `--dry-run --format json` emits exactly six keys with the documented types.
#[tokio::test]
async fn test_update_install_dry_run_json_has_the_documented_fields() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    mount_installer_mock(&installer_server, &failure_installer_body()).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (_, mut cmd) = ags_copied_to(tmp.path());

    let output = cmd
        .args(["update", "--install", "--dry-run", "--format", "json"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "exit code must be 0, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON");
    let obj = json.as_object().expect("top level must be an object");
    assert_eq!(
        obj.len(),
        6,
        "expected exactly 6 keys, got {}: {:?}",
        obj.len(),
        obj.keys().collect::<Vec<_>>()
    );
    assert_eq!(json["action"], "dry_run");
    assert!(
        json["latest"].is_null(),
        "latest must be null under dry-run"
    );
    assert!(
        json["installer_url"].is_string(),
        "installer_url must be a string"
    );
}

/// Installer exits 3; `--yes`; exit 5; stderr names `3` and says the
/// previous binary is still in place; original bytes preserved.
#[tokio::test]
async fn test_update_install_reports_installer_failure_and_restores_previous_binary() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    mount_installer_mock(&installer_server, &failure_installer_body()).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());
    let original_bytes = std::fs::read(&binary_path).unwrap();

    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(5),
        "exit code must be 5 for installer failure, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("3"),
        "stderr must name exit code 3; got: {stderr}"
    );
    assert!(
        stderr.contains("still in place") || stderr.contains("previous"),
        "stderr must mention the previous binary; got: {stderr}"
    );

    // Original bytes restored.
    assert_eq!(
        std::fs::read(&binary_path).unwrap(),
        original_bytes,
        "binary must have the original bytes after failed installer"
    );

    // No .old remains.
    let old_path = binary_path.with_extension(if cfg!(windows) { "exe.old" } else { "old" });
    assert!(!old_path.exists(), ".old must not exist after restore");
}

/// Installer mock returns 404; `--yes`; exit 4; original bytes unchanged.
#[tokio::test]
async fn test_update_install_exits_4_when_installer_download_fails() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let release_server = MockServer::start().await;
    let installer_server = MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;

    // Serve a 404 for the installer download.
    Mock::given(method("GET"))
        .and(path(installer_mock_path()))
        .respond_with(ResponseTemplate::new(404))
        .mount(&installer_server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());
    let original_bytes = std::fs::read(&binary_path).unwrap();

    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(4),
        "exit code must be 4 for download failure, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        std::fs::read(&binary_path).unwrap(),
        original_bytes,
        "binary must have the original bytes after failed download"
    );

    let old_path = binary_path.with_extension(if cfg!(windows) { "exe.old" } else { "old" });
    assert!(
        !old_path.exists(),
        ".old must not exist after restore on download failure"
    );
}

/// Installer mock returns a 2 MiB body; exit 4; nothing was executed.
#[tokio::test]
async fn test_update_install_exits_4_when_installer_script_is_oversized() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let release_server = MockServer::start().await;
    let installer_server = MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;

    let oversized = "x".repeat(2 * 1024 * 1024);
    Mock::given(method("GET"))
        .and(path(installer_mock_path()))
        .respond_with(ResponseTemplate::new(200).set_body_string(oversized))
        .mount(&installer_server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (_, mut cmd) = ags_copied_to(tmp.path());

    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(4),
        "exit code must be 4 for oversized installer, got {:?}",
        output.status.code()
    );

    // No installer-env.txt anywhere under the temp dirs.
    fn has_marker(dir: &std::path::Path) -> bool {
        if !dir.exists() {
            return false;
        }
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if has_marker(&path) {
                    return true;
                }
            } else if entry.file_name() == "installer-env.txt" {
                return true;
            }
        }
        false
    }
    assert!(
        !has_marker(tmp.path()) && !has_marker(ags_home.path()),
        "installer-env.txt must not exist after oversized rejection"
    );
}

/// Installer mock answers 302 with Location: http://...; exit 4; nothing executed.
#[tokio::test]
async fn test_update_install_exits_4_when_redirected_to_http() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let release_server = MockServer::start().await;
    let installer_server = MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;

    Mock::given(method("GET"))
        .and(path(installer_mock_path()))
        .respond_with(
            ResponseTemplate::new(302).append_header("Location", "http://127.0.0.1:9/installer"),
        )
        .mount(&installer_server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (_, mut cmd) = ags_copied_to(tmp.path());

    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(4),
        "exit code must be 4 for HTTP redirect, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Pre-created lock file; `--yes`; exit 1; stderr names the lock path.
#[tokio::test]
async fn test_update_install_refuses_when_another_install_holds_the_lock() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    mount_installer_mock(&installer_server, &failure_installer_body()).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (_, mut cmd) = ags_copied_to(tmp.path());

    // Pre-create the lock file in the cache directory.
    let cache_dir = ags_home.path().join("cache");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let lock_path = cache_dir.join("update-install.lock");
    std::fs::write(&lock_path, "fake-pid").unwrap();

    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "exit code must be 1 for lock conflict, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("update-install.lock"),
        "stderr must name the lock path; got: {stderr}"
    );
    assert_eq!(
        installer_request_count(&installer_server).await,
        0,
        "zero installer requests expected when lock is held"
    );
}

// ── Unix-only tests ──
//
// These tests need a runnable fake binary that prints a version string.
// On Windows, a script cannot be named `ags.exe` and still execute, so
// these tests run on Unix only and are exercised in CI on Linux.

#[cfg(unix)]
#[tokio::test]
async fn test_update_install_runs_installer_with_receipt_prefix() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    mount_installer_mock(&installer_server, &success_installer_body("99.0.0")).await;

    let prefix = tempfile::tempdir().unwrap();
    let bin_dir = prefix.path().join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();

    let ags_home = tempfile::tempdir().unwrap();
    let config_home = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(&bin_dir);

    write_receipt(config_home.path(), prefix.path());

    let output = cmd
        .args(["update", "--install", "--yes", "--format", "json"])
        .env("AGS_HOME", ags_home.path())
        .env("XDG_CONFIG_HOME", config_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "exit code must be 0, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    // Check installer-env.txt marker.
    let marker_path = bin_dir.join("installer-env.txt");
    let marker = std::fs::read_to_string(&marker_path)
        .unwrap_or_else(|e| panic!("marker file must exist at {}: {e}", marker_path.display()));

    assert!(
        marker.contains(&format!(
            "CARGO_DIST_FORCE_INSTALL_DIR={}",
            prefix.path().display()
        )),
        "marker must show CARGO_DIST_FORCE_INSTALL_DIR; got: {marker}"
    );
    assert!(
        marker.contains("ACCELBYTE_AGS_CLI_NO_MODIFY_PATH=1"),
        "marker must show NO_MODIFY_PATH; got: {marker}"
    );
    assert!(
        !marker.contains("ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL"),
        "marker must not show unmanaged variable; got: {marker}"
    );

    // The fake binary is at the same path.
    assert!(binary_path.exists(), "binary must exist after install");

    // JSON output has the documented fields.
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON");
    assert_eq!(json["action"], "installed");
    assert_eq!(json["latest"], "99.0.0");
    assert_eq!(
        json["previous"],
        env!("CARGO_PKG_VERSION"),
        "previous must match the running version"
    );

    // No .old remains.
    let old_path = binary_path.with_extension("old");
    assert!(!old_path.exists(), ".old must not exist after success");
}

#[cfg(unix)]
#[tokio::test]
async fn test_update_install_runs_installer_unmanaged_for_manual_copy() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    mount_installer_mock(&installer_server, &success_installer_body("99.0.0")).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());
    let binary_dir = binary_path.parent().unwrap();

    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "exit code must be 0, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    // The marker must show the unmanaged variable.
    let marker_path = binary_dir.join("installer-env.txt");
    let marker = std::fs::read_to_string(&marker_path)
        .unwrap_or_else(|e| panic!("marker file must exist at {}: {e}", marker_path.display()));

    assert!(
        marker.contains(&format!(
            "ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL={}",
            binary_dir.display()
        )),
        "marker must show ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL; got: {marker}"
    );
}

/// The installer subprocess receives null stdin: a script that tries to
/// read the user's terminal gets end-of-file at once instead of stealing
/// the parent's input.
#[cfg(unix)] // The fake installer is a `sh` script.
#[tokio::test]
async fn test_update_install_installer_cannot_read_the_users_stdin() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());
    let binary_dir = binary_path.parent().unwrap();
    let marker_path = binary_dir.join("stdin-marker.txt");

    // The fake installer tries to read one line from stdin, then records
    // what it got in a marker file. With null stdin it gets empty; with
    // inherited stdin it would steal the "y\n" the parent piped in.
    let installer_body = format!(
        r#"#!/bin/sh
IFS= read -r line
printf 'stdin=[%s]\n' "$line" >> "{marker}"
if [ -n "$CARGO_DIST_FORCE_INSTALL_DIR" ]; then
  D="$CARGO_DIST_FORCE_INSTALL_DIR/bin"
elif [ -n "$ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL" ]; then
  D="$ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL"
else
  exit 1
fi
mkdir -p "$D"
printf '#!/bin/sh\necho "ags 99.0.0"\n' > "$D/ags"
chmod +x "$D/ags"
"#,
        marker = marker_path.display()
    );

    mount_installer_mock(&installer_server, &installer_body).await;

    // Pass `--yes` so the parent never reads stdin; pipe "y\n" so the
    // child could steal it if stdin were inherited.
    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .write_stdin("y\n")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "exit code must be 0, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let marker = std::fs::read_to_string(&marker_path)
        .unwrap_or_else(|e| panic!("marker must exist at {}: {e}", marker_path.display()));
    assert!(
        marker.contains("stdin=[]"),
        "installer must see empty stdin (null), not the parent's input; marker: {marker}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_update_install_restores_previous_binary_on_version_mismatch() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    // The installer writes a binary that reports 1.2.3 (lower than latest 99.0.0).
    mount_installer_mock(&installer_server, &success_installer_body("1.2.3")).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());
    let original_bytes = std::fs::read(&binary_path).unwrap();

    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(5),
        "exit code must be 5 for version mismatch, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("1.2.3"),
        "stderr must name the rejected version; got: {stderr}"
    );
    assert!(
        stderr.contains("restored"),
        "stderr must say the binary was restored; got: {stderr}"
    );

    assert_eq!(
        std::fs::read(&binary_path).unwrap(),
        original_bytes,
        "binary must have the original bytes after version mismatch"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_update_install_accepts_a_release_newer_than_the_check() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    // The fake binary prints 99.1.0 — newer than the checked 99.0.0.
    mount_installer_mock(&installer_server, &success_installer_body("99.1.0")).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (_, mut cmd) = ags_copied_to(tmp.path());

    let output = cmd
        .args(["update", "--install", "--yes", "--format", "json"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "exit code must be 0, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON");
    assert_eq!(json["action"], "installed");
    assert_eq!(json["latest"], "99.1.0");
}

#[cfg(unix)]
#[tokio::test]
async fn test_update_install_restores_previous_binary_when_health_check_hangs() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    // The fake binary just sleeps.
    let hang_body = r#"#!/bin/sh
if [ -n "$CARGO_DIST_FORCE_INSTALL_DIR" ]; then
  D="$CARGO_DIST_FORCE_INSTALL_DIR/bin"
elif [ -n "$ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL" ]; then
  D="$ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL"
else
  exit 1
fi
mkdir -p "$D"
printf '#!/bin/sh\nsleep 30\n' > "$D/ags"
chmod +x "$D/ags"
"#;
    mount_installer_mock(&installer_server, hang_body).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());
    let original_bytes = std::fs::read(&binary_path).unwrap();

    let start = std::time::Instant::now();
    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env("AGS_UPDATE_HEALTH_TIMEOUT_SECS", "1")
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(
        output.status.code(),
        Some(5),
        "exit code must be 5 for health-check hang, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("did not answer"),
        "stderr must mention the timeout; got: {stderr}"
    );

    assert_eq!(
        std::fs::read(&binary_path).unwrap(),
        original_bytes,
        "binary must have the original bytes after health-check hang"
    );

    // Must complete well under 30 s (the sleep in the fake binary).
    assert!(
        elapsed.as_secs() < 15,
        "test must finish in well under 30 s; took {} s",
        elapsed.as_secs()
    );
}

/// A SIGINT (Ctrl-C) during the installer run restores the previous binary
/// and exits cleanly rather than leaving the binary missing.
#[cfg(unix)] // Signals are process-level on Unix; this test sends SIGINT via kill(1).
#[tokio::test]
async fn test_update_install_ctrl_c_during_install_restores_previous_binary() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;

    // The installer writes a marker, then sleeps. The test sends SIGINT
    // after the marker appears, so the CLI must kill the sleeping installer
    // and restore the previous binary.
    let installer_body = r#"#!/bin/sh
if [ -n "$CARGO_DIST_FORCE_INSTALL_DIR" ]; then
  D="$CARGO_DIST_FORCE_INSTALL_DIR/bin"
elif [ -n "$ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL" ]; then
  D="$ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL"
else
  exit 1
fi
mkdir -p "$D"
touch "$D/installer-started.txt"
sleep 20
"#;
    mount_installer_mock(&installer_server, installer_body).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (binary_path, _) = ags_copied_to(tmp.path());
    let original_bytes = std::fs::read(&binary_path).unwrap();
    let binary_dir = binary_path.parent().unwrap();
    let marker_path = binary_dir.join("installer-started.txt");

    let mut child = std::process::Command::new(&binary_path)
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env("AGS_NO_UPDATE_CHECK", "1")
        .env("AGS_NO_KEYCHAIN", "1")
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to spawn ags process");

    // Wait for the installer to start (marker file appears).
    let poll_start = std::time::Instant::now();
    while !marker_path.exists() {
        assert!(
            poll_start.elapsed() < std::time::Duration::from_secs(5),
            "installer-started.txt did not appear within 5 s"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Send SIGINT to the CLI process.
    let pid = child.id().to_string();
    let kill_status = std::process::Command::new("kill")
        .args(["-INT", &pid])
        .status()
        .expect("failed to send SIGINT");
    assert!(kill_status.success(), "kill -INT must succeed");

    // Wait for exit with a 10 s ceiling (poll try_wait).
    let wait_start = std::time::Instant::now();
    let exit_status = loop {
        match child.try_wait().expect("try_wait failed") {
            Some(status) => break status,
            None => {
                assert!(
                    wait_start.elapsed() < std::time::Duration::from_secs(10),
                    "CLI process did not exit within 10 s after SIGINT"
                );
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    };

    // Read stderr after the child has exited.
    let mut stderr_buf = Vec::new();
    if let Some(mut handle) = child.stderr.take() {
        use std::io::Read;
        handle.read_to_end(&mut stderr_buf).ok();
    }
    let stderr = String::from_utf8_lossy(&stderr_buf);

    // The process exited within the 10 s ceiling (the 20 s sleep in the
    // installer was not waited for), proving the installer child was killed.

    // Exit code: InvocationOutcome::Cancelled -> 2, matching the
    // CLI's cancelled-outcome convention (same as workflow cancellation).
    assert_eq!(
        exit_status.code(),
        Some(2),
        "exit code must be 2 for cancellation, got {:?}\nstderr: {stderr}",
        exit_status.code()
    );

    // The original binary is restored at the expected path.
    assert_eq!(
        std::fs::read(&binary_path).unwrap(),
        original_bytes,
        "binary must have the original bytes after Ctrl-C"
    );

    // No .old file remains.
    let old_path = binary_path.with_extension("old");
    assert!(
        !old_path.exists(),
        ".old must not exist after Ctrl-C restore"
    );

    // Stderr contains the cancellation message.
    assert!(
        stderr.contains("Cancelled."),
        "stderr must contain 'Cancelled.'; got: {stderr}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_update_install_reports_read_only_install_directory() {
    use std::os::unix::fs::PermissionsExt;

    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    mount_installer_mock(&installer_server, &success_installer_body("99.0.0")).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());
    let original_bytes = std::fs::read(&binary_path).unwrap();
    let binary_dir = binary_path.parent().unwrap();

    // Make the directory read-only + execute (no write).
    std::fs::set_permissions(binary_dir, std::fs::Permissions::from_mode(0o500)).unwrap();

    // If running as root, permissions are not enforced. Probe first.
    let probe = binary_dir.join(".write-probe");
    if std::fs::write(&probe, b"test").is_ok() {
        let _ = std::fs::remove_file(&probe);
        // Restore mode for temp dir cleanup.
        std::fs::set_permissions(binary_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        println!("skipped: running as root, permissions are not enforced");
        return;
    }

    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    // Restore mode before assertions so temp dir cleanup works.
    std::fs::set_permissions(binary_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(
        output.status.code(),
        Some(5),
        "exit code must be 5 for read-only directory, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        std::fs::read(&binary_path).unwrap(),
        original_bytes,
        "binary must have the original bytes"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_update_install_json_has_the_documented_fields() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    mount_installer_mock(&installer_server, &success_installer_body("99.0.0")).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (_, mut cmd) = ags_copied_to(tmp.path());

    let output = cmd
        .args(["update", "--install", "--yes", "--format", "json"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "exit code must be 0, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout must be valid JSON");
    let obj = json.as_object().expect("top level must be an object");

    // Exactly six keys.
    assert_eq!(
        obj.len(),
        6,
        "expected exactly 6 keys, got {}: {:?}",
        obj.len(),
        obj.keys().collect::<Vec<_>>()
    );

    // Installer output must be on stderr only.
    let stdout_text = String::from_utf8_lossy(&output.stdout);
    let stderr_text = String::from_utf8_lossy(&output.stderr);
    // The stdout must be exactly one JSON object (parseable, no extra lines).
    assert!(
        serde_json::from_str::<serde_json::Value>(&stdout_text).is_ok(),
        "stdout must be one JSON object"
    );
    // Installer progress lines go to stderr.
    assert!(
        stderr_text.contains("Installing") || stderr_text.contains("available"),
        "progress lines must be on stderr; stderr: {stderr_text}"
    );
}

/// On Unix, a stale `.old` file left by a prior upgrade must survive a
/// subsequent `ags` invocation — it is the user's last-resort rollback copy,
/// and the next-start cleanup is Windows-only.
#[cfg(unix)]
#[tokio::test]
async fn test_update_install_stale_old_survives_next_start_on_unix() {
    let tmp = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());

    // Place a .old file with known bytes.
    let old_path = binary_path.with_extension("old");
    let old_bytes = b"previous-ags-binary-for-rollback";
    std::fs::write(&old_path, old_bytes).unwrap();

    let ags_home = tempfile::tempdir().unwrap();
    let output = cmd
        .args(["version"])
        .env("AGS_HOME", ags_home.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "exit code must be 0 for version, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    // The .old file must still exist with the same bytes.
    assert!(
        old_path.exists(),
        ".old must survive a subsequent invocation on Unix"
    );
    assert_eq!(
        std::fs::read(&old_path).unwrap(),
        old_bytes,
        ".old content must be unchanged on Unix"
    );
}

/// On Windows, a stale `.old` file left by a prior upgrade is removed at the
/// next `ags` start — Windows cannot delete the running binary during the
/// upgrade itself, so the next invocation cleans it up.
#[cfg(windows)]
#[tokio::test]
async fn test_update_install_stale_old_is_removed_at_next_start_on_windows() {
    let tmp = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());

    // Place a .old file with known bytes.
    let old_path = {
        let mut p = binary_path.as_os_str().to_os_string();
        p.push(".old");
        std::path::PathBuf::from(p)
    };
    let old_bytes = b"previous-ags-binary-for-rollback";
    std::fs::write(&old_path, old_bytes).unwrap();

    let ags_home = tempfile::tempdir().unwrap();
    let output = cmd
        .args(["version"])
        .env("AGS_HOME", ags_home.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "exit code must be 0 for version, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    // The .old file must be gone on Windows.
    assert!(
        !old_path.exists(),
        ".old must be removed at the next start on Windows"
    );
}

/// `AGS_UPDATE_INSTALLER_URL` pointing at a remote host over plain HTTP is
/// refused before any request is made, with exit 1 and a message naming the
/// env var.
#[tokio::test]
async fn test_update_install_refuses_plain_http_override_to_a_remote_host() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;
    mount_installer_mock(&installer_server, &failure_installer_body()).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());
    let original_bytes = std::fs::read(&binary_path).unwrap();

    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        // A documentation-range address that will never be contacted.
        .env("AGS_UPDATE_INSTALLER_URL", "http://198.51.100.7:9/")
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "exit code must be 1 for plain HTTP to remote host, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("AGS_UPDATE_INSTALLER_URL"),
        "stderr must name the env var; got: {stderr}"
    );

    // Zero requests to the installer host (it is never contacted).
    assert_eq!(
        installer_request_count(&installer_server).await,
        0,
        "zero installer requests expected for plain HTTP remote host"
    );

    // Binary unchanged.
    assert_eq!(
        std::fs::read(&binary_path).unwrap(),
        original_bytes,
        "binary must be unchanged when plain HTTP is refused"
    );

    // No .old file.
    let old_path = binary_path.with_extension(if cfg!(windows) { "exe.old" } else { "old" });
    assert!(
        !old_path.exists(),
        ".old must not exist when plain HTTP is refused"
    );
}

/// When `restore_previous_binary` fails (directory made read-only by the
/// installer), the error message MUST name the failure and the reinstall
/// instruction, and MUST NOT claim the binary was restored or is still in
/// place. Exit 5.
#[cfg(unix)]
#[tokio::test]
async fn test_update_install_reports_a_failed_restore_and_exits_5() {
    use std::os::unix::fs::PermissionsExt;

    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;

    // The installer makes the binary's directory read-only, then exits 3.
    // The subsequent restore_previous_binary rename will fail because the
    // directory no longer allows writes.
    let installer_body = r#"#!/bin/sh
if [ -n "$CARGO_DIST_FORCE_INSTALL_DIR" ]; then
  D="$CARGO_DIST_FORCE_INSTALL_DIR/bin"
elif [ -n "$ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL" ]; then
  D="$ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL"
else
  exit 1
fi
chmod 555 "$D"
exit 3
"#;
    mount_installer_mock(&installer_server, installer_body).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());
    let binary_dir = binary_path.parent().unwrap();

    // Probe: if running as root, permissions are not enforced.
    std::fs::set_permissions(binary_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let probe = binary_dir.join(".write-probe");
    if std::fs::write(&probe, b"test").is_ok() {
        let _ = std::fs::remove_file(&probe);
        std::fs::set_permissions(binary_dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        println!("skipped: running as root, permissions are not enforced");
        return;
    }
    // Restore writable so the binary can preserve (rename ags → ags.old).
    std::fs::set_permissions(binary_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    // Restore directory mode so temp dir cleanup works.
    std::fs::set_permissions(binary_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(
        output.status.code(),
        Some(5),
        "exit code must be 5 for failed restore, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Restoring the previous ags binary at"),
        "stderr must name the restore failure; got: {stderr}"
    );
    assert!(
        stderr.contains("failed"),
        "stderr must say 'failed'; got: {stderr}"
    );
    assert!(
        stderr.contains("releases/latest"),
        "stderr must contain the releases URL; got: {stderr}"
    );
    assert!(
        !stderr.contains("was restored"),
        "stderr must NOT claim the binary was restored; got: {stderr}"
    );
    assert!(
        !stderr.contains("still in place"),
        "stderr must NOT claim the binary is still in place; got: {stderr}"
    );
}

/// When the new binary exits non-zero (even if it prints a valid version
/// string), the health check MUST reject it, restore the previous binary,
/// and exit 5.
#[cfg(unix)]
#[tokio::test]
async fn test_update_install_restores_previous_binary_when_new_binary_exits_non_zero() {
    let release_server = wiremock::MockServer::start().await;
    let installer_server = wiremock::MockServer::start().await;

    mount_release_mock(&release_server, "v99.0.0").await;

    // The installer writes a binary that prints a valid version line but
    // exits 1. The health check must reject this.
    let installer_body = r#"#!/bin/sh
if [ -n "$CARGO_DIST_FORCE_INSTALL_DIR" ]; then
  D="$CARGO_DIST_FORCE_INSTALL_DIR/bin"
elif [ -n "$ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL" ]; then
  D="$ACCELBYTE_AGS_CLI_UNMANAGED_INSTALL"
else
  exit 1
fi
mkdir -p "$D"
printf '#!/bin/sh\necho "ags 99.0.0"\nexit 1\n' > "$D/ags"
chmod +x "$D/ags"
"#;
    mount_installer_mock(&installer_server, installer_body).await;

    let tmp = tempfile::tempdir().unwrap();
    let ags_home = tempfile::tempdir().unwrap();
    let (binary_path, mut cmd) = ags_copied_to(tmp.path());
    let original_bytes = std::fs::read(&binary_path).unwrap();

    let output = cmd
        .args(["update", "--install", "--yes"])
        .env("AGS_HOME", ags_home.path())
        .env(
            "AGS_UPDATE_CHECK_URL",
            format!("{}/releases/latest", release_server.uri()),
        )
        .env("AGS_UPDATE_INSTALLER_URL", installer_server.uri())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(5),
        "exit code must be 5 when the new binary exits non-zero, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("status"),
        "stderr must name the exit status; got: {stderr}"
    );
    assert!(
        stderr.contains("version"),
        "stderr must mention the version check; got: {stderr}"
    );

    // Original bytes must be restored.
    assert_eq!(
        std::fs::read(&binary_path).unwrap(),
        original_bytes,
        "binary must have the original bytes after non-zero exit health check"
    );
}
