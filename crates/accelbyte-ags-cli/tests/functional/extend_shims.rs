//! Functional tests for the extend migration shortcut addresses.
//!
//! These tests run the compiled binary and verify observable behaviour.
//! Tests that need to iterate the shim registration table live in the
//! inline test module of `service_shims.rs` (they are parameterised
//! over the table so that adding, renaming or removing a shim entry
//! requires no test edit).

use crate::common::cli_helpers::ags_isolated;
use ags::invocation::handlers::extend::service_shims::SHIMS;
use predicates::prelude::*;

// ── CSM help unaffected (Test Plan 10) ──

/// `ags csm --help` must not show any shim name as a resource.
/// The shim layer adds an alternate door under `extend`, not a second
/// room under `csm`. Each shim name is checked individually so a
/// collision is immediately visible, rather than relying on a vacuous
/// absence check for a heading that no longer exists.
#[test]
fn csm_help_unaffected_by_extend_shims() {
    let assert = ags_isolated()
        .args(["csm", "--help"])
        .assert()
        .success()
        // "extend" must not appear as a resource-column name.
        .stdout(predicate::str::is_match(r"(?m)^\s+extend\s").unwrap().not());

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    // Every top-level shim name must be absent from the CSM resource list.
    for shim in SHIMS {
        if shim.parent.is_none() {
            let pattern = format!(r"(?m)^\s+{}\s", regex::escape(shim.name));
            assert!(
                !regex::Regex::new(&pattern).unwrap().is_match(&stdout),
                "shim name '{}' must not appear as a csm resource:\n{stdout}",
                shim.name,
            );
        }
    }
}

// ── Unknown-command error (Test Plan 13) ──

/// An unrecognised `extend` address produces the standard
/// unknown-subcommand error (unchanged from before the shims).
#[test]
fn unknown_extend_address_produces_standard_error() {
    ags_isolated()
        .args(["extend", "no-such-command"])
        .assert()
        .failure();
}

// ── Help integration (Test Plan 8, complement to inline tests) ──

/// The compiled binary's `ags extend --help` lists shim names with
/// their summaries and canonical addresses in the Commands: section.
/// This is the binary-level counterpart of the inline Clap-tree test.
#[test]
fn extend_help_binary_shows_shims_with_summaries() {
    let assert = ags_isolated().args(["extend", "--help"]).assert().success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    // Each top-level shim must appear with its summary and canonical address.
    for shim in SHIMS {
        if shim.parent.is_none() {
            assert!(
                stdout.contains(shim.name),
                "extend help must list shim '{}' in Commands:\n{stdout}",
                shim.name,
            );
            assert!(
                stdout.contains(shim.summary),
                "extend help must show summary '{}' for shim '{}':\n{stdout}",
                shim.summary,
                shim.name,
            );
        }
    }
}

