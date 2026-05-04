use crate::common::cli_helpers::ags_isolated;
use serde_json::Value;

// ── Root catalogue ──

/// `ags describe` returns a catalogue of all 24 services
#[test]
fn test_root_catalogue_returns_all_services() {
    let output = ags_isolated().args(["describe"]).output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["schema_version"], "1");
    assert_eq!(json["kind"], "catalogue");
    assert_eq!(json["path"], Value::Array(vec![]));
    assert_eq!(json["data"]["node_type"], "root");
    assert_eq!(json["data"]["name"], "ags");

    let children = json["data"]["children"].as_array().unwrap();
    assert_eq!(children.len(), 24);
}

/// Root catalogue children have the expected structure
#[test]
fn test_root_catalogue_child_structure() {
    let output = ags_isolated().args(["describe"]).output().unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();

    let children = json["data"]["children"].as_array().unwrap();
    let iam = children.iter().find(|c| c["name"] == "iam").unwrap();
    assert_eq!(iam["node_type"], "service");
    assert_eq!(iam["path"], serde_json::json!(["iam"]));
    assert!(!iam["summary"].as_str().unwrap().is_empty());
}

/// Root catalogue includes the generated_by metadata
#[test]
fn test_root_catalogue_has_generator_info() {
    let output = ags_isolated().args(["describe"]).output().unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["generated_by"]["cli"], "ags");
    assert!(!json["generated_by"]["version"].as_str().unwrap().is_empty());
}

// ── Service catalogue ──

/// `ags describe iam` returns a catalogue of IAM resources
#[test]
fn test_service_catalogue_returns_resources() {
    let output = ags_isolated().args(["describe", "iam"]).output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["kind"], "catalogue");
    assert_eq!(json["path"], serde_json::json!(["iam"]));
    assert_eq!(json["data"]["node_type"], "service");
    assert_eq!(json["data"]["name"], "iam");

    let children = json["data"]["children"].as_array().unwrap();
    assert!(!children.is_empty());

    // All children are resources
    for child in children {
        assert_eq!(child["node_type"], "resource");
    }
}

