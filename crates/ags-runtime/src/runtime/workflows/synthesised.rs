//! Build a 1-step `WorkflowDefinition` from a parsed CLI command, auto-
//! declaring one workflow input per operation field (after body flattening).
//! When a path/query parameter and a body property share a name, they are
//! coupled into a single workflow input rather than treated as an error.

use ags_protocol::catalogue::{ParameterLocation, ParameterSchema, ServiceSchema, ValueType};
use ags_protocol::error::RuntimeError;
use ags_protocol::workflow::{
    OperationReference, StepDefinition, StepFieldLocation, WorkflowDefinition, WorkflowId,
    WorkflowInputSpec,
};

use crate::runtime::dispatch::requires_confirmation;
use crate::runtime::workflows::auto_derive::find_operation;

/// Prefix of a `WorkflowId` minted by [`synthesise_workflow_definition`] for a
/// single CLI command, distinguishing it from a registered multi-step workflow.
pub const SYNTHESISED_ID_PREFIX: &str = "synth(";

/// True when `id` belongs to a synthesised single-command workflow (as opposed
/// to a registered workflow). Used to phrase user-facing messages as "command"
/// rather than "workflow"/"step 'main'".
pub fn is_synthesised_single_command(id: &WorkflowId) -> bool {
    id.as_str().starts_with(SYNTHESISED_ID_PREFIX)
}

/// Build a synthesised 1-step workflow for a single OpenAPI operation. The
/// resulting `WorkflowDefinition.inputs` carries one entry per operation
/// field (path/query/header/body), schemas populated from OpenAPI. The
/// single step has empty `inputs`/`outputs` — auto-derive at compile time
/// fills in `auto_derived` with WorkflowInput scope for every field.
pub fn synthesise_workflow_definition(
    service: &ags_protocol::catalogue::ServiceId,
    operation_id: &ags_protocol::catalogue::OperationId,
    service_schema: &ServiceSchema,
) -> Result<WorkflowDefinition, RuntimeError> {
    let operation = find_operation(service_schema, operation_id).ok_or_else(|| {
        RuntimeError::internal(format!(
            "cannot synthesise workflow: operation '{}.{}' not found",
            service.as_str(),
            operation_id.as_str()
        ))
    })?;

    // One `WorkflowInputSpec` per unique operation-field name. When a
    // path/query parameter and a body property share a name (a "collision"),
    // they are coupled: a single workflow input feeds both destinations.
    // Merge rules: the parameter-side schema is authoritative (path routing
    // depends on it); `required` is the OR of both sides; `description`
    // falls back from the parameter to the body field. The single CLI flag
    // built from this input then reaches both the path parameter and the
    // body property via `assemble_command_request`'s by-name resolution.
    //
    // Coupling is defined ONLY for a non-body-parameter/body-property pair.
    // Two non-body parameters sharing a name remains a synthesis failure —
    // the design did not approve coupling that shape, and no bundled spec
    // contains such a case (the parity test in this task confirms it).
    let mut input_specs: Vec<WorkflowInputSpec> = Vec::new();
    let mut index_by_name: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    for param in &operation.parameters {
        if param.location == ParameterLocation::Body {
            continue;
        }
        if index_by_name.contains_key(&param.name) {
            return Err(parameter_collision_error(
                service,
                operation_id,
                &param.name,
            ));
        }
        index_by_name.insert(param.name.clone(), input_specs.len());
        input_specs.push(WorkflowInputSpec {
            name: param.name.clone(),
            description: param.description.clone(),
            schema: Some(parameter_schema_value(param)),
            required: param.required,
            default: param.default.clone(),
            sensitive: false,
            options_source: None,
            location: param_location_to_field_location(param.location),
            file_picker: None,
        });
    }

    if let Some(body) = &operation.request_body {
        for field in &body.fields {
            match index_by_name.get(&field.name) {
                Some(&index) => {
                    // Path/query parameter and body property share a name —
                    // couple them. Keep the parameter-side schema; OR the
                    // `required` flags; fall back to the body field's
                    // description when the parameter had none.
                    // Do NOT overwrite the location set by the parameter side.
                    // For defaults: param default wins when present; only
                    // use the body-field default when no param default was set.
                    input_specs[index].required |= field.required;
                    if input_specs[index].description.is_none() {
                        input_specs[index].description = field.description.clone();
                    }
                    if input_specs[index].default.is_none() {
                        input_specs[index].default = field.default.clone();
                    }
                }
                None => {
                    index_by_name.insert(field.name.clone(), input_specs.len());
                    input_specs.push(WorkflowInputSpec {
                        name: field.name.clone(),
                        description: field.description.clone(),
                        schema: Some(
                            crate::runtime::workflows::auto_derive::operation_field_schema(
                                operation,
                                &field.name,
                            )
                            .unwrap_or_else(|| serde_json::json!({"type": "string"})),
                        ),
                        required: field.required,
                        default: field.default.clone(),
                        sensitive: false,
                        options_source: None,
                        location: StepFieldLocation::Body,
                        file_picker: None,
                    });
                }
            }
        }
    }

    let workflow_id = WorkflowId::new(format!(
        "{SYNTHESISED_ID_PREFIX}{}.{})",
        service.as_str(),
        operation_id.as_str()
    ));
    Ok(WorkflowDefinition {
        id: workflow_id,
        name: format!("{} {}", service.as_str(), operation_id.as_str()),
        workflow_protocol_version: None,
        intent: None,
        description: None,
        briefing: None,
        inputs: input_specs,
        is_reviewed_by_default: true,
        steps: vec![StepDefinition {
            id: "main".into(),
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: service.clone(),
                operation: operation_id.clone(),
            }),
            dependencies: vec![],
            confirm: requires_confirmation(operation.http_method, &operation.name),
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![],
        }],
        outputs: vec![],
        completion: None,
    })
}

