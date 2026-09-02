//! Step-input resolution: compute which inputs the executor still needs to
//! gather, then assemble the final dispatch request from declared bindings,
//! gathered values, auto-bound workflow inputs, and OpenAPI defaults.

use std::collections::BTreeMap;

use ags_protocol::catalogue::{OperationSchema, ParameterLocation, ServiceSchema};
use ags_protocol::error::{RuntimeError, RuntimeErrorKind};
use ags_protocol::request::{CommandRequest, RequestBody};
use ags_protocol::workflow::{
    ArithmeticOp, ArithmeticOperand, AutoDeriveScope, BindingSource, CompiledStep, GatherSlotId,
    ReferenceTarget, StepField, StepFieldId, StepFieldLocation, StepFieldPlan, TransformKind,
    WorkflowInputNeeded, WorkflowInputSpec,
};

use crate::runtime::workflows::auto_derive::{
    body_field_schema, find_operation_or_error, parameter_schema,
};
use crate::runtime::workflows::jsonpath::apply_jsonpath_subset;
use crate::runtime::workflows::nested_path::{FieldPath, PathSegment};
use crate::runtime::workflows::WorkflowContext;
use crate::support::strings::to_kebab_case;

/// Enumerate the inputs the step still needs gathered. Walks explicit
/// bindings — a `from: workflow/X` source, an Arithmetic transform operand
/// `workflow/X`, or a Format-template placeholder `{X}` whose X is unsupplied —
/// and the step's auto-derived fields (for missing auto-bind and step-local
/// fields). Slot ids are minted contiguously from 0 and labels are
/// name-prefixed per scope.
pub fn compute_needed_inputs(
    step: &CompiledStep,
    workflow_supplied: &BTreeMap<String, serde_json::Value>,
    workflow_input_specs: &[ags_protocol::workflow::WorkflowInputSpec],
) -> Vec<WorkflowInputNeeded> {
    let mut needed: Vec<WorkflowInputNeeded> = Vec::new();
    let mut seen_workflow_inputs: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    let input_specs: BTreeMap<&str, &ags_protocol::workflow::WorkflowInputSpec> =
        workflow_input_specs
            .iter()
            .map(|s| (s.name.as_str(), s))
            .collect();

    let push_workflow_input = |needed: &mut Vec<WorkflowInputNeeded>,
                               seen: &mut std::collections::BTreeSet<String>,
                               name: &str| {
        if seen.contains(name) {
            return;
        }
        seen.insert(name.to_string());
        let spec = input_specs.get(name).copied();
        needed.push(WorkflowInputNeeded {
            id: GatherSlotId(needed.len() as u32),
            label: to_kebab_case(name),
            description: spec.and_then(|s| s.description.clone()),
            schema: spec
                .and_then(|s| s.schema.clone())
                .unwrap_or_else(|| serde_json::json!({"type": "string"})),
            default: spec.and_then(|s| s.default.clone()),
            required: spec.map(|s| s.required).unwrap_or(true),
            sensitive: spec.map(|s| s.sensitive).unwrap_or(false),
            scope: AutoDeriveScope::WorkflowInput {
                name: name.to_string(),
            },
            location: spec.map(|s| s.location).unwrap_or_default(),
        });
    };

    // 1. Walk explicit bindings. A workflow input is needed when it is the
    //    binding's direct source, the operand of an Arithmetic transform, or a
    //    placeholder in a Format template — each reads from `workflow_supplied`
    //    at resolve time, so an unsupplied one must be gathered even when it
    //    binds no field directly. The binding traversal is shared with the
    //    gather-form ordering walk so the two can't drift.
    crate::runtime::workflows::compile::visit_binding_workflow_inputs(step, |name| {
        if !workflow_supplied.contains_key(name) {
            push_workflow_input(&mut needed, &mut seen_workflow_inputs, name);
        }
    });

    // 2. Walk auto-derived fields.
    for field in &step.auto_derived {
        match &field.scope {
            AutoDeriveScope::WorkflowInput { name } => {
                if !workflow_supplied.contains_key(name) {
                    push_workflow_input(&mut needed, &mut seen_workflow_inputs, name);
                }
            }
            AutoDeriveScope::StepLocal { field_name } => {
                let label = format!("{}.{}", step.id, to_kebab_case(field_name));
                needed.push(WorkflowInputNeeded {
                    id: GatherSlotId(needed.len() as u32),
                    label,
                    description: field.description.clone(),
                    schema: field.schema.clone(),
                    default: None,
                    required: field.required,
                    sensitive: field.sensitive,
                    scope: AutoDeriveScope::StepLocal {
                        field_name: field_name.clone(),
                    },
                    location: field.location,
                });
            }
        }
    }

    needed
}

/// Resolve every operation field of the step into a final dispatch
/// request. Walks `step.inputs` first (literal or reference), then auto-bind
/// against workflow_supplied, then step_local, then OpenAPI default; absent
/// optional fields are omitted. Returns a `CommandRequest` with path/query
/// params and optional body populated.
///
/// Caller supplies the resolved `ServiceSchema` (loaded via `Catalogue`)
/// because this helper is pure-functional and the catalogue lookup is
/// expensive enough to hoist.
///
/// `run_options` threads `--output`, `--verbose`, pagination, and the
/// output format into the produced `CommandRequest` so dispatch reproduces
/// the legacy single-command behavioral surface.
#[allow(clippy::too_many_arguments)]
pub fn assemble_command_request(
    step: &CompiledStep,
    ctx: &WorkflowContext,
    workflow_supplied: &BTreeMap<String, serde_json::Value>,
    step_local: &BTreeMap<String, serde_json::Value>,
    workflow_input_specs: &[ags_protocol::workflow::WorkflowInputSpec],
    service_schema: &ServiceSchema,
    namespace: Option<String>,
    run_options: &super::RunOptions,
) -> Result<CommandRequest, RuntimeError> {
    let op_ref = step.operation.as_ref().ok_or_else(|| {
        RuntimeError::internal(format!(
            "step '{}': API step reached assemble_command_request without an operation",
            step.id
        ))
    })?;
    let operation =
        find_operation_or_error(service_schema, op_ref, &format!("step '{}'", step.id))?;

    // Lookup maps derived once from the step's bindings and the workflow inputs,
    // shared by both assembly passes. `nested_binding_roots` holds the roots a
    // nested-path binding claims (`requestedRegions[0]` claims `requestedRegions`)
    // so a required body field satisfied only by nested bindings doesn't error in
    // pass 1 before pass 2 populates it.
    let build_ctx = RequestBuildCtx {
        bindings_by_field: step
            .inputs
            .iter()
            .map(|b| (b.field.as_str(), &b.source))
            .collect(),
        workflow_input_names: workflow_input_specs
            .iter()
            .map(|s| s.name.as_str())
            .collect(),
        nested_binding_roots: step
            .inputs
            .iter()
            .filter(|b| b.field.contains('.') || b.field.contains('['))
            .filter_map(|b| b.field.split(['.', '[']).next())
            .collect(),
    };

    let (path_params, query_params, header_params) = resolve_non_body_params(
        operation,
        step,
        &build_ctx,
        ctx,
        workflow_supplied,
        step_local,
    )?;

    let form_parts = resolve_form_data_params(
        operation,
        step,
        ctx,
        workflow_supplied,
        step_local,
        &build_ctx,
    )?;
    let body = match form_parts {
        Some(parts) => Some(RequestBody::Multipart(parts)),
        None => assemble_body(
            operation,
            step,
            ctx,
            workflow_supplied,
            step_local,
            &build_ctx,
            run_options,
        )?
        .map(RequestBody::Json),
    };

    Ok(CommandRequest {
        service: op_ref.service.clone(),
        operation_id: op_ref.operation.clone(),
        namespace,
        path_params,
        query_params,
        header_params,
        form_params: BTreeMap::new(),
        body,
        output_format: run_options.output_format,
        pagination: run_options.pagination,
        verbosity: run_options.verbosity,
        output: run_options.output.clone(),
    })
}

/// Lookup maps derived once from a step's bindings and the workflow's inputs,
/// shared by the param and body assembly passes.
struct RequestBuildCtx<'a> {
    /// Top-level field name → its binding source.
    bindings_by_field: BTreeMap<&'a str, &'a BindingSource>,
    /// Names of the workflow's declared inputs.
    workflow_input_names: std::collections::BTreeSet<&'a str>,
    /// Roots claimed by a nested-path binding (`requestedRegions[0]` claims
    /// `requestedRegions`).
    nested_binding_roots: std::collections::BTreeSet<&'a str>,
}

/// Resolved path, query, and header parameter maps for a request.
type NonBodyParams = (
    BTreeMap<String, String>,
    BTreeMap<String, String>,
    BTreeMap<String, String>,
);

/// Resolve the step's path, query, and header parameters from its bindings,
/// returning the three populated maps. Errors on a missing required
/// parameter. `formData` parameters are skipped entirely here — they never
/// populate these maps, and `resolve_form_data_params` (called separately by
/// `assemble_command_request`) owns both their value resolution and their
/// required-ness validation, so this function doesn't resolve or
/// double-check them.
fn resolve_non_body_params(
    operation: &OperationSchema,
    step: &CompiledStep,
    build_ctx: &RequestBuildCtx,
    ctx: &WorkflowContext,
    workflow_supplied: &BTreeMap<String, serde_json::Value>,
    step_local: &BTreeMap<String, serde_json::Value>,
) -> Result<NonBodyParams, RuntimeError> {
    let mut path_params: BTreeMap<String, String> = BTreeMap::new();
    let mut query_params: BTreeMap<String, String> = BTreeMap::new();
    let mut header_params: BTreeMap<String, String> = BTreeMap::new();

    for param in &operation.parameters {
        if matches!(
            param.location,
            ParameterLocation::Body | ParameterLocation::FormData
        ) {
            // Body is assembled separately by `assemble_body`; formData is
            // resolved and validated separately by `resolve_form_data_params`.
            continue;
        }
        let resolved = resolve_field_value(
            &param.name,
            &build_ctx.bindings_by_field,
            ctx,
            workflow_supplied,
            step_local,
            &build_ctx.workflow_input_names,
        )?;
        match resolved {
            Some(v) => {
                let as_string = json_to_param_string(&v);
                match param.location {
                    ParameterLocation::Path => {
                        path_params.insert(param.name.clone(), as_string);
                    }
                    ParameterLocation::Query => {
                        query_params.insert(param.name.clone(), as_string);
                    }
                    ParameterLocation::Header => {
                        header_params.insert(param.name.clone(), as_string);
                    }
                    // Body and FormData params are skipped above and never
                    // reach here.
                    ParameterLocation::Body | ParameterLocation::FormData => {
                        unreachable!("body/formData params are skipped above")
                    }
                }
            }
            None => {
                if param.required {
                    return Err(RuntimeError {
                        kind: RuntimeErrorKind::Validation,
                        message: format!(
                            "step '{}' is missing required parameter '{}'",
                            step.id, param.name
                        ),
                        details: None,
                        hint: None,
                        trace: None,
                    });
                }
            }
        }
    }

    Ok((path_params, query_params, header_params))
}

/// Reconstruct the request body for the step. An explicit `--json` body is used
/// verbatim. Otherwise the body is assembled in two passes: pass 1 fills
/// single-segment (schema) fields from their bindings; pass 2 merges
/// multi-segment (nested-path) bindings on top, with step-local per-leaf edits
/// taking precedence. Object-array bodies are wrapped in a single-element array.
fn assemble_body(
    operation: &OperationSchema,
    step: &CompiledStep,
    ctx: &WorkflowContext,
    workflow_supplied: &BTreeMap<String, serde_json::Value>,
    step_local: &BTreeMap<String, serde_json::Value>,
    build_ctx: &RequestBuildCtx,
    run_options: &super::RunOptions,
) -> Result<Option<serde_json::Value>, RuntimeError> {
    if let Some(explicit) = &run_options.explicit_body {
        return Ok(Some(explicit.clone()));
    }

    let mut body_obj: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();

    // Pass 1: schema-driven, single-segment field assembly.
    if let Some(body_schema) = &operation.request_body {
        for field in &body_schema.fields {
            let resolved = resolve_field_value(
                &field.name,
                &build_ctx.bindings_by_field,
                ctx,
                workflow_supplied,
                step_local,
                &build_ctx.workflow_input_names,
            )?;
            match resolved {
                Some(v) => {
                    body_obj.insert(field.name.clone(), v);
                }
                None => {
                    // Required + no top-level binding is OK if a nested
                    // binding will populate the field in pass 2.
                    if field.required
                        && !build_ctx.nested_binding_roots.contains(field.name.as_str())
                    {
                        return Err(RuntimeError {
                            kind: RuntimeErrorKind::Validation,
                            message: format!(
                                "step '{}' is missing required body field '{}'",
                                step.id, field.name
                            ),
                            details: None,
                            hint: None,
                            trace: None,
                        });
                    }
                }
            }
        }
    }

    // Pass 2: merge multi-segment (nested) bindings into the assembled body.
    // Ignoring parse errors here is safe: the compile-time validator guarantees
    // all binding paths are well-formed before a workflow runs. Step-local
    // overrides (from a per-step review form's per-leaf edit) take precedence
    // over the binding's declared value.
    let mut body_value = serde_json::Value::Object(body_obj);
    for binding in &step.inputs {
        let field = &binding.field;
        if !field.contains('.') && !field.contains('[') {
            continue; // single-segment — already handled
        }
        let Ok(path) = FieldPath::parse(field) else {
            continue; // unreachable for compile-time-validated workflows
        };
        if path.segments.len() <= 1 {
            continue;
        }
        let resolved = if let Some(local_val) = step_local.get(field.as_str()) {
            Some(local_val.clone())
        } else {
            resolve_binding(
                &binding.source,
                ctx,
                workflow_supplied,
                &build_ctx.bindings_by_field,
                step_local,
            )?
        };
        if let Some(v) = resolved {
            merge_nested_binding(&mut body_value, &path, v);
        }
    }

    let body_obj = match body_value {
        serde_json::Value::Object(m) => m,
        _ => Default::default(),
    };
    let is_array_body = operation.request_body.as_ref().is_some_and(|b| b.is_array);
    // This wrapping only applies to *object* arrays: the gathered fields become
    // a single-element array of one object. Scalar-array bodies (`item_type` is
    // `Some`) have no named fields to gather, so `body_obj` is always empty here
    // and they fall through to `None` — supplied wholesale via `--json`.
    if body_obj.is_empty() {
        Ok(None)
    } else if is_array_body {
        Ok(Some(serde_json::Value::Array(vec![
            serde_json::Value::Object(body_obj),
        ])))
    } else {
        Ok(Some(serde_json::Value::Object(body_obj)))
    }
}

/// Resolve every `formData` parameter of the step into `FormPart`s, in the
/// operation's declared parameter order. A file-typed parameter's resolved
/// value is a local filesystem path; it is validated (exists, is a regular
/// file) here so a bad path surfaces as a clean `Validation` error before
/// any network call, in both normal and `--dry-run` runs. Returns `None`
/// when the operation has no `formData` parameters (the common case — most
/// operations use a JSON body instead).
fn resolve_form_data_params(
    operation: &OperationSchema,
    step: &CompiledStep,
    ctx: &WorkflowContext,
    workflow_supplied: &BTreeMap<String, serde_json::Value>,
    step_local: &BTreeMap<String, serde_json::Value>,
    build_ctx: &RequestBuildCtx,
) -> Result<Option<Vec<ags_protocol::request::FormPart>>, RuntimeError> {
    use ags_protocol::request::FormPart;

    let mut parts = Vec::new();
    for param in &operation.parameters {
        if param.location != ParameterLocation::FormData {
            continue;
        }
        let resolved = resolve_field_value(
            &param.name,
            &build_ctx.bindings_by_field,
            ctx,
            workflow_supplied,
            step_local,
            &build_ctx.workflow_input_names,
        )?;
        let Some(value) = resolved else {
            if param.required {
                return Err(RuntimeError {
                    kind: RuntimeErrorKind::Validation,
                    message: format!(
                        "step '{}' is missing required form field '{}'",
                        step.id, param.name
                    ),
                    details: None,
                    hint: None,
                    trace: None,
                });
            }
            continue;
        };
        let as_string = json_to_param_string(&value);
        if param.is_file {
            parts.push(validate_and_build_file_part(&param.name, &as_string)?);
        } else {
            parts.push(FormPart::Text {
                name: param.name.clone(),
                value: as_string,
            });
        }
    }
    Ok((!parts.is_empty()).then_some(parts))
}

