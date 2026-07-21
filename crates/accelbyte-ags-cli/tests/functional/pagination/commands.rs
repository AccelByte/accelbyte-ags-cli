use crate::common::cli_helpers::ags_isolated;

// ── Flag parsing ──

/// --page-all flag is accepted without error
#[test]
fn test_page_all_flag_accepted() {
    let output = ags_isolated()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--page-all",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// --page-limit flag is accepted with a value
#[test]
fn test_page_limit_flag_accepted() {
    let output = ags_isolated()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--page-all",
            "--page-limit",
            "5",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// --page-limit rejects values over 100
#[test]
fn test_page_limit_rejects_over_100() {
    let output = ags_isolated()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--page-limit",
            "200",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid page limit"), "stderr: {stderr}");
}

/// --page-limit rejects 0 — previously interpreted as "use hard cap", now an
/// error so users get a deterministic max via the documented 1..=100 range.
#[test]
fn test_page_limit_rejects_zero() {
    let output = ags_isolated()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--page-limit",
            "0",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid page limit"), "stderr: {stderr}");
}

/// --page-limit rejects non-numeric values
#[test]
fn test_page_limit_rejects_non_numeric() {
    let output = ags_isolated()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--page-limit",
            "abc",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Invalid page limit"), "stderr: {stderr}");
}

/// --page-limit validation error must not leak ANSI escape codes when stderr is
/// not a terminal. The style module's TTY detection runs during `style::init`;
/// if validation fires before init, the error message renders with default
/// `color_enabled = true` and leaks raw SGR sequences into captured output.
#[test]
fn test_page_limit_error_does_not_leak_ansi_codes() {
    let output = ags_isolated()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--page-limit",
            "200",
            "--dry-run",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains('\x1b'),
        "stderr leaked ANSI escape (0x1b): {stderr:?}"
    );
}

// ── Config validation ──

/// config set page-limit rejects non-numeric values
#[test]
fn test_config_page_limit_rejects_non_numeric() {
    let tmp = tempfile::tempdir().unwrap();
    let output = ags_isolated()
        .env("AGS_HOME", tmp.path().to_str().unwrap())
        .args(["config", "set", "page-limit", "abc"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

/// config set page-limit rejects values over 100
#[test]
fn test_config_page_limit_rejects_over_100() {
    let tmp = tempfile::tempdir().unwrap();
    let output = ags_isolated()
        .env("AGS_HOME", tmp.path().to_str().unwrap())
        .args(["config", "set", "page-limit", "200"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

/// config set page-limit accepts valid values
#[test]
fn test_config_page_limit_accepts_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let output = ags_isolated()
        .env("AGS_HOME", tmp.path().to_str().unwrap())
        .args(["config", "set", "page-limit", "50"])
        .output()
        .unwrap();
    assert!(output.status.success());
}
