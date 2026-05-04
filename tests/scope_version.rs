//! End-to-end integration tests for `--api-scope` and `--api-version` error paths.
//!
//! All tests that make API requests use `--dry-run` so no network or credentials
//! are required.

use assert_cmd::Command;

/// Thin output wrapper with `.status.code()`, `.stderr_str()`, and `.stdout_str()`.
struct CmdOutput {
    inner: std::process::Output,
}

impl CmdOutput {
    fn status_code(&self) -> Option<i32> {
        self.inner.status.code()
    }

    fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.inner.stderr).into_owned()
    }

    fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.inner.stdout).into_owned()
    }
}

/// Spawn the `ags` binary with the given args and return its output.
fn cli(args: &[&str]) -> CmdOutput {
    let inner = Command::cargo_bin("ags")
        .unwrap()
        .args(args)
        .env("AGS_NO_KEYCHAIN", "1")
        .output()
        .expect("failed to execute ags binary");
    CmdOutput { inner }
}

/// An unsupported `--api-scope` value must produce the spec-shaped error message
/// and exit with code 1, not Clap's "missing required argument" error.
#[test]
fn test_unsupported_scope_error_matches_spec_shape() {
    // `iam users create` supports admin and public; "server" is not a valid scope.
    let out = cli(&["iam", "users", "create", "--api-scope", "server"]);
    assert_eq!(
        out.status_code(),
        Some(1),
        "expected exit code 1, stderr: {}",
        out.stderr_str()
    );
    assert!(
        out.stderr_str()
            .contains("api scope 'server' is not supported"),
        "stderr did not contain expected shape, got: {}",
        out.stderr_str()
    );
}

/// An unsupported `--api-version` value must produce the spec-shaped error message
/// and also mention `--api-scope` so users know how to combine the two flags.
#[test]
fn test_unsupported_version_error_matches_spec_shape() {
    // `iam users get` has a default scope (public), so no scope required.
    let out = cli(&["iam", "users", "get", "--api-version", "v99"]);
    assert_eq!(
        out.status_code(),
        Some(1),
        "expected exit code 1, stderr: {}",
        out.stderr_str()
    );
    assert!(
        out.stderr_str()
            .contains("api version 'v99' is not supported"),
        "stderr did not contain expected shape, got: {}",
        out.stderr_str()
    );
    assert!(
        out.stderr_str().contains("--api-scope"),
        "stderr should mention --api-scope, got: {}",
        out.stderr_str()
    );
}

/// With no `--api-scope` or `--api-version`, the default contract for
/// `iam users create` routes to the `admin` endpoint at the bundled spec's
/// current default version. Asserted with a `/iam/v\d+/admin/` regex so a
/// future spec version bump (e.g. v4 → v5) doesn't break the test.
#[test]
fn test_default_contract_routes_to_admin_default_version() {
    let out = cli(&[
        "--dry-run",
        "iam",
        "users",
        "create",
        "--namespace",
        "ns",
        "--json",
        r#"{"authType":"EMAILPASSWD","country":"US","emailAddress":"x@y.z","username":"u","password":"pw","dateOfBirth":"1990-01-01"}"#,
    ]);
    assert_eq!(
        out.status_code(),
        Some(0),
        "expected success, stderr: {}",
        out.stderr_str()
    );
    let stdout = out.stdout_str();
    let admin_re = regex::Regex::new(r"/iam/v\d+/admin/").unwrap();
    assert!(
        admin_re.is_match(&stdout),
        "dry-run stdout did not route to /iam/v<N>/admin/, got: {stdout}"
    );
    assert!(
        !stdout.contains("/public/"),
        "dry-run stdout should not contain /public/, got: {stdout}"
    );
}

/// With `--api-scope public`, the same command must route to the `public`
/// endpoint at the bundled spec's current default public version.
#[test]
fn test_explicit_api_scope_public_routes_to_public_endpoint() {
    let out = cli(&[
        "--dry-run",
        "iam",
        "users",
        "create",
        "--api-scope",
        "public",
        "--namespace",
        "ns",
        "--json",
        r#"{"authType":"EMAILPASSWD","country":"US","emailAddress":"x@y.z","username":"u","password":"pw","dateOfBirth":"1990-01-01"}"#,
    ]);
    assert_eq!(
        out.status_code(),
        Some(0),
        "expected success, stderr: {}",
        out.stderr_str()
    );
    let stdout = out.stdout_str();
    let public_re = regex::Regex::new(r"/iam/v\d+/public/").unwrap();
    assert!(
        public_re.is_match(&stdout),
        "dry-run stdout did not route to /iam/v<N>/public/, got: {stdout}"
    );
}
