use crate::common::cli_helpers::ags_isolated;

// ── Dry-run output ──

/// Dry-run prints the HTTP method, URL, and auth header so users can preview requests before executing
#[test]
fn test_dry_run_output() {
    let output = ags_isolated()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--dry-run",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout, @"
    GET https://<base-url>/iam/v3/admin/namespaces/test/users/platforms/justice
    Authorization: Bearer <token>
    ");
}

// ── Auth status when not logged in ──

/// Unauthenticated status shows a clear "Not authenticated" headline on stdout
#[test]
fn test_auth_status_not_logged_in_headline() {
    let output = ags_isolated().args(["auth", "status"]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    insta::assert_snapshot!(stdout, @"✕ Not authenticated");
}

// ── Invalid --json input ──

/// Inline --json with bad syntax surfaces the file/stdin alternatives so PowerShell
/// users (where shell quoting commonly mangles inline JSON) see a path forward.
#[test]
fn test_invalid_inline_json_suggests_file_or_stdin_alternative() {
    let output = ags_isolated()
        .args([
            "social",
            "stat-definitions",
            "create",
            "--namespace",
            "test",
            "--json",
            "not valid json",
            "-y",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Invalid JSON for --json:"),
        "expected parse-error headline; got: {stderr}"
    );
    assert!(
        stderr.contains("--json @file.json") && stderr.contains("--json @-"),
        "expected file/stdin alternatives in suggestion; got: {stderr}"
    );
}

/// Unauthenticated status includes next-step and env-var tip on stderr so users know how to proceed
#[test]
fn test_auth_status_not_logged_in_details() {
    let output = ags_isolated().args(["auth", "status"]).output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    insta::assert_snapshot!(stderr, @"
    → Next: Run 'ags auth login'.
        Tip: You can also set AGS_BASE_URL, AGS_CLIENT_ID, AGS_CLIENT_SECRET for non-interactive workflows.
    ");
}