/// Service catalogue children are sorted alphabetically
#[test]
fn test_service_catalogue_children_sorted() {
    let output = ags_isolated().args(["describe", "iam"]).output().unwrap();
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();

    let children = json["data"]["children"].as_array().unwrap();
    let names: Vec<&str> = children
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

// ── Resource catalogue ──

/// `ags describe iam users` returns a catalogue of methods in the users resource
#[test]
fn test_resource_catalogue_returns_methods() {
    let output = ags_isolated()
        .args(["describe", "iam", "users"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["kind"], "catalogue");
    assert_eq!(json["path"], serde_json::json!(["iam", "users"]));
    assert_eq!(json["data"]["node_type"], "resource");
    assert_eq!(json["data"]["name"], "users");

    let children = json["data"]["children"].as_array().unwrap();
    assert!(!children.is_empty());
    for child in children {
        assert_eq!(child["node_type"], "method");
    }
}

// ── Method introspection ──

/// `ags describe iam users list` returns full command introspection
#[test]
fn test_method_returns_command_kind() {
    let output = ags_isolated()
        .args([
            "describe",
            "iam",
            "users",
            "list-users-with-accelbyte-account",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["kind"], "command");
    assert_eq!(
        json["path"],
        serde_json::json!(["iam", "users", "list-users-with-accelbyte-account"])
    );
}

/// Method describe exposes the canonical command string and default scope
#[test]
fn test_method_has_command_and_default_scope() {
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

    assert_eq!(
        json["data"]["command"],
        "iam users list-users-with-accelbyte-account"
    );
    // `list-users-with-accelbyte-account` lives under admin scope in the IAM spec; default_scope must be
    // populated whenever the method has any contracts.
    assert!(json["data"]["default_scope"].is_string());
}

/// Method describe exposes a scope → version → contract matrix with full
/// per-contract detail (path, verb, x-operationId, permissions).
#[test]
fn test_method_has_scope_version_matrix() {
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
    assert!(!scopes.is_empty());
    let default_scope = json["data"]["default_scope"].as_str().unwrap();
    let scope_block = &scopes[default_scope];

    assert!(scope_block["default_version"]
        .as_str()
        .unwrap()
        .starts_with('v'));
    let supported = scope_block["supported_versions"].as_array().unwrap();
    assert!(!supported.is_empty());

    let default_version = scope_block["default_version"].as_str().unwrap();
    let contract = &scope_block["contracts"][default_version];
    assert_eq!(contract["http_method"], "GET");
    assert!(contract["path_template"]
        .as_str()
        .unwrap()
        .contains("/iam/"));
    assert!(contract["x_operation_id"].is_string());
    assert!(contract["permissions"].is_array());
    assert!(contract["parameters"].is_array());
}

// ── Error cases ──

/// Unknown service returns an error envelope with suggestions
#[test]
fn test_unknown_service_returns_error() {
    let output = ags_isolated().args(["describe", "iamx"]).output().unwrap();
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json["kind"], "error");
    assert_eq!(json["data"]["code"], "unknown_service");
}

/// Unknown resource returns an error envelope
#[test]
fn test_unknown_resource_returns_error() {
    let output = ags_isolated()
        .args(["describe", "iam", "userz"])
        .output()
        .unwrap();
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json["kind"], "error");
    assert_eq!(json["data"]["code"], "unknown_resource");
    // Suggestions array is present (may be empty if fuzzy matching not yet implemented)
    assert!(json["data"]["suggestions"].is_array());
}

/// Unknown method returns an error envelope
#[test]
fn test_unknown_method_returns_error() {
    let output = ags_isolated()
        .args(["describe", "iam", "users", "listz"])
        .output()
        .unwrap();
    assert!(!output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(json["kind"], "error");
    assert_eq!(json["data"]["code"], "unknown_method");
}

/// Too many arguments returns an error
#[test]
fn test_too_many_args_returns_error() {
    let output = ags_isolated()
        .args([
            "describe",
            "iam",
            "users",
            "list-users-with-accelbyte-account",
            "extra",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

// ── Parameter name vs CLI flag name ──

/// Parameter names emitted by describe must match the actual Clap flag that
/// the CLI accepts. Snake_case spec names are kebab-cased when built into
/// Clap flags, and describe output must follow the same transform.
///
/// Regression: before the fix, describe emitted `client_id` / `response_type`
/// but the CLI accepted only `--client-id` / `--response-type`.
#[test]
fn test_describe_parameter_names_match_cli_flags() {
    let output = ags_isolated()
        .args(["describe", "iam", "oauth2", "authorize"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let scopes = json["data"]["scopes"].as_object().unwrap();
    let scope = scopes.values().next().unwrap();
    let contracts = scope["contracts"].as_object().unwrap();
    let contract = contracts.values().next().unwrap();
    let params = contract["parameters"].as_array().unwrap();

    let names: Vec<&str> = params.iter().map(|p| p["name"].as_str().unwrap()).collect();

    // No emitted name may contain an underscore — they must be kebab-cased
    // to match `builder.rs` which does `param.name.replace('_', '-')`.
    for n in &names {
        assert!(
            !n.contains('_'),
            "describe emitted parameter '{n}' with an underscore; \
             CLI will reject --{n} because flags are kebab-cased"
        );
    }

    // Spot-check two known snake_case params from the spec.
    assert!(
        names.contains(&"client-id"),
        "expected client-id in {names:?}"
    );
    assert!(
        names.contains(&"response-type"),
        "expected response-type in {names:?}"
    );
}

/// camelCase parameter names (e.g. userId) are kebab-cased to match the CLI flag
/// so that `--user-id` is the flag name rather than `--userId`.
#[test]
fn test_describe_kebab_cases_parameter_names() {
    let output = ags_isolated()
        .args(["describe", "iam", "users", "get"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let scopes = json["data"]["scopes"].as_object().unwrap();
    let scope = scopes.values().next().unwrap();
    let contracts = scope["contracts"].as_object().unwrap();
    let contract = contracts.values().next().unwrap();
    let params = contract["parameters"].as_array().unwrap();

    let names: Vec<&str> = params.iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert!(
        names.contains(&"user-id"),
        "camelCase parameter should be kebab-cased; got {names:?}"
    );
}

// ── Response content-type metadata ──

/// Binary-producing endpoints surface their content type and an is_binary flag.
#[test]
fn test_describe_surfaces_response_content_type_for_binary_endpoint() {
    let output = ags_isolated()
        .args(["describe", "platform", "payment-station", "get-qr-code"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let scopes = json["data"]["scopes"].as_object().unwrap();
    let scope = scopes.values().next().unwrap();
    let contracts = scope["contracts"].as_object().unwrap();
    let contract = contracts.values().next().unwrap();

    assert_eq!(contract["response_content_type"], "image/png");
    assert_eq!(contract["response_is_binary"], true);
}

/// Text endpoints omit response_is_binary (or set it to false) and may
/// include application/json as the content type.
#[test]
fn test_describe_does_not_mark_json_endpoint_as_binary() {
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
    let scopes = json["data"]["scopes"].as_object().unwrap();
    let scope = scopes.values().next().unwrap();
    let contracts = scope["contracts"].as_object().unwrap();
    let contract = contracts.values().next().unwrap();

    let is_binary = contract["response_is_binary"].as_bool().unwrap_or(false);
    assert!(
        !is_binary,
        "iam users list should not be marked binary; got {contract:?}"
    );
}

// ── Display name aliases ──

/// Display name aliases appear in the root catalogue (e.g. match2 → matchmaking)
#[test]
fn test_aliased_service_name_in_root_catalogue() {
    // "matchmaking" is the display name for internal "match2"
    let output = ags_isolated().args(["describe"]).output().unwrap();
    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let children = json["data"]["children"].as_array().unwrap();

    // The root catalogue should list "matchmaking" (the display name), not "match2"
    let matchmaking = children.iter().find(|c| c["name"] == "matchmaking");
    assert!(
        matchmaking.is_some(),
        "Expected 'matchmaking' in root catalogue"
    );
    assert_eq!(matchmaking.unwrap()["node_type"], "service");
}
