use crate::common::cli_helpers::ags_isolated;
use predicates::prelude::*;

// ── Help ──

/// Profile help shows all available subcommands
#[test]
fn test_profile_help() {
    ags_isolated()
        .args(["profile", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("create"))
        .stdout(predicate::str::contains("use"))
        .stdout(predicate::str::contains("show"))
        .stdout(predicate::str::contains("delete"))
        .stdout(predicate::str::contains("rename"));
}

// ── List ──

/// List with no profiles shows "No profiles found" message
#[test]
fn test_profile_list_empty() {
    ags_isolated()
        .args(["profile", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No profiles found"));
}

/// List displays all created profiles
#[test]
fn test_profile_list_shows_profiles() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    // Create two profiles
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
        .args(["profile", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("prod"))
        .stdout(predicate::str::contains("staging"));
}

/// List marks the active profile with an "(active)" suffix
#[test]
fn test_profile_list_marks_active() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "use", "staging"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("staging (active)"));
}

/// List in JSON format returns a profiles array with name fields
#[test]
fn test_profile_list_json() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "list", "--format", "json"])
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["profiles"].is_array());
    assert_eq!(json["profiles"][0]["name"], "staging");
}

// ── Create ──

/// Create a new profile succeeds with a confirmation message
#[test]
fn test_profile_create_success() {
    ags_isolated()
        .args(["profile", "create", "staging"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Profile 'staging' created"));
}

/// Create a profile with a name that already exists fails
#[test]
fn test_profile_create_duplicate_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

/// Create a profile with an invalid name (contains spaces) fails
#[test]
fn test_profile_create_invalid_name_fails() {
    ags_isolated()
        .args(["profile", "create", "my profile"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Invalid profile name"));
}

/// Create normalises uppercase profile names to lowercase
#[test]
fn test_profile_create_normalises_case() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "Staging"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Profile 'staging' created"));
}

// ── Use ──

/// Use switches the active profile with a confirmation message
#[test]
fn test_profile_use_success() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "use", "staging"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Switched to profile 'staging'"));
}

/// Use a nonexistent profile fails with "does not exist" error
#[test]
fn test_profile_use_nonexistent_fails() {
    ags_isolated()
        .args(["profile", "use", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

// ── Show ──

/// Show displays the active profile details including config fields
#[test]
fn test_profile_show_active_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "use", "staging"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("staging"))
        .stdout(predicate::str::contains("active"))
        .stdout(predicate::str::contains("Base URL"))
        .stdout(predicate::str::contains("Client ID"));
}

/// Show a specific profile by name displays that profile's details
#[test]
fn test_profile_show_named_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "show", "staging"])
        .assert()
        .success()
        .stdout(predicate::str::contains("staging"));
}

/// Show a nonexistent profile fails with "does not exist" error
#[test]
fn test_profile_show_nonexistent_fails() {
    ags_isolated()
        .args(["profile", "show", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

/// Show in JSON format returns profile name, base_url, and has_token fields
#[test]
fn test_profile_show_json() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();

    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "show", "staging", "--format", "json"])
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["profile"], "staging");
    assert!(json.get("base_url").is_some());
    assert!(json.get("has_token").is_some());
}

// ── Delete ──

/// Delete removes a profile and it no longer appears in show
#[test]
fn test_profile_delete_success() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "delete", "staging"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Profile 'staging' deleted"));

    // Verify it's gone
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "show", "staging"])
        .assert()
        .failure();
}

/// Delete a nonexistent profile fails with "does not exist" error
#[test]
fn test_profile_delete_nonexistent_fails() {
    ags_isolated()
        .args(["profile", "delete", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

/// Delete the active profile clears the active-profile setting in global config
#[test]
fn test_profile_delete_active_clears_global_config() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "use", "staging"])
        .assert()
        .success();
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "delete", "staging"])
        .assert()
        .success();

    // Active profile should be cleared — list should not show any active marker
    let output = ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "list", "--format", "json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["active_profile"].is_null());
}

// ── Rename ──

/// Rename changes a profile's name and the old name no longer exists
#[test]
fn test_profile_rename_success() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "rename", "staging", "dev"])
        .assert()
        .success()
        .stdout(predicate::str::contains("renamed to 'dev'"));

    // Old name should not exist
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "show", "staging"])
        .assert()
        .failure();

    // New name should exist
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "show", "dev"])
        .assert()
        .success();
}

/// Rename a nonexistent source profile fails with "does not exist" error
#[test]
fn test_profile_rename_nonexistent_source_fails() {
    ags_isolated()
        .args(["profile", "rename", "nonexistent", "new-name"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

/// Rename to a name that already exists fails with "already exists" error
#[test]
fn test_profile_rename_duplicate_target_fails() {
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
        .args(["profile", "rename", "staging", "prod"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

/// Rename the active profile updates the active-profile reference in global config
#[test]
fn test_profile_rename_active_updates_global_config() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().to_str().unwrap();

    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "create", "staging"])
        .assert()
        .success();
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "use", "staging"])
        .assert()
        .success();
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "rename", "staging", "dev"])
        .assert()
        .success();

    // Active profile should now be "dev"
    ags_isolated()
        .env("AGS_HOME", home)
        .args(["profile", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dev (active)"));
}