/// Validate a formData file parameter's local path (exists, is a regular
/// file) and build its `FormPart::File`. `field_name` names the parameter in
/// error messages so the user knows which flag to fix.
///
/// This check reflects the file's state at resolve time only — it is not
/// re-verified when the file is later streamed for dispatch, so a file
/// changed after a confirm-gated pause is not caught here. Symlinks are
/// followed deliberately, matching typical CLI upload semantics (e.g.
/// `curl -F`). There is no upload size cap: the file is streamed from disk
/// at send time (`dispatch::http::ReqwestHttpClient::send`) rather than
/// read into memory, so an arbitrarily large file never spikes RAM.
fn validate_and_build_file_part(
    field_name: &str,
    raw_path: &str,
) -> Result<ags_protocol::request::FormPart, RuntimeError> {
    use ags_protocol::request::FormPart;

    let path = std::path::PathBuf::from(raw_path);
    let metadata = std::fs::metadata(&path).map_err(|_| RuntimeError {
        kind: RuntimeErrorKind::Validation,
        message: format!(
            "form field '{field_name}': file '{}' does not exist or is not readable",
            path.display()
        ),
        details: None,
        hint: None,
        trace: None,
    })?;
    if !metadata.is_file() {
        return Err(RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message: format!(
                "form field '{field_name}': '{}' is not a regular file",
                path.display()
            ),
            details: None,
            hint: None,
            trace: None,
        });
    }
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("upload")
        .to_string();
    Ok(FormPart::File {
        name: field_name.to_string(),
        path,
        filename,
    })
}

/// Append the workflow-input source names a binding references to `sources`
/// (deduped) — covering reference targets, arithmetic operands, and format
/// placeholders. Returns whether the binding is a pure literal (contributing no
/// source), so the caller can maintain its `all_literal` flag.
fn collect_binding_sources(source: &BindingSource, sources: &mut Vec<String>) -> bool {
    /// Append `name` to `sources` if not already present.
    fn push_unique(sources: &mut Vec<String>, name: &str) {
        if !sources.iter().any(|s| s == name) {
            sources.push(name.to_string());
        }
    }
    match source {
        BindingSource::Literal(_) => return true,
        BindingSource::Reference(r) => match &r.from {
            ReferenceTarget::Workflow { input } => {
                push_unique(sources, input);
                // Also collect the RHS workflow input when an Arithmetic
                // transform uses one as its operand.
                if let Some(TransformKind::Arithmetic(arith)) = &r.transform {
                    if let ArithmeticOperand::WorkflowInput(op_name) = &arith.operand {
                        push_unique(sources, op_name);
                    }
                }
            }
            // Prior-output reference: contributes no named workflow-input
            // source, but is still non-literal — falls through to `false` below
            // so `all_literal` is cleared. A new `ReferenceTarget` variant must
            // decide the same: push any sources, then leave this `false` return.
            ReferenceTarget::Step { .. } => {}
        },
        BindingSource::Format(format) => {
            let placeholders =
                crate::runtime::workflows::compile::format_placeholders(&format.template)
                    .unwrap_or_default();
            for name in placeholders {
                push_unique(sources, &name);
            }
        }
        // A mirror copies a same-step field: it contributes no workflow-input
        // source of its own (the target binding contributes any). Falls through
        // to `false`; with no collected sources the caller still classifies the
        // synthesized parent as Literal.
        BindingSource::Mirror(_) => {}
    }
    false
}

/// Merge `value` into `body` at the location described by `path`.
///
/// Each `PathSegment::Key` descends into an object (creating one if needed);
/// each `PathSegment::Index` descends into an array element (extending the
/// array with `Null` sentinels if needed). The final segment writes the value.
fn merge_nested_binding(body: &mut serde_json::Value, path: &FieldPath, value: serde_json::Value) {
    let mut current = body;
    let segments = &path.segments;
    for (i, seg) in segments.iter().enumerate() {
        let is_last = i + 1 == segments.len();
        match seg {
            PathSegment::Key(name) => {
                if !current.is_object() {
                    *current = serde_json::Value::Object(Default::default());
                }
                let object = current.as_object_mut().unwrap();
                if is_last {
                    object.insert(name.clone(), value);
                    return;
                }
                current = object
                    .entry(name.clone())
                    .or_insert_with(|| serde_json::Value::Object(Default::default()));
            }
            PathSegment::Index(index) => {
                if !current.is_array() {
                    *current = serde_json::Value::Array(Vec::new());
                }
                let array = current.as_array_mut().unwrap();
                while array.len() <= *index {
                    array.push(serde_json::Value::Null);
                }
                if is_last {
                    array[*index] = value;
                    return;
                }
                current = &mut array[*index];
            }
        }
    }
}

/// Resolve a single operation field's final value by walking declared
/// bindings first, then auto-bind, then step-local. Returns `None` when no
/// source produces a value (caller decides whether that's an error based on
/// the field's `required` flag).
fn resolve_field_value(
    field_name: &str,
    bindings: &BTreeMap<&str, &BindingSource>,
    ctx: &WorkflowContext,
    workflow_supplied: &BTreeMap<String, serde_json::Value>,
    step_local: &BTreeMap<String, serde_json::Value>,
    workflow_input_names: &std::collections::BTreeSet<&str>,
) -> Result<Option<serde_json::Value>, RuntimeError> {
    // Step-local overrides (from a per-step review form's per-leaf edit) take
    // precedence over the field's declared binding — same convention pass 2 of
    // `assemble_body` already applies to nested (dotted/indexed) bindings.
    if let Some(v) = step_local.get(field_name) {
        return Ok(Some(v.clone()));
    }
    if let Some(source) = bindings.get(field_name) {
        return resolve_binding(source, ctx, workflow_supplied, bindings, step_local);
    }
    if workflow_input_names.contains(field_name) {
        if let Some(v) = workflow_supplied.get(field_name) {
            return Ok(Some(v.clone()));
        }
    }
    Ok(None)
}

/// A resolved field: its value (if any), its provenance, and the workflow-input
/// name to route form edits back to (if any).
type FieldResolution = (
    Option<serde_json::Value>,
    ags_protocol::workflow::StepFieldSource,
    Option<String>,
);

/// Like `resolve_field_value`, but also reports the field's provenance and its
/// workflow-input propagation target (for the per-step review plan).
/// `default_names` is the set of workflow inputs whose value came from their
/// declared default (the executor's `default_added`), used to distinguish
/// `WorkflowInput` from `Default`. (No `flag_names` is needed: any supplied
/// workflow input not in `default_names` is `WorkflowInput`.)
///
/// `step_bindings` is the step's full inputs list and is used to synthesize a
/// displayed value when the field has no top-level binding but does have nested
/// bindings (e.g. `requestedRegions[0]` with no `requestedRegions` binding).
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_field_with_source(
    field_name: &str,
    bindings: &BTreeMap<&str, &BindingSource>,
    step_bindings: &[ags_protocol::workflow::StepInputBinding],
    ctx: &WorkflowContext,
    workflow_supplied: &BTreeMap<String, serde_json::Value>,
    step_local: &BTreeMap<String, serde_json::Value>,
    workflow_input_names: &std::collections::BTreeSet<&str>,
    default_names: &std::collections::BTreeSet<String>,
) -> Result<FieldResolution, RuntimeError> {
    use ags_protocol::workflow::StepFieldSource;

    // Provenance for a value that came from a workflow input named `name`.
    let wf_source = |name: &str| {
        if default_names.contains(name) {
            StepFieldSource::Default {
                name: name.to_string(),
            }
        } else {
            StepFieldSource::WorkflowInput {
                name: name.to_string(),
            }
        }
    };

    // Step-local overrides (from a per-step review form's per-leaf edit) take
    // precedence over the field's declared binding — same convention as
    // `resolve_field_value` and pass 2 of `assemble_body`. An edited field is
    // reported as a literal, same as `push_nested_literal_review_fields` does
    // for edited nested-path literals.
    if let Some(v) = step_local.get(field_name) {
        return Ok((Some(v.clone()), StepFieldSource::Literal, None));
    }
    if let Some(source) = bindings.get(field_name) {
        let value = resolve_binding(source, ctx, workflow_supplied, bindings, step_local)?;
        return Ok(match source {
            // A mirror displays its target's effective value; like a literal,
            // it has no workflow input to route edits back to.
            BindingSource::Mirror(_) => (value, StepFieldSource::Literal, None),
            BindingSource::Literal(_) => (value, StepFieldSource::Literal, None),
            BindingSource::Reference(r) => match &r.from {
                ReferenceTarget::Workflow { input: _ } => {
                    let transform = r.transform.as_ref();
                    let src = if value.is_some() || transform.is_some() {
                        let classified = classify_binding_source(source, transform);
                        // Restore Default provenance for plain WorkflowInput bindings whose
                        // value came from the input's declared default. classify_binding_source
                        // doesn't have access to default_names, so we patch it here.
                        match &classified {
                            StepFieldSource::WorkflowInput { name }
                                if default_names.contains(name.as_str()) =>
                            {
                                StepFieldSource::Default { name: name.clone() }
                            }
                            _ => classified,
                        }
                    } else {
                        StepFieldSource::Unset
                    };
                    // For plain WorkflowInput/Default references, propagate the input name so
                    // form edits route back to the correct workflow input.
                    let wf_input = match &src {
                        StepFieldSource::WorkflowInput { name }
                        | StepFieldSource::Default { name } => Some(name.clone()),
                        // Derived fields route edits back via the listed sources, not
                        // a single workflow_input propagation target.
                        _ => None,
                    };
                    (value, src, wf_input)
                }
                ReferenceTarget::Step { .. } => (value, StepFieldSource::PriorOutput, None),
            },
            BindingSource::Format(_) => {
                let src = classify_binding_source(source, None);
                (value, src, None)
            }
        });
    }
    if workflow_input_names.contains(field_name) {
        if let Some(v) = workflow_supplied.get(field_name) {
            return Ok((
                Some(v.clone()),
                wf_source(field_name),
                Some(field_name.to_string()),
            ));
        }
        // Named workflow input but not yet supplied → unset, still propagates.
        return Ok((None, StepFieldSource::Unset, Some(field_name.to_string())));
    }

    // Nested-binding synthesis: no top-level binding for `field_name`, but the
    // step may have bindings like `field_name[0]` or `field_name.key` whose
    // resolved values should be visible in the review UI.
    if let Some(result) = synthesize_nested_field(
        field_name,
        step_bindings,
        ctx,
        workflow_supplied,
        bindings,
        step_local,
    )? {
        return Ok(result);
    }

    Ok((None, StepFieldSource::Unset, None))
}

/// Synthesize a displayed value for a field that has no top-level binding but
/// does have nested bindings (e.g. `requestedRegions[0]`). Walks every binding
/// whose path starts with `field_name` and has more than one segment, merging
/// their resolved values (with the leading segment stripped) in order, and
/// computes provenance from the collected source names. Returns `Ok(None)` when
/// the field has no nested bindings.
fn synthesize_nested_field(
    field_name: &str,
    step_bindings: &[ags_protocol::workflow::StepInputBinding],
    ctx: &WorkflowContext,
    workflow_supplied: &BTreeMap<String, serde_json::Value>,
    bindings: &BTreeMap<&str, &BindingSource>,
    step_local: &BTreeMap<String, serde_json::Value>,
) -> Result<Option<FieldResolution>, RuntimeError> {
    use ags_protocol::workflow::StepFieldSource;

    let mut synthesized = serde_json::Value::Null;
    // Collect workflow-input source names (deduplicated, order of first appearance).
    let mut sources: Vec<String> = Vec::new();
    let mut has_nested = false;
    let mut all_literal = true; // true when every nested binding is Literal

    for binding in step_bindings {
        let Ok(path) = FieldPath::parse(&binding.field) else {
            continue;
        };
        // Must be multi-segment and start with `field_name`.
        if path.segments.len() <= 1 {
            continue;
        }
        let first_is_target = matches!(
            path.segments.first(),
            Some(PathSegment::Key(k)) if k == field_name
        );
        if !first_is_target {
            continue;
        }
        // Resolve the nested binding's value and merge it into the synthesized
        // result. Strip the leading field-name segment so the merge represents
        // just the field's value, not the body containing it.
        let resolved = resolve_binding(
            &binding.source,
            ctx,
            workflow_supplied,
            bindings,
            step_local,
        )?;
        if let Some(v) = resolved {
            let sub_path = FieldPath {
                segments: path.segments[1..].to_vec(),
            };
            merge_nested_binding(&mut synthesized, &sub_path, v);
        }
        has_nested = true;
        all_literal &= collect_binding_sources(&binding.source, &mut sources);
    }

    if !has_nested {
        return Ok(None);
    }

    let provenance = if all_literal || sources.is_empty() {
        StepFieldSource::Literal
    } else {
        StepFieldSource::Derived { sources }
    };
    let value = if synthesized == serde_json::Value::Null {
        None
    } else {
        Some(synthesized)
    };
    Ok(Some((value, provenance, None)))
}

