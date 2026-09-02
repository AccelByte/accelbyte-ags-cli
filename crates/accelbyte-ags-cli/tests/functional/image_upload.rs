//! Functional tests for `ags extend image-upload`.
//!
//! Exercises the full CLI path: argv → clap parse → router classification →
//! route handler → rendered output. The command is an imperative handler (not
//! workflow-backed), so `--dry-run` requires Docker to be on PATH (the handler
//! probes Docker availability before the dry-run short-circuit).

use crate::common::cli_helpers::ags_isolated;
use predicates::prelude::*;

/// The docker-not-found error message emitted by `check_docker_available()`.
/// Used to detect whether a failure is due to Docker being absent on the
/// system rather than a real test failure. Must match the message in
/// `image_upload/mod.rs::check_docker_available`.
const DOCKER_NOT_FOUND_MSG: &str = "not installed or not found on PATH";

/// Returns true if stderr contains the docker-not-found error message,
/// indicating the failure is environmental (no Docker), not a bug.
fn is_docker_not_found(stderr: &str) -> bool {
    stderr.contains(DOCKER_NOT_FOUND_MSG)
}

// ── --help ──

/// `--help` renders usage text on stdout and exits cleanly.
/// This exercises the full CLI path (argv → router → route → help render)
/// with stdin closed, proving non-interactive operation.
#[test]
fn test_help_renders_usage() {
    ags_isolated()
        .args(["extend", "image-upload", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("image-upload"))
        .stdout(predicate::str::contains("--app"))
        .stdout(predicate::str::contains("--image-tag"))
        .stdout(predicate::str::contains("--namespace"));
}

// ── Missing required flags ──

/// Without `--app` the command fails with a usage error.
/// Clap enforces `required(true)` before the handler runs, so no Docker or
/// network dependency.
#[test]
fn test_missing_app_fails() {
    ags_isolated()
        .args([
            "extend",
            "image-upload",
            "--namespace",
            "ns",
            "--image-tag",
            "v1.0",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--app"));
}

/// Without `--image-tag` the command fails with a usage error.
/// Clap enforces `required(true)` before the handler runs.
#[test]
fn test_missing_image_tag_fails() {
    ags_isolated()
        .args([
            "extend",
            "image-upload",
            "--namespace",
            "ns",
            "--app",
            "myapp",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--image-tag"));
}

/// Without `--namespace` (and no `AGS_NAMESPACE` env, no profile config), the
/// handler's `resolve_namespace` fails with a Usage error mentioning
/// `--namespace`. This is NOT a clap rejection — `--namespace` is a global
/// flag that falls through to env and profile config per
/// `cli-reference.md §10.5.12` and the runtime's `resolve_namespace`
/// function. The error fires only after Docker is found and the dry-run
/// check passes (step 5 of the handler), so this test requires Docker on
/// PATH.
///
/// Contract: `ags_runtime::runtime::execution::resolve_namespace` returns
/// `None` when flag, env, and profile config are all unset; the handler
/// maps this to `CliError::Usage` mentioning `--namespace`.
#[test]
fn test_missing_namespace_fails_after_docker_check() {
    let mut cmd = ags_isolated();
    // Clear AGS_NAMESPACE to guarantee the "all sources unset" path.
    cmd.env_remove("AGS_NAMESPACE");

    let output = cmd
        .args([
            "extend",
            "image-upload",
            "--app",
            "myapp",
            "--image-tag",
            "v1.0",
        ])
        .output()
        .unwrap();

    // The command must fail — either because Docker is not on PATH (step 1)
    // or because no namespace source is available (step 5). Both are non-zero
    // exit codes. We assert failure and check for EITHER the docker-not-found
    // message or a namespace error, covering both CI environments.
    assert!(
        !output.status.success(),
        "must fail when no namespace and no --namespace flag"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    let mentions_namespace = stderr.contains("--namespace") || stderr.contains("namespace");
    let mentions_docker_missing = is_docker_not_found(&stderr);
    assert!(
        mentions_namespace || mentions_docker_missing,
        "stderr must mention either --namespace or docker-not-found: {stderr}"
    );
}

// ── --dry-run ──

/// `--dry-run` produces a preview on stderr and exits 0.
/// Requires Docker on PATH because `check_docker_available()` runs before
/// the dry-run short-circuit (handler steps 1 → 2).
#[test]
fn test_dry_run_produces_preview() {
    let output = ags_isolated()
        .args([
            "--dry-run",
            "extend",
            "image-upload",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--image-tag",
            "v1.0",
        ])
        .output()
        .unwrap();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // If Docker is not on PATH, the command fails before the dry-run
        // check. This is expected in CI environments without Docker.
        if is_docker_not_found(&stderr) {
            eprintln!("skipped: docker not on PATH");
            return;
        }
        panic!("dry-run must succeed (or fail only due to docker not found):\nstderr: {stderr}");
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Dry run") || stderr.contains("dry_run") || stderr.contains("dry run"),
        "dry-run preview must appear on stderr:\n{stderr}"
    );
}

/// `--dry-run --format json` does not emit any chrome on stdout.
/// The handler's dry-run preview writes to stderr; stdout must be clean
/// for automation consumers.
#[test]
fn test_dry_run_format_json_no_chrome_on_stdout() {
    let output = ags_isolated()
        .args([
            "--dry-run",
            "--format",
            "json",
            "extend",
            "image-upload",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--image-tag",
            "v1.0",
        ])
        .output()
        .unwrap();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_docker_not_found(&stderr) {
            eprintln!("skipped: docker not on PATH");
            return;
        }
        panic!("dry-run --format json must succeed (or fail only due to docker not found):\nstderr: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().is_empty(),
        "stdout must be empty (no chrome) in --format json --dry-run mode:\n{stdout}"
    );
}

/// `--dry-run` makes no network call. Proves the dry-run guard fires.
/// Since no `AGS_BASE_URL` is set (and no credentials), any network call
/// would fail. Success implies no network call was attempted.
#[test]
fn test_dry_run_makes_no_network_call() {
    let output = ags_isolated()
        .args([
            "--dry-run",
            "extend",
            "image-upload",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--image-tag",
            "v1.0",
        ])
        .output()
        .unwrap();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_docker_not_found(&stderr) {
            eprintln!("skipped: docker not on PATH");
            return;
        }
        panic!("dry-run must succeed (or fail only due to docker not found):\nstderr: {stderr}");
    }

    // The command succeeded with --dry-run. Since no AGS_BASE_URL was set
    // (and no credentials), any network call would have failed. Success
    // implies no network call was attempted.
}

// ── --yes and --no-input ──

/// `--yes` is a global flag that must be accepted without error.
/// image-upload has no confirmation prompts so --yes has no observable
/// effect beyond being accepted by the prescan. Combined with --dry-run
/// to avoid needing real credentials.
#[test]
fn test_yes_flag_is_accepted() {
    let output = ags_isolated()
        .args([
            "--dry-run",
            "--yes",
            "extend",
            "image-upload",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--image-tag",
            "v1.0",
        ])
        .output()
        .unwrap();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_docker_not_found(&stderr) {
            eprintln!("skipped: docker not on PATH");
            return;
        }
        panic!("--yes --dry-run must succeed:\nstderr: {stderr}");
    }
}

/// `--no-input` is a global flag that must be accepted without error.
/// The image-upload handler is fully non-interactive — it never prompts,
/// so --no-input has no observable effect beyond being accepted.
#[test]
fn test_no_input_flag_is_accepted() {
    let output = ags_isolated()
        .args([
            "--dry-run",
            "--no-input",
            "extend",
            "image-upload",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--image-tag",
            "v1.0",
        ])
        .output()
        .unwrap();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_docker_not_found(&stderr) {
            eprintln!("skipped: docker not on PATH");
            return;
        }
        panic!("--no-input --dry-run must succeed:\nstderr: {stderr}");
    }
}

// ── Retry flag validation ──

/// A negative `--retry-interval` is rejected at the CLI parse boundary
/// with a usage error naming the flag. The validator enforces finite
/// non-negative values before any Docker or network call runs.
///
/// Contract: `--retry-interval` custom value_parser rejects values < 0.
/// The error names the flag so the user knows which input is wrong.
#[test]
fn test_negative_retry_interval_rejected() {
    let output = ags_isolated()
        .args([
            "extend",
            "image-upload",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--image-tag",
            "v1.0",
            "--retry-limit",
            "1",
            "--retry-interval",
            "-1",
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "must fail with negative --retry-interval"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("retry-interval"),
        "error must name the --retry-interval flag: {stderr}"
    );
}

/// A negative `--retry-rate` is rejected at the CLI parse boundary
/// with a usage error naming the flag.
///
/// Contract: `--retry-rate` custom value_parser rejects values < 0.
#[test]
fn test_negative_retry_rate_rejected() {
    let output = ags_isolated()
        .args([
            "extend",
            "image-upload",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--image-tag",
            "v1.0",
            "--retry-rate",
            "-2",
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "must fail with negative --retry-rate"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("retry-rate"),
        "error must name the --retry-rate flag: {stderr}"
    );
}

// ── Image tag validation ──

/// A `--image-tag` containing `/` is rejected at the CLI parse boundary.
/// Docker tags follow `[A-Za-z0-9_][A-Za-z0-9._-]{0,127}`; slashes
/// are not permitted and would redirect authenticated OCI registry
/// requests to a different path.
///
/// Contract: the custom tag validator rejects `/` as an invalid character.
#[test]
fn test_image_tag_with_slash_rejected() {
    let output = ags_isolated()
        .args([
            "extend",
            "image-upload",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--image-tag",
            "v1/evil",
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "must fail with tag containing slash"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("image-tag"),
        "error must name the --image-tag flag: {stderr}"
    );
}

/// A `--image-tag` containing `..` is rejected. Path traversal segments
/// in a tag could redirect authenticated OCI manifest requests.
///
/// Contract: the custom tag validator rejects `..` as path traversal.
#[test]
fn test_image_tag_with_dot_dot_rejected() {
    let output = ags_isolated()
        .args([
            "extend",
            "image-upload",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--image-tag",
            "../../admin",
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "must fail with tag containing path traversal"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("image-tag"),
        "error must name the --image-tag flag: {stderr}"
    );
}

// ── --app "" ──

/// An empty `--app ""` value passes clap's `required(true)` check (which
/// validates presence, not content). At runtime the empty string would be
/// rejected by `encode_url_path_segment` in the CSM API call (handler
/// step 5), but reaching that point requires Docker on PATH and valid
/// auth. In most CI environments the command fails earlier: at step 1
/// (Docker absent) or at step 5's auth resolution (no credentials
/// configured).
///
/// This test asserts the command fails without panicking when given an
/// empty `--app` value. The specific rejection layer depends on the
/// runtime environment.
///
/// Contract: `ags_runtime::support::strings::encode_url_path_segment("", "app")`
/// returns Err (empty parameter).
#[test]
fn test_empty_app_value_fails() {
    let output = ags_isolated()
        .args([
            "extend",
            "image-upload",
            "--namespace",
            "ns",
            "--app",
            "",
            "--image-tag",
            "v1.0",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success(), "must fail with empty --app value");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.trim().is_empty(),
        "must produce a diagnostic error message, not fail silently"
    );
}

/// An empty `--image-tag ""` value is rejected at the clap layer by the
/// custom tag-format validator, which enforces the Docker tag charset
/// `[A-Za-z0-9_][A-Za-z0-9._-]{0,127}` (minimum one character).
///
/// Contract: the custom tag value_parser rejects empty strings.
#[test]
fn test_empty_image_tag_value_fails() {
    let output = ags_isolated()
        .args([
            "extend",
            "image-upload",
            "--namespace",
            "ns",
            "--app",
            "myapp",
            "--image-tag",
            "",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success(), "must fail with empty --image-tag");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("image-tag"),
        "error must name the --image-tag flag: {stderr}"
    );
}
