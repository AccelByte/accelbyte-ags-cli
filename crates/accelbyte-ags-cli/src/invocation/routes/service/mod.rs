//! Handle dynamic service commands: parse args, resolve auth, run the
//! synthesised 1-step workflow through the executor.

pub(crate) mod clap_tree;
mod help;
mod parser;
mod request;

use ags_protocol::output::ResolutionTrace;
use ags_runtime::catalogue::SpecSource;
use ags_runtime::runtime::workflows::auto_derive::find_operation;
use ags_runtime::runtime::workflows::compile::compile_workflow;
use ags_runtime::runtime::workflows::synthesised::synthesise_workflow_definition;
use ags_runtime::runtime::workflows::RunOptions;
use ags_runtime::support::strings::to_kebab_case;

use crate::errors::CliError;
use crate::invocation::flags;
use crate::invocation::phase_execution::{run_phase_owned_execution, AdapterMode};
use crate::invocation::shape::{
    classify_shape, AuthSubcommand, MissingInputs, OutputOnly, RouteKind,
};
use crate::invocation::workflows::cli_flags_matching_workflow_inputs;
use crate::invocation::InvocationOutcome;

/// Dispatch a dynamic service command by synthesising a one-step workflow and
/// running it through the workflow executor.
///
/// Pre-execution work runs before any phase surfaces exist. After that, the
/// service command uses the same split as workflow runs: interaction/progress
/// may use an inline surface, while final output stays on the plain-terminal
/// path.
/// Returns the invocation outcome plus the final call's raw JSON response body
/// (when the run produced one), which wait-capable callers chain on — see
/// [`run_phase_owned_execution`].
pub(crate) async fn route_service(
    service_arg: &str,
    service_args: &[String],
    flags: &flags::GlobalFlags,
    render_options: crate::frontend::RenderOptions,
    frontend_context: &crate::invocation::context::FrontendContext,
    shim_presentation: Option<
        &crate::invocation::handlers::extend::service_shims::ShimPresentation,
    >,
) -> Result<(InvocationOutcome, Option<serde_json::Value>), CliError> {
    // `ams upload` is a hand-written resource, not a catalogued operation, so
    // it branches off before spec loading and workflow synthesis. Everything
    // downstream of here assumes an `OperationSchema` exists.
    if is_ams_service(service_arg) && super::ams_upload::is_ams_upload(service_args) {
        // `ams upload` is not a catalogued service call and produces no
        // chainable response body — no wait consumer, so `None`.
        return super::ams_upload::route_ams_upload(
            service_args,
            flags,
            frontend_context.surface_backend(),
            render_options,
            frontend_context,
        )
        .await
        .map(|outcome| (outcome, None));
    }

    let (selectors, stripped_args) = flags::pre_scan_leaf_selectors(service_args)?;
    // Parse/help/skeleton run with NO phase surfaces; `parse_service_args`
    // builds its own fresh pre-surface frontend where it needs to render.
    let parsed = match parser::parse_service_args(
        service_arg,
        &stripped_args,
        &selectors,
        flags,
        frontend_context,
        render_options.clone(),
        shim_presentation,
    )? {
        parser::ParseServiceOutcome::Continue(parsed) => parsed,
        parser::ParseServiceOutcome::Exit(code) => {
            return Ok((InvocationOutcome::Exit(code), None))
        }
        parser::ParseServiceOutcome::Complete => return Ok((InvocationOutcome::Complete, None)),
    };

    // Runtime prologue (shared with `route_workflow_run`): resolves auth/base
    // URL and renders any access-token warnings on a fresh pre-surface frontend,
    // all with NO phase surfaces. A failure `?`-propagates as `Err` for the top
    // level to render on a fresh human frontend. The pre-surface backend is
    // `StructuredJson` for automation else `PlainTerminal`, so a `--ui=fullscreen` run
    // never acquires the terminal just for these warnings.
    let (context, http_client) =
        super::run_prologue(flags, frontend_context, &render_options).await?;

    let mut runtime =
        ags_runtime::runtime::Runtime::from_reqwest(context.clone(), http_client.clone());

    // Synthesise a 1-step workflow for the parsed operation and compile it.
    let definition = synthesise_workflow_definition(
        &parsed.service_id,
        &parsed.command_request.operation_id,
        &parsed.service_schema,
    )?;
    let compiled = compile_workflow(&definition, runtime.catalogue_mut())?;

    // Map the parsed CLI flags onto the workflow's declared inputs.
    let mut pre_supplied = cli_flags_matching_workflow_inputs(&compiled, &parsed.command_request);

    // An explicit `--json` body is authoritative: pass it to the executor
    // verbatim (via `RunOptions::explicit_body`) and mark every body-field
    // input as pre-supplied so the executor does not gather the fields the
    // `--json` object omits — the body is sent as-is and the server validates.
    // Path/query/header inputs are untouched, so a genuinely missing path
    // param still gathers/errors.
    let explicit_body = match &parsed.command_request.body {
        Some(ags_protocol::request::RequestBody::Json(v)) => Some(v.clone()),
        Some(ags_protocol::request::RequestBody::Multipart(_)) | None => None,
    };
    if explicit_body.is_some() {
        if let Some(operation) =
            find_operation(&parsed.service_schema, &parsed.command_request.operation_id)
        {
            if let Some(request_body) = &operation.request_body {
                for field in &request_body.fields {
                    pre_supplied
                        .entry(field.name.clone())
                        .or_insert(serde_json::Value::Null);
                }
            }
        }
    }

    // A non-interactive invocation cannot gather a missing required input, so
    // reject up front. This covers an automation consumer (`--format=json`),
    // `--no-input`, AND a human/plain run whose terminal can't prompt (piped
    // stdin or non-TTY stderr) — gating on `!allows_input()` matches
    // `route_workflow_run` and keeps such a run from falling through to a
    // prompt that can never be answered. The error is structured as:
    // error line → context (why interactive input is unavailable) → suggested
    // next step (which flags to pass, or how to run interactively).
    // `CliError::Usage` exits 1 — clap does not run for these relaxed args.
    reject_if_inputs_unavailable(
        &compiled,
        &pre_supplied,
        frontend_context,
        &parsed.service_schema,
        &parsed.command_request.operation_id,
    )?;

    let pagination = flags.pagination_hint();
    let options = RunOptions {
        dry_run: flags.is_dry_run,
        assume_yes: flags.is_auto_confirmed,
        // A non-promptable run (automation, `--no-input`, or a non-TTY
        // stdin/stderr) sets `no_input` so the executor never attempts to
        // gather — matching `route_workflow_run`. Any missing required input
        // was already rejected by the `!allows_input()` gate above.
        no_input: !frontend_context.allows_input(),
        // Per-step review is a workflow-only, fullscreen-only feature; the
        // service-command path keeps its inline gather behaviour.
        review_steps: false,
        // Service commands never run the workflow upfront-gather branch, so this
        // is inert here; false keeps the literal explicit.
        pickers_available: false,
        output_format: frontend_context.protocol_output_format(),
        output: flags.output.clone(),
        verbosity: flags.verbosity,
        pagination,
        explicit_body,
        // A single-command run is a synthesised, never-registered workflow —
        // never bundled, so its failed step's `input_fields` withhold every
        // value, matching an external workflow's telemetry treatment.
        is_bundled_workflow: false,
    };

    // `--dry-run --verbose` renders a resolution trace before teardown. The
    // live (non-dry-run) verbose trace is already embedded in `ApiOutput` by
    // `run_command` and shown by the standard renderer.
    let resolution_trace = if flags.is_dry_run && flags.verbosity.is_verbose() {
        Some(build_resolution_trace(
            &context,
            parsed.spec_source,
            parsed.service_id.as_str(),
        ))
    } else {
        None
    };

    // Classify the interaction shape from the inputs still missing after flag
    // resolution, then finalize the surface decision (decision matrix). This must
    // happen after `pre_supplied` is fully populated (including the explicit-
    // body pre-population above) so the missing-input counts are accurate, and
    // before any phase surface is constructed.
    let required_scalars = compiled
        .inputs
        .iter()
        .filter(|spec| spec.required && !pre_supplied.contains_key(&spec.name))
        .filter(|spec| !is_body_field_input(spec))
        .count();
    let has_body_field = compiled.inputs.iter().any(|spec| {
        is_body_field_input(spec) && spec.required && !pre_supplied.contains_key(&spec.name)
    });
    let shape = classify_shape(
        RouteKind::Service,
        MissingInputs {
            required_scalars,
            has_body_field,
        },
        1, // a synthesised service command is always a single step
        AuthSubcommand::Other,
        OutputOnly::No,
    );
    let frontend_context = frontend_context.finalize_surface(RouteKind::Service, shape);
    crate::invocation::register_reporter_if_plain(&frontend_context);
    // Help exits before finalization, so a service command here is never meta.
    // The hint only fires here because prologue/synth/compile succeeded; see
    // the first_run module doc for the deferral trade-off on early errors.
    crate::invocation::try_emit_first_run_hint(&frontend_context, false);

    // The prologue/synth/compile all succeeded — construct the phase surfaces
    // now. For `--ui=inline`/`--ui=fullscreen` this is the single terminal
    // acquisition point.
    let surfaces = if matches!(
        frontend_context.surface_backend(),
        crate::invocation::context::PhaseBackend::FullscreenTerminalUi
    ) {
        use crate::frontend::terminal::fullscreen::step_strip::{
            HeaderKind, Step, StepRowKind, StepState,
        };
        // A synthesised command is a single step; its runtime id is "main", but
        // the surface reads as a gather-inputs-then-run command, so the strip
        // row is labelled `gather-inputs` to match the workflow Inputs row. It
        // starts `Current` (active) — gathering its inputs is the first thing
        // the command does — so the strip highlights it from the outset rather
        // than showing it dimmed until dispatch.
        let fullscreen_steps: Vec<Step> = compiled
            .steps
            .iter()
            .map(|s| Step {
                kind: StepRowKind::Workflow {
                    runtime_index: s.index,
                },
                title: "gather-inputs".to_string(),
                state: StepState::Current,
            })
            .collect();
        crate::frontend::select_fullscreen_workflow_surfaces(
            &frontend_context,
            render_options,
            parsed.command_path.clone(),
            HeaderKind::Command,
            fullscreen_steps,
            None,
            |_surface| None, // no dynamic-enum resolver for synthesised service commands
        )?
    } else {
        crate::frontend::select_workflow_phase_surfaces(
            &frontend_context,
            render_options,
            Some(compiled.inputs.clone()),
            None,
        )?
    };

    // The shared post-prologue helper owns `RunStarted` onward. A service
    // failure preserves `CliError::exit_code()` — the helper returns
    // `Exit(error.exit_code())`, never a hardcoded `Exit(1)`.
    //
    // `SuppressedLifecycle` never carries step telemetry, so the reclaimed
    // `TelemetryClient` here is always `None` — nothing to flush.
    let (outcome, _telemetry_client, final_raw_body) = run_phase_owned_execution(
        surfaces,
        &compiled,
        pre_supplied,
        &mut runtime,
        &options,
        AdapterMode::SuppressedLifecycle,
        resolution_trace,
    )
    .await?;
    Ok((outcome, final_raw_body))
}

