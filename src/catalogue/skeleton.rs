//! JSON request body template generation for --skeleton and help text examples.

use serde_json::{json, Map, Value};

use crate::protocol::catalogue::{BodyField, BodySchema, OperationSchema};
use crate::support::strings::to_kebab_case;

/// Build a `serde_json::Value` skeleton for an operation's request body.
///
/// Returns an empty object when the operation has no body. Used by both
/// `--skeleton` (direct stdout output) and help text example generation.
pub fn build_body_skeleton(operation: &OperationSchema) -> Value {
    match &operation.request_body {
        Some(body) => build_body_schema_skeleton(body),
        None => Value::Object(Map::new()),
    }
}

/// Build a skeleton from a request body schema.
pub(crate) fn build_body_schema_skeleton(schema: &BodySchema) -> Value {
    build_fields_object(&schema.fields)
}

/// Build an object skeleton from a slice of body fields.
fn build_fields_object(fields: &[BodyField]) -> Value {
    let mut map = Map::new();

    for field in fields {
        let value = field_to_value(field);
        map.insert(field.name.clone(), value);
    }

    Value::Object(map)
}

/// Convert a single body field into its placeholder JSON value.
fn field_to_value(field: &BodyField) -> Value {
    use crate::protocol::catalogue::BodyFieldType;
    match &field.field_type {
        BodyFieldType::Enum(values) => values
            .first()
            .map(|v| json!(v))
            .unwrap_or_else(|| scalar_placeholder_string(&field.name)),
        BodyFieldType::Array(inner) => {
            if !field.children.is_empty() {
                json!([build_fields_object(&field.children)])
            } else {
                json!([scalar_placeholder_for_type(inner, &field.name)])
            }
        }
        BodyFieldType::Object => {
            if !field.children.is_empty() {
                build_fields_object(&field.children)
            } else {
                Value::Object(Map::new())
            }
        }
        BodyFieldType::Reference(_) => {
            if !field.children.is_empty() {
                build_fields_object(&field.children)
            } else {
                Value::Object(Map::new())
            }
        }
        scalar @ (BodyFieldType::String
        | BodyFieldType::Integer
        | BodyFieldType::Boolean
        | BodyFieldType::Number) => scalar_placeholder_for_type(scalar, &field.name),
    }
}

/// Build a scalar placeholder value from a typed `BodyFieldType` and field name.
fn scalar_placeholder_for_type(
    field_type: &crate::protocol::catalogue::BodyFieldType,
    name: &str,
) -> Value {
    use crate::protocol::catalogue::BodyFieldType;
    match field_type {
        BodyFieldType::Boolean => json!(field_hash(name) % 2 == 0),
        BodyFieldType::Integer | BodyFieldType::Number => json!(field_hash(name) % 100),
        _ => scalar_placeholder_string(name),
    }
}

/// Build a string placeholder value from a field name.
fn scalar_placeholder_string(name: &str) -> Value {
    let kebab = to_kebab_case(name);
    json!(format!("my-{kebab}"))
}

