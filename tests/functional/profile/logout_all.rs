use crate::common::cli_helpers::ags_isolated;
use predicates::prelude::*;

/// Logout --all clears credentials from all profiles and names each one
#[test]
fn test_logout_all_clears_multiple_profiles() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "prod"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["auth", "logout", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Credentials cleared from"))
        .stdout(predicate::str::contains("staging"))
        .stdout(predicate::str::contains("prod"));
}

/// Logout --all with no profiles succeeds gracefully
#[test]
fn test_logout_all_with_no_profiles() {
    ags_isolated()
        .args(["auth", "logout", "--all"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("No profiles to clear")
                .or(predicate::str::contains("Credentials cleared from")),
        );
}

/// Logout --all combined with --profile flag fails as mutually exclusive
#[test]
fn test_logout_all_with_profile_flag_is_error() {
    ags_isolated()
        .args(["auth", "logout", "--all", "--profile", "staging"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("mutually exclusive"));
}

/// Logout --all in JSON format returns cleared status and profiles array
#[test]
fn test_logout_all_json_format() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["auth", "logout", "--all", "--format", "json"])
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "cleared");
    assert!(json["profiles"].is_array());
}
