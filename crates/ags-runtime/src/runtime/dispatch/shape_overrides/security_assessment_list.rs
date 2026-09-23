//! Bespoke columns for `ags csm security-assessment list`
//! (`csm/admin/security-assessment/v1/list`).

use ags_protocol::result::{CommandResult, FieldValue};
use serde_json::{Map, Value};

use crate::support::time::format_rfc3339_human;

pub(super) const OPERATION_ID: &str = "csm/admin/security-assessment/v1/list";

pub(super) fn shape(body: &Value) -> Option<CommandResult> {
    super::fixed_columns_from_array(
        body,
        "pentestings",
        "security assessment requests",
        &[
            ("ID", |e| super::text_field(e.get("engagementId"))),
            ("App Name", |e| super::text_field(e.get("targetApp"))),
            ("Requested At", requested_at),
            ("Endpoints", endpoint_count),
            ("Image Tag", |e| {
                super::text_field(e.get("targetAppVersion"))
            }),
            ("Status", |e| super::text_field(e.get("status"))),
        ],
    )
}

fn requested_at(engagement: &Map<String, Value>) -> FieldValue {
    FieldValue::Text(
        engagement
            .get("createdAt")
            .and_then(Value::as_str)
            .map(format_rfc3339_human)
            .unwrap_or_else(|| "—".to_string()),
    )
}

fn endpoint_count(engagement: &Map<String, Value>) -> FieldValue {
    FieldValue::Text(
        engagement
            .get("endpoints")
            .and_then(Value::as_array)
            .map(|items| items.len().to_string())
            .unwrap_or_else(|| "0".to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::result::CommandResult;
    use serde_json::json;

    #[test]
    fn shapes_engagement_list_into_six_fixed_columns() {
        let body = json!({
            "pentestings": [{
                "engagementId": 12345,
                "targetApp": "my-service",
                "createdAt": "2026-08-10T10:17:56Z",
                "endpoints": [{"path": "/a"}, {"path": "/b"}],
                "targetAppVersion": "v1.2.3",
                "status": "COMPLETED"
            }]
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
                "ID",
                "App Name",
                "Requested At",
                "Endpoints",
                "Image Tag",
                "Status"
            ]
        );
        assert_eq!(
            collection.rows[0].cells,
            vec![
                FieldValue::Text("12345".to_string()),
                FieldValue::Text("my-service".to_string()),
                FieldValue::Text("Aug 10, 2026, 10:17:56 UTC".to_string()),
                FieldValue::Text("2".to_string()),
                FieldValue::Text("v1.2.3".to_string()),
                FieldValue::Text("COMPLETED".to_string()),
            ]
        );
    }

    #[test]
    fn missing_optional_fields_render_as_dash() {
        let body = json!({ "pentestings": [{ "engagementId": 1, "endpoints": [] }] });
        let CommandResult::Collection(collection) = shape(&body).expect("body matches shape")
        else {
            panic!("expected Collection");
        };
        assert_eq!(
            collection.rows[0].cells[3],
            FieldValue::Text("0".to_string())
        );
        assert_eq!(
            collection.rows[0].cells[1],
            FieldValue::Text("—".to_string())
        );
    }

    #[test]
    fn returns_none_when_body_lacks_pentestings_array() {
        assert!(shape(&json!({"unexpected": true})).is_none());
    }

    #[test]
    fn strips_terminal_control_sequences_from_app_owned_fields() {
        let body = json!({
            "pentestings": [{
                "engagementId": 1,
                "targetApp": "my-service\u{1b}]0;PWNED\u{7}",
                "targetAppVersion": "v1\u{1b}[31m",
                "status": "COMPLETED\u{1b}[0m"
            }]
        });
        let CommandResult::Collection(collection) = shape(&body).expect("body matches shape")
        else {
            panic!("expected Collection");
        };
        assert_eq!(
            collection.rows[0].cells,
            vec![
                FieldValue::Text("1".to_string()),
                FieldValue::Text("my-service".to_string()),
                FieldValue::Text("—".to_string()),
                FieldValue::Text("0".to_string()),
                FieldValue::Text("v1".to_string()),
                FieldValue::Text("COMPLETED".to_string()),
            ]
        );
    }
}