/// Deterministic djb2 hash of a field name for stable placeholder values.
/// Hand-rolled rather than `DefaultHasher` because the output reaches users via
/// `--skeleton` placeholders and must stay stable across Rust toolchain upgrades.
pub(crate) fn field_hash(name: &str) -> u64 {
    let mut hash_value: u64 = 5381;
    for byte in name.bytes() {
        hash_value = hash_value.wrapping_mul(33).wrapping_add(byte as u64);
    }
    hash_value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::catalogue::{
        ApiVersion, BodyFieldType, BodySchema, HttpMethod, MutationClass, OperationSchema,
    };

    /// Build a minimal body field for skeleton-generation tests.
    fn make_field(name: &str, field_type: BodyFieldType, required: bool) -> BodyField {
        BodyField {
            name: name.to_string(),
            field_type,
            required,
            description: None,
            children: vec![],
        }
    }

    /// Build a minimal POST operation carrying the supplied request-body fields.
    fn operation_with_body(fields: Vec<BodyField>) -> OperationSchema {
        OperationSchema {
            id: crate::protocol::catalogue::OperationId::new("TestOp"),
            name: "test".to_string(),
            summary: "Test operation".to_string(),
            description: None,
            mutation_class: MutationClass::Mutating,
            http_method: HttpMethod::Post,
            path_template: "/test".to_string(),
            parameters: vec![],
            request_body: Some(BodySchema {
                definition_name: "TestBody".to_string(),
                fields,
            }),
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(0),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// Operations without a request body should yield an empty JSON object.
    #[test]
    fn test_operation_without_body_produces_empty_object() {
        let operation = OperationSchema {
            id: crate::protocol::catalogue::OperationId::new("TestOp"),
            name: "test".to_string(),
            summary: "Test operation".to_string(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/test".to_string(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(0),
            deprecated: false,
            response_content_type: None,
        };
        assert_eq!(build_body_skeleton(&operation), Value::Object(Map::new()));
    }

    /// String fields should use a stable kebab-case placeholder.
    #[test]
    fn test_string_field_produces_placeholder() {
        let operation =
            operation_with_body(vec![make_field("displayName", BodyFieldType::String, true)]);
        let value = build_body_skeleton(&operation);
        assert_eq!(value["displayName"], "my-display-name");
    }

    /// Integer fields should produce numeric placeholders.
    #[test]
    fn test_integer_field_produces_number() {
        let operation =
            operation_with_body(vec![make_field("limit", BodyFieldType::Integer, false)]);
        let value = build_body_skeleton(&operation);
        assert!(value["limit"].is_number());
    }

    /// Boolean fields should produce boolean placeholders.
    #[test]
    fn test_boolean_field_produces_bool() {
        let operation =
            operation_with_body(vec![make_field("enabled", BodyFieldType::Boolean, false)]);
        let value = build_body_skeleton(&operation);
        assert!(value["enabled"].is_boolean());
    }

    /// Enum fields should use the first declared value as the placeholder.
    #[test]
    fn test_enum_field_uses_first_value() {
        let field = make_field(
            "status",
            BodyFieldType::Enum(vec!["active".to_string(), "inactive".to_string()]),
            false,
        );
        let operation = operation_with_body(vec![field]);
        let value = build_body_skeleton(&operation);
        assert_eq!(value["status"], "active");
    }

    /// Nested object fields should recurse into child placeholders.
    #[test]
    fn test_nested_object_produces_nested_json() {
        let child = make_field("street", BodyFieldType::String, true);
        let parent = BodyField {
            name: "address".to_string(),
            field_type: BodyFieldType::Object,
            required: false,
            description: None,
            children: vec![child],
        };
        let operation = operation_with_body(vec![parent]);
        let value = build_body_skeleton(&operation);
        assert!(value["address"].is_object());
        assert_eq!(value["address"]["street"], "my-street");
    }

    /// Array fields over primitive values should produce a one-element array.
    #[test]
    fn test_array_of_primitives_produces_array() {
        let field = BodyField {
            name: "tags".to_string(),
            field_type: BodyFieldType::Array(Box::new(BodyFieldType::String)),
            required: false,
            description: None,
            children: vec![],
        };
        let operation = operation_with_body(vec![field]);
        let value = build_body_skeleton(&operation);
        assert!(value["tags"].is_array());
    }

    /// Array fields over object values should produce one nested object entry.
    #[test]
    fn test_array_of_objects_produces_array_with_one_element() {
        let child = make_field("key", BodyFieldType::String, true);
        let parent = BodyField {
            name: "items".to_string(),
            field_type: BodyFieldType::Array(Box::new(BodyFieldType::Object)),
            required: false,
            description: None,
            children: vec![child],
        };
        let operation = operation_with_body(vec![parent]);
        let value = build_body_skeleton(&operation);
        let array_value = value["items"].as_array().unwrap();
        assert_eq!(array_value.len(), 1);
        assert_eq!(array_value[0]["key"], "my-key");
    }

    /// The generated skeleton should always round-trip as valid JSON.
    #[test]
    fn test_output_is_valid_json() {
        let fields = vec![
            make_field("name", BodyFieldType::String, true),
            make_field("count", BodyFieldType::Integer, false),
            make_field("active", BodyFieldType::Boolean, false),
        ];
        let operation = operation_with_body(fields);
        let value = build_body_skeleton(&operation);
        let json_str = serde_json::to_string_pretty(&value).unwrap();
        let reparsed: Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(value, reparsed);
    }
}