/// Reject a service command up front when the run cannot gather missing required
/// inputs (automation, `--no-input`, or a non-TTY stdin/stderr). Builds a usage
/// error naming the missing inputs and the flags that set them (`--<name>` for
/// params, `--json` for body fields). Returns `Ok(())` when input is available
/// or nothing required is missing.
fn reject_if_inputs_unavailable(
    compiled: &ags_protocol::workflow::CompiledWorkflow,
    pre_supplied: &std::collections::BTreeMap<String, serde_json::Value>,
    frontend_context: &crate::invocation::context::FrontendContext,
    service_schema: &ags_protocol::catalogue::ServiceSchema,
    operation_id: &ags_protocol::catalogue::OperationId,
) -> Result<(), CliError> {
    if frontend_context.allows_input() {
        return Ok(());
    }
    let mut missing: Vec<String> = compiled
        .inputs
        .iter()
        .filter(|spec| spec.required && !pre_supplied.contains_key(&spec.name))
        .map(|spec| spec.name.clone())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    missing.sort();
    let reason = crate::invocation::context::input_unavailable_reason(&frontend_context.terminal);
    // Headline only — the `✕` symbol and the `Reason:`/`→ Fix:` lines are added
    // by the structured error renderer from the metadata below.
    let message = if missing.len() == 1 {
        format!("Missing required input '{}'", missing[0])
    } else {
        // Cap a long list so the error stays readable: show the first few and a
        // "(+ N more)" tail. Only truncate when it hides at least two names.
        const MAX_LISTED_MISSING: usize = 3;
        let truncated = missing.len() > MAX_LISTED_MISSING + 1;
        let shown = if truncated {
            MAX_LISTED_MISSING
        } else {
            missing.len()
        };
        let mut lines: Vec<String> = missing[..shown]
            .iter()
            .map(|name| format!("    '{name}'"))
            .collect();
        if truncated {
            lines.push(format!("    (+ {} more)", missing.len() - shown));
        }
        format!("Missing required inputs:\n{}", lines.join("\n"))
    };
    // Body fields are settable only via `--json`; path/query/header params each
    // have a `--<kebab-name>` flag. Suggest the right remedy per input so we
    // never point the user at a flag that doesn't exist.
    let body_fields: std::collections::HashSet<&str> = find_operation(service_schema, operation_id)
        .and_then(|operation| operation.request_body.as_ref())
        .map(|body| {
            body.fields
                .iter()
                .map(|field| field.name.as_str())
                .collect()
        })
        .unwrap_or_default();
    let mut hints: Vec<String> = missing
        .iter()
        .filter(|name| !body_fields.contains(name.as_str()))
        .map(|name| format!("--{}", to_kebab_case(name)))
        .collect();
    if missing
        .iter()
        .any(|name| body_fields.contains(name.as_str()))
    {
        hints.push("--json".to_string());
    }
    let flags_hint = hints.join(", ");
    Err(CliError::Usage {
        message,
        metadata: Some(Box::new(crate::errors::ErrorMetadata {
            reason: Some(reason.to_string()),
            suggestion: Some(format!(
                "Pass {flags_hint}, or run interactively with both stdin and \
                 stderr attached to a terminal."
            )),
            ..Default::default()
        })),
    })
}

