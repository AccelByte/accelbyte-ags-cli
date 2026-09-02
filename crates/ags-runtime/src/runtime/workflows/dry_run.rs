//! Dry-run helpers: synthesise typed placeholder outputs for each step's
//! declared captures, and build the per-step `StepDryRunPreview` envelope
//! that flows into `CommandOutput::WorkflowDryRun`.

use std::collections::BTreeMap;

use ags_protocol::error::RuntimeError;
use ags_protocol::request::CommandRequest;
use ags_protocol::workflow::{CaptureSource, CompiledStep, StepDryRunAction, StepDryRunPreview};

use crate::catalogue::Catalogue;
use crate::runtime::workflows::auto_derive::find_operation_or_error;
use crate::runtime::Runtime;

/// Build a typed placeholder map for every output the step declares. Types
/// come from the response schema at the capture's JSONPath location, with
/// the fallback table from the spec for absent schemas.
pub fn synthesise_dry_run_outputs(
    step: &CompiledStep,
    catalogue: &mut Catalogue,
) -> Result<BTreeMap<String, serde_json::Value>, RuntimeError> {
    let mut out = BTreeMap::new();
    let op_ref = step.operation.as_ref().ok_or_else(|| {
        RuntimeError::internal(format!(
            "step '{}': API step reached synthesise_dry_run_outputs without an operation",
            step.id
        ))
    })?;
    let service_schema = catalogue.get_or_load(op_ref.service.as_str())?;
    let operation =
        find_operation_or_error(service_schema, op_ref, &format!("step '{}'", step.id))?;
    for capture in &step.outputs {
        let placeholder = match &capture.source {
            CaptureSource::ResponseBody { path } => placeholder_for_body_path(
                operation,
                path,
                &step.id,
                &capture.name,
                capture.default.as_ref(),
            ),
        };
        out.insert(capture.name.clone(), placeholder);
    }
    Ok(out)
}

/// Choose a placeholder value for a body-source capture. Today, v2 cannot
/// descend the response schema with confidence at arbitrary JSONPaths
/// (response shapes are not always typed in the catalogue), so the fallback
/// is the spec's type table applied to a string placeholder.
fn placeholder_for_body_path(
    _operation: &ags_protocol::catalogue::OperationSchema,
    _path: &str,
    step_id: &str,
    output_name: &str,
    default: Option<&serde_json::Value>,
) -> serde_json::Value {
    // v2: we do not type-resolve via response schema (`ResponseSchema`
    // currently exposes content type but not field-level typing). The
    // string placeholder is type-correct for the most common capture shape.
    // Authors needing typed dry-run output can declare `default:` on the
    // capture to override.
    if let Some(default) = default {
        return default.clone();
    }
    serde_json::Value::String(format!("<placeholder-{step_id}-{output_name}>"))
}

/// Build the per-step dry-run preview envelope for a workflow run. Calls
/// `runtime.dry_run_command` so the embedded `DryRunResult` is byte-
/// identical to today's single-command `--dry-run` output.
pub fn build_step_dry_run_preview(
    step: &CompiledStep,
    final_request: &CommandRequest,
    synthesised_outputs: &BTreeMap<String, serde_json::Value>,
    runtime: &mut Runtime,
) -> Result<StepDryRunPreview, RuntimeError> {
    let command = runtime.dry_run_command(final_request)?;
    Ok(StepDryRunPreview {
        step_id: step.id.clone(),
        step_index: step.index,
        action: StepDryRunAction::Request(command),
        synthesised_outputs: synthesised_outputs.clone(),
    })
}

/// Build the dry-run preview for a local-action step. `produced` is the value
/// the action's dry run returned; `synthesised_outputs` are the step's
/// declared captures already resolved against it, so downstream references
/// and the preview agree.
pub fn build_local_dry_run_preview(
    step: &CompiledStep,
    action: &str,
    produced: serde_json::Value,
    synthesised_outputs: BTreeMap<String, serde_json::Value>,
) -> StepDryRunPreview {
    StepDryRunPreview {
        step_id: step.id.clone(),
        step_index: step.index,
        action: StepDryRunAction::Local {
            action: action.to_string(),
            preview: produced,
        },
        synthesised_outputs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::workflow::{CompiledStep, OperationReference, StepOutputCapture};

    /// Build a compiled step with the given output captures and a synthetic
    /// operation id. Real catalogue lookups are bypassed for the synthesise
    /// helper's tests via a stripped-down runtime path; instead, we test the
    /// pure placeholder selection function directly.
    #[allow(dead_code)]
    fn step_with_outputs(outputs: Vec<StepOutputCapture>) -> CompiledStep {
        CompiledStep {
            id: "s1".into(),
            index: 0,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ags_protocol::catalogue::ServiceId::new("svc"),
                operation: ags_protocol::catalogue::OperationId::new("Op"),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs,
            auto_derived: vec![],
        }
    }

    #[test]
    fn test_placeholder_for_body_path_uses_default_when_present() {
        let dummy_op = dummy_operation();
        let placeholder =
            placeholder_for_body_path(&dummy_op, "$.id", "s1", "id", Some(&serde_json::json!(42)));
        assert_eq!(placeholder, serde_json::json!(42));
    }

    #[test]
    fn test_placeholder_for_body_path_falls_back_to_string_token() {
        let dummy_op = dummy_operation();
        let placeholder =
            placeholder_for_body_path(&dummy_op, "$.id", "create-stat", "statCode", None);
        assert_eq!(
            placeholder,
            serde_json::json!("<placeholder-create-stat-statCode>")
        );
    }

    #[test]
    fn test_api_step_without_operation_returns_internal_error() {
        let malformed = CompiledStep {
            id: "bad".into(),
            index: 0,
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
            auto_derived: vec![],
        };
        let mut catalogue = crate::catalogue::Catalogue::new();
        let err = synthesise_dry_run_outputs(&malformed, &mut catalogue).unwrap_err();
        assert_eq!(err.kind, ags_protocol::error::RuntimeErrorKind::Internal);
        assert!(
            err.message.contains("without an operation"),
            "error must explain the missing operation: {err}"
        );
    }

    /// Build a placeholder `OperationSchema` fixture.
    fn dummy_operation() -> ags_protocol::catalogue::OperationSchema {
        ags_protocol::catalogue::OperationSchema {
            id: ags_protocol::catalogue::OperationId::new("Op"),
            name: "op".into(),
            summary: String::new(),
            description: None,
            mutation_class: ags_protocol::catalogue::MutationClass::ReadOnly,
            http_method: ags_protocol::catalogue::HttpMethod::Get,
            path_template: "/".into(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ags_protocol::catalogue::ApiVersion::new(1),
            deprecated: false,
            response_content_type: None,
        }
    }
}
