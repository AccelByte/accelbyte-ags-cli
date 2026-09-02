use crate::common::cli_helpers::ags_isolated;
use serde_json::Value;

// -- Envelope contract --

/// Every describe response has the required envelope fields so consumers can rely on a stable shape
#[test]
fn test_envelope_has_required_fields() {
    let output = ags_isolated().args(["describe"]).output().unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(
        json["schema_version"].is_string(),
        "schema_version must be a string"
    );
    assert!(json["kind"].is_string(), "kind must be a string");
    assert!(json["path"].is_array(), "path must be an array");
    assert!(
        json["generated_by"].is_object(),
        "generated_by must be an object"
    );
    assert!(json["data"].is_object(), "data must be an object");
}

/// The generated_by field always includes cli name and version
#[test]
fn test_generated_by_has_required_fields() {
    let output = ags_isolated().args(["describe"]).output().unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(json["generated_by"]["cli"], "ags");
    assert!(json["generated_by"]["version"].is_string());
    assert!(!json["generated_by"]["version"].as_str().unwrap().is_empty());
}

// -- Catalogue contract --

/// Catalogue responses always have node_type, name, summary, and children
#[test]
fn test_catalogue_data_has_required_fields() {
    let output = ags_isolated().args(["describe"]).output().unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(json["data"]["node_type"].is_string(), "node_type required");
    assert!(json["data"]["name"].is_string(), "name required");
    assert!(json["data"]["summary"].is_string(), "summary required");
    assert!(json["data"]["children"].is_array(), "children required");
}

/// Every catalogue child has node_type, name, path, and summary
#[test]
fn test_catalogue_children_have_required_fields() {
    let output = ags_isolated().args(["describe"]).output().unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();

    for child in json["data"]["children"].as_array().unwrap() {
        assert!(child["node_type"].is_string(), "child node_type required");
        assert!(child["name"].is_string(), "child name required");
        assert!(child["path"].is_array(), "child path required");
        assert!(child["summary"].is_string(), "child summary required");
    }
}

// -- Command contract --

/// Command (method-level) responses expose a scope/version matrix: `command`,
/// `default_scope`, and a `scopes` map of per-scope contract detail.
#[test]
fn test_command_data_has_required_sections() {
    let output = ags_isolated()
        .args([
            "describe",
            "iam",
            "users",
            "list-users-with-accelbyte-account",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert!(json["data"]["command"].is_string(), "command required");
    assert!(
        json["data"]["default_scope"].is_string() || json["data"]["default_scope"].is_null(),
        "default_scope required (string or null)"
    );
    assert!(
        json["data"]["scopes"].is_object(),
        "scopes section required"
    );
}

/// Every contract in the matrix carries the full callable contract: HTTP verb,
/// path template, parameters, request body (nullable), response (nullable),
/// permissions, and the stable `x_operation_id`.
#[test]
fn test_contract_entries_have_required_fields() {
    let output = ags_isolated()
        .args([
            "describe",
            "iam",
            "users",
            "list-users-with-accelbyte-account",
        ])
        .output()
        .unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();

    let scopes = json["data"]["scopes"].as_object().unwrap();
    assert!(!scopes.is_empty(), "at least one scope required");

    for (_scope_name, scope_block) in scopes {
        assert!(
            scope_block["default_version"]
                .as_str()
                .map(|s| s.starts_with('v'))
                .unwrap_or(false),
            "default_version must be a `v<N>` string"
        );
        assert!(
            scope_block["supported_versions"].is_array(),
            "supported_versions required"
        );
        let contracts = scope_block["contracts"].as_object().unwrap();
        for (_version_key, contract) in contracts {
            assert!(contract["http_method"].is_string(), "http_method required");
            assert!(
                contract["path_template"].is_string(),
                "path_template required"
            );
            assert!(contract["parameters"].is_array(), "parameters required");
            assert!(contract["permissions"].is_array(), "permissions required");
            assert!(
                contract["x_operation_id"].is_string(),
                "x_operation_id required"
            );
        }
    }
}

// -- Error contract --

/// Error responses use kind "error" and include code, message, and suggestions
#[test]
fn test_error_envelope_has_required_fields() {
    let output = ags_isolated()
        .args(["describe", "nonexistent"])
        .output()
        .unwrap();
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["kind"], "error");
    assert!(json["data"]["code"].is_string(), "error code required");
    assert!(
        json["data"]["message"].is_string(),
        "error message required"
    );
    assert!(
        json["data"]["suggestions"].is_array(),
        "suggestions required"
    );
}

// -- Channel routing contract --

/// Describe success output goes to stdout only, never stderr
#[test]
fn test_success_output_on_stdout_only() {
    let output = ags_isolated().args(["describe"]).output().unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!stdout.is_empty(), "describe must produce stdout");
    assert!(
        stderr.is_empty(),
        "describe success must not produce stderr: {stderr}"
    );
}