/// Whether the user-typed service token selects the AMS service, accounting
/// for the manifest's display-name aliasing.
fn is_ams_service(service_arg: &str) -> bool {
    ags_runtime::catalogue::Catalogue::find_id(service_arg)
        .is_some_and(|id| id.as_str() == clap_tree::AMS_SERVICE_NAME)
}

/// Whether a synthesised service input is a *structured* body field (object or
/// array) rather than a scalar a plain prompt can gather line-by-line; structured
/// body inputs force FORM shape (and render as the inline JSON editor).
///
/// This mirrors [`schema_to_field_type`](crate::frontend::terminal::inline::form_builder)'s
/// `JsonBody` rule — `"type": "object"` or `"array"`, or a bare `"properties"` —
/// so the shape classifier and the inline form agree on which fields are
/// structured. Scalar / enum / boolean body fields are NOT structured: plain can
/// prompt them, so they count as `required_scalars`, not `has_body_field`.
///
/// Schema shape (not `spec.location`) is the signal: a *scalar* body field such
/// as a `name` string must stay plain-promptable (a `required_scalar`), so only
/// object/array fields are treated as structured — exactly as `schema_to_field_type`
/// decides `JsonBody` without consulting location. Sound for the synthesised
/// single-step workflows this route builds (path params are always scalar
/// schemas, so they never trip the object/array test); do NOT reuse it against
/// authored registry workflows, where a structured input may be gathered
/// differently.
fn is_body_field_input(spec: &ags_protocol::workflow::WorkflowInputSpec) -> bool {
    let Some(schema) = spec.schema.as_ref() else {
        return false;
    };
    // Enum-first, matching `schema_to_field_type`'s priority: an enum schema — even
    // an object-typed one — is plain-promptable (the form cycles it), so it is a
    // scalar here, not a structured body field.
    if schema.get("enum").and_then(|v| v.as_array()).is_some() {
        return false;
    }
    let type_str = schema.get("type").and_then(|t| t.as_str()).unwrap_or("");
    type_str == "object" || type_str == "array" || schema.get("properties").is_some()
}