/// Build the internal error raised when an operation has two parameters sharing
/// one name, which cannot be synthesised into distinct workflow inputs.
fn parameter_collision_error(
    service: &ags_protocol::catalogue::ServiceId,
    operation_id: &ags_protocol::catalogue::OperationId,
    field: &str,
) -> RuntimeError {
    RuntimeError::internal(format!(
        "cannot synthesise single-command workflow for {}.{}: two parameters share the name '{}'",
        service.as_str(),
        operation_id.as_str(),
        field
    ))
}

/// Project a parameter's value type into the JSON-schema fragment used to
/// describe the synthesised workflow input.
fn parameter_schema_value(param: &ParameterSchema) -> serde_json::Value {
    match &param.value_type {
        ValueType::String => serde_json::json!({"type": "string"}),
        ValueType::Integer => serde_json::json!({"type": "integer"}),
        ValueType::Number => serde_json::json!({"type": "number"}),
        ValueType::Boolean => serde_json::json!({"type": "boolean"}),
        ValueType::Array(_) => serde_json::json!({"type": "array"}),
        ValueType::Enum(values) => serde_json::json!({"type": "string", "enum": values}),
    }
}

/// Map an OpenAPI `ParameterLocation` to its `StepFieldLocation` counterpart.
/// `Body` is not used in the parameter loop (skipped before this is called);
/// `FormData` is used and maps to its own `StepFieldLocation::FormData`.
fn param_location_to_field_location(loc: ParameterLocation) -> StepFieldLocation {
    match loc {
        ParameterLocation::Path => StepFieldLocation::Path,
        ParameterLocation::Query => StepFieldLocation::Query,
        ParameterLocation::Header => StepFieldLocation::Header,
        ParameterLocation::FormData => StepFieldLocation::FormData,
        ParameterLocation::Body => StepFieldLocation::Body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::catalogue::{
        ApiVersion, BodyField, BodyFieldType, BodySchema, HttpMethod, MethodSchema, MutationClass,
        OperationId, OperationSchema, ParameterLocation, ParameterSchema, ResourceSchema,
        ScopeEntry, ServiceId,
    };

    /// Build a `ServiceSchema` fixture containing the given operation.
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

    /// Build an `OperationSchema` fixture.
    fn operation(
        id: &str,
        mutation_class: MutationClass,
        parameters: Vec<ParameterSchema>,
        request_body: Option<BodySchema>,
    ) -> OperationSchema {
        OperationSchema {
            id: OperationId::new(id),
            name: id.into(),
            summary: String::new(),
            description: None,
            mutation_class,
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

    /// Build a path `ParameterSchema` fixture.
    fn path_param(name: &str) -> ParameterSchema {
        ParameterSchema {
            name: name.into(),
            location: ParameterLocation::Path,
            required: true,
            value_type: ValueType::String,
            is_file: false,
            description: None,
            default: None,
        }
    }

    /// Build a `BodyField` fixture.
    fn body_field(name: &str) -> BodyField {
        BodyField {
            name: name.into(),
            field_type: BodyFieldType::String,
            required: true,
            description: None,
            children: vec![],
            default: None,
        }
    }

    #[test]
    fn test_synth_simple_op_three_inputs() {
        let op = operation(
            "CreateStat",
            MutationClass::Mutating,
            vec![path_param("namespace")],
            Some(BodySchema {
                item_type: None,
                is_array: false,
                definition_name: "Body".into(),
                fields: vec![body_field("statCode"), body_field("name")],
            }),
        );
        let service = ServiceId::new("svc");
        let op_id = OperationId::new("CreateStat");
        let schema = service_schema_with(op);

        let def = synthesise_workflow_definition(&service, &op_id, &schema).unwrap();

        assert_eq!(def.inputs.len(), 3);
        assert_eq!(def.inputs[0].name, "namespace");
        assert_eq!(def.inputs[1].name, "statCode");
        assert_eq!(def.inputs[2].name, "name");
        assert_eq!(def.steps.len(), 1);
        assert!(def.steps[0].inputs.is_empty());
    }

    #[test]
    fn test_synth_name_collision_dedups_into_single_input() {
        // `id` appears as both a path parameter and a body property.
        let op = operation(
            "UpdateItem",
            MutationClass::Mutating,
            vec![path_param("id")],
            Some(BodySchema {
                item_type: None,
                is_array: false,
                definition_name: "Body".into(),
                fields: vec![body_field("id"), body_field("name")],
            }),
        );
        let service = ServiceId::new("svc");
        let op_id = OperationId::new("UpdateItem");
        let schema = service_schema_with(op);

        let def = synthesise_workflow_definition(&service, &op_id, &schema).unwrap();

        // `id` is coupled into one input; `name` is the other — 2 total.
        let names: Vec<&str> = def.inputs.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["id", "name"]);
        // The coupled `id` input is required (the path-parameter side is).
        let id_input = def.inputs.iter().find(|i| i.name == "id").unwrap();
        assert!(id_input.required);
        // The coupled input keeps the path-parameter's schema, not a body-derived one.
        assert_eq!(id_input.schema, Some(serde_json::json!({"type": "string"})));
    }

    #[test]
    fn test_synth_collision_description_falls_back_to_body() {
        // Path param `note` has no description; the body field `note` does.
        // The coupled input must take the body field's description.
        let mut body_note = body_field("note");
        body_note.description = Some("the note text".into());
        let op = operation(
            "UpdateNote",
            MutationClass::Mutating,
            vec![path_param("note")],
            Some(BodySchema {
                item_type: None,
                is_array: false,
                definition_name: "Body".into(),
                fields: vec![body_note],
            }),
        );
        let service = ServiceId::new("svc");
        let op_id = OperationId::new("UpdateNote");
        let schema = service_schema_with(op);

        let def = synthesise_workflow_definition(&service, &op_id, &schema).unwrap();
        let note_input = def.inputs.iter().find(|i| i.name == "note").unwrap();
        assert_eq!(note_input.description.as_deref(), Some("the note text"));
    }

    #[test]
    fn test_synth_coupled_value_reaches_path_and_body() {
        use crate::catalogue::Catalogue;
        use crate::runtime::workflows::compile::compile_workflow;
        use crate::runtime::workflows::resolve::assemble_command_request;
        use crate::runtime::workflows::{RunOptions, WorkflowContext};
        use std::collections::BTreeMap;

        // `namespace` is both a path parameter and a required body field.
        let op = operation(
            "CreateThing",
            MutationClass::Mutating,
            vec![path_param("namespace")],
            Some(BodySchema {
                item_type: None,
                is_array: false,
                definition_name: "Body".into(),
                fields: vec![body_field("namespace"), body_field("statCode")],
            }),
        );
        let service = ServiceId::new("svc");
        let op_id = OperationId::new("CreateThing");
        let schema = service_schema_with(op);

        let def = synthesise_workflow_definition(&service, &op_id, &schema).unwrap();
        let mut catalogue = Catalogue::new();
        catalogue.insert_for_tests("svc", schema);
        let compiled = compile_workflow(&def, &mut catalogue).unwrap();

        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("dev"));
        supplied.insert("statCode".to_string(), serde_json::json!("mmr"));

        let service_schema = catalogue.get_or_load("svc").unwrap().clone();
        let request = assemble_command_request(
            &compiled.steps[0],
            &WorkflowContext::new(),
            &supplied,
            &BTreeMap::new(),
            &compiled.inputs,
            &service_schema,
            Some("dev".to_string()),
            &RunOptions::default(),
        )
        .unwrap();

        // The single `--namespace` value reaches both destinations.
        assert_eq!(
            request.path_params.get("namespace"),
            Some(&"dev".to_string())
        );
        assert_eq!(
            match &request.body {
                Some(ags_protocol::request::RequestBody::Json(v)) => v.get("namespace"),
                _ => None,
            },
            Some(&serde_json::json!("dev"))
        );
    }

    #[test]
    fn test_all_bundled_specs_synthesise_without_collision() {
        let mut catalogue = crate::catalogue::Catalogue::new();
        let mut failures: Vec<String> = Vec::new();
        for service_id in crate::catalogue::Catalogue::service_ids() {
            let schema = catalogue
                .get_or_load(service_id)
                .expect("bundled spec must load")
                .clone();
            let service = ags_protocol::catalogue::ServiceId::new(service_id);
            for resource in &schema.resources {
                for operation in resource.operations() {
                    if let Err(error) =
                        synthesise_workflow_definition(&service, &operation.id, &schema)
                    {
                        failures.push(format!(
                            "{service_id}.{}: {}",
                            operation.id.as_str(),
                            error.message
                        ));
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "every bundled operation must synthesise into a 1-step workflow; \
             failures:\n{}",
            failures.join("\n")
        );
    }

    /// Build an operation with an explicit HTTP method and name, so the
    /// confirmation classifier (`requires_confirmation`) can be exercised.
    fn operation_with_method(id: &str, http_method: HttpMethod) -> OperationSchema {
        OperationSchema {
            id: OperationId::new(id),
            name: id.into(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::Mutating,
            http_method,
            path_template: "/".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// Return the `confirm` flag a synthesised single-command workflow assigns to `http_method`.
    fn confirm_for(id: &str, http_method: HttpMethod) -> bool {
        let op = operation_with_method(id, http_method);
        let service = ServiceId::new("svc");
        let op_id = op.id.clone();
        let schema = service_schema_with(op);
        synthesise_workflow_definition(&service, &op_id, &schema)
            .unwrap()
            .steps[0]
            .confirm
    }

    // The confirm flag now follows `requires_confirmation` (risky-keyword
    // classifier), not `MutationClass::Mutating`. This replaces the previous
    // `test_synth_mutating_operation_sets_confirm_true` /
    // `test_synth_reading_operation_sets_confirm_false`, which asserted the
    // old mutation-class behaviour — an intentional change, since these are
    // unit tests of the now-changed function (not parity guardrail tests).

    #[test]
    fn test_synth_post_without_risky_keyword_confirm_false() {
        // A plain POST create — `ags iam roles create` must not prompt.
        assert!(!confirm_for("create-role", HttpMethod::Post));
    }

    #[test]
    fn test_synth_post_with_risky_keyword_confirm_true() {
        assert!(confirm_for("delete-role", HttpMethod::Post));
        assert!(confirm_for("ban-user", HttpMethod::Post));
    }

    #[test]
    fn test_synth_delete_method_confirm_true() {
        // DELETE always confirms regardless of the operation name.
        assert!(confirm_for("list-items", HttpMethod::Delete));
    }

    #[test]
    fn test_synth_get_method_confirm_false() {
        assert!(!confirm_for("delete-user", HttpMethod::Get));
    }

    /// Build a `ServiceSchema` fixture with one `csm` upload operation
    /// exposing a required `file` formData parameter (`is_file: true`).
    fn service_schema_with_formdata_upload_operation() -> ServiceSchema {
        let op = operation(
            "csm/admin/app-ui/v1/upload-assets",
            MutationClass::Mutating,
            vec![ParameterSchema {
                name: "file".into(),
                location: ParameterLocation::FormData,
                required: true,
                value_type: ValueType::String,
                is_file: true,
                description: None,
                default: None,
            }],
            None,
        );
        service_schema_with(op)
    }

    /// A `formData` parameter's synthesised `WorkflowInputSpec.location` is
    /// `FormData`, not `Body` — the two are not interchangeable once
    /// multipart assembly depends on this to route the value correctly.
    #[test]
    fn test_formdata_param_location_is_formdata_not_body() {
        let schema = service_schema_with_formdata_upload_operation();
        let definition = synthesise_workflow_definition(
            &ServiceId::new("csm"),
            &OperationId::new("csm/admin/app-ui/v1/upload-assets"),
            &schema,
        )
        .unwrap();
        let file_input = definition.inputs.iter().find(|i| i.name == "file").unwrap();
        assert_eq!(file_input.location, StepFieldLocation::FormData);
    }
}
