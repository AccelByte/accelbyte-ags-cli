use crate::common::cli_helpers::ags_isolated;

/// Auth errors appear on stderr so scripts consuming stdout never see error text
#[test]
fn test_auth_error_routes_to_stderr() {
    let output = ags_isolated()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--yes",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stderr.contains('\u{2715}'),
        "Error should be on stderr, got stdout: {stdout}, stderr: {stderr}"
    );
}

/// Users never see Rust panics or backtraces, only curated error messages
#[test]
fn test_error_output_has_no_stack_traces() {
    let output = ags_isolated()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--yes",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panic"),
        "Error output should not contain panic traces: {stderr}"
    );
    assert!(
        !stderr.contains("thread '"),
        "Error output should not contain thread traces: {stderr}"
    );
    assert!(
        !stderr.contains("stack backtrace"),
        "Error output should not contain stack backtrace: {stderr}"
    );
}

/// Stdout stays empty on failure so piped consumers (jq, scripts) get no partial data
#[test]
fn test_stdout_empty_on_error() {
    let output = ags_isolated()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--namespace",
            "test",
            "--yes",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.is_empty(),
        "Stdout should be empty on error, got: {stdout}"
    );
}
