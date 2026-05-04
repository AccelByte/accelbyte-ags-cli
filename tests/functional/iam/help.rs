use crate::common::cli_helpers::ags;
use predicates::prelude::*;

/// `iam --help` lists all dynamically generated resource subcommands from the OpenAPI spec
#[test]
fn test_iam_help_lists_resources() {
    ags()
        .args(["iam", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Resources:"))
        .stdout(predicate::str::contains("users"))
        .stdout(predicate::str::contains("roles"))
        .stdout(predicate::str::contains("bans"))
        .stdout(predicate::str::contains("oauth2"))
        .stdout(predicate::str::contains("clients"));
}

/// Resource descriptions from RESOURCE_DESCRIPTIONS appear in help for discoverability
#[test]
fn test_iam_help_shows_resource_descriptions() {
    ags()
        .args(["iam", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "User accounts, profiles, bans, and permissions",
        ))
        .stdout(predicate::str::contains(
            "Ban management and ban type configuration",
        ));
}

/// `iam users --help` lists available methods so users know what operations exist
#[test]
fn test_iam_users_help_lists_operations() {
    ags()
        .args(["iam", "users", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Methods:"))
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("get"))
        .stdout(predicate::str::contains("ban"));
}

/// Operation-level help shows the dynamically generated flags from the OpenAPI spec parameters
#[test]
fn test_iam_users_list_help_shows_params() {
    ags()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--help",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("--namespace"))
        .stdout(predicate::str::contains("--limit"))
        .stdout(predicate::str::contains("--offset"));
}

/// HTML entities from OpenAPI descriptions are decoded before rendering in help text
#[test]
fn test_operation_help_no_html_entities() {
    let output = ags()
        .args([
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "--help",
        ])
        .output()
        .expect("failed to run ags");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("&#39;"),
        "Help output should not contain &#39;, got:\n{}",
        stdout
    );
    assert!(
        !stdout.contains("&amp;"),
        "Help output should not contain &amp;, got:\n{}",
        stdout
    );
    assert!(
        !stdout.contains("&lt;"),
        "Help output should not contain &lt;, got:\n{}",
        stdout
    );
    assert!(
        !stdout.contains("&gt;"),
        "Help output should not contain &gt;, got:\n{}",
        stdout
    );
}
