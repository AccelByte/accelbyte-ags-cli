//! Expand a `StepDefinition` into `AutoDerivedField` entries by descending
//! the operation's OpenAPI schema (with body flattening) and classifying
//! each field's scope as `WorkflowInput` or `StepLocal`.

use std::collections::BTreeSet;

use ags_protocol::catalogue::{
    BodyField, BodyFieldType, ParameterLocation, ParameterSchema, ServiceSchema, ValueType,
};
use ags_protocol::error::RuntimeError;
use ags_protocol::workflow::{
    AutoDeriveScope, AutoDerivedField, StepDefinition, StepFieldLocation, WorkflowInputSpec,
};

/// Build a minimal JSON-schema object describing a parameter's value type.
/// Mirrors the subset of OpenAPI types that AGS already exposes.
pub(crate) fn parameter_schema(param: &ParameterSchema) -> serde_json::Value {
    match &param.value_type {
        ValueType::String => serde_json::json!({"type": "string"}),
        ValueType::Integer => serde_json::json!({"type": "integer"}),
        ValueType::Number => serde_json::json!({"type": "number"}),
        ValueType::Boolean => serde_json::json!({"type": "boolean"}),
        ValueType::Array(inner) => {
            let inner_schema = value_type_schema(inner);
            serde_json::json!({"type": "array", "items": inner_schema})
        }
        ValueType::Enum(values) => serde_json::json!({"type": "string", "enum": values}),
    }
}

/// Convert a `ValueType` to its JSON-schema fragment (used recursively for
/// array `items`).
pub(crate) fn value_type_schema(t: &ValueType) -> serde_json::Value {
    match t {
        ValueType::String => serde_json::json!({"type": "string"}),
        ValueType::Integer => serde_json::json!({"type": "integer"}),
        ValueType::Number => serde_json::json!({"type": "number"}),
        ValueType::Boolean => serde_json::json!({"type": "boolean"}),
        ValueType::Array(inner) => {
            serde_json::json!({"type": "array", "items": value_type_schema(inner)})
        }
        ValueType::Enum(values) => serde_json::json!({"type": "string", "enum": values}),
    }
}

/// Build a JSON-schema object describing a body field, expanding the resolved
/// `children` of object/reference/array-of-object fields into `properties` so the
/// JSON editor can render structured rows. Childless object/reference fields
/// (depth-capped or opaque) fall back to a bare `{"type":"object"}`.
pub(crate) fn body_field_schema(field: &BodyField) -> serde_json::Value {
    match &field.field_type {
        BodyFieldType::String => serde_json::json!({"type": "string"}),
        BodyFieldType::Integer => serde_json::json!({"type": "integer"}),
        BodyFieldType::Number => serde_json::json!({"type": "number"}),
        BodyFieldType::Boolean => serde_json::json!({"type": "boolean"}),
        BodyFieldType::Enum(values) => serde_json::json!({"type": "string", "enum": values}),
        BodyFieldType::Object | BodyFieldType::Reference(_) => object_schema(&field.children),
        BodyFieldType::Array(inner) => {
            let items = if field.children.is_empty() {
                body_field_type_schema(inner)
            } else {
                object_schema(&field.children)
            };
            serde_json::json!({"type": "array", "items": items})
        }
    }
}

/// Build an object schema from a body field's resolved children. Empty children
/// yields a bare `{"type":"object"}` (raw-mode fallback).
fn object_schema(children: &[BodyField]) -> serde_json::Value {
    if children.is_empty() {
        return serde_json::json!({"type": "object"});
    }
    let mut properties = serde_json::Map::new();
    let mut required: Vec<String> = Vec::new();
    for child in children {
        let mut child_schema = body_field_schema(child);
        if let (Some(obj), Some(desc)) = (child_schema.as_object_mut(), child.description.as_ref())
        {
            obj.insert(
                "description".to_string(),
                serde_json::Value::String(desc.clone()),
            );
        }
        properties.insert(child.name.clone(), child_schema);
        if child.required {
            required.push(child.name.clone());
        }
    }
    let mut schema = serde_json::json!({"type": "object", "properties": properties});
    if !required.is_empty() {
        schema["required"] = serde_json::json!(required);
    }
    schema
}