/// Build the editable field plan for a step: one entry per unique binding key,
/// each with its resolved value + provenance. Required-but-unresolved fields
/// appear with `value: null` and `source: Unset` (the form collects them).
///
/// `default_names` is the executor's `default_added` set (workflow inputs whose
/// value came from their declared default). Workflow runs never set
/// `RunOptions.explicit_body` (that's a service-command path, which doesn't use
/// per-step review), so this builds straight from params + top-level body fields.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_step_fields(
    step: &CompiledStep,
    ctx: &WorkflowContext,
    workflow_supplied: &BTreeMap<String, serde_json::Value>,
    step_local: &BTreeMap<String, serde_json::Value>,
    workflow_input_specs: &[ags_protocol::workflow::WorkflowInputSpec],
    service_schema: &ServiceSchema,
    default_names: &std::collections::BTreeSet<String>,
) -> Result<StepFieldPlan, RuntimeError> {
    let op_ref = step.operation.as_ref().ok_or_else(|| {
        RuntimeError::internal(format!(
            "step '{}': API step reached resolve_step_fields without an operation",
            step.id
        ))
    })?;
    let operation =
        find_operation_or_error(service_schema, op_ref, &format!("step '{}'", step.id))?;

    let workflow_input_names: std::collections::BTreeSet<&str> = workflow_input_specs
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    let bindings_by_field: BTreeMap<&str, &BindingSource> = step
        .inputs
        .iter()
        .map(|b| (b.field.as_str(), &b.source))
        .collect();

    // Build show_in_review and bound_roots maps.
    //
    // show_in_review_by_root: only top-level bindings (single-segment field)
    // contribute. Nested bindings with show_in_review=true get their own
    // synthetic per-leaf StepField rows (see below); they do NOT aggregate up
    // to the parent's StepField.
    //
    // bound_roots: every root that has at least one binding (top-level or
    // nested) so the `individual` display check can promote fields with only
    // nested bindings.
    let (show_in_review_by_root, bound_roots) = build_review_roots(step);

    // Full binding lookup (keyed by top-level field name only, not nested paths)
    // for description and show_in_review lookups.
    let full_bindings_by_field: BTreeMap<&str, &ags_protocol::workflow::StepInputBinding> = step
        .inputs
        .iter()
        .filter(|b| !b.field.contains('.') && !b.field.contains('['))
        .map(|b| (b.field.as_str(), b))
        .collect();

    let mut fields: Vec<StepField> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut next_id: u32 = 0;

    let push = |fields: &mut Vec<StepField>,
                seen: &mut std::collections::BTreeSet<String>,
                next_id: &mut u32,
                name: &str,
                description: Option<String>,
                location: StepFieldLocation,
                schema: serde_json::Value,
                required: bool,
                show_in_review: bool|
     -> Result<(), RuntimeError> {
        if seen.contains(name) {
            return Ok(()); // deduped by binding key
        }
        seen.insert(name.to_string());
        let (value, source, workflow_input) = resolve_field_with_source(
            name,
            &bindings_by_field,
            &step.inputs,
            ctx,
            workflow_supplied,
            step_local,
            &workflow_input_names,
            default_names,
        )?;
        fields.push(StepField {
            id: StepFieldId(*next_id),
            field: name.to_string(),
            label: to_kebab_case(name),
            description,
            location,
            schema,
            value: value.unwrap_or(serde_json::Value::Null),
            source,
            required,
            workflow_input,
            body_overflow: false,
            show_in_review,
        });
        *next_id += 1;
        Ok(())
    };

    // Path / query / header / formData parameters.
    for param in &operation.parameters {
        let location = match param.location {
            ParameterLocation::Path => StepFieldLocation::Path,
            ParameterLocation::Query => StepFieldLocation::Query,
            ParameterLocation::Header => StepFieldLocation::Header,
            ParameterLocation::FormData => StepFieldLocation::FormData,
            // Body is handled via the body schema below.
            ParameterLocation::Body => continue,
        };
        let param_description = if param.is_file {
            Some("Path to local file to upload".to_string())
        } else {
            find_workflow_input_description(
                &param.name,
                &full_bindings_by_field,
                workflow_input_specs,
            )
            .or_else(|| param.description.clone())
        };
        let param_show_in_review = show_in_review_by_root
            .get(param.name.as_str())
            .copied()
            .unwrap_or(false);
        push(
            &mut fields,
            &mut seen,
            &mut next_id,
            &param.name,
            param_description,
            location,
            parameter_schema(param),
            param.required,
            param_show_in_review,
        )?;
    }

    // Top-level body fields. Fields the workflow actually sets (author-bound, or
    // auto-bound to a workflow input) and required-but-unset fields are shown
    // individually; the remaining unbound optional fields collapse into one
    // "more body" JSON field so the form is not flooded with operation-schema
    // fields the workflow never touches.
    if let Some(body) = &operation.request_body {
        let mut overflow_names: Vec<String> = Vec::new();
        let mut overflow_props = serde_json::Map::new();
        for field in &body.fields {
            let individual = bindings_by_field.contains_key(field.name.as_str())
                || bound_roots.contains(field.name.as_str())
                || workflow_input_names.contains(field.name.as_str())
                || field.required;
            if individual {
                let field_description = find_workflow_input_description(
                    &field.name,
                    &full_bindings_by_field,
                    workflow_input_specs,
                )
                .or_else(|| field.description.clone());
                let field_show_in_review = show_in_review_by_root
                    .get(field.name.as_str())
                    .copied()
                    .unwrap_or(false);
                push(
                    &mut fields,
                    &mut seen,
                    &mut next_id,
                    &field.name,
                    field_description,
                    StepFieldLocation::Body,
                    body_field_schema(field),
                    field.required,
                    field_show_in_review,
                )?;
            } else if !seen.contains(&field.name) {
                overflow_names.push(field.name.clone());
                overflow_props.insert(field.name.clone(), body_field_schema(field));
            }
        }
        if !overflow_props.is_empty() {
            // One collapsed JSON field for the optional fields the workflow does
            // not set. Its schema carries each optional field as a property so
            // the JSON editor shows them as editable rows; edited keys are merged
            // into the body by the executor (see `StepField::body_overflow`).
            fields.push(StepField {
                id: StepFieldId(next_id),
                field: String::new(),
                label: "Additional Parameters".to_string(),
                description: Some(format!(
                    "Optional body fields you can add: {}",
                    overflow_names.join(", ")
                )),
                location: StepFieldLocation::Body,
                schema: serde_json::json!({"type": "object", "properties": overflow_props}),
                value: serde_json::json!({}),
                source: ags_protocol::workflow::StepFieldSource::Unset,
                required: false,
                workflow_input: None,
                body_overflow: true,
                show_in_review: false,
            });
            next_id += 1;
        }
    }
    push_nested_literal_review_fields(step, step_local, &mut fields, &mut next_id);

    let _ = next_id; // final id count; no further fields appended

    Ok(StepFieldPlan {
        step_index: step.index,
        step_label: step.id.clone(),
        step_description: step.description.clone(),
        optional: step.is_optional,
        fields,
    })
}

/// Build the per-root `show_in_review` flags and the set of bound roots for a
/// step. Only top-level bindings contribute to `show_in_review`; nested bindings
/// get per-leaf rows instead but still register their root as bound (so a field
/// with only nested bindings is still shown individually).
fn build_review_roots(
    step: &CompiledStep,
) -> (BTreeMap<&str, bool>, std::collections::BTreeSet<&str>) {
    let mut show_in_review_by_root: BTreeMap<&str, bool> = BTreeMap::new();
    let mut bound_roots: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for binding in &step.inputs {
        let root = binding
            .field
            .split_once('.')
            .map(|(r, _)| r)
            .or_else(|| binding.field.split_once('[').map(|(r, _)| r))
            .unwrap_or(binding.field.as_str());
        bound_roots.insert(root);
        let is_top_level = !binding.field.contains('.') && !binding.field.contains('[');
        if is_top_level {
            let entry = show_in_review_by_root.entry(root).or_insert(false);
            *entry = *entry || binding.show_in_review;
        } else {
            // Ensure the root appears (defaulting to false) so a root with only
            // nested bindings still has an entry.
            show_in_review_by_root.entry(root).or_insert(false);
        }
    }
    (show_in_review_by_root, bound_roots)
}

/// Append synthetic per-leaf `StepField` rows for nested `Literal` bindings
/// marked `show_in_review` — each gets its own editable row rather than
/// aggregating into the parent field. Only `Literal` sources are eligible;
/// provenance-traced nested bindings stay in the parent's synthesis path.
fn push_nested_literal_review_fields(
    step: &CompiledStep,
    step_local: &BTreeMap<String, serde_json::Value>,
    fields: &mut Vec<StepField>,
    next_id: &mut u32,
) {
    for binding in &step.inputs {
        // Must be a multi-segment, show_in_review, Literal binding.
        if !binding.field.contains('.') && !binding.field.contains('[') {
            continue;
        }
        if !binding.show_in_review {
            continue;
        }
        let BindingSource::Literal(lit) = &binding.source else {
            continue;
        };

        // Derive the label from the last Key segment of the path.
        let label = {
            let Ok(path) = FieldPath::parse(&binding.field) else {
                continue;
            };
            let last_key = path.segments.iter().rev().find_map(|seg| {
                if let PathSegment::Key(name) = seg {
                    Some(name.clone())
                } else {
                    None
                }
            });
            match last_key {
                Some(k) => k,
                None => {
                    // All segments are indices — fall back to the root field name.
                    binding
                        .field
                        .split(['.', '['])
                        .next()
                        .unwrap_or(binding.field.as_str())
                        .to_string()
                }
            }
        };

        // Infer the schema from the literal value's JSON type.
        let schema = match &lit.value {
            serde_json::Value::Bool(_) => serde_json::json!({"type": "boolean"}),
            serde_json::Value::Number(n) if n.is_i64() => serde_json::json!({"type": "integer"}),
            serde_json::Value::Number(_) => serde_json::json!({"type": "number"}),
            serde_json::Value::String(_) => serde_json::json!({"type": "string"}),
            serde_json::Value::Array(_) => serde_json::json!({"type": "array"}),
            serde_json::Value::Object(_) => serde_json::json!({"type": "object"}),
            serde_json::Value::Null => serde_json::json!({"type": "string"}),
        };

        // Prefer the step-local override (form was edited) over the literal.
        let value = step_local
            .get(binding.field.as_str())
            .cloned()
            .unwrap_or_else(|| lit.value.clone());

        fields.push(StepField {
            id: StepFieldId(*next_id),
            field: binding.field.clone(),
            label,
            description: binding.description.clone(),
            location: StepFieldLocation::Body,
            schema,
            value,
            source: ags_protocol::workflow::StepFieldSource::Literal,
            required: false,
            workflow_input: None,
            body_overflow: false,
            show_in_review: true,
        });
        *next_id += 1;
    }
}

/// Look up the description to use for an operation field's `StepField`.
///
/// When the field has a binding to a workflow input (`Reference(Workflow { input })`
/// with no Arithmetic transform — i.e. not Derived), the workflow input's own
/// description takes precedence over the OpenAPI field description. Callers
/// fall back to the OpenAPI description with `.or_else(|| field.description.clone())`
/// when this returns `None`.
fn find_workflow_input_description(
    field_name: &str,
    full_bindings_by_field: &BTreeMap<&str, &ags_protocol::workflow::StepInputBinding>,
    workflow_input_specs: &[WorkflowInputSpec],
) -> Option<String> {
    let binding = full_bindings_by_field.get(field_name)?;
    let BindingSource::Reference(r) = &binding.source else {
        return None;
    };
    let ReferenceTarget::Workflow { input } = &r.from else {
        return None;
    };
    // Arithmetic transforms produce Derived provenance — no single description to use.
    if matches!(r.transform, Some(TransformKind::Arithmetic(_))) {
        return None;
    }
    let spec = workflow_input_specs.iter().find(|s| s.name == *input)?;
    spec.description.clone()
}

/// Classify the provenance of a binding source into a `StepFieldSource`.
///
/// For `Format` bindings, extracts placeholder names from the template and
/// emits `Derived { sources }`. For `Reference(Workflow)` bindings with an
/// `Arithmetic` transform, emits `Derived { sources }` containing the LHS
/// workflow input name and optionally the RHS workflow input name (when the
/// operand is `WorkflowInput`). All other reference bindings follow the
/// existing `WorkflowInput` / `PriorOutput` pattern; `Literal` stays
/// `Literal`.
///
/// `transform` is only consulted for `Reference` sources; it should be `None`
/// for `Format` and `Literal`.
fn classify_binding_source(
    source: &BindingSource,
    transform: Option<&TransformKind>,
) -> ags_protocol::workflow::StepFieldSource {
    use ags_protocol::workflow::StepFieldSource;
    match source {
        BindingSource::Format(format) => {
            // Template was validated at compile time; Err here is unreachable.
            let placeholders =
                crate::runtime::workflows::compile::format_placeholders(&format.template)
                    .unwrap_or_default();
            StepFieldSource::Derived {
                sources: placeholders,
            }
        }
        BindingSource::Reference(reference) => match (&reference.from, transform) {
            (ReferenceTarget::Workflow { input }, Some(TransformKind::Arithmetic(arith))) => {
                let mut sources = vec![input.clone()];
                if let ArithmeticOperand::WorkflowInput(op_name) = &arith.operand {
                    sources.push(op_name.clone());
                }
                StepFieldSource::Derived { sources }
            }
            (ReferenceTarget::Workflow { input }, _) => StepFieldSource::WorkflowInput {
                name: input.clone(),
            },
            (ReferenceTarget::Step { .. }, _) => StepFieldSource::PriorOutput,
        },
        BindingSource::Literal(_) => StepFieldSource::Literal,
        // A mirror shows its target's value; provenance-wise it reads as an
        // authored (literal) value, not a workflow input.
        BindingSource::Mirror(_) => StepFieldSource::Literal,
    }
}

/// Resolve one `BindingSource` to a final value. Applies `transform` at
/// use-time; returns `Err` only for irrecoverable binding errors (step
/// reference points at a missing capture with no default). `bindings` and
/// `step_local` supply the same-step context a `Mirror` source resolves
/// against: the target's review edit wins over its authored binding.
fn resolve_binding(
    source: &BindingSource,
    ctx: &WorkflowContext,
    workflow_supplied: &BTreeMap<String, serde_json::Value>,
    bindings: &BTreeMap<&str, &BindingSource>,
    step_local: &BTreeMap<String, serde_json::Value>,
) -> Result<Option<serde_json::Value>, RuntimeError> {
    match source {
        BindingSource::Mirror(mirror) => {
            if let Some(edited) = step_local.get(&mirror.mirror_of) {
                return Ok(Some(edited.clone()));
            }
            let Some(target) = bindings.get(mirror.mirror_of.as_str()) else {
                return Err(RuntimeError::internal(format!(
                    "mirror target '{}' is not bound in this step",
                    mirror.mirror_of
                )));
            };
            if matches!(target, BindingSource::Mirror(_)) {
                return Err(RuntimeError::internal(format!(
                    "mirror target '{}' is itself a mirror",
                    mirror.mirror_of
                )));
            }
            resolve_binding(target, ctx, workflow_supplied, bindings, step_local)
        }
        BindingSource::Literal(literal) => Ok(Some(literal.value.clone())),
        BindingSource::Reference(reference) => match &reference.from {
            ReferenceTarget::Workflow { input } => {
                let Some(raw) = workflow_supplied.get(input) else {
                    return Ok(None); // gather should have populated this
                };
                Ok(Some(apply_optional_transform(
                    raw,
                    reference.transform.as_ref(),
                    workflow_supplied,
                )))
            }
            ReferenceTarget::Step { id } => {
                let Some(output_name) = reference.output.as_deref() else {
                    return Err(RuntimeError::internal(format!(
                        "binding from step '{id}' is missing `output:`"
                    )));
                };
                match ctx.resolve_step_reference(id, output_name) {
                    Some(raw) => Ok(Some(apply_optional_transform(
                        raw,
                        reference.transform.as_ref(),
                        workflow_supplied,
                    ))),
                    None => Err(RuntimeError::internal(format!(
                        "step output 'step/{id}/{output_name}' was not produced and has no default"
                    ))),
                }
            }
        },
        BindingSource::Format(format) => {
            Ok(
                interpolate_template(&format.template, workflow_supplied, QuoteMode::Plain)
                    .map(serde_json::Value::String),
            )
        }
    }
}

/// Resolve every declared `inputs:` binding of a `kind: local` step into a
/// flat `field → value` map. Reuses the shared [`resolve_binding`] path so
/// the resolution rules (step references, workflow inputs, literals, format
/// templates, mirrors) are identical to the API-step path and cannot drift.
///
/// The returned map is intended to be *merged on top of* `workflow_supplied`
/// before invoking the handler, so a binding's resolved value takes
/// precedence over a same-named workflow flag or default. This matches the
/// API-step convention where explicit bindings override auto-bound workflow
/// inputs.
///
/// Errors propagate as `RuntimeError` — a step-reference pointing at a
/// capture that was never produced (and has no default) fails the step.
pub(crate) fn resolve_local_step_bindings(
    step: &CompiledStep,
    ctx: &WorkflowContext,
    workflow_supplied: &BTreeMap<String, serde_json::Value>,
) -> Result<BTreeMap<String, serde_json::Value>, RuntimeError> {
    if step.inputs.is_empty() {
        return Ok(BTreeMap::new());
    }
    let bindings_by_field: BTreeMap<&str, &BindingSource> = step
        .inputs
        .iter()
        .map(|b| (b.field.as_str(), &b.source))
        .collect();
    // Local steps have no per-step review form, so `step_local` is always empty.
    let step_local: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut resolved = BTreeMap::new();
    for binding in &step.inputs {
        let value = resolve_binding(
            &binding.source,
            ctx,
            workflow_supplied,
            &bindings_by_field,
            &step_local,
        )?;
        if let Some(v) = value {
            resolved.insert(binding.field.clone(), v);
        }
    }
    Ok(resolved)
}

/// Apply an optional transform to a resolved value. Exhaustive over
/// `TransformKind`; unknown variants would be a compile error (we add
/// new variants in dedicated tasks).
fn apply_optional_transform(
    value: &serde_json::Value,
    transform: Option<&TransformKind>,
    workflow_supplied: &BTreeMap<String, serde_json::Value>,
) -> serde_json::Value {
    match transform {
        Some(TransformKind::JsonPath { path }) => {
            apply_jsonpath_subset(value, path).unwrap_or_else(|| value.clone())
        }
        Some(TransformKind::Arithmetic(arith)) => {
            let Some(lhs) = value.as_i64() else {
                return value.clone();
            };
            let rhs = match &arith.operand {
                ArithmeticOperand::Integer(n) => *n,
                ArithmeticOperand::WorkflowInput(name) => {
                    workflow_supplied
                        .get(name)
                        .and_then(|v| v.as_i64())
                        // Missing/non-integer operand defaults to 0. B2 validates
                        // at compile time that the operand input exists and is
                        // integer-typed, so this fallback is defensive — most
                        // commonly hit during partial gather (input not yet supplied).
                        .unwrap_or(0)
                }
            };
            let result = match arith.op {
                ArithmeticOp::Mul => lhs.saturating_mul(rhs),
                ArithmeticOp::Add => lhs.saturating_add(rhs),
            };
            serde_json::Value::Number(result.into())
        }
        None => value.clone(),
    }
}

