//! Compile a `WorkflowDefinition` into a `CompiledWorkflow` by expanding
//! auto-derived inputs against the OpenAPI catalogue, resolving schemas,
//! and validating that step dependencies and references are satisfiable in
//! array order.

use std::collections::{BTreeMap, BTreeSet};

use ags_protocol::error::RuntimeError;
use ags_protocol::workflow::{
    ArithmeticOperand, BindingSource, CompiledStep, CompiledWorkflow, ReferenceTarget,
    TransformKind, WorkflowDefinition, WorkflowInputSpec,
};
#[cfg(test)]
use ags_protocol::workflow::{FilePickerSpec, StepDefinition};

/// Extract `{name}` placeholders from a Format template. Returns placeholder
/// names in order of first appearance. Handles `{{` and `}}` escapes.
pub(crate) fn format_placeholders(template: &str) -> Result<Vec<String>, RuntimeError> {
    let mut out = Vec::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' if i + 1 < bytes.len() && bytes[i + 1] == b'{' => i += 2,
            b'}' if i + 1 < bytes.len() && bytes[i + 1] == b'}' => i += 2,
            b'{' => {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end] != b'}' {
                    end += 1;
                }
                if end >= bytes.len() {
                    return Err(RuntimeError::internal(format!(
                        "Format template '{template}' has an unclosed `{{`"
                    )));
                }
                let name = &template[start..end];
                if name.is_empty() {
                    return Err(RuntimeError::internal(format!(
                        "Format template '{template}' has an empty `{{}}` placeholder"
                    )));
                }
                if !out.iter().any(|n: &String| n == name) {
                    out.push(name.into());
                }
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    Ok(out)
}

/// Visit every declared workflow-input name a step references *through its
/// bindings* — the reference target, an arithmetic transform operand, and each
/// format-template placeholder — calling `visit` once per occurrence (in
/// binding order; callers dedupe). Format templates that fail to parse are
/// skipped. Auto-derived scopes are NOT visited here: they read differently per
/// caller (first-use ordering vs. needed-input gathering, the latter also
/// handling `StepLocal`), so each caller walks `step.auto_derived` itself.
///
/// Shared by [`order_inputs_by_first_use`](crate::runtime::workflows::executor)
/// and [`compute_needed_inputs`](crate::runtime::workflows::resolve) so the
/// binding traversal — the part that drifted when `Format` and `Arithmetic`
/// support landed — lives in one place.
pub(crate) fn visit_binding_workflow_inputs(step: &CompiledStep, mut visit: impl FnMut(&str)) {
    for binding in &step.inputs {
        match &binding.source {
            BindingSource::Reference(r) => {
                if let ReferenceTarget::Workflow { input } = &r.from {
                    visit(input);
                }
                if let Some(TransformKind::Arithmetic(arith)) = &r.transform {
                    if let ArithmeticOperand::WorkflowInput(operand) = &arith.operand {
                        visit(operand);
                    }
                }
            }
            BindingSource::Format(format) => {
                for name in format_placeholders(&format.template).unwrap_or_default() {
                    visit(&name);
                }
            }
            BindingSource::Literal(_) => {}
            // A mirror reads a same-step field; any workflow input involved is
            // visited through the target field's own binding.
            BindingSource::Mirror(_) => {}
        }
    }
}

use crate::catalogue::Catalogue;
use crate::runtime::workflows::auto_derive::{auto_derive_step, find_operation_or_error};
use crate::runtime::workflows::nested_path::{FieldPath, PathResolution};

/// Compile a `WorkflowDefinition` into a `CompiledWorkflow`. Runs all
/// validation rules; resolves every workflow input's schema; expands each
/// step's auto-derived fields.
pub fn compile_workflow(
    definition: &WorkflowDefinition,
    catalogue: &mut Catalogue,
) -> Result<CompiledWorkflow, RuntimeError> {
    validate_unique_step_ids(definition)?;
    validate_step_kinds(definition)?;
    validate_dependencies(definition)?;
    validate_step_output_references(definition)?;
    validate_workflow_references(definition)?;
    validate_format_bindings(definition)?;
    validate_mirror_bindings(definition)?;
    validate_completion(definition)?;
    validate_arithmetic_bindings(definition)?;
    validate_no_sensitive(definition)?;
    validate_skippable_outputs_have_defaults(definition)?;
    // Cycle detection: trivial under array-order execution (no forward
    // references survive `validate_step_output_references`), but the check
    // exists so the YAML loader can re-use it. Currently a no-op.
    validate_no_cycles(definition)?;

    let resolved_inputs = resolve_input_schemas(definition, catalogue)?;

    validate_options_sources(definition, &resolved_inputs, catalogue)?;
    validate_file_pickers(&resolved_inputs)?;
    validate_nested_paths(definition, catalogue)?;
    validate_leaf_collisions(definition)?;

    let mut compiled_steps = Vec::with_capacity(definition.steps.len());
    for (index, step) in definition.steps.iter().enumerate() {
        use ags_protocol::workflow::StepKind;
        let auto_derived = match step.kind {
            StepKind::Api => {
                let op_ref = step.operation.as_ref().ok_or_else(|| {
                    RuntimeError::internal(format!(
                        "step '{}': API step reached compile_workflow without an operation",
                        step.id
                    ))
                })?;
                let service_schema = catalogue.get_or_load(op_ref.service.as_str())?;
                auto_derive_step(step, service_schema, &resolved_inputs)?
            }
            StepKind::Local => {
                let action = step.action.as_deref().ok_or_else(|| {
                    RuntimeError::internal(format!(
                        "step '{}': local step reached compile_workflow without an action",
                        step.id
                    ))
                })?;
                let handler = match crate::runtime::workflows::local_actions::lookup(action) {
                    Some(handler) => handler,
                    None => {
                        let known = crate::runtime::workflows::local_actions::known_names();
                        return Err(RuntimeError::internal(format!(
                            "step '{}': unknown local action '{}'; known actions: {}",
                            step.id,
                            action,
                            if known.is_empty() {
                                "(none)".to_string()
                            } else {
                                known.join(", ")
                            },
                        )));
                    }
                };

                // Validate input bindings against the action's declared inputs,
                // matching the checks the deleted validate_native_step performed.
                let declared = handler.inputs();
                for binding in &step.inputs {
                    let root = binding
                        .field
                        .split(['.', '['])
                        .next()
                        .unwrap_or(&binding.field);
                    if !declared.iter().any(|input| input.name == root) {
                        return Err(RuntimeError::internal(format!(
                            "step '{}' binds '{}', which local action '{}' does not accept",
                            step.id, binding.field, action
                        )));
                    }
                }
                for input in declared.iter().filter(|input| input.required) {
                    let is_bound = step.inputs.iter().any(|binding| {
                        binding.field == input.name
                            || binding.field.starts_with(&format!("{}.", input.name))
                    });
                    if !is_bound {
                        return Err(RuntimeError::internal(format!(
                            "step '{}' leaves required input '{}' of local action '{}' unbound",
                            step.id, input.name, action
                        )));
                    }
                }

                Vec::new()
            }
        };
        compiled_steps.push(CompiledStep {
            id: step.id.clone(),
            index,
            description: step.description.clone(),
            kind: step.kind,
            action: step.action.clone(),
            operation: step.operation.clone(),
            dependencies: step.dependencies.clone(),
            confirm: step.confirm,
            is_optional: step.is_optional,
            continue_on_failure: step.continue_on_failure,
            skip_if_exists: step.skip_if_exists,
            is_reviewed: step.is_reviewed,
            inputs: step.inputs.clone(),
            outputs: step.outputs.clone(),
            auto_derived,
        });
    }

    Ok(CompiledWorkflow {
        id: definition.id.clone(),
        name: definition.name.clone(),
        intent: definition.intent.clone(),
        description: definition.description.clone(),
        briefing: definition.briefing.clone(),
        inputs: resolved_inputs,
        is_reviewed_by_default: definition.is_reviewed_by_default,
        steps: compiled_steps,
        outputs: definition.outputs.clone(),
        completion: definition.completion.clone(),
    })
}

/// Rule 1: every step id is unique within the workflow.
fn validate_unique_step_ids(definition: &WorkflowDefinition) -> Result<(), RuntimeError> {
    let mut seen = BTreeSet::new();
    for step in &definition.steps {
        if !seen.insert(step.id.as_str()) {
            return Err(RuntimeError::internal(format!(
                "workflow '{}' has duplicate step id '{}'",
                definition.id.as_str(),
                step.id
            )));
        }
    }
    Ok(())
}

/// Rule 1b: every step's `kind` / `operation` / `action` triple is
/// consistent. API steps must have `operation` and no `action`; local
/// steps must have `action` and no `operation`.
fn validate_step_kinds(definition: &WorkflowDefinition) -> Result<(), RuntimeError> {
    use ags_protocol::workflow::StepKind;
    for step in &definition.steps {
        match step.kind {
            StepKind::Api => {
                if step.operation.is_none() {
                    return Err(RuntimeError::internal(format!(
                        "step '{}': kind 'api' requires an 'operation' field",
                        step.id
                    )));
                }
                if step.action.is_some() {
                    return Err(RuntimeError::internal(format!(
                        "step '{}': kind 'api' must not have an 'action' field",
                        step.id
                    )));
                }
            }
            StepKind::Local => {
                if step.action.is_none() {
                    return Err(RuntimeError::internal(format!(
                        "step '{}': kind 'local' requires an 'action' field",
                        step.id
                    )));
                }
                if step.operation.is_some() {
                    return Err(RuntimeError::internal(format!(
                        "step '{}': kind 'local' must not have an 'operation' field",
                        step.id
                    )));
                }
                // Local actions do not participate in the executor's
                // flow-control machinery that these flags gate. A workflow
                // that sets any of them would silently ignore the author's
                // intent. Reject early with a clear error instead.
                for (flag, value) in [
                    ("confirm", step.confirm),
                    ("is_optional", step.is_optional),
                    ("continue_on_failure", step.continue_on_failure),
                    ("skip_if_exists", step.skip_if_exists),
                ] {
                    if value {
                        let rationale = if flag == "skip_if_exists" {
                            " (it responds to HTTP 409, which local actions do not produce)"
                        } else {
                            ""
                        };
                        return Err(RuntimeError::internal(format!(
                            "step '{}': '{flag}' is not supported on kind 'local' steps{rationale}",
                            step.id
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Rule 2: every `dependencies` entry references an earlier step in array
/// order. Forward references rejected.
fn validate_dependencies(definition: &WorkflowDefinition) -> Result<(), RuntimeError> {
    let positions: BTreeMap<&str, usize> = definition
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();
    for (i, step) in definition.steps.iter().enumerate() {
        for dep in &step.dependencies {
            match positions.get(dep.as_str()) {
                Some(&j) if j < i => continue,
                Some(_) => {
                    return Err(RuntimeError::internal(format!(
                        "step '{}' depends on '{}' which is not declared earlier",
                        step.id, dep
                    )))
                }
                None => {
                    return Err(RuntimeError::internal(format!(
                        "step '{}' depends on unknown step '{}'",
                        step.id, dep
                    )))
                }
            }
        }
    }
    Ok(())
}

/// Rule 3: every `from: step/X, output: Y` reference targets a step that
/// (a) exists, (b) was declared earlier, (c) declares an `outputs.Y` entry.
fn validate_step_output_references(definition: &WorkflowDefinition) -> Result<(), RuntimeError> {
    let positions: BTreeMap<&str, usize> = definition
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();
    let outputs_by_step: BTreeMap<&str, BTreeSet<&str>> = definition
        .steps
        .iter()
        .map(|s| {
            (
                s.id.as_str(),
                s.outputs.iter().map(|o| o.name.as_str()).collect(),
            )
        })
        .collect();
    for (i, step) in definition.steps.iter().enumerate() {
        for binding in &step.inputs {
            if let BindingSource::Reference(r) = &binding.source {
                if let ReferenceTarget::Step { id } = &r.from {
                    let Some(&j) = positions.get(id.as_str()) else {
                        return Err(RuntimeError::internal(format!(
                            "step '{}' input '{}' references unknown step '{}'",
                            step.id, binding.field, id
                        )));
                    };
                    if j >= i {
                        return Err(RuntimeError::internal(format!(
                            "step '{}' input '{}' references step '{}' which is not declared earlier",
                            step.id, binding.field, id
                        )));
                    }
                    let Some(output_name) = r.output.as_deref() else {
                        return Err(RuntimeError::internal(format!(
                            "step '{}' input '{}' references step '{}' without naming an output",
                            step.id, binding.field, id
                        )));
                    };
                    let outputs = outputs_by_step
                        .get(id.as_str())
                        .expect("positions and outputs_by_step share keys");
                    if !outputs.contains(output_name) {
                        return Err(RuntimeError::internal(format!(
                            "step '{}' input '{}' references undeclared output '{}' of step '{}'",
                            step.id, binding.field, output_name, id
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Rule 4: every `from: workflow/X` reference targets a declared workflow
/// input, and `output:` is absent (workflow inputs have no outputs).
fn validate_workflow_references(definition: &WorkflowDefinition) -> Result<(), RuntimeError> {
    let declared: BTreeSet<&str> = definition.inputs.iter().map(|i| i.name.as_str()).collect();
    for step in &definition.steps {
        for binding in &step.inputs {
            if let BindingSource::Reference(r) = &binding.source {
                if let ReferenceTarget::Workflow { input } = &r.from {
                    if !declared.contains(input.as_str()) {
                        return Err(RuntimeError::internal(format!(
                            "step '{}' input '{}' references undeclared workflow input '{}'",
                            step.id, binding.field, input
                        )));
                    }
                    if r.output.is_some() {
                        return Err(RuntimeError::internal(format!(
                            "step '{}' input '{}' may not name an `output:` when targeting a workflow input",
                            step.id, binding.field
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Rule 4b: every `{placeholder}` in a Format template must reference a
/// declared workflow input by name.
fn validate_format_bindings(definition: &WorkflowDefinition) -> Result<(), RuntimeError> {
    let declared: BTreeSet<&str> = definition.inputs.iter().map(|i| i.name.as_str()).collect();
    for step in &definition.steps {
        for binding in &step.inputs {
            if let BindingSource::Format(format) = &binding.source {
                let placeholders = format_placeholders(&format.template)?;
                for name in &placeholders {
                    if !declared.contains(name.as_str()) {
                        return Err(RuntimeError::internal(format!(
                            "Format template '{}' references undeclared workflow input '{}'",
                            format.template, name,
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

/// True if `path` is a strict prefix of `other` at a segment boundary (dot
/// or bracket), e.g. `"localizations.en-US"` is a prefix of
/// `"localizations.en-US.title"` but not of `"localizations.en-US2"`.
fn is_path_prefix(path: &str, other: &str) -> bool {
    other.len() > path.len()
        && other.starts_with(path)
        && matches!(other.as_bytes()[path.len()], b'.' | b'[')
}

/// Rule 4d: every Mirror binding targets a *different* field that is bound in
/// the same step by a non-Mirror source (single hop — no chains, no cycles),
/// and a Mirror binding never claims its own review row — the target field
/// is the sole edit surface, so a mirror marked `show_in_review: true` would
/// silently diverge from it on edit (the exact desync class this binding
/// exists to prevent).
fn validate_mirror_bindings(definition: &WorkflowDefinition) -> Result<(), RuntimeError> {
    for step in &definition.steps {
        let sources_by_field: BTreeMap<&str, &BindingSource> = step
            .inputs
            .iter()
            .map(|b| (b.field.as_str(), &b.source))
            .collect();
        for binding in &step.inputs {
            let BindingSource::Mirror(mirror) = &binding.source else {
                continue;
            };
            if mirror.mirror_of == binding.field {
                return Err(RuntimeError::internal(format!(
                    "step '{}' input '{}' mirrors itself",
                    step.id, binding.field
                )));
            }
            if binding.show_in_review {
                return Err(RuntimeError::internal(format!(
                    "step '{}' input '{}' is a mirror binding and must not set \
                     show_in_review (the mirrored target field owns the review \
                     row; edit it there instead)",
                    step.id, binding.field
                )));
            }
            match sources_by_field.get(mirror.mirror_of.as_str()) {
                None => {
                    let hint = step.inputs.iter().map(|b| b.field.as_str()).find(|field| {
                        is_path_prefix(mirror.mirror_of.as_str(), field)
                            || is_path_prefix(field, mirror.mirror_of.as_str())
                    });
                    return Err(RuntimeError::internal(match hint {
                        Some(candidate) => format!(
                            "step '{}' input '{}' mirrors '{}', which is not bound in \
                             that step (did you mean '{}'?)",
                            step.id, binding.field, mirror.mirror_of, candidate
                        ),
                        None => format!(
                            "step '{}' input '{}' mirrors '{}', which is not bound in that step",
                            step.id, binding.field, mirror.mirror_of
                        ),
                    }));
                }
                Some(BindingSource::Mirror(_)) => {
                    return Err(RuntimeError::internal(format!(
                        "step '{}' input '{}' mirrors '{}', which is itself a mirror; \
                         mirrors must target a non-mirror binding",
                        step.id, binding.field, mirror.mirror_of
                    )));
                }
                Some(_) => {}
            }
        }
    }
    Ok(())
}

/// Rule: a `completion`, when present, must (1) be on a workflow with at least
/// two steps (a 1-step run emits `Service`/`BinaryWritten`, never `Workflow`, so
/// a completion could never surface), (2) have at least one non-empty section
/// (an empty-empty completion recreates the blank panel this feature exists to
/// fix), and (3) reference only declared inputs in its templates.
fn validate_completion(definition: &WorkflowDefinition) -> Result<(), RuntimeError> {
    let Some(completion) = &definition.completion else {
        return Ok(());
    };
    if definition.steps.len() < 2 {
        return Err(RuntimeError::internal(format!(
            "workflow '{}' declares a completion, which requires at least two steps; \
             single-step workflows do not surface one",
            definition.id.as_str()
        )));
    }
    if completion.created.is_empty() && completion.next_steps.is_empty() {
        return Err(RuntimeError::internal(format!(
            "workflow '{}' declares a completion with no content; \
             at least one of `created` or `next_steps` must be non-empty",
            definition.id.as_str()
        )));
    }
    let declared: BTreeSet<&str> = definition.inputs.iter().map(|i| i.name.as_str()).collect();
    let templates = completion
        .created
        .iter()
        .map(|r| &r.value)
        .chain(completion.next_steps.iter().map(|s| &s.command));
    for template in templates {
        for name in &format_placeholders(template)? {
            if !declared.contains(name.as_str()) {
                return Err(RuntimeError::internal(format!(
                    "workflow '{}' completion references undeclared workflow input '{}'",
                    definition.id.as_str(),
                    name
                )));
            }
        }
    }
    Ok(())
}

/// Rule 4c: every Arithmetic transform requires that (a) the source binding
/// targets a workflow input, (b) that input's schema type is `"integer"`, and
/// (c) if the operand is a `WorkflowInput`, that input also has type `"integer"`.
fn validate_arithmetic_bindings(definition: &WorkflowDefinition) -> Result<(), RuntimeError> {
    let declared_input_specs: BTreeMap<&str, &WorkflowInputSpec> = definition
        .inputs
        .iter()
        .map(|i| (i.name.as_str(), i))
        .collect();

    for step in &definition.steps {
        for binding in &step.inputs {
            if let BindingSource::Reference(reference) = &binding.source {
                if let Some(TransformKind::Arithmetic(arith)) = &reference.transform {
                    let ReferenceTarget::Workflow { input: src_name } = &reference.from else {
                        return Err(RuntimeError::internal(format!(
                            "Arithmetic transform on field '{}' requires a workflow-input source (step references not supported)",
                            binding.field,
                        )));
                    };
                    let src_input =
                        declared_input_specs.get(src_name.as_str()).ok_or_else(|| {
                            RuntimeError::internal(format!(
                                "Arithmetic transform on field '{}' references undeclared workflow input '{}'",
                                binding.field, src_name,
                            ))
                        })?;
                    if !is_integer_schema(&src_input.schema) {
                        return Err(RuntimeError::internal(format!(
                            "Arithmetic transform on field '{}' requires an integer-typed source input ('{}' is {})",
                            binding.field,
                            src_name,
                            schema_type_name(&src_input.schema),
                        )));
                    }
                    if let ArithmeticOperand::WorkflowInput(op_name) = &arith.operand {
                        let op_input = declared_input_specs
                            .get(op_name.as_str())
                            .ok_or_else(|| {
                                RuntimeError::internal(format!(
                                    "Arithmetic transform on field '{}' references undeclared workflow input '{}' as operand",
                                    binding.field, op_name,
                                ))
                            })?;
                        if !is_integer_schema(&op_input.schema) {
                            return Err(RuntimeError::internal(format!(
                                "Arithmetic transform on field '{}' requires an integer-typed operand input ('{}' is {})",
                                binding.field,
                                op_name,
                                schema_type_name(&op_input.schema),
                            )));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// True when the JSON schema declares `type: integer`.
fn is_integer_schema(schema: &Option<serde_json::Value>) -> bool {
    schema
        .as_ref()
        .and_then(|s| s.get("type"))
        .and_then(|t| t.as_str())
        == Some("integer")
}

/// Return the schema's declared `type`, or `<unspecified>` when none is present.
fn schema_type_name(schema: &Option<serde_json::Value>) -> String {
    schema
        .as_ref()
        .and_then(|s| s.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("<unspecified>")
        .into()
}

/// Reject any workflow that declares `sensitive: true` on an input, a literal
/// binding, a step output capture, or a workflow output alias. End-to-end
/// masking of sensitive values is not yet implemented; until it is, a
/// sensitive-bearing workflow would leak the value into step summaries,
/// scrollback, and resolution traces. Failing compilation makes the leak
/// impossible. The synthesised single-command path and the built-in
/// workflows all declare `sensitive: false`, so this guard never fires for
/// shipped behaviour.
fn validate_no_sensitive(definition: &WorkflowDefinition) -> Result<(), RuntimeError> {
    let has_sensitive = definition.inputs.iter().any(|i| i.sensitive)
        || definition.outputs.iter().any(|o| o.sensitive)
        || definition.steps.iter().any(|step| {
            step.outputs.iter().any(|o| o.sensitive)
                || step.inputs.iter().any(|binding| {
                    matches!(
                        &binding.source,
                        BindingSource::Literal(literal) if literal.sensitive
                    )
                })
        });
    if has_sensitive {
        return Err(RuntimeError::internal(format!(
            "workflow '{}' declares a sensitive value, which is not yet supported",
            definition.id.as_str()
        )));
    }
    Ok(())
}

/// Every step that may be skipped — user-optional (`is_optional`) or
/// failure-tolerant (`continue_on_failure`) — must give each declared output a
/// `default`, so `bind_skipped_outputs` can always resolve downstream
/// references when the step does not run.
fn validate_skippable_outputs_have_defaults(def: &WorkflowDefinition) -> Result<(), RuntimeError> {
    for step in &def.steps {
        if !(step.is_optional || step.continue_on_failure || step.skip_if_exists) {
            continue;
        }
        for out in &step.outputs {
            if out.default.is_none() {
                return Err(RuntimeError::internal(format!(
                    "step '{}' is skippable but its output '{}' has no default",
                    step.id, out.name
                )));
            }
        }
    }
    Ok(())
}

/// Rule 6: no cycles in step output → input chains. Under array-order
/// execution this is structurally impossible (forward refs already rejected
/// by rule 3), but the check exists so a future YAML loader / DAG executor
/// can reuse it. No-op for v2.
fn validate_no_cycles(_definition: &WorkflowDefinition) -> Result<(), RuntimeError> {
    Ok(())
}

/// Rule 5: every declared workflow input has a resolvable schema. Resolution
/// order: (i) author override, (ii) all use-site OpenAPI schemas agree, or
/// (iii) inferred from a `default:`'s JSON type. Errors otherwise.
fn resolve_input_schemas(
    definition: &WorkflowDefinition,
    catalogue: &mut Catalogue,
) -> Result<Vec<WorkflowInputSpec>, RuntimeError> {
    let mut out = Vec::with_capacity(definition.inputs.len());
    for input in &definition.inputs {
        if input.schema.is_some() {
            out.push(input.clone());
            continue;
        }
        let candidates = collect_use_site_schemas(input, definition, catalogue)?;
        let resolved_schema = reconcile_use_site_schemas(&input.name, &candidates)?;
        if let Some(schema) = resolved_schema {
            let mut resolved = input.clone();
            resolved.schema = Some(schema);
            out.push(resolved);
            continue;
        }
        if let Some(default) = &input.default {
            let mut resolved = input.clone();
            resolved.schema = Some(infer_schema_from_value(default));
            out.push(resolved);
            continue;
        }
        // A Format template interpolates its placeholders into a string, so an
        // input used only as a `{name}` reference has no OpenAPI use site but is
        // still well-typed: it is collected as a string. This is a last-resort
        // fallback below use-site agreement and default inference, so an input
        // also bound to a typed field keeps that field's schema.
        if input_referenced_by_format(input, definition)? {
            let mut resolved = input.clone();
            resolved.schema = Some(serde_json::json!({"type": "string"}));
            out.push(resolved);
            continue;
        }
        return Err(RuntimeError::internal(format!(
            "workflow input '{}' has no resolvable schema; bind it to a step field, declare a schema, or provide a default",
            input.name
        )));
    }
    Ok(out)
}

/// Whether any step binds a field via a Format template that references the
/// named workflow input as a `{name}` placeholder. Templates are already
/// validated well-formed and placeholder-declared by `validate_format_bindings`
/// (which runs before schema resolution), so `format_placeholders` cannot error
/// here.
fn input_referenced_by_format(
    input: &WorkflowInputSpec,
    definition: &WorkflowDefinition,
) -> Result<bool, RuntimeError> {
    for step in &definition.steps {
        for binding in &step.inputs {
            if let BindingSource::Format(format) = &binding.source {
                if format_placeholders(&format.template)?
                    .iter()
                    .any(|name| name == &input.name)
                {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

/// Gather every use-site OpenAPI schema for the named workflow input.
/// A use site is any step field bound via `from: workflow/<name>` or any
/// step field that auto-binds (same field name, no explicit binding).
fn collect_use_site_schemas(
    input: &WorkflowInputSpec,
    definition: &WorkflowDefinition,
    catalogue: &mut Catalogue,
) -> Result<Vec<serde_json::Value>, RuntimeError> {
    let mut sites = Vec::new();
    for step in &definition.steps {
        // Local steps have no operation — skip schema collection.
        let op_ref = match &step.operation {
            Some(r) => r,
            None => continue,
        };
        let service_schema = catalogue.get_or_load(op_ref.service.as_str())?;
        let operation =
            find_operation_or_error(service_schema, op_ref, &format!("step '{}'", step.id))?;

        let mut explicitly_bound_fields = BTreeSet::new();
        for binding in &step.inputs {
            explicitly_bound_fields.insert(binding.field.as_str());
            if let BindingSource::Reference(r) = &binding.source {
                if let ReferenceTarget::Workflow { input: target } = &r.from {
                    if target == &input.name {
                        if let Some(schema) =
                            crate::runtime::workflows::auto_derive::operation_field_schema(
                                operation,
                                &binding.field,
                            )
                        {
                            sites.push(schema);
                        }
                    }
                }
            }
        }
        if !explicitly_bound_fields.contains(input.name.as_str()) {
            if let Some(schema) = crate::runtime::workflows::auto_derive::operation_field_schema(
                operation,
                &input.name,
            ) {
                sites.push(schema);
            }
        }
    }
    Ok(sites)
}

/// Pick one schema from the collected use sites, requiring agreement. Empty
/// list → `None` (caller falls through to default-based inference).
fn reconcile_use_site_schemas(
    input_name: &str,
    candidates: &[serde_json::Value],
) -> Result<Option<serde_json::Value>, RuntimeError> {
    let Some(first) = candidates.first() else {
        return Ok(None);
    };
    for other in &candidates[1..] {
        if other != first {
            return Err(RuntimeError::internal(format!(
                "workflow input '{input_name}' has conflicting schemas across use sites; declare an explicit `schema:` to disambiguate"
            )));
        }
    }
    Ok(Some(first.clone()))
}

/// Build a JSON schema fragment from the JSON type of a literal value. Used
/// when an input has no use site but does have a `default:`.
fn infer_schema_from_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Null => serde_json::json!({"type": "null"}),
        serde_json::Value::Bool(_) => serde_json::json!({"type": "boolean"}),
        serde_json::Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                serde_json::json!({"type": "integer"})
            } else {
                serde_json::json!({"type": "number"})
            }
        }
        serde_json::Value::String(_) => serde_json::json!({"type": "string"}),
        serde_json::Value::Array(_) => serde_json::json!({"type": "array"}),
        serde_json::Value::Object(_) => serde_json::json!({"type": "object"}),
    }
}

/// Rule B3a: for every step binding whose field path has more than one segment,
/// resolve the path against the operation's body schema. A typed path must
/// reach a declared leaf; a free-form ancestor falls back to structural-only
/// (path well-formedness already guaranteed by `FieldPath::parse`).
fn validate_nested_paths(
    definition: &WorkflowDefinition,
    catalogue: &mut Catalogue,
) -> Result<(), RuntimeError> {
    for step in &definition.steps {
        // Local steps have no operation — skip nested-path validation.
        let op_ref = match &step.operation {
            Some(r) => r,
            None => continue,
        };
        let service_schema = catalogue.get_or_load(op_ref.service.as_str())?;
        let operation =
            find_operation_or_error(service_schema, op_ref, &format!("step '{}'", step.id))?;
        let root_fields = operation
            .request_body
            .as_ref()
            .map(|b| b.fields.as_slice())
            .unwrap_or(&[]);

        for binding in &step.inputs {
            let path = FieldPath::parse(&binding.field).map_err(|e| {
                RuntimeError::internal(format!(
                    "Binding field '{}' is not a valid path: {}",
                    binding.field, e,
                ))
            })?;
            if path.segments.len() > 1 {
                match path.resolve_against_schema(root_fields) {
                    Ok(PathResolution::SchemaLeaf(_)) | Ok(PathResolution::StructuralOnly) => {}
                    Err(e) => {
                        return Err(RuntimeError::internal(format!(
                            "Binding field '{}' could not be resolved against the operation schema: {}",
                            binding.field, e,
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Rule B3b: within each step, every concrete leaf path may be written by at
/// most one binding. A `Literal` source claims every leaf inside its JSON
/// value (prefixed by the binding's field); any other source claims only the
/// binding's field itself.
fn validate_leaf_collisions(definition: &WorkflowDefinition) -> Result<(), RuntimeError> {
    for step in &definition.steps {
        let mut claimed: BTreeMap<String, String> = BTreeMap::new();
        for binding in &step.inputs {
            for leaf in binding_claimed_leaves(binding) {
                if let Some(existing) = claimed.insert(leaf.clone(), binding.field.clone()) {
                    return Err(RuntimeError::internal(format!(
                        "Step '{}': leaf '{}' is bound by both binding '{}' and binding '{}' — each leaf must be bound at most once across literal-base and nested bindings",
                        step.id, leaf, existing, binding.field,
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Enumerate every concrete leaf path a binding writes to. For a `Literal`
/// source, that is every leaf inside the literal value prefixed with the
/// binding's field path. For any other source, it is just the binding's
/// field path itself.
fn binding_claimed_leaves(binding: &ags_protocol::workflow::StepInputBinding) -> Vec<String> {
    let prefix = binding.field.clone();
    match &binding.source {
        BindingSource::Literal(lit) => {
            let mut out = Vec::new();
            walk_value_leaves(&lit.value, &prefix, &mut out);
            if out.is_empty() {
                out.push(prefix);
            }
            out
        }
        _ => vec![prefix],
    }
}

/// Rule: every input declaring an `options_source` must be safe to surface as a
/// runtime-fetched picker. A broken `options_source` fails hard here so it can
/// never ship silently degrading to free text. Errors are `RuntimeError::internal`
/// (consistent with the other reference rules).
fn validate_options_sources(
    definition: &WorkflowDefinition,
    resolved_inputs: &[WorkflowInputSpec],
    catalogue: &mut Catalogue,
) -> Result<(), RuntimeError> {
    use crate::runtime::workflows::auto_derive::operation_field_schema;
    use crate::runtime::workflows::jsonpath::jsonpath_is_valid;
    use ags_protocol::catalogue::HttpMethod;
    use ags_protocol::workflow::OptionParameterBinding;

    let declared: BTreeSet<&str> = definition.inputs.iter().map(|i| i.name.as_str()).collect();

    for input in resolved_inputs {
        let Some(source) = &input.options_source else {
            continue;
        };

        // Rule 1: target input is string-typed.
        let is_string = input
            .schema
            .as_ref()
            .and_then(|s| s.get("type"))
            .and_then(|t| t.as_str())
            == Some("string");
        if !is_string {
            return Err(RuntimeError::internal(format!(
                "input '{}' declares an options_source but is not string-typed; \
                 options_source attaches only to {{\"type\":\"string\"}} inputs",
                input.name
            )));
        }

        // Rule 2: operation resolves to a contract in the catalogue.
        let service_schema = catalogue.get_or_load(source.operation.service.as_str())?;
        let operation = find_operation_or_error(
            service_schema,
            &source.operation,
            &format!("input '{}' options_source", input.name),
        )?;

        // Rule 2a: operation is read-only (GET).
        if operation.http_method != HttpMethod::Get {
            return Err(RuntimeError::internal(format!(
                "input '{}' options_source operation '{}' is {}, not GET; \
                 a dynamic-enum fetch must never run a mutating operation",
                input.name,
                source.operation.operation.as_str(),
                operation.http_method.as_str()
            )));
        }

        // Rules 3 / 3a / 4: parameter bindings.
        for (param_name, binding) in &source.parameters {
            let field_schema = operation_field_schema(operation, param_name).ok_or_else(|| {
                RuntimeError::internal(format!(
                    "input '{}' options_source binds unknown parameter '{}' on operation '{}'",
                    input.name,
                    param_name,
                    source.operation.operation.as_str()
                ))
            })?;
            match binding {
                OptionParameterBinding::Literal(value) => {
                    if !literal_matches_schema_type(value, &field_schema) {
                        return Err(RuntimeError::internal(format!(
                            "input '{}' options_source Literal for parameter '{}' is type-incompatible \
                             with the operation field schema",
                            input.name, param_name
                        )));
                    }
                }
                OptionParameterBinding::FromInput(name)
                | OptionParameterBinding::FromInputOptional(name) => {
                    if !declared.contains(name.as_str()) {
                        return Err(RuntimeError::internal(format!(
                            "input '{}' options_source FromInput('{}') references an undeclared workflow input",
                            input.name, name
                        )));
                    }
                }
            }
        }

        // Rule 5: projection paths parse in the subset.
        for (label, path) in [
            ("items_path", source.items_path.as_str()),
            ("value", source.value.as_str()),
        ] {
            if !jsonpath_is_valid(path) {
                return Err(RuntimeError::internal(format!(
                    "input '{}' options_source {label} '{}' is not a valid JSONPath subset expression",
                    input.name, path
                )));
            }
        }
        if let Some(label_path) = &source.label {
            if !jsonpath_is_valid(label_path) {
                return Err(RuntimeError::internal(format!(
                    "input '{}' options_source label '{}' is not a valid JSONPath subset expression",
                    input.name, label_path
                )));
            }
        }
        if let Some(filter) = &source.filter {
            if !jsonpath_is_valid(&filter.path) {
                return Err(RuntimeError::internal(format!(
                    "input '{}' options_source filter path '{}' is not a valid JSONPath subset expression",
                    input.name, filter.path
                )));
            }
        }

        // Rule 5a: `items_path` must be either the root `$` (the response body IS
        // the array — non-paginated list endpoints that return a top-level array,
        // e.g. `platform/admin/stores/v1/list`) or a single-segment `$.<key>`
        // path. The options paginator (`fetch_all_pages_at_path`) merges paginated
        // results only by a top-level array key; a nested `items_path` (e.g.
        // `$.data.images`) would work on page 1 but silently stop aggregating, and
        // a root array is non-paginated so needs no merge. Reject anything else at
        // author time rather than degrading silently.
        let root_array = source.items_path == "$";
        let single_segment = source
            .items_path
            .strip_prefix("$.")
            .map(|k| !k.is_empty() && !k.contains(['.', '[']))
            .unwrap_or(false);
        if !root_array && !single_segment {
            return Err(RuntimeError::internal(format!(
                "input '{}' options_source items_path '{}' must be `$` (root array) or a \
                 single-segment path like `$.images` (nested item-array paths are not \
                 supported in v1)",
                input.name, source.items_path
            )));
        }

        // Rule 6 (best-effort scalar value path): response schemas are commonly
        // under-described in the bundled specs, so deep static scalar checking is
        // intentionally not attempted here. The runtime projection already skips
        // non-scalar elements; this hook is documented for a future enhancement.
    }
    Ok(())
}

/// Rule: an input declaring a `file_picker` must be a coherent, safe local-
/// file browsing affordance. Errors are `RuntimeError::internal`, matching
/// `validate_options_sources`'s convention — a broken declaration fails hard
/// at compile time rather than shipping silently degrading to free text.
fn validate_file_pickers(resolved_inputs: &[WorkflowInputSpec]) -> Result<(), RuntimeError> {
    for input in resolved_inputs {
        let Some(picker) = &input.file_picker else {
            continue;
        };

        // Rule 1: target input is string-typed (same rule as options_source).
        let is_string = input
            .schema
            .as_ref()
            .and_then(|s| s.get("type"))
            .and_then(|t| t.as_str())
            == Some("string");
        if !is_string {
            return Err(RuntimeError::internal(format!(
                "input '{}' declares a file_picker but is not string-typed; \
                 file_picker attaches only to {{\"type\":\"string\"}} inputs",
                input.name
            )));
        }

        // Rule 2: mutually exclusive with options_source — both are alternate
        // value-resolution mechanisms for a string field.
        if input.options_source.is_some() {
            return Err(RuntimeError::internal(format!(
                "input '{}' declares both a file_picker and an options_source; \
                 these are alternate value-resolution mechanisms and cannot combine",
                input.name
            )));
        }

        // Rule 3: extensions, if present, are well-formed.
        if let Some(extensions) = &picker.extensions {
            for ext in extensions {
                if ext.is_empty() || ext.starts_with('.') {
                    return Err(RuntimeError::internal(format!(
                        "input '{}' file_picker extension '{}' must be non-empty and \
                         must not start with '.' (write \"png\", not \".png\")",
                        input.name, ext
                    )));
                }
            }
        }

        // Rule 4: start_dir, if present, must be non-empty. Existence on disk
        // is deliberately NOT checked here — see the type's own doc comment.
        if let Some(start_dir) = &picker.start_dir {
            if start_dir.is_empty() {
                return Err(RuntimeError::internal(format!(
                    "input '{}' file_picker start_dir must not be empty when present",
                    input.name
                )));
            }
        }
    }
    Ok(())
}

/// Whether a `Literal` value's JSON type is compatible with an operation field
/// schema's declared `type`. Permissive when the schema has no `type`.
fn literal_matches_schema_type(value: &serde_json::Value, schema: &serde_json::Value) -> bool {
    let Some(expected) = schema.get("type").and_then(|t| t.as_str()) else {
        return true;
    };
    match expected {
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true,
    }
}

/// Collect the dotted path to every leaf (scalar or empty container) in a JSON
/// value into `out`, used to enumerate nested fields for validation.
fn walk_value_leaves(value: &serde_json::Value, path: &str, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if map.is_empty() {
                out.push(path.into());
                return;
            }
            for (k, v) in map {
                let child = format!("{path}.{k}");
                walk_value_leaves(v, &child, out);
            }
        }
        serde_json::Value::Array(arr) => {
            if arr.is_empty() {
                out.push(path.into());
                return;
            }
            for (i, v) in arr.iter().enumerate() {
                let child = format!("{path}[{i}]");
                walk_value_leaves(v, &child, out);
            }
        }
        _ => out.push(path.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::catalogue::{
        ApiVersion, BodyField, BodyFieldType, BodySchema, HttpMethod, MethodSchema, MutationClass,
        OperationId, OperationSchema, ParameterLocation, ParameterSchema, ResourceSchema,
        ScopeEntry, ServiceId, ValueType,
    };
    use ags_protocol::workflow::{
        ArithmeticOp, ArithmeticOperand, ArithmeticTransform, BindingSource, CaptureSource,
        FormatBinding, OperationReference, ReferenceBinding, ReferenceTarget, StepInputBinding,
        StepOutputCapture, TransformKind, WorkflowBriefing, WorkflowId,
    };

    use crate::catalogue::Catalogue;

    // ---------------------------------------------------------------------------
    // Test helpers
    // ---------------------------------------------------------------------------

    /// Build a default `OperationSchema` fixture.
    fn default_operation(id: &str) -> OperationSchema {
        OperationSchema {
            id: OperationId::new(id),
            name: id.into(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
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

    /// Build a `ServiceSchema` fixture wrapping the given operation.
    fn service_with(ops: Vec<OperationSchema>) -> ags_protocol::catalogue::ServiceSchema {
        let methods: Vec<MethodSchema> = ops
            .into_iter()
            .map(|op| MethodSchema {
                name: op.name.clone(),
                summary: String::new(),
                default_scope: Some(op.scope.clone()),
                scopes: vec![ScopeEntry {
                    scope: op.scope.clone(),
                    default_version: op.api_version,
                    contracts: vec![op],
                }],
            })
            .collect();
        ags_protocol::catalogue::ServiceSchema {
            name: "svc".into(),
            description: String::new(),
            resources: vec![ResourceSchema {
                name: "res".into(),
                description: String::new(),
                methods,
            }],
        }
    }

    /// Build an `OperationReference` fixture.
    fn op_ref(op_id: &str) -> OperationReference {
        OperationReference {
            service: ServiceId::new("svc"),
            operation: OperationId::new(op_id),
        }
    }

    /// Build a minimal `StepDefinition` fixture.
    fn simple_step(id: &str, op_id: &str) -> StepDefinition {
        StepDefinition {
            id: id.into(),
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(op_ref(op_id)),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: vec![],
            outputs: vec![],
        }
    }

    /// Build a minimal `WorkflowDefinition` fixture.
    fn simple_definition(steps: Vec<StepDefinition>) -> WorkflowDefinition {
        WorkflowDefinition {
            id: WorkflowId::new("wf"),
            name: "Test Workflow".into(),
            workflow_protocol_version: None,
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps,
            outputs: vec![],
            completion: None,
        }
    }

    /// Build a `Catalogue` seeded with one service schema.
    fn catalogue_with_svc(schema: ags_protocol::catalogue::ServiceSchema) -> Catalogue {
        let mut cat = Catalogue::new();
        cat.insert_for_tests("svc", schema);
        cat
    }

    /// Build a string `BodyField` fixture.
    fn body_field(name: &str, required: bool) -> BodyField {
        BodyField {
            name: name.into(),
            field_type: BodyFieldType::String,
            required,
            description: None,
            children: vec![],
            default: None,
        }
    }

    /// Build an integer `BodyField` fixture.
    fn integer_body_field(name: &str, required: bool) -> BodyField {
        BodyField {
            name: name.into(),
            field_type: BodyFieldType::Integer,
            required,
            description: None,
            children: vec![],
            default: None,
        }
    }

    /// Build a workflow-input reference binding fixture.
    fn workflow_ref_binding(field: &str, input: &str) -> StepInputBinding {
        StepInputBinding {
            field: field.into(),
            source: BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: input.into(),
                },
                output: None,
                transform: None,
            }),
            show_in_review: false,
            description: None,
        }
    }

    /// Build a step-output reference binding fixture.
    fn step_ref_binding(field: &str, step_id: &str, output: &str) -> StepInputBinding {
        StepInputBinding {
            field: field.into(),
            source: BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Step { id: step_id.into() },
                output: Some(output.into()),
                transform: None,
            }),
            show_in_review: false,
            description: None,
        }
    }

    /// Build a mirror binding fixture (`field` copies `target`).
    fn mirror_binding(field: &str, target: &str) -> StepInputBinding {
        StepInputBinding {
            field: field.into(),
            source: BindingSource::Mirror(ags_protocol::workflow::MirrorBinding {
                mirror_of: target.into(),
            }),
            show_in_review: false,
            description: None,
        }
    }

    /// Build a `WorkflowInputSpec` fixture.
    fn workflow_input(name: &str) -> WorkflowInputSpec {
        WorkflowInputSpec {
            name: name.into(),
            description: None,
            schema: None,
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        }
    }

    /// Build a `StepDefinition` fixture with the given input bindings.
    fn step_with_bindings(
        op: &ags_protocol::catalogue::OperationSchema,
        bindings: Vec<StepInputBinding>,
    ) -> StepDefinition {
        StepDefinition {
            id: "step1".into(),
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(op_ref(op.id.as_str())),
            dependencies: vec![],
            confirm: false,
            is_optional: false,
            continue_on_failure: false,
            skip_if_exists: false,
            is_reviewed: None,
            inputs: bindings,
            outputs: vec![],
        }
    }

    /// Build a simple JSON schema fixture.
    fn simple_schema(
        op: ags_protocol::catalogue::OperationSchema,
    ) -> ags_protocol::catalogue::ServiceSchema {
        service_with(vec![op])
    }

    // ---------------------------------------------------------------------------
    // Tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_unique_step_ids() {
        let op = default_operation("Op");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let def = WorkflowDefinition {
            steps: vec![simple_step("s1", "Op"), simple_step("s1", "Op")],
            ..simple_definition(vec![])
        };
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("duplicate step id"),
            "expected 'duplicate step id' in: {}",
            err.message
        );
    }

    #[test]
    fn test_dependencies_must_target_existing_earlier_step() {
        let op = default_operation("Op");
        let mut cat = catalogue_with_svc(service_with(vec![op]));

        // Case 1: depends on a step that doesn't exist.
        let mut s1 = simple_step("s1", "Op");
        s1.dependencies = vec!["unknown".into()];
        let def = simple_definition(vec![s1]);
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("unknown step 'unknown'"),
            "expected 'unknown step' in: {}",
            err.message
        );

        // Case 2: s1 depends on s2 which comes after it (forward reference).
        let mut s1 = simple_step("s1", "Op");
        s1.dependencies = vec!["s2".into()];
        let s2 = simple_step("s2", "Op");
        let def = simple_definition(vec![s1, s2]);
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("not declared earlier"),
            "expected 'not declared earlier' in: {}",
            err.message
        );
    }

    #[test]
    fn test_step_output_reference_must_target_declared_output() {
        let op = default_operation("Op");
        let mut cat = catalogue_with_svc(service_with(vec![op]));

        // s2 references output "missing" on s1, but s1 declares no outputs.
        let s1 = simple_step("s1", "Op");
        let mut s2 = simple_step("s2", "Op");
        s2.inputs = vec![step_ref_binding("field", "s1", "missing")];

        let def = simple_definition(vec![s1, s2]);
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("undeclared output 'missing'"),
            "expected 'undeclared output' in: {}",
            err.message
        );
    }

    #[test]
    fn test_workflow_reference_must_target_declared_input() {
        let op = default_operation("Op");
        let mut cat = catalogue_with_svc(service_with(vec![op]));

        // Step references workflow input "X" but workflow declares no inputs.
        let mut s1 = simple_step("s1", "Op");
        s1.inputs = vec![workflow_ref_binding("field", "X")];

        let def = WorkflowDefinition {
            steps: vec![s1],
            inputs: vec![],
            ..simple_definition(vec![])
        };
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("undeclared workflow input 'X'"),
            "expected 'undeclared workflow input' in: {}",
            err.message
        );
    }

    #[test]
    fn test_workflow_input_schema_resolvable_via_use_site() {
        // Operation has a required string param "userId"; workflow input "userId"
        // has no explicit schema — it should be resolved from the use-site.
        let mut op = default_operation("Op");
        op.parameters = vec![ParameterSchema {
            name: "userId".into(),
            location: ParameterLocation::Query,
            required: true,
            value_type: ValueType::String,
            is_file: false,
            description: None,
            default: None,
        }];

        let mut cat = catalogue_with_svc(service_with(vec![op]));

        let def = WorkflowDefinition {
            inputs: vec![workflow_input("userId")],
            steps: vec![simple_step("s1", "Op")],
            ..simple_definition(vec![])
        };
        let compiled = compile_workflow(&def, &mut cat).unwrap();
        let resolved = compiled.inputs.iter().find(|i| i.name == "userId").unwrap();
        assert_eq!(resolved.schema, Some(serde_json::json!({"type": "string"})));
    }

    #[test]
    fn test_workflow_input_schema_resolvable_via_default() {
        // Workflow input "count" has no schema and no use site, but default 42.
        let op = default_operation("Op");
        let mut cat = catalogue_with_svc(service_with(vec![op]));

        let def = WorkflowDefinition {
            inputs: vec![WorkflowInputSpec {
                name: "count".into(),
                description: None,
                schema: None,
                required: false,
                default: Some(serde_json::json!(42)),
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
                file_picker: None,
            }],
            steps: vec![simple_step("s1", "Op")],
            ..simple_definition(vec![])
        };
        let compiled = compile_workflow(&def, &mut cat).unwrap();
        let resolved = compiled.inputs.iter().find(|i| i.name == "count").unwrap();
        assert_eq!(
            resolved.schema,
            Some(serde_json::json!({"type": "integer"}))
        );
    }

    #[test]
    fn test_workflow_input_schema_unresolvable() {
        // Workflow input "x" has no schema, no use site, no default.
        let op = default_operation("Op");
        let mut cat = catalogue_with_svc(service_with(vec![op]));

        let def = WorkflowDefinition {
            inputs: vec![WorkflowInputSpec {
                name: "x".into(),
                description: None,
                schema: None,
                required: true,
                default: None,
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
                file_picker: None,
            }],
            steps: vec![simple_step("s1", "Op")],
            ..simple_definition(vec![])
        };
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("no resolvable schema"),
            "expected 'no resolvable schema' in: {}",
            err.message
        );
    }

    #[test]
    fn test_sensitive_workflow_input_rejected() {
        let op = default_operation("Op");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let def = WorkflowDefinition {
            inputs: vec![WorkflowInputSpec {
                name: "apiKey".into(),
                description: None,
                schema: Some(serde_json::json!({"type": "string"})),
                required: true,
                default: None,
                sensitive: true,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
                file_picker: None,
            }],
            steps: vec![simple_step("s1", "Op")],
            ..simple_definition(vec![])
        };
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("sensitive value"),
            "expected a sensitive-value rejection: {}",
            err.message
        );
    }

    #[test]
    fn test_sensitive_literal_binding_rejected() {
        use ags_protocol::workflow::{BindingSource, LiteralBinding, StepInputBinding};
        let op = default_operation("Op");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut s1 = simple_step("s1", "Op");
        s1.inputs = vec![StepInputBinding {
            field: "token".into(),
            source: BindingSource::Literal(LiteralBinding {
                value: serde_json::json!("secret"),
                sensitive: true,
            }),
            show_in_review: false,
            description: None,
        }];
        let def = simple_definition(vec![s1]);
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("sensitive value"),
            "expected a sensitive-value rejection: {}",
            err.message
        );
    }

    #[test]
    fn test_auto_derive_expansion_required_fields_only() {
        // Operation has one required body field and one optional one.
        let mut op = default_operation("Op");
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![
                body_field("required_field", true),
                body_field("optional_field", false),
            ],
        });

        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let def = simple_definition(vec![simple_step("s1", "Op")]);

        let compiled = compile_workflow(&def, &mut cat).unwrap();
        let step = &compiled.steps[0];
        assert_eq!(step.auto_derived.len(), 1);
        assert_eq!(step.auto_derived[0].field, "required_field");
    }

    #[test]
    fn test_compile_rejects_format_with_unknown_placeholder() {
        let mut op = default_operation("Op");
        op.request_body = Some(ags_protocol::catalogue::BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![body_field("name", true)],
        });
        let step = step_with_bindings(
            &op,
            vec![StepInputBinding {
                field: "name".into(),
                source: BindingSource::Format(FormatBinding {
                    template: "{noSuchInput}-fleet".into(),
                }),
                show_in_review: false,
                description: None,
            }],
        );
        let definition = simple_definition(vec![step]);
        let mut cat = catalogue_with_svc(simple_schema(op));
        let err = compile_workflow(&definition, &mut cat).unwrap_err();
        assert!(
            err.message.contains("noSuchInput"),
            "error must name the unknown placeholder: {}",
            err.message
        );
    }

    #[test]
    fn test_compile_accepts_format_referencing_declared_input() {
        let mut op = default_operation("Op");
        op.request_body = Some(ags_protocol::catalogue::BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![body_field("name", true)],
        });
        let step = step_with_bindings(
            &op,
            vec![StepInputBinding {
                field: "name".into(),
                source: BindingSource::Format(FormatBinding {
                    template: "{resourcePrefix}-fleet".into(),
                }),
                show_in_review: false,
                description: None,
            }],
        );
        let mut definition = simple_definition(vec![step]);
        definition.inputs.push(workflow_input("resourcePrefix"));
        let mut cat = catalogue_with_svc(simple_schema(op));
        compile_workflow(&definition, &mut cat).expect("must compile");
    }

    #[test]
    fn test_compile_rejects_mirror_of_unbound_field() {
        let mut op = default_operation("Op");
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![body_field("name", false), body_field("title", false)],
        });
        let step = step_with_bindings(&op, vec![mirror_binding("name", "title")]);
        let def = simple_definition(vec![step]);
        let mut cat = catalogue_with_svc(simple_schema(op));
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("not bound in that step"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_compile_rejects_mirror_of_mirror() {
        let mut op = default_operation("Op");
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![
                body_field("a", false),
                body_field("b", false),
                body_field("c", false),
            ],
        });
        let step = step_with_bindings(
            &op,
            vec![
                literal_binding("a", serde_json::json!("x")),
                mirror_binding("b", "a"),
                mirror_binding("c", "b"),
            ],
        );
        let def = simple_definition(vec![step]);
        let mut cat = catalogue_with_svc(simple_schema(op));
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("itself a mirror"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_compile_rejects_self_mirror() {
        let mut op = default_operation("Op");
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![body_field("name", false)],
        });
        let step = step_with_bindings(&op, vec![mirror_binding("name", "name")]);
        let def = simple_definition(vec![step]);
        let mut cat = catalogue_with_svc(simple_schema(op));
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("mirrors itself"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_compile_accepts_mirror_of_bound_field() {
        let mut op = default_operation("Op");
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![body_field("name", false), body_field("title", false)],
        });
        let step = step_with_bindings(
            &op,
            vec![
                literal_binding("title", serde_json::json!("Starter Skin")),
                mirror_binding("name", "title"),
            ],
        );
        let def = simple_definition(vec![step]);
        let mut cat = catalogue_with_svc(simple_schema(op));
        compile_workflow(&def, &mut cat).expect("mirror of a bound field must compile");
    }

    #[test]
    fn test_compile_rejects_mirror_of_unbound_field_hints_nested_binding() {
        // Only the leaf `localizations.en-US.title` is bound; the mirror
        // targets one level up (`localizations.en-US`), a mistake an author
        // could plausibly make. The error should point at the real target.
        let mut op = default_operation("Op");
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![
                body_field("name", false),
                // Free-form (no declared children) so nested paths under it
                // resolve StructuralOnly rather than failing schema lookup.
                BodyField {
                    name: "localizations".into(),
                    field_type: BodyFieldType::Object,
                    required: false,
                    description: None,
                    children: vec![],
                    default: None,
                },
            ],
        });
        let step = step_with_bindings(
            &op,
            vec![
                literal_binding(
                    "localizations.en-US.title",
                    serde_json::json!("Starter Skin"),
                ),
                mirror_binding("name", "localizations.en-US"),
            ],
        );
        let def = simple_definition(vec![step]);
        let mut cat = catalogue_with_svc(simple_schema(op));
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message
                .contains("did you mean 'localizations.en-US.title'"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_compile_rejects_mirror_binding_with_show_in_review() {
        let mut op = default_operation("Op");
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![body_field("name", false), body_field("title", false)],
        });
        let mut mirror = mirror_binding("name", "title");
        mirror.show_in_review = true;
        let step = step_with_bindings(
            &op,
            vec![
                literal_binding("title", serde_json::json!("Starter Skin")),
                mirror,
            ],
        );
        let def = simple_definition(vec![step]);
        let mut cat = catalogue_with_svc(simple_schema(op));
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("show_in_review"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_compile_rejects_completion_with_unknown_placeholder() {
        use ags_protocol::workflow::{CompletionStep, WorkflowCompletion};
        let op = default_operation("Op");
        let mut def = simple_definition(vec![simple_step("s1", "Op"), simple_step("s2", "Op")]);
        def.completion = Some(WorkflowCompletion {
            created: vec![],
            next_steps: vec![CompletionStep {
                description: "x".into(),
                command: "ags x --ns {undeclared}".into(),
            }],
        });
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(err.message.contains("undeclared"), "got: {}", err.message);
    }

    #[test]
    fn test_compile_rejects_completion_on_single_step_workflow() {
        use ags_protocol::workflow::{CompletionResource, WorkflowCompletion};
        let op = default_operation("Op");
        let mut def = simple_definition(vec![simple_step("s1", "Op")]);
        def.completion = Some(WorkflowCompletion {
            created: vec![CompletionResource {
                label: "X".into(),
                value: "x".into(),
            }],
            next_steps: vec![],
        });
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("at least two steps"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_compile_rejects_empty_completion() {
        use ags_protocol::workflow::WorkflowCompletion;
        let op = default_operation("Op");
        let mut def = simple_definition(vec![simple_step("s1", "Op"), simple_step("s2", "Op")]);
        def.completion = Some(WorkflowCompletion {
            created: vec![],
            next_steps: vec![],
        });
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(err.message.contains("at least one"), "got: {}", err.message);
    }

    #[test]
    fn test_compile_accepts_valid_completion_and_carries_it() {
        use ags_protocol::workflow::{CompletionResource, WorkflowCompletion};
        let op = default_operation("Op");
        let mut def = simple_definition(vec![simple_step("s1", "Op"), simple_step("s2", "Op")]);
        def.completion = Some(WorkflowCompletion {
            created: vec![CompletionResource {
                label: "X".into(),
                value: "x".into(),
            }],
            next_steps: vec![],
        });
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let compiled = compile_workflow(&def, &mut cat).unwrap();
        assert!(compiled.completion.is_some());
    }

    #[test]
    fn test_cycle_detection_no_op_in_v2() {
        // A clean two-step workflow (s1 then s2, s2 depends on s1) should compile
        // successfully. The no-op cycle validator should not introduce any errors.
        use ags_protocol::workflow::CaptureSource;

        let op1 = default_operation("Op1");
        let op2 = default_operation("Op2");

        let mut cat = catalogue_with_svc(service_with(vec![op1, op2]));

        let mut s1 = simple_step("s1", "Op1");
        s1.outputs = vec![StepOutputCapture {
            name: "result".into(),
            source: CaptureSource::ResponseBody { path: "$".into() },
            default: None,
            sensitive: false,
        }];

        let mut s2 = simple_step("s2", "Op2");
        s2.dependencies = vec!["s1".into()];
        s2.inputs = vec![step_ref_binding("field", "s1", "result")];

        let def = simple_definition(vec![s1, s2]);
        let compiled = compile_workflow(&def, &mut cat).unwrap();
        assert_eq!(compiled.steps.len(), 2);
    }

    #[test]
    fn test_compile_rejects_arithmetic_on_non_integer_input() {
        let mut op = default_operation("Op");
        op.request_body = Some(ags_protocol::catalogue::BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![integer_body_field("minPlayers", true)],
        });
        let step = step_with_bindings(
            &op,
            vec![StepInputBinding {
                field: "minPlayers".into(),
                source: BindingSource::Reference(ReferenceBinding {
                    from: ReferenceTarget::Workflow {
                        input: "namespace".into(),
                    },
                    output: None,
                    transform: Some(TransformKind::Arithmetic(ArithmeticTransform {
                        op: ArithmeticOp::Mul,
                        operand: ArithmeticOperand::Integer(2),
                    })),
                }),
                show_in_review: false,
                description: None,
            }],
        );
        let mut definition = simple_definition(vec![step]);
        definition.inputs.push(WorkflowInputSpec {
            name: "namespace".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        });
        let mut cat = catalogue_with_svc(simple_schema(op));
        let err = compile_workflow(&definition, &mut cat).unwrap_err();
        assert!(
            err.message.to_lowercase().contains("integer"),
            "must mention integer type: {}",
            err.message
        );
        assert!(
            err.message.contains("string"),
            "must mention actual schema type: {}",
            err.message
        );
    }

    #[test]
    fn test_compile_accepts_arithmetic_on_integer_inputs() {
        let mut op = default_operation("Op");
        op.request_body = Some(ags_protocol::catalogue::BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![integer_body_field("minPlayers", true)],
        });
        let step = step_with_bindings(
            &op,
            vec![StepInputBinding {
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
            }],
        );
        let mut definition = simple_definition(vec![step]);
        for name in &["playersPerTeam", "teamCount"] {
            definition.inputs.push(WorkflowInputSpec {
                name: (*name).into(),
                description: None,
                schema: Some(serde_json::json!({"type": "integer"})),
                required: true,
                default: None,
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
                file_picker: None,
            });
        }
        let mut cat = catalogue_with_svc(simple_schema(op));
        compile_workflow(&definition, &mut cat).expect("must compile");
    }

    // ---------------------------------------------------------------------------
    // B3: nested-path validation tests
    // ---------------------------------------------------------------------------

    /// Build an object `BodyField` fixture.
    fn object_body_field(name: &str, required: bool, children: Vec<BodyField>) -> BodyField {
        BodyField {
            name: name.into(),
            field_type: BodyFieldType::Object,
            required,
            description: None,
            children,
            default: None,
        }
    }

    /// Build a literal binding fixture.
    fn literal_binding(field: &str, value: serde_json::Value) -> StepInputBinding {
        use ags_protocol::workflow::LiteralBinding;
        StepInputBinding {
            field: field.into(),
            source: BindingSource::Literal(LiteralBinding {
                value,
                sensitive: false,
            }),
            show_in_review: false,
            description: None,
        }
    }

    #[test]
    fn test_compile_validates_nested_path_against_schema_typed_field() {
        // dsHostConfiguration.instanceId — both segments typed.
        let mut op = default_operation("Op");
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![object_body_field(
                "dsHostConfiguration",
                false,
                vec![body_field("instanceId", false)],
            )],
        });
        let step = step_with_bindings(
            &op,
            vec![workflow_ref_binding(
                "dsHostConfiguration.instanceId",
                "fleetInstanceId",
            )],
        );
        let mut definition = simple_definition(vec![step]);
        definition.inputs.push(WorkflowInputSpec {
            name: "fleetInstanceId".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        });
        let mut cat = catalogue_with_svc(simple_schema(op));
        compile_workflow(&definition, &mut cat).expect("must compile");
    }

    #[test]
    fn test_compile_rejects_nested_path_with_unknown_leaf_under_schema_typed() {
        // dsHostConfiguration is typed with children, but "notReal" is not among them.
        let mut op = default_operation("Op");
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![object_body_field(
                "dsHostConfiguration",
                false,
                vec![body_field("instanceId", false)],
            )],
        });
        let step = step_with_bindings(
            &op,
            vec![workflow_ref_binding(
                "dsHostConfiguration.notReal",
                "fleetInstanceId",
            )],
        );
        let mut definition = simple_definition(vec![step]);
        definition.inputs.push(WorkflowInputSpec {
            name: "fleetInstanceId".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        });
        let mut cat = catalogue_with_svc(simple_schema(op));
        let err = compile_workflow(&definition, &mut cat).unwrap_err();
        assert!(
            err.message.contains("notReal"),
            "error must name the unknown segment: {}",
            err.message
        );
    }

    #[test]
    fn test_compile_allows_structural_path_under_freeform_ancestor() {
        // "data" is an Object with no declared children — free-form.
        let mut op = default_operation("Op");
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![object_body_field("data", false, vec![])],
        });
        let step = step_with_bindings(
            &op,
            vec![workflow_ref_binding(
                "data.matching_rule[0].attribute",
                "statCode",
            )],
        );
        let mut definition = simple_definition(vec![step]);
        definition.inputs.push(WorkflowInputSpec {
            name: "statCode".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        });
        let mut cat = catalogue_with_svc(simple_schema(op));
        compile_workflow(&definition, &mut cat).expect("must compile");
    }

    // ---------------------------------------------------------------------------
    // B3: leaf-collision detection tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_compile_rejects_collision_between_literal_leaf_and_nested_binding() {
        // Literal claims dsHostConfiguration.instanceId; second binding also targets it.
        let mut op = default_operation("Op");
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![object_body_field(
                "dsHostConfiguration",
                false,
                vec![
                    body_field("instanceId", false),
                    integer_body_field("serversPerVm", false),
                ],
            )],
        });
        let step = step_with_bindings(
            &op,
            vec![
                literal_binding(
                    "dsHostConfiguration",
                    serde_json::json!({"instanceId": "x", "serversPerVm": 1}),
                ),
                workflow_ref_binding("dsHostConfiguration.instanceId", "instanceType"),
            ],
        );
        let mut definition = simple_definition(vec![step]);
        definition.inputs.push(WorkflowInputSpec {
            name: "instanceType".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        });
        let mut cat = catalogue_with_svc(simple_schema(op));
        let err = compile_workflow(&definition, &mut cat).unwrap_err();
        assert!(
            err.message.contains("dsHostConfiguration.instanceId"),
            "error must name the colliding leaf: {}",
            err.message
        );
        assert!(
            err.message.to_lowercase().contains("bound"),
            "error must mention 'bound': {}",
            err.message
        );
    }

    #[test]
    fn test_compile_accepts_layered_literal_and_nested_when_leaves_disjoint() {
        // Literal covers only serversPerVm; nested binding covers instanceId — no collision.
        let mut op = default_operation("Op");
        op.request_body = Some(BodySchema {
            item_type: None,
            is_array: false,
            definition_name: "Body".into(),
            fields: vec![object_body_field(
                "dsHostConfiguration",
                false,
                vec![
                    body_field("instanceId", false),
                    integer_body_field("serversPerVm", false),
                ],
            )],
        });
        let step = step_with_bindings(
            &op,
            vec![
                literal_binding(
                    "dsHostConfiguration",
                    serde_json::json!({"serversPerVm": 1}),
                ),
                workflow_ref_binding("dsHostConfiguration.instanceId", "instanceType"),
            ],
        );
        let mut definition = simple_definition(vec![step]);
        definition.inputs.push(WorkflowInputSpec {
            name: "instanceType".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        });
        let mut cat = catalogue_with_svc(simple_schema(op));
        compile_workflow(&definition, &mut cat).expect("must compile");
    }

    // ---------------------------------------------------------------------------
    // options_source validation tests
    // ---------------------------------------------------------------------------

    /// Build a `WorkflowInputSpec` fixture with an options source.
    fn input_with_source(
        name: &str,
        schema: serde_json::Value,
        source: ags_protocol::workflow::OptionsSource,
    ) -> WorkflowInputSpec {
        WorkflowInputSpec {
            name: name.into(),
            description: None,
            schema: Some(schema),
            required: true,
            default: None,
            sensitive: false,
            options_source: Some(source),
            location: ags_protocol::workflow::StepFieldLocation::Body,
            file_picker: None,
        }
    }

    /// Build an `OptionsSource` fixture shaped like the images list operation.
    fn images_like_source(op_id: &str) -> ags_protocol::workflow::OptionsSource {
        use ags_protocol::workflow::{OperationReference, OptionParameterBinding};
        ags_protocol::workflow::OptionsSource {
            operation: OperationReference {
                service: ServiceId::new("svc"),
                operation: OperationId::new(op_id),
            },
            parameters: std::collections::BTreeMap::from([(
                "namespace".to_string(),
                OptionParameterBinding::FromInput("namespace".to_string()),
            )]),
            items_path: "$.images".into(),
            value: "$.id".into(),
            label: Some("$.name".into()),
            label_detail: None,
            fallback_description: None,
            filter: None,
        }
    }

    /// Build a GET list `OperationSchema` fixture with a `{namespace}` path param.
    fn list_op_with_namespace(op_id: &str) -> OperationSchema {
        let mut op = default_operation(op_id); // GET, ReadOnly by default
        op.parameters = vec![ParameterSchema {
            name: "namespace".into(),
            location: ParameterLocation::Path,
            required: true,
            value_type: ValueType::String,
            is_file: false,
            description: None,
            default: None,
        }];
        op
    }

    #[test]
    fn test_validate_options_rejects_non_string_target() {
        let op = list_op_with_namespace("List");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "List")]);
        def.inputs = vec![
            workflow_input("namespace"),
            input_with_source(
                "count",
                serde_json::json!({"type": "integer"}),
                images_like_source("List"),
            ),
        ];
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(err.message.contains("string-typed"), "got: {}", err.message);
    }

    #[test]
    fn test_validate_options_rejects_unknown_operation() {
        let op = list_op_with_namespace("List");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "List")]);
        def.inputs = vec![
            workflow_input("namespace"),
            input_with_source(
                "imgId",
                serde_json::json!({"type": "string"}),
                images_like_source("NoSuchOp"),
            ),
        ];
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("unknown operation") || err.message.contains("NoSuchOp"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_validate_options_rejects_non_get_operation() {
        let mut op = list_op_with_namespace("Create");
        op.http_method = HttpMethod::Post;
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "Create")]);
        def.inputs = vec![
            workflow_input("namespace"),
            input_with_source(
                "imgId",
                serde_json::json!({"type": "string"}),
                images_like_source("Create"),
            ),
        ];
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.to_uppercase().contains("GET"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_validate_options_rejects_unknown_parameter_name() {
        let op = list_op_with_namespace("List");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "List")]);
        let mut src = images_like_source("List");
        src.parameters.insert(
            "bogusParam".into(),
            ags_protocol::workflow::OptionParameterBinding::Literal(serde_json::json!("x")),
        );
        def.inputs = vec![
            workflow_input("namespace"),
            input_with_source("imgId", serde_json::json!({"type": "string"}), src),
        ];
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(err.message.contains("bogusParam"), "got: {}", err.message);
    }

    #[test]
    fn test_validate_options_rejects_schema_incompatible_literal() {
        let op = list_op_with_namespace("List");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "List")]);
        let mut src = images_like_source("List");
        src.parameters.insert(
            "namespace".into(),
            ags_protocol::workflow::OptionParameterBinding::Literal(serde_json::json!(42)),
        );
        def.inputs = vec![
            workflow_input("namespace"),
            input_with_source("imgId", serde_json::json!({"type": "string"}), src),
        ];
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.to_lowercase().contains("literal"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_validate_options_rejects_undeclared_from_input() {
        let op = list_op_with_namespace("List");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "List")]);
        let mut src = images_like_source("List");
        src.parameters.insert(
            "namespace".into(),
            ags_protocol::workflow::OptionParameterBinding::FromInput("notDeclared".into()),
        );
        def.inputs = vec![
            workflow_input("namespace"),
            input_with_source("imgId", serde_json::json!({"type": "string"}), src),
        ];
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(err.message.contains("notDeclared"), "got: {}", err.message);
    }

    #[test]
    fn test_validate_options_rejects_unparseable_projection() {
        let op = list_op_with_namespace("List");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "List")]);
        let mut src = images_like_source("List");
        src.value = "$.items[*].id".into();
        def.inputs = vec![
            workflow_input("namespace"),
            input_with_source("imgId", serde_json::json!({"type": "string"}), src),
        ];
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("projection") || err.message.contains("JSONPath"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_validate_options_rejects_nested_items_path() {
        let op = list_op_with_namespace("List");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "List")]);
        let mut src = images_like_source("List");
        src.items_path = "$.data.images".into();
        def.inputs = vec![
            workflow_input("namespace"),
            input_with_source("imgId", serde_json::json!({"type": "string"}), src),
        ];
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("single-segment"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_validate_options_accepts_valid_source() {
        let op = list_op_with_namespace("List");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "List")]);
        def.inputs = vec![
            workflow_input("namespace"),
            input_with_source(
                "imgId",
                serde_json::json!({"type": "string"}),
                images_like_source("List"),
            ),
        ];
        compile_workflow(&def, &mut cat).expect("valid options_source must compile");
    }

    #[test]
    fn test_validate_options_accepts_root_array_items_path() {
        let op = list_op_with_namespace("List");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "List")]);
        let mut src = images_like_source("List");
        src.items_path = "$".into(); // response body IS the array (root array)
        def.inputs = vec![
            workflow_input("namespace"),
            input_with_source("imgId", serde_json::json!({"type": "string"}), src),
        ];
        compile_workflow(&def, &mut cat).expect("root-array items_path must compile");
    }

    #[test]
    fn test_validate_options_rejects_malformed_filter_path() {
        use ags_protocol::workflow::OptionFilter;
        let op = list_op_with_namespace("List");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "List")]);
        let mut source = images_like_source("List");
        source.filter = Some(OptionFilter {
            path: "images".to_string(), // no `$` prefix → invalid subset
            equals: serde_json::json!(false),
        });
        def.inputs = vec![
            workflow_input("namespace"),
            input_with_source("store", serde_json::json!({"type": "string"}), source),
        ];
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(err.message.contains("filter path"), "got: {}", err.message);
    }

    #[test]
    fn test_validate_options_accepts_valid_filter_path() {
        // A source with a well-formed filter path compiles cleanly — proves the
        // new validation does not reject legitimate filters.
        use ags_protocol::workflow::OptionFilter;
        let op = list_op_with_namespace("List");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "List")]);
        let mut source = images_like_source("List");
        source.filter = Some(OptionFilter {
            path: "$.published".to_string(),
            equals: serde_json::json!(false),
        });
        def.inputs = vec![
            workflow_input("namespace"),
            input_with_source("store", serde_json::json!({"type": "string"}), source),
        ];
        assert!(compile_workflow(&def, &mut cat).is_ok());
    }

    // ---------------------------------------------------------------------------
    // file_picker validation tests
    // ---------------------------------------------------------------------------

    #[test]
    fn test_compile_workflow_file_picker_on_non_string_input_errors() {
        let op = default_operation("Op");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "Op")]);
        def.inputs.push(WorkflowInputSpec {
            name: "iconFile".to_string(),
            description: None,
            schema: Some(serde_json::json!({"type": "integer"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            file_picker: Some(FilePickerSpec {
                extensions: None,
                start_dir: None,
            }),
            location: ags_protocol::workflow::StepFieldLocation::Body,
        });
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("not string-typed"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_compile_workflow_file_picker_and_options_source_together_errors() {
        let op = list_op_with_namespace("List");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "List")]);
        let mut img_id_input = input_with_source(
            "imgId",
            serde_json::json!({"type": "string"}),
            images_like_source("List"),
        );
        img_id_input.file_picker = Some(FilePickerSpec {
            extensions: None,
            start_dir: None,
        });
        def.inputs = vec![workflow_input("namespace"), img_id_input];
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message
                .contains("both a file_picker and an options_source"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_compile_workflow_file_picker_extension_with_leading_dot_errors() {
        let op = default_operation("Op");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "Op")]);
        def.inputs.push(WorkflowInputSpec {
            name: "iconFile".to_string(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            file_picker: Some(FilePickerSpec {
                extensions: Some(vec![".png".to_string()]),
                start_dir: None,
            }),
            location: ags_protocol::workflow::StepFieldLocation::Body,
        });
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("must not start with"),
            "got: {}",
            err.message
        );
    }

    #[test]
    fn test_compile_workflow_valid_file_picker_compiles() {
        let op = default_operation("Op");
        let mut cat = catalogue_with_svc(service_with(vec![op]));
        let mut def = simple_definition(vec![simple_step("s1", "Op")]);
        def.inputs.push(WorkflowInputSpec {
            name: "iconFile".to_string(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            file_picker: Some(FilePickerSpec {
                extensions: Some(vec!["png".to_string(), "jpg".to_string()]),
                start_dir: Some("/tmp".to_string()),
            }),
            location: ags_protocol::workflow::StepFieldLocation::Body,
        });
        compile_workflow(&def, &mut cat).expect("valid file_picker input compiles");
    }

    #[test]
    fn test_compile_workflow_passes_briefing_through() {
        let briefing = WorkflowBriefing {
            overview: "ov".into(),
            prerequisites: vec!["p1".into()],
            creates: vec!["c1".into()],
        };
        let def = WorkflowDefinition {
            id: WorkflowId::new("wf"),
            name: "WF".into(),
            workflow_protocol_version: None,
            intent: None,
            description: None,
            briefing: Some(briefing.clone()),
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![],
            outputs: vec![],
            completion: None,
        };
        let mut catalogue = Catalogue::new();
        let compiled = compile_workflow(&def, &mut catalogue).expect("compile");
        assert_eq!(compiled.briefing, Some(briefing));
    }

    // ---------------------------------------------------------------------------
    // validate_skippable_outputs_have_defaults tests
    // ---------------------------------------------------------------------------

    /// Build a minimal two-step `WorkflowDefinition` backed by a single "Op".
    fn minimal_two_step_definition() -> WorkflowDefinition {
        simple_definition(vec![simple_step("s1", "Op"), simple_step("s2", "Op")])
    }

    /// Build a minimal `Catalogue` that satisfies the `minimal_two_step_definition`.
    fn fake_catalogue() -> Catalogue {
        catalogue_with_svc(service_with(vec![default_operation("Op")]))
    }

    #[test]
    fn test_compile_rejects_skippable_step_output_without_default() {
        let mut def = minimal_two_step_definition();
        // Make the first step user-optional and give it an output with no default.
        def.steps[0].is_optional = true;
        def.steps[0].outputs = vec![StepOutputCapture {
            name: "id".into(),
            source: CaptureSource::ResponseBody {
                path: "$.id".into(),
            },
            default: None,
            sensitive: false,
        }];
        let err = compile_workflow(&def, &mut fake_catalogue()).unwrap_err();
        assert!(err.to_string().contains("has no default"));
    }

    #[test]
    fn test_compile_accepts_skippable_step_output_with_default() {
        let mut def = minimal_two_step_definition();
        def.steps[0].continue_on_failure = true;
        def.steps[0].outputs = vec![StepOutputCapture {
            name: "id".into(),
            source: CaptureSource::ResponseBody {
                path: "$.id".into(),
            },
            default: Some(serde_json::json!("fallback")),
            sensitive: false,
        }];
        assert!(compile_workflow(&def, &mut fake_catalogue()).is_ok());
    }

    #[test]
    fn test_skip_if_exists_step_requires_output_defaults() {
        // A skip_if_exists step whose captured output has no default must fail to compile.
        let mut def = minimal_two_step_definition();
        def.steps[0].skip_if_exists = true;
        def.steps[0].outputs = vec![StepOutputCapture {
            name: "id".into(),
            source: CaptureSource::ResponseBody {
                path: "$.id".into(),
            },
            default: None,
            sensitive: false,
        }];
        let err = compile_workflow(&def, &mut fake_catalogue()).unwrap_err();
        assert!(err.to_string().contains("has no default"), "got: {err}");
    }

    // -----------------------------------------------------------------
    // Test Plan case 2: compile_workflow rejects an unknown local action
    // name with an error that lists the known action names.
    // -----------------------------------------------------------------
    #[test]
    fn test_compile_rejects_unknown_local_action() {
        let def = WorkflowDefinition {
            id: WorkflowId::new("wf"),
            name: "test".into(),
            workflow_protocol_version: None,
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![StepDefinition {
                id: "bad-step".into(),
                description: None,
                kind: ags_protocol::workflow::StepKind::Local,
                action: Some("does-not-exist".into()),
                operation: None,
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
            }],
            outputs: vec![],
            completion: None,
        };
        let mut cat = Catalogue::new();
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("unknown local action"),
            "error must mention 'unknown local action': {err}"
        );
        assert!(
            err.message.contains("does-not-exist"),
            "error must echo the bad action name: {err}"
        );
        assert!(
            err.message.contains("docker-login"),
            "error must list known action names: {err}"
        );
    }

    // -----------------------------------------------------------------
    // Test Plan case 4: an existing API workflow compiles without error
    // after the additive discriminator is added (regression guard).
    // -----------------------------------------------------------------
    #[test]
    fn test_compile_api_workflow_unchanged() {
        let op = default_operation("Op");
        let schema = service_with(vec![op]);
        let mut cat = catalogue_with_svc(schema);
        let def = simple_definition(vec![simple_step("s1", "Op")]);

        let compiled =
            compile_workflow(&def, &mut cat).expect("existing API workflow must still compile");
        assert_eq!(compiled.steps.len(), 1);
        assert_eq!(
            compiled.steps[0].kind,
            ags_protocol::workflow::StepKind::Api
        );
        assert!(
            compiled.steps[0].operation.is_some(),
            "compiled API step must retain its operation"
        );
        assert!(
            compiled.steps[0].action.is_none(),
            "compiled API step must not gain an action"
        );
    }

    // -----------------------------------------------------------------
    // Compile validates local step kind/action/operation consistency.
    // -----------------------------------------------------------------
    #[test]
    fn test_compile_local_step_with_operation_rejected() {
        let def = WorkflowDefinition {
            steps: vec![StepDefinition {
                id: "bad".into(),
                description: None,
                kind: ags_protocol::workflow::StepKind::Local,
                action: Some("docker-login".into()),
                operation: Some(op_ref("Op")),
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
            }],
            ..simple_definition(vec![])
        };
        let mut cat = Catalogue::new();
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("must not have an 'operation' field"),
            "got: {err}"
        );
    }

    #[test]
    fn test_compile_local_step_skip_if_exists_rejected() {
        let def = WorkflowDefinition {
            steps: vec![StepDefinition {
                id: "bad".into(),
                description: None,
                kind: ags_protocol::workflow::StepKind::Local,
                action: Some("docker-login".into()),
                operation: None,
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: true,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
            }],
            ..simple_definition(vec![])
        };
        let mut cat = Catalogue::new();
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("skip_if_exists"),
            "error must mention skip_if_exists: {err}"
        );
    }

    // -----------------------------------------------------------------
    // Compile rejects local steps that set confirm / is_optional /
    // continue_on_failure. These flags rely on the API dispatch path's
    // flow-control machinery, which a local action does not use.
    // Ported from the original validate_local_step four-flag loop.
    // -----------------------------------------------------------------

    #[test]
    fn test_compile_local_step_confirm_rejected() {
        let def = WorkflowDefinition {
            steps: vec![StepDefinition {
                id: "bad".into(),
                description: None,
                kind: ags_protocol::workflow::StepKind::Local,
                action: Some("docker-login".into()),
                operation: None,
                dependencies: vec![],
                confirm: true,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
            }],
            ..simple_definition(vec![])
        };
        let mut cat = Catalogue::new();
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("confirm"),
            "error must mention confirm: {err}"
        );
    }

    #[test]
    fn test_compile_local_step_is_optional_rejected() {
        let def = WorkflowDefinition {
            steps: vec![StepDefinition {
                id: "bad".into(),
                description: None,
                kind: ags_protocol::workflow::StepKind::Local,
                action: Some("docker-login".into()),
                operation: None,
                dependencies: vec![],
                confirm: false,
                is_optional: true,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
            }],
            ..simple_definition(vec![])
        };
        let mut cat = Catalogue::new();
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("is_optional"),
            "error must mention is_optional: {err}"
        );
    }

    #[test]
    fn test_compile_local_step_continue_on_failure_rejected() {
        let def = WorkflowDefinition {
            steps: vec![StepDefinition {
                id: "bad".into(),
                description: None,
                kind: ags_protocol::workflow::StepKind::Local,
                action: Some("docker-login".into()),
                operation: None,
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: true,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
            }],
            ..simple_definition(vec![])
        };
        let mut cat = Catalogue::new();
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("continue_on_failure"),
            "error must mention continue_on_failure: {err}"
        );
    }

    // -----------------------------------------------------------------
    // Compile accepts a valid local step with a known action.
    // -----------------------------------------------------------------
    #[test]
    fn test_compile_valid_local_step_succeeds() {
        let op = default_operation("Op");
        let schema = service_with(vec![op]);
        let mut cat = catalogue_with_svc(schema);
        let def = WorkflowDefinition {
            steps: vec![
                simple_step("fetch-token", "Op"),
                StepDefinition {
                    id: "authenticate-docker".into(),
                    description: Some("Login".into()),
                    kind: ags_protocol::workflow::StepKind::Local,
                    // The test-echo action is available under #[cfg(test)].
                    action: Some("test-echo".into()),
                    operation: None,
                    dependencies: vec!["fetch-token".into()],
                    confirm: false,
                    is_optional: false,
                    continue_on_failure: false,
                    skip_if_exists: false,
                    is_reviewed: None,
                    inputs: vec![],
                    outputs: vec![],
                },
            ],
            ..simple_definition(vec![])
        };
        let compiled = compile_workflow(&def, &mut cat).expect("valid local step must compile");
        assert_eq!(compiled.steps.len(), 2);
        assert_eq!(
            compiled.steps[1].kind,
            ags_protocol::workflow::StepKind::Local
        );
        assert_eq!(compiled.steps[1].action.as_deref(), Some("test-echo"));
        assert!(compiled.steps[1].auto_derived.is_empty());
    }

    // -----------------------------------------------------------------
    // FIX 1a: compile_workflow rejects a local step that binds a field
    // the action does not declare. The error names the offending field.
    // -----------------------------------------------------------------
    #[test]
    fn test_compile_local_step_rejects_unknown_binding_field() {
        let def = WorkflowDefinition {
            steps: vec![StepDefinition {
                id: "bad".into(),
                description: None,
                kind: ags_protocol::workflow::StepKind::Local,
                action: Some("test-echo".into()),
                operation: None,
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![StepInputBinding {
                    field: "bogus".into(),
                    source: BindingSource::Literal(ags_protocol::workflow::LiteralBinding {
                        value: serde_json::json!("value"),
                        sensitive: false,
                    }),
                    show_in_review: false,
                    description: None,
                }],
                outputs: vec![],
            }],
            ..simple_definition(vec![])
        };
        let mut cat = Catalogue::new();
        let err = compile_workflow(&def, &mut cat)
            .expect_err("binding a field the action does not declare must be rejected");
        assert!(
            err.message.contains("bogus"),
            "error must name the unknown field: {err}"
        );
        assert!(
            err.message.contains("does not accept"),
            "error must say the action does not accept the field: {err}"
        );
    }

    // -----------------------------------------------------------------
    // FIX 1b: compile_workflow rejects a local step that leaves a
    // required declared input unbound. The error names the missing input.
    // -----------------------------------------------------------------
    #[test]
    fn test_compile_local_step_rejects_unbound_required_input() {
        // docker-login declares registry, username, password as required.
        // Binding none of them must fail.
        let def = WorkflowDefinition {
            steps: vec![StepDefinition {
                id: "bad".into(),
                description: None,
                kind: ags_protocol::workflow::StepKind::Local,
                action: Some("docker-login".into()),
                operation: None,
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: false,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
            }],
            ..simple_definition(vec![])
        };
        let mut cat = Catalogue::new();
        let err = compile_workflow(&def, &mut cat)
            .expect_err("leaving required inputs unbound must be rejected");
        assert!(
            err.message.contains("leaves required input"),
            "error must mention the missing input: {err}"
        );
    }

    // -----------------------------------------------------------------
    // FIX 3: skip_if_exists rejection includes the HTTP 409 rationale.
    // -----------------------------------------------------------------
    #[test]
    fn test_compile_local_step_skip_if_exists_includes_rationale() {
        let def = WorkflowDefinition {
            steps: vec![StepDefinition {
                id: "bad".into(),
                description: None,
                kind: ags_protocol::workflow::StepKind::Local,
                action: Some("docker-login".into()),
                operation: None,
                dependencies: vec![],
                confirm: false,
                is_optional: false,
                continue_on_failure: false,
                skip_if_exists: true,
                is_reviewed: None,
                inputs: vec![],
                outputs: vec![],
            }],
            ..simple_definition(vec![])
        };
        let mut cat = Catalogue::new();
        let err = compile_workflow(&def, &mut cat).unwrap_err();
        assert!(
            err.message.contains("409"),
            "skip_if_exists rejection must include the HTTP 409 rationale: {err}"
        );
    }
}