/// Build the `ResolutionTrace` shown for `--dry-run --verbose`.
fn build_resolution_trace(
    context: &ags_runtime::runtime::execution::ExecutionContext,
    spec_source: SpecSource,
    service_id: &str,
) -> ResolutionTrace {
    use ags_runtime::catalogue::Catalogue;

    let service_display = Catalogue::display_name(service_id).unwrap_or(service_id);
    let spec_source_label = match spec_source {
        SpecSource::Cache => format!("{} loaded from cache", service_display.to_uppercase()),
        SpecSource::Bundled => {
            format!(
                "{} decompressed from bundle",
                service_display.to_uppercase()
            )
        }
    };
    let token_expiry_label = context
        .access_token_expiry
        .as_ref()
        .map(|duration| format!("expires in {duration}"));
    ResolutionTrace {
        spec_source: spec_source_label,
        profile: (
            context.profile.clone(),
            context.profile_source.label().to_string(),
        ),
        base_url: (
            context.base_url.clone(),
            context.base_url_source.label().to_string(),
        ),
        namespace: context
            .namespace
            .as_ref()
            .zip(context.namespace_source.as_ref())
            .map(|(namespace, source)| (namespace.clone(), source.label().to_string())),
        token_source: context.access_token_source.label().to_string(),
        token_expiry: token_expiry_label,
    }
}