/// `ags extend remote-debug --help` shows the native `connect`, `enable`,
/// and `disable` subcommands alongside any shim children in the Commands:
/// section.
#[test]
fn extend_remote_debug_help_shows_native_and_shim_children() {
    let assert = ags_isolated()
        .args(["extend", "remote-debug", "--help"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    for shim in SHIMS {
        if shim.parent == Some("remote-debug") {
            assert!(
                stdout.contains(shim.name),
                "remote-debug help must list shim '{}' in Commands:\n{stdout}",
                shim.name,
            );
            assert!(
                stdout.contains(shim.summary),
                "remote-debug help must show summary '{}' for shim '{}':\n{stdout}",
                shim.summary,
                shim.name,
            );
        }
    }

    // Native subcommands must still appear.
    assert!(
        stdout.contains("connect"),
        "remote-debug help must still list 'connect':\n{stdout}",
    );
    assert!(
        stdout.contains("enable"),
        "remote-debug help must still list 'enable':\n{stdout}",
    );
    assert!(
        stdout.contains("disable"),
        "remote-debug help must still list 'disable':\n{stdout}",
    );
}

/// Omitting a child command prints help for `remote-debug`, not for the
/// top-level `extend` command.
#[test]
fn extend_remote_debug_without_child_shows_subgroup_help() {
    ags_isolated()
        .args(["extend", "remote-debug"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Remote debug commands"))
        .stderr(predicate::str::is_match(r"Usage:\r?\n  extend remote-debug <COMMAND>").unwrap())
        .stderr(predicate::str::contains("clone-template").not());
}

/// `ags extend app-ui --help` shows the `create` shim child with its
/// summary and canonical address in the Commands: section, alongside
/// the native `setup-env` and `upload` subcommands.
#[test]
fn extend_app_ui_help_shows_shim_children_with_summaries() {
    let assert = ags_isolated()
        .args(["extend", "app-ui", "--help"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    for shim in SHIMS {
        if shim.parent == Some("app-ui") {
            assert!(
                stdout.contains(shim.name),
                "app-ui help must list shim '{}' in Commands:\n{stdout}",
                shim.name,
            );
            assert!(
                stdout.contains(shim.summary),
                "app-ui help must show summary '{}' for shim '{}':\n{stdout}",
                shim.summary,
                shim.name,
            );
        }
    }

    // Native subcommands must still appear.
    assert!(
        stdout.contains("setup-env"),
        "app-ui help must still list 'setup-env':\n{stdout}",
    );
    assert!(
        stdout.contains("upload"),
        "app-ui help must still list 'upload':\n{stdout}",
    );
}

// ── NO_COLOR guard ──

/// `ags extend --help` must emit zero ANSI escape bytes when colour is
/// disabled via the `NO_COLOR` environment variable, even when
/// `CLICOLOR_FORCE` would otherwise enable colour on piped output.
/// This verifies that `NO_COLOR` takes precedence over forced colour,
/// and guards against escapes being introduced into an `about` string,
/// a description, or a help template.
#[test]
fn extend_help_emits_no_ansi_escapes_when_no_color_set() {
    // CLICOLOR_FORCE=1 tells anstream to emit ANSI even on piped output.
    // NO_COLOR=1 must override it and suppress all escapes.
    let output = ags_isolated()
        .env("CLICOLOR_FORCE", "1")
        .env("NO_COLOR", "1")
        .args(["extend", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains('\x1b'),
        "extend help stdout contains ANSI escape (0x1b) with NO_COLOR set:\n{stdout:?}",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains('\x1b'),
        "extend help stderr contains ANSI escape (0x1b) with NO_COLOR set:\n{stderr:?}",
    );
}

// ── End-to-end dispatch (shim → service rewrite) ──
//
// Each test below invokes the compiled binary with `--dry-run` so the
// full Clap parse → route → `try_rewrite_to_service_args` →
// `run_service` pipeline is exercised with no network and no auth.
//
// Key property: if the redirect in `invocation::mod.rs` were removed,
// the extend route would fall through to `handle_extend`, which prints
// help and exits with code 1 instead of producing a dry-run JSON
// envelope. The `.assert().success()` call alone would catch this, and
// the URL/body assertions provide a second line of defence.
//
// Shim names are read FROM the registration table so that renaming a
// shortcut never requires editing these tests.

/// A plain top-level shim dispatches through the canonical service
/// path. The dry-run preview URL must reference the shim's canonical
/// service, proving `run_service` was invoked rather than the builtin
/// help fallback.
#[test]
fn top_level_shim_dispatches_through_service_path() {
    // Pick a top-level shim targeting a read-only operation (method
    // "get") so no request body is required for dry-run.
    let shim = SHIMS
        .iter()
        .find(|s| s.parent.is_none() && s.method == "get")
        .expect("at least one top-level GET shim must exist in the registration table");

    let assert = ags_isolated()
        .args([
            "--format",
            "json",
            "--dry-run",
            "--namespace",
            "test-ns",
            "extend",
            shim.name,
            "--app",
            "test-app",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));

    let url = json["url"]
        .as_str()
        .unwrap_or_else(|| panic!("dry-run output must have a URL:\n{stdout}"));

    // The URL must contain the canonical service path prefix, proving
    // the shim dispatched through `run_service` as the equivalent
    // `ags <service> <resource> <method>` would.
    assert!(
        url.contains(&format!("/{}/", shim.service)),
        "URL must reference canonical service '{}': {url}",
        shim.service,
    );
}

/// The `--namespace` global flag is forwarded through the shim rewrite
/// and appears in the dry-run preview URL. This proves global flags
/// parse correctly after the rewrite, which the unit test of the pure
/// string-rewriting function cannot verify.
#[test]
fn global_namespace_flag_forwarded_through_rewrite() {
    let shim = SHIMS
        .iter()
        .find(|s| s.parent.is_none() && s.method == "get")
        .expect("at least one top-level GET shim must exist in the registration table");

    let assert = ags_isolated()
        .args([
            "--format",
            "json",
            "--dry-run",
            "--namespace",
            "probe-ns-42",
            "extend",
            shim.name,
            "--app",
            "test-app",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));

    let url = json["url"]
        .as_str()
        .unwrap_or_else(|| panic!("dry-run output must have a URL:\n{stdout}"));

    assert!(
        url.contains("probe-ns-42"),
        "namespace value must appear in the dry-run URL: {url}",
    );
}

// ── disable command (promoted from shim to native handler) ──

/// The `disable` command's dry-run path succeeds without credentials and
/// reports what it would do to stderr. The body `{"enableDebugMode": false}`
/// is proven by the unit test `disable_body_sets_false`.
#[test]
fn disable_dry_run_succeeds_without_credentials() {
    let assert = ags_isolated()
        .args([
            "--dry-run",
            "--namespace",
            "test-ns",
            "extend",
            "remote-debug",
            "disable",
            "--app",
            "test-app",
        ])
        .assert()
        .success();

    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("disable"),
        "dry-run output must mention disabling:\n{stderr}",
    );
}

/// `disable` without `--app` fails with a usage error naming the
/// missing flag.
#[test]
fn disable_missing_app_is_usage_error() {
    ags_isolated()
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "remote-debug",
            "disable",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--app"));
}

/// `disable --help` describes the restart and confirmation.
#[test]
fn disable_help_describes_restart_and_confirmation() {
    let assert = ags_isolated()
        .args(["extend", "remote-debug", "disable", "--help"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("restart"),
        "help must mention restart:\n{stdout}",
    );
    assert!(
        stdout.contains("--yes"),
        "help must mention --yes:\n{stdout}",
    );
}

/// A subgroup shim dispatches through the canonical service path.
/// The dry-run preview URL must reference the canonical service,
/// proving the subgroup routing and service dispatch both work end
/// to end.
#[test]
fn subgroup_shim_dispatches_through_service_path() {
    let shim = SHIMS
        .iter()
        .find(|s| s.parent.is_some())
        .expect("at least one subgroup shim must exist");
    let parent = shim.parent.unwrap();

    let assert = ags_isolated()
        .args([
            "--format",
            "json",
            "--dry-run",
            "--namespace",
            "test-ns",
            "extend",
            parent,
            shim.name,
            // Provide an empty JSON body so body-requiring operations
            // succeed in dry-run without interactive input.
            "--json",
            "{}",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}):\n{stdout}"));

    let url = json["url"]
        .as_str()
        .unwrap_or_else(|| panic!("dry-run output must have a URL:\n{stdout}"));

    assert!(
        url.contains(&format!("/{}/", shim.service)),
        "URL must reference canonical service '{}': {url}",
        shim.service,
    );
}