/// Coerce a JSON value to the string form path/query/header params need.
/// Numbers and booleans become their natural string form; strings pass
/// through; everything else stringifies via `to_string`.
/// How a substituted value is rendered into the surrounding template text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuoteMode {
    /// Substitute the raw stringified value (Format-binding behavior).
    Plain,
    /// POSIX single-quote the value when it contains anything outside
    /// `[A-Za-z0-9._-]`. For values shown inside copy-paste shell commands.
    ShellQuoted,
}

/// Interpolate `{name}` placeholders in `template`, reading values from
/// `values` and stringifying with `json_to_param_string`. `{{` and `}}` are
/// literal braces. Returns `None` if a referenced name is absent (caller treats
/// that as "unresolved") or a `{` is unterminated.
pub(crate) fn interpolate_template(
    template: &str,
    values: &BTreeMap<String, serde_json::Value>,
    mode: QuoteMode,
) -> Option<String> {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.char_indices().peekable();
    while let Some((idx, ch)) = chars.next() {
        match ch {
            '{' if chars.peek().map(|(_, c)| *c) == Some('{') => {
                out.push('{');
                chars.next();
            }
            '}' if chars.peek().map(|(_, c)| *c) == Some('}') => {
                out.push('}');
                chars.next();
            }
            '{' => {
                let name_start = idx + '{'.len_utf8();
                let name_end;
                loop {
                    match chars.next() {
                        Some((i, '}')) => {
                            name_end = i;
                            break;
                        }
                        Some(_) => continue,
                        None => return None,
                    }
                }
                let name = &template[name_start..name_end];
                let value = values.get(name)?;
                let rendered = json_to_param_string(value);
                match mode {
                    QuoteMode::Plain => out.push_str(&rendered),
                    QuoteMode::ShellQuoted => out.push_str(&shell_quote(&rendered)),
                }
            }
            other => out.push(other),
        }
    }
    Some(out)
}

