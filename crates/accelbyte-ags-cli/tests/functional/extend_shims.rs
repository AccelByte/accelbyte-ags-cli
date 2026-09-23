//! Functional tests for the extend migration shortcut addresses.
//!
//! These tests run the compiled binary and verify observable behaviour.
//! Tests that need to iterate the shim registration table live in the
//! inline test module of `service_shims.rs` (they are parameterised
//! over the table so that adding, renaming or removing a shim entry
//! requires no test edit).

use crate::common::cli_helpers::ags_isolated;
use ags::invocation::handlers::extend::service_shims::{ExtendShim, SHIMS};
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

/// A wait-capable shim's `--help` renders the shortcut's own help page (via
/// the shim-presentation overrides) showing the operation flags
/// (--app / --json / --api-version) AND the shim-only `--wait*` flags, so one
/// page documents both.
#[test]
fn wait_capable_shim_help_shows_operation_flags_and_wait_flags() {
    let assert = ags_isolated()
        .args(["extend", "deploy-app", "--help"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    for flag in [
        "--app",
        "--json",
        "--api-version",
        "--wait",
        "--wait-interval",
        "--wait-limit",
    ] {
        assert!(
            stdout.contains(flag),
            "deploy-app --help must document '{flag}':\n{stdout}"
        );
    }
}

/// A non-wait shim's `--help` shows the operation help with no `--wait*` flags.
#[test]
fn non_wait_shim_help_omits_wait_flags() {
    let assert = ags_isolated()
        .args(["extend", "get-app-info", "--help"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        !stdout.contains("--wait"),
        "get-app-info (no --wait support) must not document any --wait flag:\n{stdout}"
    );
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

// ── Shortcut help page rendering ──
//
// Every test below is table-driven over the SHIMS slice so that
// adding or removing a shim entry requires no test edit.

/// Derive the `ags extend <address>` display address for a shim,
/// the same derivation the production path must use.
fn shim_display_address(shim: &ExtendShim) -> String {
    match shim.parent {
        Some(parent) => format!("ags extend {} {}", parent, shim.name),
        None => format!("ags extend {}", shim.name),
    }
}

/// CLI tokens to request `--help` for a shim.
fn shim_help_tokens(shim: &ExtendShim) -> Vec<String> {
    let mut tokens = vec!["extend".to_string()];
    if let Some(parent) = shim.parent {
        tokens.push(parent.to_string());
    }
    tokens.push(shim.name.to_string());
    tokens.push("--help".to_string());
    tokens
}

/// CLI tokens to request `-h` for a shim.
fn shim_short_help_tokens(shim: &ExtendShim) -> Vec<String> {
    let mut tokens = vec!["extend".to_string()];
    if let Some(parent) = shim.parent {
        tokens.push(parent.to_string());
    }
    tokens.push(shim.name.to_string());
    tokens.push("-h".to_string());
    tokens
}

/// Canonical address string for a shim entry.
fn shim_canonical_address(shim: &ExtendShim) -> String {
    format!("ags {} {} {}", shim.service, shim.resource, shim.method)
}

/// Test 1+7: Every shortcut's help Usage: line shows the display
/// address and NOT the canonical CSM address. The child shortcut
/// (`app-ui create`) is covered by the same iteration.
#[test]
fn shim_help_shows_display_address_in_usage_line() {
    for shim in SHIMS {
        let tokens = shim_help_tokens(shim);
        let output = ags_isolated()
            .env("NO_COLOR", "1")
            .args(&tokens)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let display = shim_display_address(shim);

        let usage_pattern = format!("{} [OPTIONS]", display);
        assert!(
            stdout.contains(&usage_pattern),
            "shim '{}' usage must contain '{}'; got:\n{}",
            shim.name,
            usage_pattern,
            stdout,
        );
    }
}

/// Test 2+7: Every shortcut's help opens with the shim's `summary`
/// field from the registration table.
#[test]
fn shim_help_opens_with_summary() {
    for shim in SHIMS {
        let tokens = shim_help_tokens(shim);
        let output = ags_isolated()
            .env("NO_COLOR", "1")
            .args(&tokens)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);

        let first_line = stdout.lines().next().unwrap_or("");
        assert_eq!(
            first_line, shim.summary,
            "shim '{}' help must open with summary '{}'; first line: '{}'",
            shim.name, shim.summary, first_line,
        );
    }
}

/// Test 3+7: Every shortcut's `Example:` line begins with the
/// display address, not the canonical address.
#[test]
fn shim_help_example_uses_display_address() {
    for shim in SHIMS {
        let tokens = shim_help_tokens(shim);
        let output = ags_isolated()
            .env("NO_COLOR", "1")
            .args(&tokens)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let display = shim_display_address(shim);

        let example_prefix = format!("  {}", display);
        // The after-help Example: block is the LAST "Example:" section.
        // Earlier occurrences inside the `--json` schema long-help must be
        // skipped, so we scan for the final header and then check the first
        // non-empty line after it.
        let lines: Vec<&str> = stdout.lines().collect();
        let last_example_header = lines.iter().rposition(|line| {
            let t = line.trim();
            t.starts_with("Example") && t.contains(':') && !t.contains('{')
        });
        let found = if let Some(header_idx) = last_example_header {
            let first_content = lines[header_idx + 1..]
                .iter()
                .find(|line| !line.trim().is_empty())
                .copied()
                .unwrap_or("");
            assert!(
                first_content.starts_with(&example_prefix),
                "shim '{}' example must start with '{}'; got: '{}'\nfull:\n{}",
                shim.name,
                example_prefix,
                first_content,
                stdout,
            );
            true
        } else {
            false
        };
        assert!(
            found,
            "shim '{}' help must contain an Example: section;\n{}",
            shim.name, stdout,
        );
    }
}

/// Every shortcut's help contains both sentences of the canonical
/// command block, derived from the SHIMS table.
#[test]
fn shim_help_contains_canonical_block() {
    for shim in SHIMS {
        let tokens = shim_help_tokens(shim);
        let output = ags_isolated()
            .env("NO_COLOR", "1")
            .args(&tokens)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let canonical = shim_canonical_address(shim);

        assert!(
            stdout.contains(&format!("Canonical command: {}", canonical)),
            "shim '{}' help must contain 'Canonical command: {}'; got:\n{}",
            shim.name,
            canonical,
            stdout,
        );
        assert!(
            stdout.contains("This shortcut forwards to it. Both addresses are supported."),
            "shim '{}' help must contain both canonical sentences; got:\n{}",
            shim.name,
            stdout,
        );
    }
}

/// Test 4: For every shortcut, every option flag from the canonical
/// page also appears on the shortcut's help page.
#[test]
fn shim_help_preserves_options_and_contract() {
    for shim in SHIMS {
        let shim_out = ags_isolated()
            .env("NO_COLOR", "1")
            .args(shim_help_tokens(shim))
            .output()
            .unwrap();
        let shim_help = String::from_utf8_lossy(&shim_out.stdout);

        let canon_out = ags_isolated()
            .env("NO_COLOR", "1")
            .args([shim.service, shim.resource, shim.method, "--help"])
            .output()
            .unwrap();
        let canon_help = String::from_utf8_lossy(&canon_out.stdout);

        // Every `--flag` in the canonical page must appear in the shim page.
        // Flags inside the `--json` long help (input forms, schema) are
        // included — both pages render the same operation, so the body
        // schema section must match too.
        for line in canon_help.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("--") {
                let flag = trimmed.split_whitespace().next().unwrap();
                assert!(
                    shim_help.contains(flag),
                    "shim '{}' help must contain flag '{}' from canonical page",
                    shim.name,
                    flag,
                );
            }
        }

        // The Default contract: block must be present.
        assert!(
            shim_help.contains("Default contract:"),
            "shim '{}' help must contain 'Default contract:' block",
            shim.name,
        );
    }
}

/// Test 5: The canonical `ags csm ...` pages are unaffected by the
/// shortcut override. No `Canonical command:` block must appear.
#[test]
fn canonical_help_pages_unaffected_by_shim_overrides() {
    for shim in SHIMS {
        let output = ags_isolated()
            .env("NO_COLOR", "1")
            .args([shim.service, shim.resource, shim.method, "--help"])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let canonical = shim_canonical_address(shim);

        assert!(
            !stdout.contains("Canonical command:"),
            "canonical '{}' help must not contain 'Canonical command:'; got:\n{}",
            canonical,
            stdout,
        );
        let usage_pattern = format!("{} [OPTIONS]", canonical);
        assert!(
            stdout.contains(&usage_pattern),
            "canonical '{}' usage must show '{}'; got:\n{}",
            canonical,
            usage_pattern,
            stdout,
        );
    }
}

/// Test 6: `-h` and `--help` produce the same page for every shortcut.
#[test]
fn shim_help_short_and_long_flags_produce_same_page() {
    for shim in SHIMS {
        let long_out = ags_isolated()
            .env("NO_COLOR", "1")
            .args(shim_help_tokens(shim))
            .output()
            .unwrap();
        let short_out = ags_isolated()
            .env("NO_COLOR", "1")
            .args(shim_short_help_tokens(shim))
            .output()
            .unwrap();

        let long_stdout = String::from_utf8_lossy(&long_out.stdout);
        let short_stdout = String::from_utf8_lossy(&short_out.stdout);

        assert_eq!(
            long_stdout, short_stdout,
            "shim '{}' --help and -h must produce identical output",
            shim.name,
        );
    }
}

/// Test 9: The canonical address on the rendered help page matches
/// the `alias_of` path in the `ags describe` envelope, for every
/// shim. Both sides read from real binary output — neither side is
/// a literal from the SHIMS table — so drift between the two
/// agent-facing surfaces is caught.
#[test]
fn shim_help_canonical_matches_describe_alias() {
    for shim in SHIMS {
        // --- Side A: read the canonical address from the help page ---
        let help_out = ags_isolated()
            .env("NO_COLOR", "1")
            .args(shim_help_tokens(shim))
            .output()
            .unwrap();
        let help_stdout = String::from_utf8_lossy(&help_out.stdout);

        let canonical_line = help_stdout
            .lines()
            .find(|line| line.starts_with("Canonical command:"))
            .unwrap_or_else(|| {
                panic!(
                    "shim '{}' help must contain a 'Canonical command:' line;\n{}",
                    shim.name, help_stdout,
                )
            });

        // Parse `Canonical command: ags csm apps create` → ["csm", "apps", "create"]
        let canonical_from_help: Vec<&str> = canonical_line
            .trim_start_matches("Canonical command:")
            .trim()
            .strip_prefix("ags ")
            .unwrap_or_else(|| {
                panic!(
                    "shim '{}' canonical line must start with 'ags ': '{}'",
                    shim.name, canonical_line,
                )
            })
            .split_whitespace()
            .collect();

        // --- Side B: read the canonical address from `ags describe` ---
        let mut describe_args: Vec<&str> = vec!["--format", "json", "describe", "extend"];
        if let Some(parent) = shim.parent {
            describe_args.push(parent);
        }
        describe_args.push(shim.name);

        let describe_out = ags_isolated().args(&describe_args).output().unwrap();
        let describe_stdout = String::from_utf8_lossy(&describe_out.stdout);
        let json: serde_json::Value = serde_json::from_str(&describe_stdout).unwrap_or_else(|e| {
            panic!(
                "describe output for '{}' must be valid JSON ({e}):\n{describe_stdout}",
                shim.name,
            )
        });

        let alias_of = json["data"]["alias_of"].as_array().unwrap_or_else(|| {
            panic!(
                "describe '{}' must have data.alias_of array:\n{}",
                shim.name, describe_stdout,
            )
        });
        let canonical_from_describe: Vec<&str> =
            alias_of.iter().filter_map(|v| v.as_str()).collect();

        assert_eq!(
            canonical_from_help, canonical_from_describe,
            "shim '{}' help-page canonical address {:?} must match describe alias_of {:?}",
            shim.name, canonical_from_help, canonical_from_describe,
        );
    }
}

/// Test 10: No deprecation language in any shortcut's help output.
#[test]
fn shim_help_contains_no_deprecation_language() {
    let forbidden = ["deprecated", "obsolete", "instead of", "use ags csm"];
    for shim in SHIMS {
        let tokens = shim_help_tokens(shim);
        let output = ags_isolated()
            .env("NO_COLOR", "1")
            .args(&tokens)
            .output()
            .unwrap();
        let stdout_lower = String::from_utf8_lossy(&output.stdout).to_lowercase();

        for word in &forbidden {
            assert!(
                !stdout_lower.contains(&word.to_lowercase()),
                "shim '{}' help must not contain '{}'; got:\n{}",
                shim.name,
                word,
                stdout_lower,
            );
        }
    }
}
