use crate::common::cli_helpers::ags_isolated;

/// Helper: parse doctor JSON and return the first profile object.
fn first_profile(json: &serde_json::Value) -> &serde_json::Value {
    let profiles = json["profiles"].as_array().expect("profiles array");
    assert!(!profiles.is_empty(), "must have at least one profile");
    &profiles[0]
}

#[test]
fn test_json_envelope_has_required_fields() {
    let output = ags_isolated()
        .args(["doctor", "--offline", "--format", "json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");

    assert!(json["status"].is_string(), "must have top-level 'status'");
    assert!(json["profiles"].is_array(), "must have 'profiles' array");

    let profile = first_profile(&json);
    assert!(profile["status"].is_string(), "profile must have 'status'");
    assert!(
        profile["profile"].is_string(),
        "profile must have 'profile'"
    );
    assert!(
        profile["warnings"].is_number(),
        "profile must have 'warnings'"
    );
    assert!(profile["errors"].is_number(), "profile must have 'errors'");
    assert!(
        profile["skipped"].is_number(),
        "profile must have 'skipped'"
    );
    assert!(profile["checks"].is_array(), "profile must have 'checks'");
}

#[test]
fn test_each_check_has_required_fields() {
    let output = ags_isolated()
        .args(["doctor", "--offline", "--format", "json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");

    let profile = first_profile(&json);
    let checks = profile["checks"].as_array().expect("checks array");
    assert!(!checks.is_empty(), "should have at least one check");

    for check in checks {
        assert!(check["tier"].is_string(), "check must have 'tier': {check}");
        assert!(check["name"].is_string(), "check must have 'name': {check}");
        assert!(
            check["title"].is_string(),
            "check must have 'title': {check}"
        );
        assert!(
            check["status"].is_string(),
            "check must have 'status': {check}"
        );
        assert!(
            check["message"].is_string(),
            "check must have 'message': {check}"
        );
    }
}

#[test]
fn test_failed_checks_have_suggestion() {
    let output = ags_isolated()
        .args(["doctor", "--offline", "--format", "json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");

    let profile = first_profile(&json);
    let checks = profile["checks"].as_array().expect("checks array");
    for check in checks {
        if check["status"] == "fail" {
            assert!(
                check["suggestion"].is_string(),
                "failed check '{}' must have suggestion",
                check["name"]
            );
        }
    }
}

#[test]
fn test_skipped_checks_have_reason() {
    let output = ags_isolated()
        .args(["doctor", "--offline", "--format", "json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");

    let profile = first_profile(&json);
    let checks = profile["checks"].as_array().expect("checks array");
    let skipped: Vec<_> = checks.iter().filter(|c| c["status"] == "skipped").collect();

    for check in skipped {
        let msg = check["message"].as_str().unwrap_or("");
        assert!(
            msg.starts_with("skipped"),
            "skipped check '{}' message must start with 'skipped': {msg}",
            check["name"]
        );
    }
}

#[test]
fn test_status_reflects_worst_result() {
    let output = ags_isolated()
        .args(["doctor", "--offline", "--format", "json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");

    let status = json["status"].as_str().expect("top-level status");
    let profile = first_profile(&json);
    let checks = profile["checks"].as_array().expect("checks array");

    let has_fail = checks.iter().any(|c| c["status"] == "fail");
    let has_warning = checks.iter().any(|c| c["status"] == "warning");

    if has_fail {
        assert_eq!(status, "fail");
    } else if has_warning {
        assert_eq!(status, "warning");
    } else {
        assert_eq!(status, "pass");
    }
}

#[test]
fn test_no_sensitive_values_in_output() {
    let output = ags_isolated()
        .env("AGS_CLIENT_SECRET", "super-secret-value-12345")
        .args(["doctor", "--offline", "--format", "json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("super-secret-value-12345"),
        "output must never contain secret values"
    );
}