/// Inner: recursive conversion of `BodyFieldType` to JSON-schema.
pub(crate) fn body_field_type_schema(t: &BodyFieldType) -> serde_json::Value {
    match t {
        BodyFieldType::String => serde_json::json!({"type": "string"}),
        BodyFieldType::Integer => serde_json::json!({"type": "integer"}),
        BodyFieldType::Number => serde_json::json!({"type": "number"}),
        BodyFieldType::Boolean => serde_json::json!({"type": "boolean"}),
        BodyFieldType::Enum(values) => {
            serde_json::json!({"type": "string", "enum": values})
        }
        BodyFieldType::Array(inner) => {
            serde_json::json!({"type": "array", "items": body_field_type_schema(inner)})
        }
        BodyFieldType::Object => serde_json::json!({"type": "object"}),
        BodyFieldType::Reference(name) => {
            serde_json::json!({"type": "object", "$ref": name})
        }
    }
}

/// Look up the JSON-schema fragment for a named field of an operation,
/// checking path/query params first then top-level body fields. Returns
/// `None` when the operation does not expose a field of that name.
pub(crate) fn operation_field_schema(
    operation: &ags_protocol::catalogue::OperationSchema,
    name: &str,
) -> Option<serde_json::Value> {
    if let Some(param) = operation
        .parameters
        .iter()
        .find(|p| p.name == name && p.location != ags_protocol::catalogue::ParameterLocation::Body)
    {
        return Some(parameter_schema(param));
    }
    if let Some(body) = &operation.request_body {
        if let Some(field) = body.fields.iter().find(|f| f.name == name) {
            return Some(body_field_schema(field));
        }
    }
    None
}

/// Locate an `OperationSchema` within a `ServiceSchema` by id. Returns
/// `None` when no resource exposes that operation.
pub fn find_operation<'a>(
    schema: &'a ServiceSchema,
    op_id: &ags_protocol::catalogue::OperationId,
) -> Option<&'a ags_protocol::catalogue::OperationSchema> {
    schema
        .resources
        .iter()
        .flat_map(|r| r.operations())
        .find(|op| &op.id == op_id)
}

/// Resolve an operation within a service schema, mapping a missing operation to
/// a uniform internal error. `context` names the referrer (e.g. `step 'create'`
/// or `options_source`) so the message reads
/// "<context> references unknown operation '<service>.<operation>'".
pub fn find_operation_or_error<'a>(
    schema: &'a ServiceSchema,
    op: &ags_protocol::workflow::OperationReference,
    context: &str,
) -> Result<&'a ags_protocol::catalogue::OperationSchema, RuntimeError> {
    find_operation(schema, &op.operation).ok_or_else(|| {
        RuntimeError::internal(format!(
            "{context} references unknown operation '{}.{}'",
            op.service.as_str(),
            op.operation.as_str()
        ))
    })
}

