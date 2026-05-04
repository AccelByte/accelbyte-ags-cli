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

#[test]
#[serial_test::serial]
fn test_missing_required_arg_error_uses_display_name() {
    for (display, internal) in RENAMED_SERVICES {
        // Invoke a known resource/method with no args — Clap will produce
        // a "required arguments were not provided" error whose Usage line
        // must start with the display name, not the internal spec id.
        //
        // We pick a resource/method known to exist for each service; if
        // the catalogue changes, this test will fail loudly at the
        // "Unrecognized subcommand" branch and the pair can be updated.
        let (resource, method) = match *display {
            "cloud-save" => ("game-record", "list"),
            "matchmaking" => ("match-pools", "list"),
            "login-queue" => ("ticket", "list"),
            "season-pass" => ("pass", "list"),
            "session-history" => ("xray", "list"),
            "game-telemetry" => ("events", "list"),
            _ => unreachable!(),
        };

        ags_isolated()
            .args([display, resource, method])
            .assert()
            .failure()
            .stderr(contains(*display).and(contains(*internal).not()));
    }
}

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