/// POSIX single-quote `s` when it contains a character outside `[A-Za-z0-9._-]`;
/// otherwise return it unchanged. Embedded single quotes become `'\''`.
fn shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if safe {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Interpolate an authored [`WorkflowCompletion`] into a resolved
/// [`WorkflowCompletionView`] using the run's supplied inputs. `created` values
/// are plain; `command`s are shell-quoted. Compile-time validation guarantees
/// every placeholder is a declared input, so a missing value only occurs for an
/// unsupplied optional-without-default input; that field renders empty.
pub fn resolve_completion(
    completion: &Option<ags_protocol::workflow::WorkflowCompletion>,
    supplied: &BTreeMap<String, serde_json::Value>,
) -> Option<ags_protocol::output_views::WorkflowCompletionView> {
    let c = completion.as_ref()?;
    let created = c
        .created
        .iter()
        .map(|r| ags_protocol::workflow::CompletionResource {
            label: r.label.clone(),
            value: interpolate_template(&r.value, supplied, QuoteMode::Plain).unwrap_or_default(),
        })
        .collect();
    let next_steps = c
        .next_steps
        .iter()
        .map(|s| ags_protocol::workflow::CompletionStep {
            description: s.description.clone(),
            command: interpolate_template(&s.command, supplied, QuoteMode::ShellQuoted)
                .unwrap_or_default(),
        })
        .collect();
    Some(ags_protocol::output_views::WorkflowCompletionView {
        created,
        next_steps,
    })
}

/// Coerce a JSON value to the string form path/query/header params need.
pub(crate) fn json_to_param_string(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        // OpenAPI 2.0 array parameters default to `collectionFormat: csv`, so an
        // array serialises as comma-separated values, not a JSON array (which the
        // server cannot parse and drops). Each element is stringified with the
        // same scalar rules; the parser does not capture `collectionFormat`, so
        // csv is applied uniformly (ssv/pipes/multi are not modelled).
        serde_json::Value::Array(items) => items
            .iter()
            .map(json_to_param_string)
            .collect::<Vec<_>>()
            .join(","),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::workflows::RunOptions;
    use ags_protocol::catalogue::{
        ApiVersion, BodyField, BodyFieldType, BodySchema, HttpMethod, MethodSchema, MutationClass,
        OperationId, OperationSchema, ParameterLocation, ParameterSchema, ResourceSchema,
        ScopeEntry, ServiceId, ValueType,
    };
    use ags_protocol::request::FormPart;
    use ags_protocol::workflow::{
        AutoDeriveScope, AutoDerivedField, BindingSource, CompiledStep, LiteralBinding,
        OperationReference, ReferenceBinding, ReferenceTarget, StepInputBinding, TransformKind,
        WorkflowInputSpec,
    };

    #[test]
    fn test_json_to_param_string_array_joins_as_csv() {
        // OpenAPI 2.0 array parameters default to `collectionFormat: csv`, so
        // an array value (e.g. a `sortBy` default) must serialise as
        // comma-separated values, not a JSON array the server would drop.
        let value = serde_json::json!(["name:asc", "displayOrder:asc"]);
        assert_eq!(json_to_param_string(&value), "name:asc,displayOrder:asc");
    }

    #[test]
    fn test_json_to_param_string_numeric_array_joins_as_csv() {
        assert_eq!(json_to_param_string(&serde_json::json!([1, 2, 3])), "1,2,3");
    }

    #[test]
    fn test_json_to_param_string_empty_array_is_empty() {
        assert_eq!(json_to_param_string(&serde_json::json!([])), "");
    }

    #[test]
    fn test_json_to_param_string_scalars_unchanged() {
        assert_eq!(
            json_to_param_string(&serde_json::json!("name:asc")),
            "name:asc"
        );
        assert_eq!(json_to_param_string(&serde_json::json!(42)), "42");
        assert_eq!(json_to_param_string(&serde_json::json!(true)), "true");
    }

    #[test]
    fn test_interpolate_template_plain_substitutes_and_keeps_literals() {
        let mut values = BTreeMap::new();
        values.insert("resourcePrefix".to_string(), serde_json::json!("ranked"));
        let out = interpolate_template("{resourcePrefix}-pool", &values, QuoteMode::Plain);
        assert_eq!(out.as_deref(), Some("ranked-pool"));
    }

    #[test]
    fn test_interpolate_template_escapes_double_braces() {
        let values = BTreeMap::new();
        let out = interpolate_template("{{literal}}", &values, QuoteMode::Plain);
        assert_eq!(out.as_deref(), Some("{literal}"));
    }

    #[test]
    fn test_interpolate_template_missing_value_is_none() {
        let values = BTreeMap::new();
        assert_eq!(
            interpolate_template("{absent}", &values, QuoteMode::Plain),
            None
        );
    }

    #[test]
    fn test_resolve_completion_interpolates_created_and_quotes_commands() {
        use ags_protocol::workflow::{CompletionResource, CompletionStep, WorkflowCompletion};
        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("dev"));
        supplied.insert("resourcePrefix".to_string(), serde_json::json!("ranked"));
        let completion = Some(WorkflowCompletion {
            created: vec![CompletionResource {
                label: "Match pool".into(),
                value: "{resourcePrefix}-pool".into(),
            }],
            next_steps: vec![CompletionStep {
                description: "Inspect".into(),
                command: "ags x --namespace {namespace} --pool {resourcePrefix}-pool".into(),
            }],
        });
        let view = resolve_completion(&completion, &supplied).unwrap();
        assert_eq!(view.created[0].value, "ranked-pool");
        assert_eq!(
            view.next_steps[0].command,
            "ags x --namespace dev --pool ranked-pool"
        );
    }

    #[test]
    fn test_resolve_completion_shell_quotes_unsafe_command_values() {
        use ags_protocol::workflow::{CompletionStep, WorkflowCompletion};
        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("my ns"));
        let completion = Some(WorkflowCompletion {
            created: vec![],
            next_steps: vec![CompletionStep {
                description: "Inspect".into(),
                command: "ags x --namespace {namespace}".into(),
            }],
        });
        let view = resolve_completion(&completion, &supplied).unwrap();
        assert_eq!(view.next_steps[0].command, "ags x --namespace 'my ns'");
    }

    #[test]
    fn test_resolve_completion_none_when_absent() {
        let supplied = BTreeMap::new();
        assert!(resolve_completion(&None, &supplied).is_none());
    }

    /// Build an `OperationSchema` fixture with a `{namespace}` path param.
    fn op_with_namespace_path() -> OperationSchema {
        OperationSchema {
            id: OperationId::new("CreateStat"),
            name: "create".into(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::Mutating,
            http_method: HttpMethod::Post,
            path_template: "/svc/v1/admin/namespaces/{namespace}/stats".into(),
            parameters: vec![ParameterSchema {
                name: "namespace".into(),
                location: ParameterLocation::Path,
                required: true,
                value_type: ValueType::String,
                is_file: false,
                description: None,
                default: None,
            }],
            request_body: Some(BodySchema {
                item_type: None,
                is_array: false,
                definition_name: "Body".into(),
                fields: vec![BodyField {
                    name: "statCode".into(),
                    field_type: BodyFieldType::String,
                    required: true,
                    description: None,
                    children: vec![],
                    default: None,
                }],
            }),
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// Build a `ServiceSchema` fixture wrapping the given operation.
    fn service_with(op: OperationSchema) -> ServiceSchema {
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

    /// Build a `CompiledStep` fixture with the given auto-derived fields.
    fn compiled_step_with_auto(auto: Vec<AutoDerivedField>) -> CompiledStep {
        CompiledStep {
            id: "s1".into(),
            index: 0,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new("svc"),
                operation: OperationId::new("CreateStat"),
            }),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![],
            auto_derived: auto,
        }
    }

    /// Schema with a `namespace` path param and a `statCode` body field, paired
    /// with a step that binds `statCode` to the literal `"mmr"`.
    fn sample_schema_and_step() -> (ServiceSchema, CompiledStep) {
        let schema = service_with(op_with_namespace_path());
        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "statCode".into(),
            source: BindingSource::Literal(LiteralBinding {
                value: serde_json::json!("mmr"),
                sensitive: false,
            }),
            show_in_review: false,
            description: None,
        });
        (schema, step)
    }

    /// Schema where `name` appears as both a path param and a body field.
    fn schema_with_name_in_path_and_body(name: &str) -> (ServiceSchema, CompiledStep) {
        let mut op = op_with_namespace_path();
        op.path_template = format!("/svc/v1/{{{name}}}");
        op.parameters = vec![ParameterSchema {
            name: name.into(),
            location: ParameterLocation::Path,
            required: true,
            value_type: ValueType::String,
            is_file: false,
            description: None,
            default: None,
        }];
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![BodyField {
                name: name.into(),
                field_type: BodyFieldType::String,
                required: false,
                description: None,
                children: vec![],
                default: None,
            }],
        });
        (service_with(op), compiled_step_with_auto(vec![]))
    }

    /// Build a sample list of workflow input specs.
    fn sample_input_specs() -> Vec<WorkflowInputSpec> {
        vec![WorkflowInputSpec {
            name: "namespace".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        }]
    }

    #[test]
    fn test_resolve_step_fields_lists_every_field_with_source() {
        let (schema, step) = sample_schema_and_step();
        let ctx = WorkflowContext::default();
        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("dev"));
        let step_local = BTreeMap::new();
        let specs = sample_input_specs();
        let defaults = std::collections::BTreeSet::new();

        let plan = resolve_step_fields(
            &step,
            &ctx,
            &supplied,
            &step_local,
            &specs,
            &schema,
            &defaults,
        )
        .unwrap();

        let by_field: std::collections::BTreeMap<_, _> =
            plan.fields.iter().map(|f| (f.field.as_str(), f)).collect();
        assert!(by_field.contains_key("namespace"));
        assert!(by_field.contains_key("statCode")); // literal-bound field is shown
        assert_eq!(by_field["statCode"].value, serde_json::json!("mmr"));
        assert!(matches!(
            by_field["statCode"].source,
            ags_protocol::workflow::StepFieldSource::Literal
        ));
        assert_eq!(
            by_field["namespace"].workflow_input.as_deref(),
            Some("namespace")
        );
    }

    #[test]
    fn test_resolve_step_fields_dedupes_same_name_across_locations() {
        let (schema, step) = schema_with_name_in_path_and_body("ns");
        let ctx = WorkflowContext::default();
        let supplied = BTreeMap::new();
        let step_local = BTreeMap::new();
        let specs = vec![];
        let defaults = std::collections::BTreeSet::new();
        let plan = resolve_step_fields(
            &step,
            &ctx,
            &supplied,
            &step_local,
            &specs,
            &schema,
            &defaults,
        )
        .unwrap();
        assert_eq!(plan.fields.iter().filter(|f| f.field == "ns").count(), 1);
    }

    #[test]
    fn test_resolve_step_fields_collapses_unbound_optional_body_into_more_body() {
        // Body has a bound field (statCode) and an unbound optional field
        // (isPublic). The bound one shows individually; the unbound one folds
        // into the single "more body" overflow field.
        let mut op = op_with_namespace_path();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![
                BodyField {
                    name: "statCode".into(),
                    field_type: BodyFieldType::String,
                    required: false,
                    description: None,
                    children: vec![],
                    default: None,
                },
                BodyField {
                    name: "isPublic".into(),
                    field_type: BodyFieldType::Boolean,
                    required: false,
                    description: None,
                    children: vec![],
                    default: None,
                },
            ],
        });
        let schema = service_with(op);
        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "statCode".into(),
            source: BindingSource::Literal(LiteralBinding {
                value: serde_json::json!("mmr"),
                sensitive: false,
            }),
            show_in_review: false,
            description: None,
        });
        let ctx = WorkflowContext::default();
        let plan = resolve_step_fields(
            &step,
            &ctx,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &[],
            &schema,
            &std::collections::BTreeSet::new(),
        )
        .unwrap();

        assert!(
            plan.fields
                .iter()
                .any(|f| f.field == "statCode" && !f.body_overflow),
            "bound body field shows individually"
        );
        assert!(
            !plan.fields.iter().any(|f| f.field == "isPublic"),
            "unbound optional body field is not an individual row"
        );
        let overflow = plan
            .fields
            .iter()
            .find(|f| f.body_overflow)
            .expect("a more-body overflow field is present");
        assert_eq!(overflow.label, "Additional Parameters");
        assert!(
            overflow
                .description
                .as_deref()
                .unwrap_or_default()
                .contains("isPublic"),
            "overflow hint lists the unbound optional field"
        );
    }

    #[test]
    fn test_compute_needed_when_workflow_input_supplied_no_entry() {
        let step = compiled_step_with_auto(vec![AutoDerivedField {
            field: "namespace".into(),
            schema: serde_json::json!({"type": "string"}),
            required: true,
            sensitive: false,
            description: None,
            scope: AutoDeriveScope::WorkflowInput {
                name: "namespace".into(),
            },
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }]);
        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".into(), serde_json::json!("dev"));
        let needed = compute_needed_inputs(&step, &supplied, &[]);
        assert!(needed.is_empty());
    }

    #[test]
    fn test_compute_needed_when_workflow_input_unsupplied_emits_entry() {
        let step = compiled_step_with_auto(vec![AutoDerivedField {
            field: "namespace".into(),
            schema: serde_json::json!({"type": "string"}),
            required: true,
            sensitive: false,
            description: None,
            scope: AutoDeriveScope::WorkflowInput {
                name: "namespace".into(),
            },
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }]);
        let needed = compute_needed_inputs(&step, &BTreeMap::new(), &[]);
        assert_eq!(needed.len(), 1);
        assert_eq!(needed[0].label, "namespace");
        assert!(matches!(
            needed[0].scope,
            AutoDeriveScope::WorkflowInput { ref name } if name == "namespace"
        ));
    }

    #[test]
    fn test_compute_needed_step_local_uses_step_prefixed_label() {
        let step = compiled_step_with_auto(vec![AutoDerivedField {
            field: "statCode".into(),
            schema: serde_json::json!({"type": "string"}),
            required: true,
            sensitive: false,
            description: None,
            scope: AutoDeriveScope::StepLocal {
                field_name: "statCode".into(),
            },
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }]);
        let needed = compute_needed_inputs(&step, &BTreeMap::new(), &[]);
        assert_eq!(needed.len(), 1);
        // The field portion is kebab-cased so it matches its CLI flag form.
        assert_eq!(needed[0].label, "s1.stat-code");
    }

    #[test]
    fn test_compute_needed_workflow_input_label_is_kebab_case() {
        let step = compiled_step_with_auto(vec![AutoDerivedField {
            field: "roleId".into(),
            schema: serde_json::json!({"type": "string"}),
            required: true,
            sensitive: false,
            description: None,
            scope: AutoDeriveScope::WorkflowInput {
                name: "roleId".into(),
            },
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }]);
        let needed = compute_needed_inputs(&step, &BTreeMap::new(), &[]);
        assert_eq!(needed.len(), 1);
        // Label matches the `--role-id` CLI flag, not the raw `roleId`.
        assert_eq!(needed[0].label, "role-id");
    }

    #[test]
    fn test_compute_needed_dedups_workflow_input_referenced_twice() {
        let mut step = compiled_step_with_auto(vec![AutoDerivedField {
            field: "namespace".into(),
            schema: serde_json::json!({"type": "string"}),
            required: true,
            sensitive: false,
            description: None,
            scope: AutoDeriveScope::WorkflowInput {
                name: "namespace".into(),
            },
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }]);
        step.inputs.push(StepInputBinding {
            field: "extra".into(),
            source: BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: "namespace".into(),
                },
                output: None,
                transform: None,
            }),
            show_in_review: false,
            description: None,
        });
        let needed = compute_needed_inputs(&step, &BTreeMap::new(), &[]);
        assert_eq!(needed.len(), 1);
    }

    /// An Arithmetic operand `workflow/X` is gathered even when X binds no field
    /// directly — otherwise the transform silently multiplies by the `unwrap_or(0)`
    /// fallback and dispatches a wrong value.
    #[test]
    fn test_compute_needed_emits_unsupplied_arithmetic_operand() {
        use ags_protocol::workflow::{ArithmeticOp, ArithmeticOperand, ArithmeticTransform};

        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "maxPlayers".into(),
            source: BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: "playersPerTeam".into(),
                },
                output: None,
                transform: Some(TransformKind::Arithmetic(ArithmeticTransform {
                    op: ArithmeticOp::Mul,
                    operand: ArithmeticOperand::WorkflowInput("teamCount".into()),
                })),
            }),
            show_in_review: false,
            description: None,
        });
        // The LHS is supplied, so only the operand remains to gather.
        let mut supplied = BTreeMap::new();
        supplied.insert("playersPerTeam".into(), serde_json::json!(4));
        let needed = compute_needed_inputs(&step, &supplied, &[]);
        assert_eq!(needed.len(), 1);
        assert_eq!(needed[0].label, "team-count");
    }

    #[test]
    fn test_compute_needed_skips_supplied_arithmetic_operand() {
        use ags_protocol::workflow::{ArithmeticOp, ArithmeticOperand, ArithmeticTransform};

        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "maxPlayers".into(),
            source: BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: "playersPerTeam".into(),
                },
                output: None,
                transform: Some(TransformKind::Arithmetic(ArithmeticTransform {
                    op: ArithmeticOp::Mul,
                    operand: ArithmeticOperand::WorkflowInput("teamCount".into()),
                })),
            }),
            show_in_review: false,
            description: None,
        });
        let mut supplied = BTreeMap::new();
        supplied.insert("playersPerTeam".into(), serde_json::json!(4));
        supplied.insert("teamCount".into(), serde_json::json!(2));
        let needed = compute_needed_inputs(&step, &supplied, &[]);
        assert!(needed.is_empty());
    }

    /// A Format-template placeholder `{X}` is gathered when X is unsupplied —
    /// otherwise the template can never resolve and a required field is left
    /// unfilled at assembly with no prompt.
    #[test]
    fn test_compute_needed_emits_unsupplied_format_placeholder() {
        use ags_protocol::workflow::FormatBinding;

        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "name".into(),
            source: BindingSource::Format(FormatBinding {
                template: "{resourcePrefix}-fleet".into(),
            }),
            show_in_review: false,
            description: None,
        });
        let needed = compute_needed_inputs(&step, &BTreeMap::new(), &[]);
        assert_eq!(needed.len(), 1);
        assert_eq!(needed[0].label, "resource-prefix");
    }

    #[test]
    fn test_compute_needed_skips_supplied_format_placeholder() {
        use ags_protocol::workflow::FormatBinding;

        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "name".into(),
            source: BindingSource::Format(FormatBinding {
                template: "{resourcePrefix}-fleet".into(),
            }),
            show_in_review: false,
            description: None,
        });
        let mut supplied = BTreeMap::new();
        supplied.insert("resourcePrefix".into(), serde_json::json!("ranked"));
        let needed = compute_needed_inputs(&step, &supplied, &[]);
        assert!(needed.is_empty());
    }

    #[test]
    fn test_assemble_command_request_populates_path_and_body() {
        let schema = service_with(op_with_namespace_path());
        let step = compiled_step_with_auto(vec![]);
        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".into(), serde_json::json!("dev"));
        let mut local = BTreeMap::new();
        local.insert("statCode".into(), serde_json::json!("mmr"));
        let inputs = vec![WorkflowInputSpec {
            name: "namespace".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        }];
        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &supplied,
            &local,
            &inputs,
            &schema,
            Some("dev".into()),
            &RunOptions::default(),
        )
        .unwrap();
        assert_eq!(req.path_params.get("namespace"), Some(&"dev".to_string()));
        assert_eq!(
            match &req.body {
                Some(RequestBody::Json(v)) => v.get("statCode"),
                _ => None,
            },
            Some(&serde_json::json!("mmr"))
        );
    }

    /// Array-bodied operations should assemble their body as a JSON array.
    #[test]
    fn test_assemble_command_request_array_body_wraps_in_array() {
        let mut op = op_with_namespace_path();
        if let Some(body) = op.request_body.as_mut() {
            body.is_array = true;
        }
        let schema = service_with(op);
        let step = compiled_step_with_auto(vec![]);
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("dev"));
        local.insert("statCode".into(), serde_json::json!("mmr"));
        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            Some("dev".into()),
            &RunOptions::default(),
        )
        .unwrap();
        let body = match req.body.expect("array body must not be dropped") {
            RequestBody::Json(v) => v,
            RequestBody::Multipart(_) => panic!("expected a JSON body"),
        };
        let array = body.as_array().expect("body must be a JSON array");
        assert_eq!(array.len(), 1);
        assert_eq!(array[0].get("statCode"), Some(&serde_json::json!("mmr")));
    }

    /// A step-local edit (from a per-step review form's per-leaf edit) must win
    /// over a top-level `const:` binding, same as `resolve_field_value`'s
    /// precedence and pass 2 of `assemble_body`'s nested-path precedence
    /// (`test_assemble_mirror_binding_prefers_step_local_edit_of_target`).
    /// `step_local` is only ever populated for fields the review form actually
    /// showed (i.e. `show_in_review: true`), so that's what's asserted here.
    #[test]
    fn test_assemble_command_request_step_local_edit_overrides_const_binding() {
        let schema = service_with(op_with_namespace_path());
        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "namespace".into(),
            source: BindingSource::Literal(LiteralBinding {
                value: serde_json::json!("override"),
                sensitive: false,
            }),
            show_in_review: true,
            description: None,
        });
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("edited"));
        local.insert("statCode".into(), serde_json::json!("mmr"));
        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            None,
            &RunOptions::default(),
        )
        .unwrap();
        assert_eq!(req.path_params.get("namespace"), Some(&"edited".into()));
    }

    /// A top-level (single-segment) *body* field bound via `const:` and marked
    /// `show_in_review: true` must let a step-review edit win, exactly like
    /// nested-path bindings already do (see
    /// `test_assemble_mirror_binding_prefers_step_local_edit_of_target` below).
    /// Regression test for a reported bug where `resolve_field_value` resolved
    /// straight from the binding whenever one existed and never consulted
    /// `step_local`.
    #[test]
    fn test_assemble_command_request_step_local_edit_overrides_top_level_const_binding() {
        let schema = service_with(op_with_namespace_path());
        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "statCode".into(),
            source: BindingSource::Literal(LiteralBinding {
                value: serde_json::json!("mmr"),
                sensitive: false,
            }),
            show_in_review: true,
            description: None,
        });
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("dev"));
        // The user edited the reviewed `statCode` field at the step review.
        local.insert("statCode".into(), serde_json::json!("edited-code"));
        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            None,
            &RunOptions::default(),
        )
        .unwrap();
        let body = match req.body.expect("body assembled") {
            RequestBody::Json(v) => v,
            RequestBody::Multipart(_) => panic!("expected a JSON body"),
        };
        assert_eq!(body["statCode"], serde_json::json!("edited-code"));
    }

    /// Operation shaped like the platform item-create: a `name` body field
    /// plus a free-form `localizations` object.
    fn op_with_name_and_localizations() -> OperationSchema {
        let mut op = op_with_namespace_path();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![
                BodyField {
                    name: "name".into(),
                    field_type: BodyFieldType::String,
                    required: true,
                    description: None,
                    children: vec![],
                    default: None,
                },
                BodyField {
                    name: "localizations".into(),
                    field_type: BodyFieldType::Object,
                    required: true,
                    description: None,
                    children: vec![],
                    default: None,
                },
            ],
        });
        op
    }

    /// Step binding `name` as a mirror of the nested localized title, with the
    /// title itself a literal — the in-game-store item shape.
    fn step_with_name_mirroring_title() -> CompiledStep {
        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "name".into(),
            source: BindingSource::Mirror(ags_protocol::workflow::MirrorBinding {
                mirror_of: "localizations.en-US.title".into(),
            }),
            show_in_review: false,
            description: None,
        });
        step.inputs.push(StepInputBinding {
            field: "localizations".into(),
            source: BindingSource::Literal(LiteralBinding {
                value: serde_json::json!({}),
                sensitive: false,
            }),
            show_in_review: false,
            description: None,
        });
        step.inputs.push(StepInputBinding {
            field: "localizations.en-US.title".into(),
            source: BindingSource::Literal(LiteralBinding {
                value: serde_json::json!("Starter Skin"),
                sensitive: false,
            }),
            show_in_review: true,
            description: None,
        });
        step
    }

    #[test]
    fn test_assemble_mirror_binding_copies_target_binding_value() {
        let schema = service_with(op_with_name_and_localizations());
        let step = step_with_name_mirroring_title();
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("dev"));
        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            None,
            &RunOptions::default(),
        )
        .unwrap();
        let body = match req.body.expect("body assembled") {
            RequestBody::Json(v) => v,
            RequestBody::Multipart(_) => panic!("expected a JSON body"),
        };
        assert_eq!(body["name"], serde_json::json!("Starter Skin"));
        assert_eq!(
            body["localizations"]["en-US"]["title"],
            serde_json::json!("Starter Skin")
        );
    }

    #[test]
    fn test_assemble_mirror_binding_prefers_step_local_edit_of_target() {
        let schema = service_with(op_with_name_and_localizations());
        let step = step_with_name_mirroring_title();
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("dev"));
        // The user typed a new title at the step review: the edit keys the
        // exact nested path, and the mirrored `name` must follow it.
        local.insert(
            "localizations.en-US.title".into(),
            serde_json::json!("Cool Sword"),
        );
        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            None,
            &RunOptions::default(),
        )
        .unwrap();
        let body = match req.body.expect("body assembled") {
            RequestBody::Json(v) => v,
            RequestBody::Multipart(_) => panic!("expected a JSON body"),
        };
        assert_eq!(body["name"], serde_json::json!("Cool Sword"));
        assert_eq!(
            body["localizations"]["en-US"]["title"],
            serde_json::json!("Cool Sword")
        );
    }

    #[test]
    fn test_assemble_command_request_missing_required_path_param_errors() {
        let schema = service_with(op_with_namespace_path());
        let step = compiled_step_with_auto(vec![]);
        let mut local = BTreeMap::new();
        local.insert("statCode".into(), serde_json::json!("mmr"));
        let err = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            None,
            &RunOptions::default(),
        );
        assert!(err.is_err());
    }

    #[test]
    fn test_assemble_command_request_transform_applies_jsonpath() {
        let schema = service_with(op_with_namespace_path());
        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "namespace".into(),
            source: BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: "env".into(),
                },
                output: None,
                transform: Some(TransformKind::JsonPath {
                    path: "$.namespace".into(),
                }),
            }),
            show_in_review: false,
            description: None,
        });
        let mut local = BTreeMap::new();
        local.insert("statCode".into(), serde_json::json!("mmr"));
        let mut supplied = BTreeMap::new();
        supplied.insert("env".into(), serde_json::json!({"namespace": "extracted"}));
        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &supplied,
            &local,
            &[],
            &schema,
            None,
            &RunOptions::default(),
        )
        .unwrap();
        assert_eq!(req.path_params.get("namespace"), Some(&"extracted".into()));
    }

    #[test]
    fn test_assemble_command_request_threads_output_verbosity_pagination() {
        let schema = service_with(op_with_namespace_path());
        let step = compiled_step_with_auto(vec![]);
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("dev"));
        local.insert("statCode".into(), serde_json::json!("mmr"));
        let options = RunOptions {
            output: Some(ags_protocol::request::OutputDestination::File(
                std::path::PathBuf::from("/tmp/out.json"),
            )),
            verbosity: ags_protocol::request::Verbosity::Verbose,
            pagination: ags_protocol::request::PaginationHint::All,
            ..RunOptions::default()
        };
        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            None,
            &options,
        )
        .unwrap();
        assert_eq!(
            req.output,
            Some(ags_protocol::request::OutputDestination::File(
                std::path::PathBuf::from("/tmp/out.json")
            ))
        );
        assert_eq!(req.verbosity, ags_protocol::request::Verbosity::Verbose);
        assert_eq!(req.pagination, ags_protocol::request::PaginationHint::All);
    }

    #[test]
    fn test_assemble_command_request_missing_required_param_is_validation_kind() {
        let schema = service_with(op_with_namespace_path());
        let step = compiled_step_with_auto(vec![]);
        let mut local = BTreeMap::new();
        local.insert("statCode".into(), serde_json::json!("mmr"));
        let err = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            None,
            &RunOptions::default(),
        )
        .unwrap_err();
        assert_eq!(err.kind, RuntimeErrorKind::Validation);
    }

    /// A `formData` text parameter that resolves to a value assembles into a
    /// `RequestBody::Multipart` with one `FormPart::Text` (`resolve.rs` used
    /// to `unreachable!()`/reject on `ParameterLocation::FormData`).
    #[test]
    fn test_assemble_command_request_formdata_text_param_becomes_multipart_text() {
        let mut op = op_with_namespace_path();
        op.parameters.push(ParameterSchema {
            name: "certificate".into(),
            location: ParameterLocation::FormData,
            required: true,
            value_type: ValueType::String,
            is_file: false,
            description: None,
            default: None,
        });
        let schema = service_with(op);
        let step = compiled_step_with_auto(vec![]);
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("dev"));
        local.insert("statCode".into(), serde_json::json!("mmr"));
        local.insert("certificate".into(), serde_json::json!("cert-bytes"));
        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            Some("dev".into()),
            &RunOptions::default(),
        )
        .unwrap();

        match req.body {
            Some(RequestBody::Multipart(parts)) => {
                assert_eq!(parts.len(), 1);
                assert!(matches!(
                    &parts[0],
                    FormPart::Text { name, value }
                        if name == "certificate" && value == "cert-bytes"
                ));
            }
            other => panic!("expected Multipart body, got {other:?}"),
        }
    }

    /// Build an `OperationSchema` with one required file-typed `formData`
    /// parameter named `file`, no body — mirrors a real single-file upload
    /// operation like `csm app-ui upload-assets`. Uses the `CreateStat`
    /// operation id (not the semantically-fitting `UploadAssets`) so it
    /// matches the id `compiled_step_with_auto`'s fixture step hardcodes;
    /// `find_operation_or_error` matches by id, not by name/path.
    fn op_with_file_formdata_param() -> OperationSchema {
        OperationSchema {
            id: OperationId::new("CreateStat"),
            name: "upload-assets".into(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::Mutating,
            http_method: HttpMethod::Post,
            path_template: "/svc/v1/admin/namespaces/{namespace}/upload".into(),
            parameters: vec![
                ParameterSchema {
                    name: "namespace".into(),
                    location: ParameterLocation::Path,
                    required: true,
                    value_type: ValueType::String,
                    description: None,
                    default: None,
                    is_file: false,
                },
                ParameterSchema {
                    name: "file".into(),
                    location: ParameterLocation::FormData,
                    required: true,
                    value_type: ValueType::String,
                    description: None,
                    default: None,
                    is_file: true,
                },
            ],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// A single required file-typed formData parameter with a valid,
    /// readable local path assembles into a `RequestBody::Multipart`
    /// containing one `FormPart::File`.
    #[test]
    fn test_assemble_command_request_single_file_formdata_becomes_multipart() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("asset.png");
        std::fs::write(&file_path, b"fake-bytes").unwrap();

        let schema = service_with(op_with_file_formdata_param());
        let step = compiled_step_with_auto(vec![]);
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("dev"));
        local.insert(
            "file".into(),
            serde_json::json!(file_path.to_string_lossy().into_owned()),
        );

        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            Some("dev".into()),
            &RunOptions::default(),
        )
        .unwrap();

        match req.body {
            Some(RequestBody::Multipart(parts)) => {
                assert_eq!(parts.len(), 1);
                match &parts[0] {
                    FormPart::File {
                        name,
                        path,
                        filename,
                    } => {
                        assert_eq!(name, "file");
                        assert_eq!(path, &file_path);
                        assert_eq!(filename, "asset.png");
                    }
                    other => panic!("expected File part, got {other:?}"),
                }
            }
            other => panic!("expected Multipart body, got {other:?}"),
        }
    }

    /// A file param plus a non-file (text) formData sidecar field both land
    /// in the same `Multipart` body, in the operation's declared order.
    #[test]
    fn test_assemble_command_request_mixed_file_and_text_formdata() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("cert.pem");
        std::fs::write(&file_path, b"cert-bytes").unwrap();

        let mut op = op_with_file_formdata_param();
        op.parameters.push(ParameterSchema {
            name: "password".into(),
            location: ParameterLocation::FormData,
            required: false,
            value_type: ValueType::String,
            description: None,
            default: None,
            is_file: false,
        });
        let schema = service_with(op);
        let step = compiled_step_with_auto(vec![]);
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("dev"));
        local.insert(
            "file".into(),
            serde_json::json!(file_path.to_string_lossy().into_owned()),
        );
        local.insert("password".into(), serde_json::json!("secret123"));

        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            Some("dev".into()),
            &RunOptions::default(),
        )
        .unwrap();

        match req.body {
            Some(RequestBody::Multipart(parts)) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(&parts[0], FormPart::File { name, .. } if name == "file"));
                assert!(
                    matches!(&parts[1], FormPart::Text { name, value } if name == "password" && value == "secret123")
                );
            }
            other => panic!("expected Multipart body, got {other:?}"),
        }
    }

    /// An optional formData text field with no supplied value is simply
    /// omitted from the assembled parts — not an error.
    #[test]
    fn test_assemble_command_request_optional_formdata_field_omitted_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("cert.pem");
        std::fs::write(&file_path, b"cert-bytes").unwrap();

        let mut op = op_with_file_formdata_param();
        op.parameters.push(ParameterSchema {
            name: "password".into(),
            location: ParameterLocation::FormData,
            required: false,
            value_type: ValueType::String,
            description: None,
            default: None,
            is_file: false,
        });
        let schema = service_with(op);
        let step = compiled_step_with_auto(vec![]);
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("dev"));
        local.insert(
            "file".into(),
            serde_json::json!(file_path.to_string_lossy().into_owned()),
        );
        // `password` deliberately not supplied.

        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            Some("dev".into()),
            &RunOptions::default(),
        )
        .unwrap();

        match req.body {
            Some(RequestBody::Multipart(parts)) => {
                assert_eq!(parts.len(), 1);
            }
            other => panic!("expected Multipart body with only the file part, got {other:?}"),
        }
    }

    /// A required file formData param with no supplied value is a clean
    /// `Validation` "missing required parameter" error, not a panic.
    #[test]
    fn test_assemble_command_request_missing_required_file_formdata_errors() {
        let schema = service_with(op_with_file_formdata_param());
        let step = compiled_step_with_auto(vec![]);
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("dev"));
        // `file` deliberately not supplied.

        let err = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            Some("dev".into()),
            &RunOptions::default(),
        )
        .unwrap_err();
        assert_eq!(err.kind, RuntimeErrorKind::Validation);
        // Asserts the specific message `resolve_form_data_params` produces
        // ("missing required form field '...'"), not the generic
        // "missing required parameter" message `resolve_non_body_params`
        // would raise for a non-formData param — this genuinely exercises
        // (and would fail if we regressed away from) the formData-specific
        // required check, since `resolve_non_body_params` now skips
        // `formData` params entirely rather than double-checking them.
        assert!(
            err.message.contains("form field") && err.message.contains("file"),
            "expected a formData-specific 'missing required form field' message, got: {}",
            err.message
        );
    }

    /// A supplied file path that does not exist on disk is a clean
    /// `Validation` error naming the path, not an I/O panic.
    #[test]
    fn test_assemble_command_request_nonexistent_file_path_errors_cleanly() {
        let schema = service_with(op_with_file_formdata_param());
        let step = compiled_step_with_auto(vec![]);
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("dev"));
        local.insert(
            "file".into(),
            serde_json::json!("/definitely/does/not/exist.png"),
        );

        let err = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            Some("dev".into()),
            &RunOptions::default(),
        )
        .unwrap_err();
        assert_eq!(err.kind, RuntimeErrorKind::Validation);
        assert!(err.message.contains("/definitely/does/not/exist.png"));
    }

    /// `resolve_step_fields` emits a `StepField` for a `formData` parameter
    /// (previously silently skipped), located as `FormData`.
    #[test]
    fn test_resolve_step_fields_includes_formdata_param() {
        let schema = service_with(op_with_file_formdata_param());
        let step = compiled_step_with_auto(vec![]);

        let plan = resolve_step_fields(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &[],
            &schema,
            &std::collections::BTreeSet::new(),
        )
        .unwrap();

        let file_field = plan.fields.iter().find(|f| f.field == "file").unwrap();
        assert_eq!(file_field.location, StepFieldLocation::FormData);
    }

    #[test]
    fn test_assemble_command_request_explicit_body_used_verbatim() {
        let schema = service_with(op_with_namespace_path());
        let step = compiled_step_with_auto(vec![]);
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("dev"));
        // `statCode` is a required body field, but the explicit body omits
        // it — this must NOT error, and the body must be used verbatim.
        let options = RunOptions {
            explicit_body: Some(serde_json::json!({"other": "value"})),
            ..RunOptions::default()
        };
        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            None,
            &options,
        )
        .unwrap();
        assert_eq!(
            req.body,
            Some(RequestBody::Json(serde_json::json!({"other": "value"})))
        );
    }

    #[test]
    fn test_resolve_field_with_source_literal() {
        use ags_protocol::workflow::{LiteralBinding, StepFieldSource};
        let ctx = WorkflowContext::default();
        let supplied = BTreeMap::new();
        let step_local = BTreeMap::new();
        let names = std::collections::BTreeSet::new();
        let defaults = std::collections::BTreeSet::new();
        let lit = BindingSource::Literal(LiteralBinding {
            value: serde_json::json!("mmr"),
            sensitive: false,
        });
        let mut bindings: BTreeMap<&str, &BindingSource> = BTreeMap::new();
        bindings.insert("statCode", &lit);
        let (value, source, wf) = resolve_field_with_source(
            "statCode",
            &bindings,
            &[],
            &ctx,
            &supplied,
            &step_local,
            &names,
            &defaults,
        )
        .unwrap();
        assert_eq!(value, Some(serde_json::json!("mmr")));
        assert!(matches!(source, StepFieldSource::Literal));
        assert_eq!(wf, None);
    }

    #[test]
    fn test_resolve_field_with_source_workflow_input_flag_vs_default() {
        use ags_protocol::workflow::StepFieldSource;
        let ctx = WorkflowContext::default();
        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("dev"));
        supplied.insert("fleetName".to_string(), serde_json::json!("ranked-fleet"));
        let step_local = BTreeMap::new();
        let names: std::collections::BTreeSet<&str> =
            ["namespace", "fleetName"].into_iter().collect();
        let defaults: std::collections::BTreeSet<String> =
            ["fleetName".to_string()].into_iter().collect();
        let bindings: BTreeMap<&str, &BindingSource> = BTreeMap::new();

        let (v1, s1, w1) = resolve_field_with_source(
            "namespace",
            &bindings,
            &[],
            &ctx,
            &supplied,
            &step_local,
            &names,
            &defaults,
        )
        .unwrap();
        assert_eq!(v1, Some(serde_json::json!("dev")));
        assert!(matches!(s1, StepFieldSource::WorkflowInput { ref name } if name == "namespace"));
        assert_eq!(w1.as_deref(), Some("namespace"));

        let (_v2, s2, w2) = resolve_field_with_source(
            "fleetName",
            &bindings,
            &[],
            &ctx,
            &supplied,
            &step_local,
            &names,
            &defaults,
        )
        .unwrap();
        assert!(matches!(s2, StepFieldSource::Default { ref name } if name == "fleetName"));
        assert_eq!(w2.as_deref(), Some("fleetName"));
    }

    #[test]
    fn test_resolve_field_with_source_unset_when_missing() {
        use ags_protocol::workflow::StepFieldSource;
        let ctx = WorkflowContext::default();
        let supplied = BTreeMap::new();
        let step_local = BTreeMap::new();
        let names = std::collections::BTreeSet::new();
        let defaults = std::collections::BTreeSet::new();
        let bindings: BTreeMap<&str, &BindingSource> = BTreeMap::new();
        let (value, source, _wf) = resolve_field_with_source(
            "imageId",
            &bindings,
            &[],
            &ctx,
            &supplied,
            &step_local,
            &names,
            &defaults,
        )
        .unwrap();
        assert_eq!(value, None);
        assert!(matches!(source, StepFieldSource::Unset));
    }

    #[test]
    fn test_assemble_command_request_missing_required_body_field_is_validation_kind() {
        let schema = service_with(op_with_namespace_path());
        let step = compiled_step_with_auto(vec![]);
        let mut local = BTreeMap::new();
        local.insert("namespace".into(), serde_json::json!("dev"));
        let err = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            &BTreeMap::new(),
            &local,
            &[],
            &schema,
            None,
            &RunOptions::default(),
        )
        .unwrap_err();
        assert_eq!(err.kind, RuntimeErrorKind::Validation);
    }

    // ---------------------------------------------------------------------------
    // B4 provenance tests
    // ---------------------------------------------------------------------------

    /// Build a minimal `StepFieldPlan` for a step with exactly one explicit
    /// binding. The operation schema is synthesised to include the binding's
    /// field as an optional body parameter, so the field-plan builder visits it.
    fn build_test_step_plan_with_binding(
        binding: StepInputBinding,
        supplied_inputs: &[(&str, serde_json::Value)],
    ) -> StepFieldPlan {
        use ags_protocol::catalogue::{BodyField, BodyFieldType, BodySchema};

        // Build an operation whose body contains the bound field.
        let mut op = op_with_namespace_path();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![BodyField {
                name: binding.field.clone(),
                field_type: BodyFieldType::String,
                required: false,
                description: None,
                children: vec![],
                default: None,
            }],
        });
        let schema = service_with(op);

        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(binding);

        let workflow_supplied: BTreeMap<String, serde_json::Value> = supplied_inputs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        let workflow_input_specs: Vec<WorkflowInputSpec> = supplied_inputs
            .iter()
            .map(|(k, _)| WorkflowInputSpec {
                name: k.to_string(),
                description: None,
                schema: Some(serde_json::json!({"type": "string"})),
                required: true,
                default: None,
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
                file_picker: None,
            })
            .collect();
        // Supply a namespace value so the path param is satisfied.
        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("test-ns"));
        supplied.extend(workflow_supplied);

        let ctx = WorkflowContext::default();
        let defaults = std::collections::BTreeSet::new();

        resolve_step_fields(
            &step,
            &ctx,
            &supplied,
            &BTreeMap::new(),
            &workflow_input_specs,
            &schema,
            &defaults,
        )
        .unwrap()
    }

    #[test]
    fn test_format_binding_emits_derived_provenance() {
        use ags_protocol::workflow::{FormatBinding, StepFieldSource};

        let plan = build_test_step_plan_with_binding(
            StepInputBinding {
                field: "name".into(),
                source: BindingSource::Format(FormatBinding {
                    template: "{resourcePrefix}-fleet".into(),
                }),
                show_in_review: false,
                description: None,
            },
            &[("resourcePrefix", serde_json::json!("ranked"))],
        );
        let field = plan.fields.iter().find(|f| f.field == "name").unwrap();
        assert_eq!(
            field.source,
            StepFieldSource::Derived {
                sources: vec!["resourcePrefix".into()]
            }
        );
    }

    #[test]
    fn test_workflow_input_arithmetic_emits_derived_provenance() {
        use ags_protocol::workflow::{
            ArithmeticOp, ArithmeticOperand, ArithmeticTransform, StepFieldSource,
        };

        let plan = build_test_step_plan_with_binding(
            StepInputBinding {
                field: "minPlayers".into(),
                source: BindingSource::Reference(ReferenceBinding {
                    from: ReferenceTarget::Workflow {
                        input: "playersPerTeam".into(),
                    },
                    output: None,
                    transform: Some(TransformKind::Arithmetic(ArithmeticTransform {
                        op: ArithmeticOp::Mul,
                        operand: ArithmeticOperand::WorkflowInput("teamCount".into()),
                    })),
                }),
                show_in_review: false,
                description: None,
            },
            &[
                ("playersPerTeam", serde_json::json!(4)),
                ("teamCount", serde_json::json!(2)),
            ],
        );
        let field = plan
            .fields
            .iter()
            .find(|f| f.field == "minPlayers")
            .unwrap();
        assert_eq!(
            field.source,
            StepFieldSource::Derived {
                sources: vec!["playersPerTeam".into(), "teamCount".into()]
            }
        );
    }

    #[test]
    fn test_workflow_input_arithmetic_integer_literal_also_emits_derived() {
        use ags_protocol::workflow::{
            ArithmeticOp, ArithmeticOperand, ArithmeticTransform, StepFieldSource,
        };

        let plan = build_test_step_plan_with_binding(
            StepInputBinding {
                field: "minPlayers".into(),
                source: BindingSource::Reference(ReferenceBinding {
                    from: ReferenceTarget::Workflow {
                        input: "playersPerTeam".into(),
                    },
                    output: None,
                    transform: Some(TransformKind::Arithmetic(ArithmeticTransform {
                        op: ArithmeticOp::Add,
                        operand: ArithmeticOperand::Integer(1),
                    })),
                }),
                show_in_review: false,
                description: None,
            },
            &[("playersPerTeam", serde_json::json!(4))],
        );
        let field = plan
            .fields
            .iter()
            .find(|f| f.field == "minPlayers")
            .unwrap();
        assert_eq!(
            field.source,
            StepFieldSource::Derived {
                sources: vec!["playersPerTeam".into()]
            }
        );
    }

    /// Build a minimal `StepFieldPlan` for a step with one explicit binding
    /// where the referenced workflow input has a declared default and the user
    /// did NOT supply the input (so the default flows through).
    fn build_test_step_plan_with_defaulted_input(
        binding: StepInputBinding,
        (input_name, default_value): (&str, serde_json::Value),
    ) -> StepFieldPlan {
        use ags_protocol::catalogue::{BodyField, BodyFieldType, BodySchema};

        // Build an operation whose body contains the bound field.
        let mut op = op_with_namespace_path();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![BodyField {
                name: binding.field.clone(),
                field_type: BodyFieldType::String,
                required: false,
                description: None,
                children: vec![],
                default: None,
            }],
        });
        let schema = service_with(op);

        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(binding);

        let workflow_input_specs = vec![WorkflowInputSpec {
            name: input_name.to_string(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: false,
            default: Some(default_value.clone()),
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        }];

        // Supply the default value in workflow_supplied (as the executor does
        // when it applies defaults) but mark the input name in default_names
        // (not in user-supplied). Namespace is always satisfied.
        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("test-ns"));
        supplied.insert(input_name.to_string(), default_value);

        let mut default_names = std::collections::BTreeSet::new();
        default_names.insert(input_name.to_string());

        let ctx = WorkflowContext::default();

        resolve_step_fields(
            &step,
            &ctx,
            &supplied,
            &BTreeMap::new(),
            &workflow_input_specs,
            &schema,
            &default_names,
        )
        .unwrap()
    }

    #[test]
    fn test_explicit_workflow_ref_with_defaulted_input_emits_default_provenance() {
        // Regression: explicit binding `from: workflow/X` where input X has a
        // declared default and the user didn't supply X must emit
        // StepFieldSource::Default, not WorkflowInput.
        use ags_protocol::workflow::StepFieldSource;

        let plan = build_test_step_plan_with_defaulted_input(
            StepInputBinding {
                field: "name".into(),
                source: BindingSource::Reference(ReferenceBinding {
                    from: ReferenceTarget::Workflow {
                        input: "resourcePrefix".into(),
                    },
                    output: None,
                    transform: None,
                }),
                show_in_review: false,
                description: None,
            },
            ("resourcePrefix", serde_json::json!("ranked")),
        );
        let field = plan.fields.iter().find(|f| f.field == "name").unwrap();
        assert_eq!(
            field.source,
            StepFieldSource::Default {
                name: "resourcePrefix".into()
            }
        );
    }

    // ---------------------------------------------------------------------------
    // C1 Format binding resolver tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_resolve_format_binding_substitutes_workflow_inputs() {
        use ags_protocol::workflow::FormatBinding;

        let source = BindingSource::Format(FormatBinding {
            template: "{prefix}-fleet".into(),
        });
        let supplied: BTreeMap<String, serde_json::Value> =
            [("prefix".to_string(), serde_json::json!("ranked"))]
                .into_iter()
                .collect();
        let ctx = WorkflowContext::default();
        let resolved =
            resolve_binding(&source, &ctx, &supplied, &BTreeMap::new(), &BTreeMap::new())
                .unwrap()
                .unwrap();
        assert_eq!(resolved, serde_json::json!("ranked-fleet"));
    }

    #[test]
    fn test_resolve_format_binding_escapes_double_braces() {
        use ags_protocol::workflow::FormatBinding;

        let source = BindingSource::Format(FormatBinding {
            template: "{{not-a-placeholder}}-{x}".into(),
        });
        let supplied: BTreeMap<String, serde_json::Value> =
            [("x".to_string(), serde_json::json!("real"))]
                .into_iter()
                .collect();
        let ctx = WorkflowContext::default();
        let resolved =
            resolve_binding(&source, &ctx, &supplied, &BTreeMap::new(), &BTreeMap::new())
                .unwrap()
                .unwrap();
        assert_eq!(resolved, serde_json::json!("{not-a-placeholder}-real"));
    }

    #[test]
    fn test_resolve_format_binding_stringifies_numeric_inputs() {
        use ags_protocol::workflow::FormatBinding;

        let source = BindingSource::Format(FormatBinding {
            template: "{count}-fleet".into(),
        });
        let supplied: BTreeMap<String, serde_json::Value> =
            [("count".to_string(), serde_json::json!(4))]
                .into_iter()
                .collect();
        let ctx = WorkflowContext::default();
        let resolved =
            resolve_binding(&source, &ctx, &supplied, &BTreeMap::new(), &BTreeMap::new())
                .unwrap()
                .unwrap();
        assert_eq!(resolved, serde_json::json!("4-fleet"));
    }

    #[test]
    fn test_resolve_format_binding_preserves_non_ascii_literals() {
        // Templates may contain non-ASCII literal text outside placeholders.
        // The resolver must preserve UTF-8 byte sequences correctly.
        use ags_protocol::workflow::FormatBinding;

        let source = BindingSource::Format(FormatBinding {
            template: "héllo-{x}".into(),
        });
        let supplied: BTreeMap<String, serde_json::Value> =
            [("x".to_string(), serde_json::json!("世界"))]
                .into_iter()
                .collect();
        let ctx = WorkflowContext::default();
        let resolved =
            resolve_binding(&source, &ctx, &supplied, &BTreeMap::new(), &BTreeMap::new())
                .unwrap()
                .unwrap();
        assert_eq!(resolved, serde_json::json!("héllo-世界"));
    }

    // ---------------------------------------------------------------------------
    // C2 Arithmetic transform resolver tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_apply_arithmetic_mul_with_workflow_input_operand() {
        use ags_protocol::workflow::{ArithmeticOp, ArithmeticOperand, ArithmeticTransform};

        let supplied: BTreeMap<String, serde_json::Value> =
            [("teamCount".to_string(), serde_json::json!(2))]
                .into_iter()
                .collect();
        let value = serde_json::json!(4); // playersPerTeam
        let transform = TransformKind::Arithmetic(ArithmeticTransform {
            op: ArithmeticOp::Mul,
            operand: ArithmeticOperand::WorkflowInput("teamCount".into()),
        });
        let result = apply_optional_transform(&value, Some(&transform), &supplied);
        assert_eq!(result, serde_json::json!(8));
    }

    #[test]
    fn test_apply_arithmetic_add_with_integer_literal() {
        use ags_protocol::workflow::{ArithmeticOp, ArithmeticOperand, ArithmeticTransform};

        let supplied = BTreeMap::new();
        let value = serde_json::json!(5);
        let transform = TransformKind::Arithmetic(ArithmeticTransform {
            op: ArithmeticOp::Add,
            operand: ArithmeticOperand::Integer(3),
        });
        let result = apply_optional_transform(&value, Some(&transform), &supplied);
        assert_eq!(result, serde_json::json!(8));
    }

    // ---------------------------------------------------------------------------
    // C3 Nested binding merge tests
    // ---------------------------------------------------------------------------

    /// Exercise the two-pass body assembler with an arbitrary set of bindings
    /// and pre-supplied workflow inputs. Returns just the request body.
    ///
    /// The operation schema is synthesised with a single top-level body field
    /// matching the first single-segment binding's field name (so required-field
    /// validation does not fire). Namespace is always satisfied via step_local.
    fn assemble_step_body_for_test(
        bindings: &[StepInputBinding],
        supplied: &BTreeMap<String, serde_json::Value>,
    ) -> serde_json::Value {
        // Derive the set of top-level schema field names from single-segment
        // bindings so the schema-driven pass recognises them.
        let top_level_names: Vec<String> = bindings
            .iter()
            .filter(|b| !b.field.contains('.') && !b.field.contains('['))
            .map(|b| b.field.clone())
            .collect();

        let mut op = op_with_namespace_path();
        op.request_body = Some(ags_protocol::catalogue::BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: top_level_names
                .iter()
                .map(|name| ags_protocol::catalogue::BodyField {
                    name: name.clone(),
                    field_type: ags_protocol::catalogue::BodyFieldType::Object,
                    required: false,
                    description: None,
                    children: vec![],
                    default: None,
                })
                .collect(),
        });
        let schema = service_with(op);

        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.extend_from_slice(bindings);

        let mut local = BTreeMap::new();
        local.insert("namespace".to_string(), serde_json::json!("test-ns"));

        let req = assemble_command_request(
            &step,
            &WorkflowContext::new(),
            supplied,
            &local,
            &[],
            &schema,
            None,
            &RunOptions::default(),
        )
        .unwrap();

        match req.body {
            Some(RequestBody::Json(v)) => v,
            Some(RequestBody::Multipart(_)) | None => serde_json::Value::Object(Default::default()),
        }
    }

    #[test]
    fn test_nested_binding_merges_into_object_literal() {
        let bindings = vec![
            StepInputBinding {
                field: "dsHostConfiguration".into(),
                source: BindingSource::Literal(LiteralBinding {
                    value: serde_json::json!({"serversPerVm": 1}),
                    sensitive: false,
                }),
                show_in_review: false,
                description: None,
            },
            StepInputBinding {
                field: "dsHostConfiguration.instanceId".into(),
                source: BindingSource::Reference(ReferenceBinding {
                    from: ReferenceTarget::Workflow {
                        input: "fleetInstanceId".into(),
                    },
                    output: None,
                    transform: None,
                }),
                show_in_review: false,
                description: None,
            },
        ];
        let supplied: BTreeMap<String, serde_json::Value> =
            [("fleetInstanceId".to_string(), serde_json::json!("m5.large"))]
                .into_iter()
                .collect();
        let body = assemble_step_body_for_test(&bindings, &supplied);
        assert_eq!(
            body["dsHostConfiguration"],
            serde_json::json!({"serversPerVm": 1, "instanceId": "m5.large"})
        );
    }

    #[test]
    fn test_nested_binding_into_array_element() {
        let bindings = vec![
            StepInputBinding {
                field: "regions".into(),
                source: BindingSource::Literal(LiteralBinding {
                    value: serde_json::json!([{
                        "minServerCount": 0,
                        "maxServerCount": 2,
                        "bufferSize": 1,
                        "dynamicBuffer": true,
                    }]),
                    sensitive: false,
                }),
                show_in_review: false,
                description: None,
            },
            StepInputBinding {
                field: "regions[0].region".into(),
                source: BindingSource::Reference(ReferenceBinding {
                    from: ReferenceTarget::Workflow {
                        input: "fleetRegion".into(),
                    },
                    output: None,
                    transform: None,
                }),
                show_in_review: false,
                description: None,
            },
        ];
        let supplied: BTreeMap<String, serde_json::Value> =
            [("fleetRegion".to_string(), serde_json::json!("us-west-2"))]
                .into_iter()
                .collect();
        let body = assemble_step_body_for_test(&bindings, &supplied);
        assert_eq!(body["regions"][0]["region"], serde_json::json!("us-west-2"));
        assert_eq!(body["regions"][0]["bufferSize"], serde_json::json!(1));
    }

    // ---------------------------------------------------------------------------
    // SR2 show_in_review propagation tests
    // ---------------------------------------------------------------------------

    /// Helper: build a plan for a step with a single body-field binding and
    /// return the `StepField` for that field.
    fn build_single_body_field_plan(binding: StepInputBinding) -> StepField {
        use ags_protocol::catalogue::{BodyField, BodyFieldType, BodySchema};

        // The body field name is the top-level field root of the binding.
        let root = binding
            .field
            .split_once('.')
            .map(|(r, _)| r)
            .or_else(|| binding.field.split_once('[').map(|(r, _)| r))
            .unwrap_or(binding.field.as_str())
            .to_string();

        let mut op = op_with_namespace_path();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![BodyField {
                name: root.clone(),
                field_type: BodyFieldType::Object,
                required: false,
                description: None,
                children: vec![],
                default: None,
            }],
        });
        let schema = service_with(op);

        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(binding);

        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("test-ns"));

        let ctx = WorkflowContext::default();
        let plan = resolve_step_fields(
            &step,
            &ctx,
            &supplied,
            &BTreeMap::new(),
            &[],
            &schema,
            &std::collections::BTreeSet::new(),
        )
        .unwrap();

        plan.fields.into_iter().find(|f| f.field == root).unwrap()
    }

    #[test]
    fn test_show_in_review_true_binding_propagates_to_step_field() {
        let field = build_single_body_field_plan(StepInputBinding {
            field: "joinability".into(),
            source: BindingSource::Literal(LiteralBinding {
                value: serde_json::json!("OPEN"),
                sensitive: false,
            }),
            show_in_review: true,
            description: None,
        });
        assert!(
            field.show_in_review,
            "show_in_review=true binding must propagate"
        );
    }

    #[test]
    fn test_show_in_review_false_binding_gives_false_on_step_field() {
        let field = build_single_body_field_plan(StepInputBinding {
            field: "setBy".into(),
            source: BindingSource::Literal(LiteralBinding {
                value: serde_json::json!("SERVER"),
                sensitive: false,
            }),
            show_in_review: false,
            description: None,
        });
        assert!(
            !field.show_in_review,
            "show_in_review=false binding must stay false"
        );
    }

    #[test]
    fn test_show_in_review_no_binding_gives_false() {
        // Field auto-derived from a supplied workflow input — no explicit binding.
        use ags_protocol::catalogue::{BodyField, BodyFieldType, BodySchema};

        let mut op = op_with_namespace_path();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![BodyField {
                name: "statCode".into(),
                field_type: BodyFieldType::String,
                required: true,
                description: None,
                children: vec![],
                default: None,
            }],
        });
        let schema = service_with(op);
        let step = compiled_step_with_auto(vec![]);

        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("test-ns"));
        // statCode is required; it will be Unset (no binding, no local value) but still shown.

        let specs = vec![WorkflowInputSpec {
            name: "namespace".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        }];

        let ctx = WorkflowContext::default();
        let plan = resolve_step_fields(
            &step,
            &ctx,
            &supplied,
            &BTreeMap::new(),
            &specs,
            &schema,
            &std::collections::BTreeSet::new(),
        )
        .unwrap();

        let field = plan.fields.iter().find(|f| f.field == "statCode").unwrap();
        assert!(
            !field.show_in_review,
            "unbound field must have show_in_review=false"
        );
    }

    #[test]
    fn test_show_in_review_nested_reference_binding_does_not_aggregate_to_parent() {
        // Parent binding show_in_review=false, nested Reference binding
        // show_in_review=true. Under the new per-leaf design, nested bindings do
        // NOT aggregate their show_in_review up to the parent StepField. The
        // parent's show_in_review comes only from its own top-level binding.
        // (Reference nested bindings stay in the parent's synthesis path and do
        // not get their own per-leaf row — only Literal nested bindings do.)
        use ags_protocol::catalogue::{BodyField, BodyFieldType, BodySchema};

        let mut op = op_with_namespace_path();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![BodyField {
                name: "data".into(),
                field_type: BodyFieldType::Object,
                required: false,
                description: None,
                children: vec![],
                default: None,
            }],
        });
        let schema = service_with(op);

        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "data".into(),
            source: BindingSource::Literal(LiteralBinding {
                value: serde_json::json!({"matchingRule": [{"attribute": ""}]}),
                sensitive: false,
            }),
            show_in_review: false,
            description: None,
        });
        step.inputs.push(StepInputBinding {
            field: "data.matchingRule[0].attribute".into(),
            source: BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: "matchAttribute".into(),
                },
                output: None,
                transform: None,
            }),
            show_in_review: true,
            description: None,
        });

        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("test-ns"));
        supplied.insert("matchAttribute".to_string(), serde_json::json!("mmr"));

        let ctx = WorkflowContext::default();
        let plan = resolve_step_fields(
            &step,
            &ctx,
            &supplied,
            &BTreeMap::new(),
            &[],
            &schema,
            &std::collections::BTreeSet::new(),
        )
        .unwrap();

        let parent_field = plan.fields.iter().find(|f| f.field == "data").unwrap();
        assert!(
            !parent_field.show_in_review,
            "nested Reference binding show_in_review must NOT aggregate to parent field"
        );
        // The Reference nested binding does not get a per-leaf row (only Literal does).
        let has_per_leaf = plan
            .fields
            .iter()
            .any(|f| f.field == "data.matchingRule[0].attribute");
        assert!(
            !has_per_leaf,
            "Reference nested binding must not produce a per-leaf row"
        );
    }

    // ---------------------------------------------------------------------------
    // SR2 workflow-input description preference tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_description_prefers_workflow_input_description_over_openapi() {
        // A field bound to a workflow input that has a description should use
        // the workflow input's description, not the OpenAPI field's description.
        use ags_protocol::catalogue::{BodyField, BodyFieldType, BodySchema};

        let mut op = op_with_namespace_path();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![BodyField {
                name: "sessionDeployment".into(),
                field_type: BodyFieldType::String,
                required: false,
                description: Some("OpenAPI description of sessionDeployment".into()),
                children: vec![],
                default: None,
            }],
        });
        let schema = service_with(op);

        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "sessionDeployment".into(),
            source: BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: "sessionDeployment".into(),
                },
                output: None,
                transform: None,
            }),
            show_in_review: false,
            description: None,
        });

        let specs = vec![WorkflowInputSpec {
            name: "sessionDeployment".into(),
            description: Some("AMS deployment to use for this session".into()),
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        }];

        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("test-ns"));
        supplied.insert(
            "sessionDeployment".to_string(),
            serde_json::json!("prod-deploy"),
        );

        let ctx = WorkflowContext::default();
        let plan = resolve_step_fields(
            &step,
            &ctx,
            &supplied,
            &BTreeMap::new(),
            &specs,
            &schema,
            &std::collections::BTreeSet::new(),
        )
        .unwrap();

        let field = plan
            .fields
            .iter()
            .find(|f| f.field == "sessionDeployment")
            .unwrap();
        assert_eq!(
            field.description.as_deref(),
            Some("AMS deployment to use for this session"),
            "should use workflow input description, not OpenAPI description"
        );
    }

    #[test]
    fn test_description_literal_binding_uses_openapi_description() {
        // A Literal-bound field has no workflow input to pull a description from —
        // must fall back to the OpenAPI field's description.
        use ags_protocol::catalogue::{BodyField, BodyFieldType, BodySchema};

        let mut op = op_with_namespace_path();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![BodyField {
                name: "setBy".into(),
                field_type: BodyFieldType::String,
                required: false,
                description: Some("Who set this value".into()),
                children: vec![],
                default: None,
            }],
        });
        let schema = service_with(op);

        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "setBy".into(),
            source: BindingSource::Literal(LiteralBinding {
                value: serde_json::json!("SERVER"),
                sensitive: false,
            }),
            show_in_review: false,
            description: None,
        });

        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("test-ns"));

        let ctx = WorkflowContext::default();
        let plan = resolve_step_fields(
            &step,
            &ctx,
            &supplied,
            &BTreeMap::new(),
            &[],
            &schema,
            &std::collections::BTreeSet::new(),
        )
        .unwrap();

        let field = plan.fields.iter().find(|f| f.field == "setBy").unwrap();
        assert_eq!(
            field.description.as_deref(),
            Some("Who set this value"),
            "literal-bound field must use OpenAPI description"
        );
    }

    #[test]
    fn test_description_workflow_input_with_no_description_falls_back_to_openapi() {
        // Workflow input exists but has no description — should fall back to
        // the OpenAPI field's description.
        use ags_protocol::catalogue::{BodyField, BodyFieldType, BodySchema};

        let mut op = op_with_namespace_path();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![BodyField {
                name: "deploymentId".into(),
                field_type: BodyFieldType::String,
                required: false,
                description: Some("OpenAPI fallback description".into()),
                children: vec![],
                default: None,
            }],
        });
        let schema = service_with(op);

        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.push(StepInputBinding {
            field: "deploymentId".into(),
            source: BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: "deploymentId".into(),
                },
                output: None,
                transform: None,
            }),
            show_in_review: false,
            description: None,
        });

        let specs = vec![WorkflowInputSpec {
            name: "deploymentId".into(),
            description: None, // no description on the workflow input
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        }];

        let mut supplied = BTreeMap::new();
        supplied.insert("namespace".to_string(), serde_json::json!("test-ns"));
        supplied.insert("deploymentId".to_string(), serde_json::json!("d-001"));

        let ctx = WorkflowContext::default();
        let plan = resolve_step_fields(
            &step,
            &ctx,
            &supplied,
            &BTreeMap::new(),
            &specs,
            &schema,
            &std::collections::BTreeSet::new(),
        )
        .unwrap();

        let field = plan
            .fields
            .iter()
            .find(|f| f.field == "deploymentId")
            .unwrap();
        assert_eq!(
            field.description.as_deref(),
            Some("OpenAPI fallback description"),
            "workflow input with no description must fall back to OpenAPI description"
        );
    }

    // ---------------------------------------------------------------------------
    // Nested-binding synthesis for parent fields with no top-level binding
    // ---------------------------------------------------------------------------

    /// Helper: build a `StepFieldPlan` for a step whose operation body has a
    /// single optional field (`parent_field`) that has no top-level binding but
    /// does have one or more nested bindings. Returns the `StepField` for
    /// `parent_field`.
    fn build_plan_with_nested_only_bindings(
        parent_field: &str,
        nested_bindings: Vec<StepInputBinding>,
        supplied: &[(&str, serde_json::Value)],
        input_specs: &[(&str, serde_json::Value)],
    ) -> StepField {
        use ags_protocol::catalogue::{BodyField, BodyFieldType, BodySchema};

        let mut op = op_with_namespace_path();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![BodyField {
                name: parent_field.into(),
                field_type: BodyFieldType::Object,
                required: false,
                description: None,
                children: vec![],
                default: None,
            }],
        });
        let schema = service_with(op);

        let mut step = compiled_step_with_auto(vec![]);
        step.inputs.extend(nested_bindings);

        let workflow_input_specs: Vec<WorkflowInputSpec> = input_specs
            .iter()
            .map(|(name, schema_val)| WorkflowInputSpec {
                name: name.to_string(),
                description: None,
                schema: Some(schema_val.clone()),
                required: true,
                default: None,
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
                file_picker: None,
            })
            .collect();

        let mut workflow_supplied: BTreeMap<String, serde_json::Value> = supplied
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        workflow_supplied.insert("namespace".to_string(), serde_json::json!("test-ns"));

        let ctx = WorkflowContext::default();
        let plan = resolve_step_fields(
            &step,
            &ctx,
            &workflow_supplied,
            &BTreeMap::new(),
            &workflow_input_specs,
            &schema,
            &std::collections::BTreeSet::new(),
        )
        .unwrap();

        plan.fields
            .into_iter()
            .find(|f| f.field == parent_field)
            .expect("parent field must appear in plan")
    }

    #[test]
    fn test_step_field_synthesized_from_single_nested_workflow_ref() {
        // `requestedRegions[0]` → workflow input "fleetRegion" with value "us-east-1".
        // Expect: top-level `requestedRegions` field shows value ["us-east-1"]
        // with Derived { sources: ["fleetRegion"] } provenance.
        use ags_protocol::workflow::StepFieldSource;

        let field = build_plan_with_nested_only_bindings(
            "requestedRegions",
            vec![StepInputBinding {
                field: "requestedRegions[0]".into(),
                source: BindingSource::Reference(ReferenceBinding {
                    from: ReferenceTarget::Workflow {
                        input: "fleetRegion".into(),
                    },
                    output: None,
                    transform: None,
                }),
                show_in_review: true,
                description: None,
            }],
            &[("fleetRegion", serde_json::json!("us-east-1"))],
            &[("fleetRegion", serde_json::json!({"type": "string"}))],
        );

        assert_eq!(
            field.value,
            serde_json::json!(["us-east-1"]),
            "synthesized value must be [\"us-east-1\"]"
        );
        assert_eq!(
            field.source,
            StepFieldSource::Derived {
                sources: vec!["fleetRegion".into()]
            },
            "provenance must be Derived from fleetRegion"
        );
    }

    #[test]
    fn test_step_field_synthesized_from_format_nested_binding() {
        // `claimKeys[0]` → Format("{resourcePrefix}-claim-key") with resourcePrefix="ranked".
        // Expect: top-level `claimKeys` field shows value ["ranked-claim-key"]
        // with Derived { sources: ["resourcePrefix"] } provenance.
        use ags_protocol::workflow::{FormatBinding, StepFieldSource};

        let field = build_plan_with_nested_only_bindings(
            "claimKeys",
            vec![StepInputBinding {
                field: "claimKeys[0]".into(),
                source: BindingSource::Format(FormatBinding {
                    template: "{resourcePrefix}-claim-key".into(),
                }),
                show_in_review: true,
                description: None,
            }],
            &[("resourcePrefix", serde_json::json!("ranked"))],
            &[("resourcePrefix", serde_json::json!({"type": "string"}))],
        );

        assert_eq!(
            field.value,
            serde_json::json!(["ranked-claim-key"]),
            "synthesized value must be [\"ranked-claim-key\"]"
        );
        assert_eq!(
            field.source,
            StepFieldSource::Derived {
                sources: vec!["resourcePrefix".into()]
            },
            "provenance must be Derived from resourcePrefix"
        );
    }

    // ---------------------------------------------------------------------------
    // Per-leaf StepField synthesis for visible Literal nested bindings
    // ---------------------------------------------------------------------------

    /// Helper: build a `StepFieldPlan` for a step with a parent body field and
    /// one or more nested bindings. The parent field is always optional Object.
    fn build_plan_with_parent_and_nested_bindings(
        parent_field: &str,
        parent_binding: Option<StepInputBinding>,
        nested_bindings: Vec<StepInputBinding>,
        supplied: &[(&str, serde_json::Value)],
    ) -> StepFieldPlan {
        use ags_protocol::catalogue::{BodyField, BodyFieldType, BodySchema};

        let mut op = op_with_namespace_path();
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![BodyField {
                name: parent_field.into(),
                field_type: BodyFieldType::Object,
                required: false,
                description: None,
                children: vec![],
                default: None,
            }],
        });
        let schema = service_with(op);

        let mut step = compiled_step_with_auto(vec![]);
        if let Some(pb) = parent_binding {
            step.inputs.push(pb);
        }
        step.inputs.extend(nested_bindings);

        let mut workflow_supplied: BTreeMap<String, serde_json::Value> = supplied
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        workflow_supplied.insert("namespace".to_string(), serde_json::json!("test-ns"));

        let ctx = WorkflowContext::default();
        resolve_step_fields(
            &step,
            &ctx,
            &workflow_supplied,
            &BTreeMap::new(),
            &[],
            &schema,
            &std::collections::BTreeSet::new(),
        )
        .unwrap()
    }

    #[test]
    fn test_per_leaf_field_synthesized_for_visible_literal_nested_binding() {
        // bind_visible("data.matching_rule[0].reference", literal(json!(200)))
        // Expect: a StepField with label "reference", field "data.matching_rule[0].reference",
        // value 200, source Literal, schema {"type": "integer"}, show_in_review true.
        use ags_protocol::workflow::StepFieldSource;

        let plan = build_plan_with_parent_and_nested_bindings(
            "data",
            Some(StepInputBinding {
                field: "data".into(),
                source: BindingSource::Literal(LiteralBinding {
                    value: serde_json::json!({"matching_rule": [{"reference": 0}]}),
                    sensitive: false,
                }),
                show_in_review: false,
                description: None,
            }),
            vec![StepInputBinding {
                field: "data.matching_rule[0].reference".into(),
                source: BindingSource::Literal(LiteralBinding {
                    value: serde_json::json!(200),
                    sensitive: false,
                }),
                show_in_review: true,
                description: None,
            }],
            &[],
        );

        let leaf = plan
            .fields
            .iter()
            .find(|f| f.field == "data.matching_rule[0].reference")
            .expect("per-leaf StepField must be synthesized for visible Literal nested binding");

        assert_eq!(
            leaf.label, "reference",
            "label must be last path Key segment"
        );
        assert_eq!(leaf.value, serde_json::json!(200));
        assert!(matches!(leaf.source, StepFieldSource::Literal));
        assert_eq!(leaf.schema, serde_json::json!({"type": "integer"}));
        assert!(leaf.show_in_review);
        assert!(!leaf.body_overflow);
        assert_eq!(leaf.location, StepFieldLocation::Body);
    }

    #[test]
    fn test_per_leaf_field_label_is_last_path_segment() {
        // bind_visible("data.auto_backfill", literal(json!(true)))
        // Expect: label "auto_backfill", value true, schema {"type": "boolean"}.
        use ags_protocol::workflow::StepFieldSource;

        let plan = build_plan_with_parent_and_nested_bindings(
            "data",
            Some(StepInputBinding {
                field: "data".into(),
                source: BindingSource::Literal(LiteralBinding {
                    value: serde_json::json!({}),
                    sensitive: false,
                }),
                show_in_review: false,
                description: None,
            }),
            vec![StepInputBinding {
                field: "data.auto_backfill".into(),
                source: BindingSource::Literal(LiteralBinding {
                    value: serde_json::json!(true),
                    sensitive: false,
                }),
                show_in_review: true,
                description: None,
            }],
            &[],
        );

        let leaf = plan
            .fields
            .iter()
            .find(|f| f.field == "data.auto_backfill")
            .expect("per-leaf StepField must be synthesized");

        assert_eq!(leaf.label, "auto_backfill");
        assert_eq!(leaf.value, serde_json::json!(true));
        assert!(matches!(leaf.source, StepFieldSource::Literal));
        assert_eq!(leaf.schema, serde_json::json!({"type": "boolean"}));
    }

    #[test]
    fn test_provenance_traced_nested_binding_does_not_synthesize_per_leaf() {
        // bind("data.matching_rule[0].attribute", workflow_ref("statCode"))
        // Expect: NO standalone StepField for "attribute". The parent's synthesis
        // (or workflow-input visibility) handles it.
        let plan = build_plan_with_parent_and_nested_bindings(
            "data",
            Some(StepInputBinding {
                field: "data".into(),
                source: BindingSource::Literal(LiteralBinding {
                    value: serde_json::json!({"matching_rule": [{"attribute": ""}]}),
                    sensitive: false,
                }),
                show_in_review: false,
                description: None,
            }),
            vec![StepInputBinding {
                field: "data.matching_rule[0].attribute".into(),
                source: BindingSource::Reference(ReferenceBinding {
                    from: ReferenceTarget::Workflow {
                        input: "statCode".into(),
                    },
                    output: None,
                    transform: None,
                }),
                show_in_review: true,
                description: None,
            }],
            &[("statCode", serde_json::json!("mmr"))],
        );

        let has_per_leaf = plan
            .fields
            .iter()
            .any(|f| f.field == "data.matching_rule[0].attribute");
        assert!(
            !has_per_leaf,
            "Reference nested binding must NOT produce a per-leaf row"
        );
    }

    #[test]
    fn test_parent_field_show_in_review_not_aggregated_from_nested() {
        // Parent `data` Literal binding has show_in_review=false.
        // Nested `data.x` Literal binding has show_in_review=true.
        // Expect: parent's StepField.show_in_review is false (NOT aggregated).
        // (The nested binding gets its own row, but the parent stays hidden.)
        let plan = build_plan_with_parent_and_nested_bindings(
            "data",
            Some(StepInputBinding {
                field: "data".into(),
                source: BindingSource::Literal(LiteralBinding {
                    value: serde_json::json!({}),
                    sensitive: false,
                }),
                show_in_review: false,
                description: None,
            }),
            vec![StepInputBinding {
                field: "data.x".into(),
                source: BindingSource::Literal(LiteralBinding {
                    value: serde_json::json!(42),
                    sensitive: false,
                }),
                show_in_review: true,
                description: None,
            }],
            &[],
        );

        let parent = plan
            .fields
            .iter()
            .find(|f| f.field == "data")
            .expect("parent field must appear in plan");
        assert!(
            !parent.show_in_review,
            "parent StepField.show_in_review must be false when parent binding has show_in_review=false, \
             regardless of nested binding flags"
        );

        // The nested per-leaf row must be present with show_in_review=true.
        let leaf =
            plan.fields.iter().find(|f| f.field == "data.x").expect(
                "nested Literal binding with show_in_review=true must produce a per-leaf row",
            );
        assert!(leaf.show_in_review);
    }

    // -----------------------------------------------------------------
    // Malformed-step defensive error tests: verify that an API step with
    // `operation: None` returns `RuntimeErrorKind::Internal` rather than
    // panicking.
    // -----------------------------------------------------------------
    fn malformed_api_step_without_operation() -> CompiledStep {
        CompiledStep {
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
        }
    }

    #[test]
    fn test_assemble_command_request_api_step_without_operation_returns_internal_error() {
        let schema = service_with(op_with_namespace_path());
        let step = malformed_api_step_without_operation();
        let ctx = WorkflowContext::new();
        let err = assemble_command_request(
            &step,
            &ctx,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &[],
            &schema,
            None,
            &RunOptions::default(),
        )
        .unwrap_err();
        assert_eq!(err.kind, ags_protocol::error::RuntimeErrorKind::Internal);
        assert!(
            err.message.contains("without an operation"),
            "error must explain the missing operation: {err}"
        );
    }

    #[test]
    fn test_resolve_step_fields_api_step_without_operation_returns_internal_error() {
        let schema = service_with(op_with_namespace_path());
        let step = malformed_api_step_without_operation();
        let ctx = WorkflowContext::new();
        let err = resolve_step_fields(
            &step,
            &ctx,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &[],
            &schema,
            &std::collections::BTreeSet::new(),
        )
        .unwrap_err();
        assert_eq!(err.kind, ags_protocol::error::RuntimeErrorKind::Internal);
        assert!(
            err.message.contains("without an operation"),
            "error must explain the missing operation: {err}"
        );
    }
}