/// For each required field of the step's operation that is not bound by
/// `step.inputs`, emit an `AutoDerivedField` entry. Body fields are
/// flattened per the spec. Scope classification:
/// - if `workflow_inputs` contains an input of the same name → scope is
///   `WorkflowInput { name }` (auto-bind)
/// - else → `StepLocal { field_name }`
pub fn auto_derive_step(
    step: &StepDefinition,
    service_schema: &ServiceSchema,
    workflow_inputs: &[WorkflowInputSpec],
) -> Result<Vec<AutoDerivedField>, RuntimeError> {
    let op_ref = step.operation.as_ref().ok_or_else(|| {
        RuntimeError::internal(format!(
            "step '{}': API step reached auto_derive_step without an operation",
            step.id
        ))
    })?;
    let operation =
        find_operation_or_error(service_schema, op_ref, &format!("step '{}'", step.id))?;

    // A binding covers its field by exact name, and a nested binding
    // (e.g. `requestedRegions[0]` or `data.region`) also covers its root
    // field (`requestedRegions`, `data`) — so a required array/object filled
    // only through nested element bindings is not re-derived as unbound. This
    // mirrors `resolve`'s nested-binding-root handling at resolution time.
    let mut bound_fields: BTreeSet<&str> = BTreeSet::new();
    for binding in &step.inputs {
        let field = binding.field.as_str();
        bound_fields.insert(field);
        if let Some(root) = field.split(['.', '[']).next() {
            if root != field {
                bound_fields.insert(root);
            }
        }
    }
    let workflow_input_names: BTreeSet<&str> =
        workflow_inputs.iter().map(|i| i.name.as_str()).collect();

    let mut out = Vec::new();

    for param in operation.required_parameters() {
        if bound_fields.contains(param.name.as_str()) {
            continue;
        }
        let scope = if workflow_input_names.contains(param.name.as_str()) {
            AutoDeriveScope::WorkflowInput {
                name: param.name.clone(),
            }
        } else {
            AutoDeriveScope::StepLocal {
                field_name: param.name.clone(),
            }
        };
        let location = match param.location {
            ParameterLocation::Path => StepFieldLocation::Path,
            ParameterLocation::Query => StepFieldLocation::Query,
            ParameterLocation::Header => StepFieldLocation::Header,
            ParameterLocation::FormData => StepFieldLocation::FormData,
            ParameterLocation::Body => StepFieldLocation::Body,
        };
        out.push(AutoDerivedField {
            field: param.name.clone(),
            schema: parameter_schema(param),
            required: true,
            sensitive: false,
            description: param.description.clone(),
            scope,
            location,
        });
    }

    for field in operation.required_body_fields() {
        if bound_fields.contains(field.name.as_str()) {
            continue;
        }
        let scope = if workflow_input_names.contains(field.name.as_str()) {
            AutoDeriveScope::WorkflowInput {
                name: field.name.clone(),
            }
        } else {
            AutoDeriveScope::StepLocal {
                field_name: field.name.clone(),
            }
        };
        out.push(AutoDerivedField {
            field: field.name.clone(),
            schema: body_field_schema(field),
            required: true,
            sensitive: false,
            description: field.description.clone(),
            scope,
            location: StepFieldLocation::Body,
        });
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::catalogue::{
        ApiVersion, BodySchema, HttpMethod, MethodSchema, MutationClass, OperationId,
        OperationSchema, ParameterLocation, ResourceSchema, ScopeEntry,
    };
    use ags_protocol::workflow::{OperationReference, StepDefinition, WorkflowInputSpec};

    /// Build a synthetic `ServiceSchema` exposing exactly one operation.
    fn service_schema_with(op: OperationSchema) -> ServiceSchema {
        ServiceSchema {
            name: "svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "res".into(),
                description: String::new(),
                methods: vec![MethodSchema {
                    name: op.name.clone(),
                    summary: String::new(),
                    default_scope: Some(op.scope.clone()),
                    scopes: vec![ScopeEntry {
                        scope: op.scope.clone(),
                        default_version: op.api_version,
                        contracts: vec![op],
                    }],
                }],
            }],
        }
    }

    /// Build a minimal `OperationSchema` with the given id, parameters, and
    /// optional body. All other fields take defaults.
    fn operation(
        id: &str,
        parameters: Vec<ParameterSchema>,
        request_body: Option<BodySchema>,
    ) -> OperationSchema {
        OperationSchema {
            id: OperationId::new(id),
            name: id.into(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::Mutating,
            http_method: HttpMethod::Post,
            path_template: "/".into(),
            parameters,
            request_body,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// Build a `StepDefinition` referencing a synthetic operation.
    fn step(
        id: &str,
        op_id: &str,
        inputs: Vec<ags_protocol::workflow::StepInputBinding>,
    ) -> StepDefinition {
        StepDefinition {
            id: id.into(),
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ags_protocol::catalogue::ServiceId::new("svc"),
                operation: OperationId::new(op_id),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs,
            outputs: vec![],
        }
    }

    #[test]
    fn test_required_path_param_becomes_step_local_when_no_workflow_input() {
        let schema = service_schema_with(operation(
            "Op",
            vec![ParameterSchema {
                name: "namespace".into(),
                location: ParameterLocation::Path,
                required: true,
                value_type: ValueType::String,
                is_file: false,
                description: None,
                default: None,
            }],
            None,
        ));
        let result = auto_derive_step(&step("s1", "Op", vec![]), &schema, &[]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].field, "namespace");
        assert!(matches!(
            result[0].scope,
            AutoDeriveScope::StepLocal { ref field_name } if field_name == "namespace"
        ));
    }

    #[test]
    fn test_required_path_param_auto_binds_to_matching_workflow_input() {
        let schema = service_schema_with(operation(
            "Op",
            vec![ParameterSchema {
                name: "namespace".into(),
                location: ParameterLocation::Path,
                required: true,
                value_type: ValueType::String,
                is_file: false,
                description: None,
                default: None,
            }],
            None,
        ));
        let inputs = vec![WorkflowInputSpec {
            name: "namespace".into(),
            description: None,
            schema: None,
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        }];
        let result = auto_derive_step(&step("s1", "Op", vec![]), &schema, &inputs).unwrap();
        assert!(matches!(
            result[0].scope,
            AutoDeriveScope::WorkflowInput { ref name } if name == "namespace"
        ));
    }

    #[test]
    fn test_required_body_fields_flatten_into_auto_derived() {
        let schema = service_schema_with(operation(
            "Op",
            vec![],
            Some(BodySchema {
                item_type: None,
                is_array: false,
                definition_name: "Body".into(),
                fields: vec![
                    BodyField {
                        name: "statCode".into(),
                        field_type: BodyFieldType::String,
                        required: true,
                        description: None,
                        children: vec![],
                        default: None,
                    },
                    BodyField {
                        name: "name".into(),
                        field_type: BodyFieldType::String,
                        required: true,
                        description: None,
                        children: vec![],
                        default: None,
                    },
                    BodyField {
                        name: "optional".into(),
                        field_type: BodyFieldType::String,
                        required: false,
                        description: None,
                        children: vec![],
                        default: None,
                    },
                ],
            }),
        ));
        let result = auto_derive_step(&step("s1", "Op", vec![]), &schema, &[]).unwrap();
        let names: Vec<&str> = result.iter().map(|f| f.field.as_str()).collect();
        assert_eq!(names, vec!["statCode", "name"]);
    }

    #[test]
    fn test_explicit_const_binding_suppresses_auto_derive() {
        use ags_protocol::workflow::{BindingSource, LiteralBinding, StepInputBinding};
        let schema = service_schema_with(operation(
            "Op",
            vec![],
            Some(BodySchema {
                item_type: None,
                is_array: false,
                definition_name: "Body".into(),
                fields: vec![BodyField {
                    name: "name".into(),
                    field_type: BodyFieldType::String,
                    required: true,
                    description: None,
                    children: vec![],
                    default: None,
                }],
            }),
        ));
        let binding = StepInputBinding {
            field: "name".into(),
            source: BindingSource::Literal(LiteralBinding {
                value: serde_json::json!("foo"),
                sensitive: false,
            }),
            show_in_review: false,
            description: None,
        };
        let result = auto_derive_step(&step("s1", "Op", vec![binding]), &schema, &[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_explicit_reference_binding_suppresses_auto_derive() {
        use ags_protocol::workflow::{
            BindingSource, ReferenceBinding, ReferenceTarget, StepInputBinding,
        };
        let schema = service_schema_with(operation(
            "Op",
            vec![ParameterSchema {
                name: "namespace".into(),
                location: ParameterLocation::Path,
                required: true,
                value_type: ValueType::String,
                is_file: false,
                description: None,
                default: None,
            }],
            None,
        ));
        let binding = StepInputBinding {
            field: "namespace".into(),
            source: BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow { input: "ns".into() },
                output: None,
                transform: None,
            }),
            show_in_review: false,
            description: None,
        };
        let inputs = vec![WorkflowInputSpec {
            name: "ns".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        }];
        let result = auto_derive_step(&step("s1", "Op", vec![binding]), &schema, &inputs).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_optional_fields_are_not_auto_derived() {
        let schema = service_schema_with(operation(
            "Op",
            vec![ParameterSchema {
                name: "optional".into(),
                location: ParameterLocation::Query,
                required: false,
                value_type: ValueType::String,
                is_file: false,
                description: None,
                default: None,
            }],
            None,
        ));
        let result = auto_derive_step(&step("s1", "Op", vec![]), &schema, &[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_unknown_operation_returns_error() {
        let schema = service_schema_with(operation("Op", vec![], None));
        let result = auto_derive_step(&step("s1", "Missing", vec![]), &schema, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_api_step_without_operation_returns_internal_error() {
        let schema = service_schema_with(operation("Op", vec![], None));
        let malformed = StepDefinition {
            id: "bad".into(),
            description: None,
            kind: ags_protocol::workflow::StepKind::Api,
            action: None,
            operation: None,
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![],
        };
        let err = auto_derive_step(&malformed, &schema, &[]).unwrap_err();
        assert_eq!(err.kind, ags_protocol::error::RuntimeErrorKind::Internal);
        assert!(
            err.message.contains("without an operation"),
            "error must explain the missing operation: {err}"
        );
    }

    /// Build a `ServiceSchema` fixture exposing one operation with a
    /// required `formData`, `type: file` parameter named `name`
    /// (`is_file: true`).
    fn service_schema_with_required_formdata_file_param(name: &str) -> ServiceSchema {
        service_schema_with(operation(
            "Op",
            vec![ParameterSchema {
                name: name.into(),
                location: ParameterLocation::FormData,
                required: true,
                value_type: ValueType::String,
                is_file: true,
                description: None,
                default: None,
            }],
            None,
        ))
    }

    /// Build an unbound `StepDefinition` referencing operation `"Op"`, with
    /// no input bindings, so `auto_derive_step` must derive every required
    /// field itself.
    fn step_definition_with_no_inputs() -> StepDefinition {
        step("s1", "Op", vec![])
    }

    /// A required file-typed formData parameter, when auto-derived because
    /// no step binding supplies it, is labeled `StepFieldLocation::FormData`
    /// — not `Body` — so the interactive gather form groups it correctly.
    #[test]
    fn test_auto_derive_required_formdata_param_location_is_formdata() {
        let step = step_definition_with_no_inputs();
        let service_schema = service_schema_with_required_formdata_file_param("file");

        let derived = auto_derive_step(&step, &service_schema, &[]).unwrap();

        let file_field = derived.iter().find(|f| f.field == "file").unwrap();
        assert_eq!(file_field.location, StepFieldLocation::FormData);
    }

    /// Build a `BodyField` fixture.
    fn bf(name: &str, ft: BodyFieldType, required: bool, children: Vec<BodyField>) -> BodyField {
        BodyField {
            name: name.into(),
            field_type: ft,
            required,
            description: None,
            children,
            default: None,
        }
    }

    #[test]
    fn test_body_field_schema_object_with_children_emits_properties() {
        let field = bf(
            "imageDeploymentProfile",
            BodyFieldType::Reference("api.ImageDeploymentProfile".into()),
            true,
            vec![
                bf("imageId", BodyFieldType::String, true, vec![]),
                bf("commandLine", BodyFieldType::String, false, vec![]),
            ],
        );
        let schema = body_field_schema(&field);
        assert_eq!(schema["type"], serde_json::json!("object"));
        let props = schema["properties"].as_object().expect("properties");
        assert!(props.contains_key("imageId"));
        assert!(props.contains_key("commandLine"));
        assert_eq!(props["imageId"]["type"], serde_json::json!("string"));
        assert_eq!(schema["required"], serde_json::json!(["imageId"]));
    }

    #[test]
    fn test_body_field_schema_nested_object_recurses() {
        let field = bf(
            "outer",
            BodyFieldType::Object,
            false,
            vec![bf(
                "inner",
                BodyFieldType::Object,
                false,
                vec![bf("leaf", BodyFieldType::Integer, false, vec![])],
            )],
        );
        let schema = body_field_schema(&field);
        let inner = &schema["properties"]["inner"];
        assert_eq!(inner["type"], serde_json::json!("object"));
        assert_eq!(
            inner["properties"]["leaf"]["type"],
            serde_json::json!("integer")
        );
    }

    #[test]
    fn test_body_field_schema_array_of_object_uses_object_items() {
        let field = bf(
            "portConfigurations",
            BodyFieldType::Array(Box::new(BodyFieldType::Reference(
                "api.PortConfiguration".into(),
            ))),
            false,
            vec![bf("port", BodyFieldType::Integer, true, vec![])],
        );
        let schema = body_field_schema(&field);
        assert_eq!(schema["type"], serde_json::json!("array"));
        let items = &schema["items"];
        assert_eq!(items["type"], serde_json::json!("object"));
        assert!(items["properties"]
            .as_object()
            .unwrap()
            .contains_key("port"));
    }

    #[test]
    fn test_body_field_schema_array_of_scalar_uses_scalar_items() {
        let field = bf(
            "tags",
            BodyFieldType::Array(Box::new(BodyFieldType::String)),
            false,
            vec![],
        );
        let schema = body_field_schema(&field);
        assert_eq!(schema["type"], serde_json::json!("array"));
        assert_eq!(schema["items"]["type"], serde_json::json!("string"));
        assert!(schema["items"].get("properties").is_none());
    }

    #[test]
    fn test_body_field_schema_childless_object_is_bare() {
        let field = bf(
            "opaque",
            BodyFieldType::Reference("api.Deep".into()),
            false,
            vec![],
        );
        let schema = body_field_schema(&field);
        assert_eq!(schema, serde_json::json!({"type": "object"}));
    }
}
