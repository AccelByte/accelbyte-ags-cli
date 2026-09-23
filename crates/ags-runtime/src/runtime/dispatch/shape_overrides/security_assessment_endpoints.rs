//! Bespoke columns for `ags csm security-assessment get-app-endpoints`
//! (`csm/admin/security-assessment/v1/get-app-endpoints`). The generic
//! shaper treats this response as a single entity — 4 scalar sibling
//! fields sit next to the `endpoints` array, so `detect_shape`'s
//! array-unwrap heuristic never triggers — collapsing the whole array into
//! an opaque "N items" field. This always renders `endpoints` as its own
//! table instead, with a notes line above it carrying the sibling flags
//! (hasAPISpec/hasGRPCReflection/isAppRunning) — see [`app_flags_note`].
//!
//! Encodes the same response shape as the native `request` handler's typed
//! `Endpoint`/`EndpointInfoResult` — see
//! crates/accelbyte-ags-cli/src/invocation/handlers/extend/security_assessment_request/api.rs.
//! The two can't share a type across the crate boundary; review both
//! together if this response shape changes.

use ags_protocol::result::{CommandResult, FieldValue};
use serde_json::{Map, Value};

pub(super) const OPERATION_ID: &str = "csm/admin/security-assessment/v1/get-app-endpoints";

pub(super) fn shape(body: &Value) -> Option<CommandResult> {
    let result = super::fixed_columns_from_array(
        body,
        "endpoints",
        "endpoints",
        &[
            ("Method", |e| super::text_field(e.get("method"))),
            ("Path", |e| super::text_field(e.get("path"))),
            ("Operation ID", |e| super::text_field(e.get("operationId"))),
            ("Permission", permission_cell),
            ("Authenticated", authenticated_cell),
        ],
    )?;
    let CommandResult::Collection(mut collection) = result else {
        return Some(result);
    };
    collection.notes = vec![app_flags_note(body)];
    Some(CommandResult::Collection(collection))
}

/// Not auto-discovered → `—` (the interactive `request` checklist would
/// prompt for one); discovered → `RESOURCE [ACTION]`, matching the wording
/// the interactive checklist already uses (`security_assessment_request::
/// checklist::PermissionCell` / `permission::display_permission` in the
/// `accelbyte-ags-cli` crate — duplicated here since `ags-runtime` can't
/// depend on that crate).
fn permission_cell(endpoint: &Map<String, Value>) -> FieldValue {
    let Some(permission) = endpoint.get("permission").and_then(Value::as_object) else {
        return FieldValue::Text("—".to_string());
    };
    let resource = permission
        .get("resource")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let action = permission
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default();
    FieldValue::Text(format!("{resource} [{action}]"))
}

fn authenticated_cell(endpoint: &Map<String, Value>) -> FieldValue {
    let authenticated = endpoint
        .get("requireAuthentication")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    FieldValue::Text(if authenticated { "Yes" } else { "No" }.to_string())
}

/// One summary line for the sibling scalar flags that sit next to
/// `endpoints` — dropped from the table itself since they describe the app,
/// not a per-endpoint row.
fn app_flags_note(body: &Value) -> String {
    let flag = |key: &str| -> &'static str {
        if body.get(key).and_then(Value::as_bool).unwrap_or(false) {
            "yes"
        } else {
            "no"
        }
    };
    format!(
        "App running: {}  ·  API spec: {}  ·  gRPC reflection: {}",
        flag("isAppRunning"),
        flag("hasAPISpec"),
        flag("hasGRPCReflection"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::result::{CommandResult, FieldValue};
    use serde_json::json;

    #[test]
    fn shapes_endpoints_into_five_columns() {
        let body = json!({
            "endpoints": [{
                "method": "GET",
                "path": "/users/{id}",
                "operationId": "a1b2c3",
                "requireAuthentication": true,
                "permission": {"resource": "NAMESPACE:ns1:USER", "action": "READ"}
            }],
            "hasAPISpec": true,
            "hasGRPCReflection": false,
            "isAppRunning": true,
            "maximumSelectableEndpoints": 20
        });
        let CommandResult::Collection(collection) = shape(&body).expect("body matches shape")
        else {
            panic!("expected Collection");
        };
        let labels: Vec<&str> = collection
            .columns
            .iter()
            .map(|c| c.label.as_str())
            .collect();
        assert_eq!(
            labels,
            vec![
                "Method",
                "Path",
                "Operation ID",
                "Permission",
                "Authenticated"
            ]
        );
        assert_eq!(
            collection.rows[0].cells,
            vec![
                FieldValue::Text("GET".to_string()),
                FieldValue::Text("/users/{id}".to_string()),
                FieldValue::Text("a1b2c3".to_string()),
                FieldValue::Text("NAMESPACE:ns1:USER [READ]".to_string()),
                FieldValue::Text("Yes".to_string()),
            ]
        );
        assert_eq!(
            collection.notes,
            vec!["App running: yes  ·  API spec: yes  ·  gRPC reflection: no".to_string()]
        );
    }

    #[test]
    fn endpoint_without_permission_or_auth_shows_dash_and_no() {
        let body = json!({
            "endpoints": [{
                "method": "GET",
                "path": "/apiCallCount",
                "operationId": "Service_GetAPICallCount"
            }],
            "hasAPISpec": true,
            "hasGRPCReflection": true,
            "isAppRunning": true,
            "maximumSelectableEndpoints": 20
        });
        let CommandResult::Collection(collection) = shape(&body).expect("body matches shape")
        else {
            panic!("expected Collection");
        };
        assert_eq!(
            collection.rows[0].cells,
            vec![
                FieldValue::Text("GET".to_string()),
                FieldValue::Text("/apiCallCount".to_string()),
                FieldValue::Text("Service_GetAPICallCount".to_string()),
                FieldValue::Text("—".to_string()),
                FieldValue::Text("No".to_string()),
            ]
        );
    }

    #[test]
    fn null_endpoints_renders_as_empty_collection() {
        let body = json!({
            "endpoints": null,
            "hasAPISpec": false,
            "hasGRPCReflection": false,
            "isAppRunning": false,
            "maximumSelectableEndpoints": 20
        });
        let CommandResult::Collection(collection) =
            shape(&body).expect("null endpoints still matches shape")
        else {
            panic!("expected Collection");
        };
        assert!(collection.rows.is_empty());
    }

    #[test]
    fn returns_none_when_endpoints_key_is_absent() {
        assert!(shape(&json!({"hasAPISpec": true})).is_none());
    }

    #[test]
    fn strips_terminal_control_sequences_from_app_owned_fields() {
        let body = json!({
            "endpoints": [{
                "method": "GET",
                "path": "/users\u{1b}]0;PWNED\u{7}",
                "operationId": "a1b2c3\u{1b}[31m",
                "requireAuthentication": true
            }],
            "hasAPISpec": true,
            "hasGRPCReflection": false,
            "isAppRunning": true,
            "maximumSelectableEndpoints": 20
        });
        let CommandResult::Collection(collection) = shape(&body).expect("body matches shape")
        else {
            panic!("expected Collection");
        };
        assert_eq!(
            collection.rows[0].cells,
            vec![
                FieldValue::Text("GET".to_string()),
                FieldValue::Text("/users".to_string()),
                FieldValue::Text("a1b2c3".to_string()),
                FieldValue::Text("—".to_string()),
                FieldValue::Text("Yes".to_string()),
            ]
        );
    }
}
