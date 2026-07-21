//! Regression guards for user-facing service names in Clap-generated errors.
//!
//! Six services are renamed between their OpenAPI spec id and the CLI
//! display name (e.g. `cloudsave` → `cloud-save`, `match2` → `matchmaking`).
//! Error messages produced by Clap must use the display name — not the
//! internal id — so users see the same name they typed.

use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

use crate::common::cli_helpers::ags_isolated;

/// (cli-name, internal-name) pairs — internal name must never leak into errors.
const RENAMED_SERVICES: &[(&str, &str)] = &[
    ("cloud-save", "cloudsave"),
    ("matchmaking", "match2"),
    ("login-queue", "loginqueue"),
    ("season-pass", "seasonpass"),
    ("session-history", "sessionhistory"),
    ("game-telemetry", "gametelemetry"),
];

// `test_missing_required_arg_error_uses_display_name` was removed in the
// workflow-executor migration (Plan D, Task 5): service-command args are no
// longer marked clap-`required`, so clap never produces the
// "required arguments were not provided" error this test guarded. Missing
// required inputs are now gathered interactively (human mode) or rejected by
// the JSON-mode strictness check in `handle_service`. The renamed-service
// display-name guarantee is still covered by the unknown-subcommand test
// below, which exercises a clap error path that remains.

#[test]
#[serial_test::serial]
fn test_unknown_subcommand_error_uses_display_name() {
    for (display, internal) in RENAMED_SERVICES {
        ags_isolated()
            .args([display, "no-such-resource"])
            .assert()
            .failure()
            .stderr(contains(*display).and(contains(*internal).not()));
    }
}

/// A corrected `x-operationId` typo keeps the former command name working as a
/// hidden Clap alias: the old `delete-publisheed` still resolves to the
/// corrected `delete-published` operation (same `DELETE /stores/published`),
/// so the rename is non-breaking. Guards the back-compat entries in
/// `ags_runtime::catalogue::former_method_names`.
#[test]
fn test_former_command_name_still_resolves_via_alias() {
    // The former (typo'd) name resolves to the corrected operation.
    ags_isolated()
        .args([
            "--dry-run",
            "platform",
            "stores",
            "delete-publisheed",
            "--namespace",
            "ns",
        ])
        .assert()
        .success()
        .stdout(contains("DELETE").and(contains("/stores/published")));

    // The corrected name resolves identically.
    ags_isolated()
        .args([
            "--dry-run",
            "platform",
            "stores",
            "delete-published",
            "--namespace",
            "ns",
        ])
        .assert()
        .success()
        .stdout(contains("DELETE").and(contains("/stores/published")));
}