/// Describe output is always valid JSON
#[test]
fn test_output_is_always_valid_json() {
    let cases = vec![
        vec!["describe"],
        vec!["describe", "iam"],
        vec!["describe", "iam", "users"],
        vec![
            "describe",
            "iam",
            "users",
            "list-users-with-accelbyte-account",
        ],
        vec!["describe", "workflow"],
        vec!["describe", "workflow", "competitive-multiplayer"],
    ];

    for args in &cases {
        let output = ags_isolated().args(args).output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: Result<Value, _> = serde_json::from_str(&stdout);
        assert!(
            result.is_ok(),
            "args {:?} must produce valid JSON: {}",
            args,
            stdout
        );
    }
}

/// Describe output is pretty-printed for readability
#[test]
fn test_output_is_pretty_printed() {
    let output = ags_isolated().args(["describe"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains('\n'), "JSON output must be pretty-printed");
}

// -- Workflow describe dispatch --

#[test]
fn test_describe_workflow_catalogue_via_cli() {
    let output = ags_isolated()
        .args(["describe", "workflow"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["kind"], "catalogue");
    assert_eq!(json["data"]["node_type"], "workflow-catalogue");
    let children = json["data"]["children"].as_array().unwrap();
    assert!(children
        .iter()
        .any(|c| c["name"] == "competitive-multiplayer"));
    // Every workflow child carries node_type "workflow" and a 2-segment
    // ["workflow", <id>] path — the machine-readable handle consumers traverse.
    for child in children {
        assert_eq!(child["node_type"], "workflow");
        let path = child["path"].as_array().unwrap();
        assert_eq!(path.len(), 2);
        assert_eq!(path[0], "workflow");
    }
}

#[test]
fn test_describe_workflow_detail_via_cli() {
    let output = ags_isolated()
        .args(["describe", "workflow", "competitive-multiplayer"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["kind"], "workflow");
    assert_eq!(json["data"]["id"], "competitive-multiplayer");
    assert!(json["data"]["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["name"] == "fleet-region" && i["dynamic"] == true));

    // A local step publishes kind "local" with an action name and empty
    // service/operation, so consumers must not assume service/operation
    // are always present.
    let upload = json["data"]["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "upload-image")
        .expect("upload-image step present");
    assert_eq!(upload["kind"], "local");
    assert_eq!(upload["action"], "ams/upload-image");

    // Required shape of the WorkflowDescribeData contract, independent of the
    // specific built-in workflow's values.
    assert!(json["data"]["name"].is_string());
    assert!(json["data"]["intent"].is_string());
    assert!(json["data"]["description"].is_string());
    assert!(json["data"]["steps"].is_array());
    for input in json["data"]["inputs"].as_array().unwrap() {
        assert!(input["name"].is_string());
        assert!(input["required"].is_boolean());
        assert!(input["sensitive"].is_boolean());
        assert!(input["dynamic"].is_boolean());
    }
    for step in json["data"]["steps"].as_array().unwrap() {
        assert!(step["id"].is_string());
        assert!(step["dependencies"].is_array());
        // An API step carries kind "api" with service + operation; a local
        // step carries kind "local" with an action name. Exactly one shape.
        assert!(step["kind"].is_string());
        if step["kind"] == "local" {
            assert!(step["action"].is_string());
        } else {
            assert!(step["service"].is_string());
            assert!(step["operation"].is_string());
        }
    }
}

#[test]
fn test_describe_workflow_unknown_id_via_cli() {
    let output = ags_isolated()
        .args(["describe", "workflow", "nope"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "unknown workflow exits non-zero");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["kind"], "error");
    assert_eq!(json["data"]["code"], "unknown_workflow");
}

#[test]
fn test_describe_workflow_extra_segment_via_cli() {
    let output = ags_isolated()
        .args(["describe", "workflow", "competitive-multiplayer", "x"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "invalid path exits non-zero");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["kind"], "error");
    assert_eq!(json["data"]["code"], "invalid_workflow_path");
}

#[test]
fn test_describe_root_lists_workflow_via_cli() {
    let output = ags_isolated().args(["describe"]).output().unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["data"]["children"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["name"] == "workflow" && c["node_type"] == "workflow-catalogue"));
}
