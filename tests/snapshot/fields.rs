use ags::catalogue::Catalogue;
use ags::protocol::catalogue::OperationSchema;
use ags::protocol::result::{CommandResult, FieldValue};
use ags::runtime::dispatch::shape::shape_response;
use serde_json::json;

fn iam_operation(resource: &str, method: &str) -> OperationSchema {
    let service = Catalogue::load_bundled("iam").expect("load IAM spec");
    let resource_entry = service
        .resources
        .into_iter()
        .find(|resource_entry| resource_entry.name == resource)
        .unwrap_or_else(|| panic!("resource '{resource}' not found"));
    let op = resource_entry
        .operations()
        .find(|operation| operation.name == method)
        .cloned();
    op.unwrap_or_else(|| panic!("method '{method}' not found on '{resource}'"))
}

/// Field selection keeps the highest-signal fields first for IAM user objects.
#[test]
fn test_field_selection_iam_user() {
    let operation = iam_operation("users", "get");
    let data = json!({
        "userId": "abc123",
        "displayName": "Jane Doe",
        "emailAddress": "jane@example.com",
        "namespace": "my-game",
        "enabled": true,
        "emailVerified": true,
        "createdAt": "2024-01-15T10:30:00Z",
        "updatedAt": "2024-06-01T08:00:00Z",
        "country": "US",
        "dateOfBirth": "1990-01-01",
        "deletionStatus": false,
        "phoneVerified": false,
        "avatarUrl": "",
        "uniqueDisplayName": "jane_doe",
        "platformUserIds": {},
        "namespacedRoles": [],
        "permissions": []
    });

    let shaped = shape_response(&data, &operation, "users", false);
    let entity = match shaped {
        CommandResult::Entity(entity) => entity,
        other => panic!("expected entity result, got {other:?}"),
    };

    assert!(entity.fields.len() <= 8);

    let labels: Vec<&str> = entity
        .fields
        .iter()
        .map(|field| field.label.as_str())
        .collect();
    assert_eq!(labels[0], "User ID");
    assert!(labels.contains(&"Email"));
}

/// Human shaping normalizes common labels and values in the final entity fields.
#[test]
fn test_field_labels_and_values_are_humanized() {
    let operation = iam_operation("users", "get");
    let data = json!({
        "userId": "abc123",
        "enabled": true,
        "createdAt": "2024-01-15T10:30:00Z",
        "baseUrl": "https://example.com",
        "displayName": "Jane Doe",
        "emailAddress": "jane@example.com"
    });

    let shaped = shape_response(&data, &operation, "users", false);
    let entity = match shaped {
        CommandResult::Entity(entity) => entity,
        other => panic!("expected entity result, got {other:?}"),
    };

    let labels: Vec<&str> = entity
        .fields
        .iter()
        .map(|field| field.label.as_str())
        .collect();
    assert!(labels.contains(&"User ID"));
    assert!(labels.contains(&"Created"));
    assert!(labels.contains(&"Email"));
    assert!(labels.contains(&"Enabled"));

    let enabled = entity
        .fields
        .iter()
        .find(|field| field.label == "Enabled")
        .expect("enabled field should be present");
    assert_eq!(enabled.value, FieldValue::Text("yes".to_string()));
}