#[cfg(test)]
mod is_body_field_input_tests {
    use super::is_body_field_input;
    use serde_json::json;

    fn spec_with_schema(schema: serde_json::Value) -> ags_protocol::workflow::WorkflowInputSpec {
        ags_protocol::workflow::WorkflowInputSpec {
            name: "field".into(),
            description: None,
            schema: Some(schema),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: Default::default(),
            file_picker: None,
        }
    }

    #[test]
    fn test_object_field_is_structured() {
        assert!(is_body_field_input(&spec_with_schema(
            json!({"type": "object"})
        )));
    }

    #[test]
    fn test_array_field_is_structured() {
        // Regression: an array body field (e.g. a `permissions` list) must count as
        // a structured body field, not a plain-promptable scalar — otherwise a
        // command whose only missing input is the array classifies as `Small` and
        // `--ui auto` wrongly picks plain instead of the inline form / JSON editor.
        assert!(is_body_field_input(&spec_with_schema(
            json!({"type": "array", "items": {"type": "object"}})
        )));
    }

    #[test]
    fn test_properties_without_type_is_structured() {
        assert!(is_body_field_input(&spec_with_schema(
            json!({"properties": {"a": {"type": "string"}}})
        )));
    }

    #[test]
    fn test_string_scalar_is_not_structured() {
        assert!(!is_body_field_input(&spec_with_schema(
            json!({"type": "string"})
        )));
    }

    #[test]
    fn test_enum_string_is_not_structured() {
        assert!(!is_body_field_input(&spec_with_schema(
            json!({"type": "string", "enum": ["A", "B"]})
        )));
    }

    #[test]
    fn test_enum_object_is_not_structured() {
        // Enum takes priority over the object/array test (mirrors
        // schema_to_field_type), so an enum-constrained object stays a scalar.
        assert!(!is_body_field_input(&spec_with_schema(
            json!({"type": "object", "enum": ["A"]})
        )));
    }

    #[test]
    fn test_bool_is_not_structured() {
        assert!(!is_body_field_input(&spec_with_schema(
            json!({"type": "boolean"})
        )));
    }

    #[test]
    fn test_missing_schema_is_not_structured() {
        let mut spec = spec_with_schema(json!({"type": "object"}));
        spec.schema = None;
        assert!(!is_body_field_input(&spec));
    }
}
