//! Workflow executor: drives a `CompiledWorkflow` through gather, confirm,
//! dispatch, capture, and finalisation; emits lifecycle events; returns the
//! run outcome and any pending error.

use ags_protocol::error::{ErrorDetails, RuntimeError, RuntimeErrorKind};
use ags_protocol::output::CommandOutput;
use ags_protocol::workflow::CompiledWorkflow;

use crate::runtime::workflows::{
    RunOptions, RunOutcome, StepOutcome, WorkflowEvent, WorkflowFrontend,
};
use crate::runtime::Runtime;

/// Resolve whether a step participates in the per-step review walk:
/// the step's `is_reviewed` override, else the workflow default.
pub fn resolved_step_review(
    step: &ags_protocol::workflow::CompiledStep,
    compiled: &ags_protocol::workflow::CompiledWorkflow,
) -> bool {
    step.is_reviewed.unwrap_or(compiled.is_reviewed_by_default)
}

/// True when a step's resolved review plan has something the user should see or
/// provide: a field the workflow marked reviewable (`show_in_review`), or a
/// required field still awaiting input (`Unset`). When false the step's fields
/// are all auto-bound (params, prior-output refs, hidden literals), so the
/// review pause is skipped — an input-less step should not stop the run.
pub(crate) fn plan_has_reviewable_fields(plan: &ags_protocol::workflow::StepFieldPlan) -> bool {
    use ags_protocol::workflow::StepFieldSource;
    plan.fields
        .iter()
        .any(|f| f.show_in_review || (f.required && matches!(f.source, StepFieldSource::Unset)))
}

/// Decide whether a step pauses for review, given the run mode. `review_steps`
/// is the surface capability (false for plain/json/`-y`); `step_reviewed` is the
/// step's resolved review flag. `RunWithoutStopping` never pauses (it takes the
/// non-review gather path, which still gathers genuinely-missing required
/// inputs — plain-mode semantics). `force_pause` overrides the field-content
/// check (used when the step is optional and non-confirm, so the user must
/// always see the skip affordance).
pub(crate) fn should_pause_for_review(
    run_mode: ags_protocol::workflow::RunMode,
    review_steps: bool,
    step_reviewed: bool,
    force_pause: bool,
    plan: &ags_protocol::workflow::StepFieldPlan,
) -> bool {
    use ags_protocol::workflow::RunMode;
    if !review_steps || !step_reviewed || run_mode == RunMode::RunWithoutStopping {
        return false;
    }
    if force_pause {
        return true;
    }
    match run_mode {
        RunMode::ReviewEveryStep => true,
        RunMode::ReviewInputSteps => plan_has_reviewable_fields(plan),
        RunMode::RunWithoutStopping => false, // unreachable (handled above)
    }
}

/// Per-invocation context shared between dispatch and preview-building.
/// Holds a `&mut Runtime` so the executor can call dispatch / preview /
/// dry-run helpers and reach the catalogue.
pub struct RunContext<'a> {
    /// Live runtime; owns catalogue, http client, and execution context.
    pub runtime: &'a mut Runtime,
    /// Per-invocation knobs (`--dry-run`, `--yes`, `--no-input`).
    pub options: &'a RunOptions,
}

impl<'a> RunContext<'a> {
    /// Build a new context with the supplied runtime and options.
    pub fn new(runtime: &'a mut Runtime, options: &'a RunOptions) -> Self {
        Self { runtime, options }
    }
}

/// Stateless executor — all per-invocation state lives in `RunContext`
/// and `WorkflowContext`.
pub struct Executor;

impl Executor {
    /// Drive a compiled workflow to completion. `pre_supplied` carries the
    /// values the caller has already resolved (typically CLI flags mapped
    /// to workflow input names); the executor seeds `workflow_supplied`
    /// from it and layers declared defaults on top of any unfilled names.
    /// Returns the final outcome, the final `CommandOutput` if any (Success
    /// path produces one; Cancelled / Failed paths produce `None`), and
    /// any pending error the caller should render after `WorkflowFinished`
    /// fires.
    pub async fn execute(
        compiled: &CompiledWorkflow,
        pre_supplied: std::collections::BTreeMap<String, serde_json::Value>,
        frontend: &mut dyn WorkflowFrontend,
        run_context: &mut RunContext<'_>,
    ) -> Result<(RunOutcome, Option<CommandOutput>, Option<RuntimeError>), RuntimeError> {
        use crate::runtime::workflows::resolve::{
            assemble_command_request, compute_needed_inputs, resolve_step_fields,
        };
        use crate::runtime::workflows::WorkflowContext;
        use std::collections::{BTreeMap, BTreeSet};

        let run_started_at = std::time::Instant::now();
        let mut run_facts = ags_protocol::workflow::RunFacts::default();

        // Phase 0: seed workflow_supplied from the caller's pre-supplied
        // map (CLI flags resolved by the invocation layer), then layer
        // declared defaults on top of any name not already present.
        let mut workflow_supplied: BTreeMap<String, serde_json::Value> = pre_supplied;
        // Snapshot the caller-supplied names BEFORE the default loop runs.
        // After the loop, workflow_supplied also holds default values, so a
        // post-loop snapshot couldn't tell flag-supplied from default-supplied
        // entries when classifying SuppliedInputView provenance per step.
        let flag_names: BTreeSet<String> = workflow_supplied.keys().cloned().collect();
        let mut default_added: BTreeSet<String> = BTreeSet::new();
        for spec in &compiled.inputs {
            if !workflow_supplied.contains_key(&spec.name) {
                if let Some(default) = &spec.default {
                    workflow_supplied.insert(spec.name.clone(), default.clone());
                    default_added.insert(spec.name.clone());
                }
            }
        }
        // Snapshot values before Phase 1 so an input the user edited in the
        // run-start form can be distinguished from one that arrived by flag or
        // default and was left alone. This is also why provenance is
        // recomputed after Phase 1 rather than read from the pre-Phase-1 name
        // sets: a value supplied by flag and then edited in the form must not
        // be double-counted as untouched.
        let pre_gather_values = workflow_supplied.clone();

        if run_context.options.no_input {
            let violations = no_input_precheck(
                compiled,
                &workflow_supplied,
                run_context.options.dry_run,
                run_context.options.assume_yes,
            );
            if !violations.is_empty() {
                let single_command =
                    crate::runtime::workflows::synthesised::is_synthesised_single_command(
                        &compiled.id,
                    );
                return Err(no_input_violations_to_error(&violations, single_command));
            }
        }

        frontend.on_event(&WorkflowEvent::WorkflowStarted {
            compiled: compiled.clone(),
        });

        let mut run_outcome = RunOutcome::Success;
        let mut pending_error: Option<RuntimeError> = None;

        // Phase 0.5: long-form briefing, if the workflow has one and the
        // user hasn't opted out with --yes. Behaves identically to
        // gather-inputs cancellation: sets run_outcome and falls through
        // to finalisation via the existing Success-guard at ~line 120.
        if let Some(briefing) = compiled.briefing.as_ref() {
            if !run_context.options.assume_yes {
                match frontend.present_briefing(briefing, compiled.name.as_str()) {
                    Ok(true) => {}
                    Ok(false) => {
                        // No step ever starts, so no `StepFinished` can carry
                        // this stage — the run aggregate is the only place a
                        // declined briefing can be reported from.
                        run_facts.reason =
                            Some(ags_protocol::workflow::StepOutcomeReason::AtBriefing);
                        run_outcome = RunOutcome::Cancelled;
                    }
                    Err(error) => {
                        pending_error = Some(error);
                        run_outcome = RunOutcome::Failed;
                    }
                }
            }
        }

        let mut ctx = WorkflowContext::new();
        let mut step_summaries: Vec<String> = Vec::new();
        let mut dry_run_previews: Vec<ags_protocol::workflow::StepDryRunPreview> = Vec::new();
        // Declared here (in scope before phase-1) so it can be assigned from the
        // collect_workflow_inputs return value. Default (`ReviewInputSteps`)
        // stands until the run-start gather reports the user's chosen mode.
        let mut run_mode = ags_protocol::workflow::RunMode::default();

        // Phase 1: collect declared inputs once (interactive fullscreen only).
        // The returned map is authoritative for declared inputs — replace, don't
        // merge, so a cleared optional becomes genuinely unset. A clean cancel
        // (Ok(None)) sets Cancelled with no pending error and flows through the
        // existing finalization (cancelled panel, not an error render).
        if run_outcome == RunOutcome::Success && run_context.options.review_steps {
            // Order inputs by the step that first references them so the
            // gather-inputs form follows the workflow's execution flow. The
            // upfront form renders inputs in the order given and does not
            // re-sort (see `collect_inputs_form`), so this is the order the
            // user sees — workflows order by first-use, single commands by
            // request location.
            // Drop picker-support inputs on surfaces without a picker: they only
            // parameterise a picker that won't run, so requiring them here forces
            // a throwaway value. Fullscreen (pickers_available) is unchanged.
            let ordered_inputs = gather_inputs_for_surface(
                &compiled.inputs,
                &compiled.steps,
                run_context.options.pickers_available,
            );
            match frontend.collect_workflow_inputs(&ordered_inputs, &workflow_supplied) {
                Ok(Some(ags_protocol::workflow::CollectOutcome {
                    inputs,
                    run_mode: chosen,
                })) => {
                    for spec in &compiled.inputs {
                        workflow_supplied.remove(&spec.name);
                    }
                    for (name, value) in inputs {
                        workflow_supplied.insert(name, value);
                    }
                    run_mode = chosen;
                    run_facts.run_mode = Some(chosen);
                }
                Ok(None) => {
                    // Same reasoning as the declined briefing above: the
                    // cancellation happens before the first step, so only the
                    // run aggregate can name the stage.
                    run_facts.reason =
                        Some(ags_protocol::workflow::StepOutcomeReason::AtInputGather);
                    run_outcome = RunOutcome::Cancelled;
                }
                Err(error) => {
                    pending_error = Some(error);
                    run_outcome = RunOutcome::Failed;
                }
            }
        }

        // Compute declared-input provenance unconditionally: even a run that
        // cancelled at the briefing or the run-start gather should report how
        // its inputs arrived. Iterate the declared inputs (not
        // `workflow_supplied`) so step-local values gathered later never leak
        // into these counts. When Phase 1 never ran (`review_steps == false`),
        // `workflow_supplied` still equals `pre_gather_values`, so
        // `inputs_from_prompt` and `inputs_edited_in_form` legitimately stay 0.
        for spec in &compiled.inputs {
            let Some(current) = workflow_supplied.get(&spec.name) else {
                continue;
            };
            if flag_names.contains(&spec.name) {
                run_facts.inputs_from_flag += 1;
            } else if default_added.contains(&spec.name) {
                run_facts.inputs_from_default += 1;
            } else {
                run_facts.inputs_from_prompt += 1;
            }
            if pre_gather_values.get(&spec.name) != Some(current) {
                run_facts.inputs_edited_in_form += 1;
            }
        }

        if run_outcome == RunOutcome::Success {
            'step_loop: for step in &compiled.steps {
                let mut step_local: BTreeMap<String, serde_json::Value> = BTreeMap::new();
                let step_started_at = std::time::Instant::now();
                let mut attempts: u32 = 0;
                frontend.on_event(&WorkflowEvent::StepStarted {
                    index: step.index,
                    id: step.id.clone(),
                });
                run_facts.steps_started += 1;
                run_facts.last_step_index = Some(step.index);

                // Local steps do not support the flow-control flags that
                // the API path uses (confirm, is_optional, continue_on_failure,
                // skip_if_exists). validate_step_kinds rejects all four at
                // compile time, so none of the corresponding executor gates
                // can fire for a local step. The only shared machinery that
                // applies is the interactive failure gate (Retry/Cancel).
                if step.kind == ags_protocol::workflow::StepKind::Local {
                    let action_name = step.action.as_deref().ok_or_else(|| {
                        RuntimeError::internal(format!(
                            "step '{}': local step reached executor without an action",
                            step.id
                        ))
                    })?;
                    let action = crate::runtime::workflows::local_actions::lookup(action_name)
                        .ok_or_else(|| {
                            RuntimeError::internal(format!(
                                "step '{}': unknown local action '{}'",
                                step.id, action_name
                            ))
                        })?;

                    // Dry-run: delegate to the action's own dry-run, then
                    // bind outputs from what it returns. This mirrors the
                    // run pattern for local actions and produces a
                    // representative preview value per action.
                    if run_context.options.dry_run {
                        // Resolve input bindings so the action sees values.
                        // Resolution is deterministic — a broken binding
                        // (bad JSONPath, missing upstream capture) will fail
                        // identically on a real run. Propagate the error so
                        // --dry-run catches it instead of previewing success.
                        let dry_run_inputs =
                            match crate::runtime::workflows::resolve::resolve_local_step_bindings(
                                step,
                                &ctx,
                                &workflow_supplied,
                            ) {
                                Ok(resolved) => {
                                    let mut merged = workflow_supplied.clone();
                                    merged.extend(resolved);
                                    merged
                                }
                                Err(resolution_error) => {
                                    // Mirror the run path: treat as fatal.
                                    finish_step_terminal(
                                        frontend,
                                        &mut step_summaries,
                                        step,
                                        format!("{} failed", step.id),
                                        StepOutcome::Failed,
                                        Some(ags_protocol::workflow::StepOutcomeReason::Assembly),
                                        0,
                                        step_started_at,
                                        None,
                                        &mut run_facts,
                                    );
                                    pending_error = Some(resolution_error);
                                    run_outcome = RunOutcome::Failed;
                                    break;
                                }
                            };

                        // Build the Map<String, Value> the action trait expects.
                        let inputs_map: serde_json::Map<String, serde_json::Value> =
                            dry_run_inputs.into_iter().collect();

                        let action_result = {
                            let mut sink = progress_adapter(step.index, frontend);
                            action
                                .run(
                                    run_context.runtime,
                                    &inputs_map,
                                    &mut sink,
                                    true, // dry_run
                                )
                                .await
                        };
                        let produced = match action_result {
                            Ok(value) => value,
                            Err(action_error) => {
                                // Mirror the binding-resolution arm above:
                                // record the failure in step bookkeeping so
                                // steps_failed, last_step_index, and
                                // StepErrorFacts are properly attached.
                                finish_step_terminal(
                                    frontend,
                                    &mut step_summaries,
                                    step,
                                    format!("{} failed", step.id),
                                    StepOutcome::Failed,
                                    Some(ags_protocol::workflow::StepOutcomeReason::Dispatch),
                                    0,
                                    step_started_at,
                                    Some(ags_protocol::workflow::StepErrorFacts::from_error(
                                        &action_error,
                                    )),
                                    &mut run_facts,
                                );
                                pending_error = Some(action_error);
                                run_outcome = RunOutcome::Failed;
                                break;
                            }
                        };

                        // Bind outputs from the action's dry-run value.
                        ctx.store_local_step_body(&step.id, produced.clone());
                        let body_json = ctx.step_body_json(&step.id);
                        let _ = ctx.bind_step_outputs(&step.id, &step.outputs, body_json.as_ref());

                        let bound: std::collections::BTreeMap<String, serde_json::Value> = step
                            .outputs
                            .iter()
                            .filter_map(|capture| {
                                let value = ctx.resolve_step_reference(&step.id, &capture.name)?;
                                Some((capture.name.clone(), value.clone()))
                            })
                            .collect();

                        let preview =
                            crate::runtime::workflows::dry_run::build_local_dry_run_preview(
                                step,
                                action_name,
                                produced,
                                bound,
                            );
                        dry_run_previews.push(preview);
                        ctx.inject_step_captures_for_dry_run(
                            &step.id,
                            &step
                                .outputs
                                .iter()
                                .map(|o| (o.name.clone(), serde_json::Value::Null))
                                .collect(),
                        );
                        finish_step_terminal(
                            frontend,
                            &mut step_summaries,
                            step,
                            format!("{} dry-run", step.id),
                            StepOutcome::Success,
                            None,
                            1,
                            step_started_at,
                            None,
                            &mut run_facts,
                        );
                        continue 'step_loop;
                    }

                    // No confirm gate for local steps: validate_step_kinds
                    // rejects confirm: true at compile time, so step.confirm
                    // is always false here.

                    // Resolve declared input bindings (step output
                    // references, literals, workflow inputs, format
                    // templates, mirrors) before invoking the handler.
                    // Binding values take precedence over same-named
                    // workflow_supplied entries: the author's explicit
                    // `from: step/X` wiring is more specific than a
                    // same-named workflow flag or default. Resolution is
                    // deterministic so it sits outside the retry loop.
                    let action_inputs =
                        match crate::runtime::workflows::resolve::resolve_local_step_bindings(
                            step,
                            &ctx,
                            &workflow_supplied,
                        ) {
                            Ok(resolved) => {
                                let mut merged = workflow_supplied.clone();
                                merged.extend(resolved);
                                merged
                            }
                            Err(resolution_error) => {
                                // Resolution is deterministic — retrying would
                                // produce the same failure. Treat as fatal.
                                finish_step_terminal(
                                    frontend,
                                    &mut step_summaries,
                                    step,
                                    format!("{} failed", step.id),
                                    StepOutcome::Failed,
                                    Some(ags_protocol::workflow::StepOutcomeReason::Assembly),
                                    0,
                                    step_started_at,
                                    None,
                                    &mut run_facts,
                                );
                                pending_error = Some(resolution_error);
                                run_outcome = RunOutcome::Failed;
                                break;
                            }
                        };

                    // Build the Map<String, Value> the widened trait expects.
                    let run_inputs_map: serde_json::Map<String, serde_json::Value> =
                        action_inputs.into_iter().collect();

                    // Execute with retry loop: decide_step_failure may
                    // return Retry (re-invoke the handler), Skip
                    // (continue_on_failure / user skip), or Fatal.
                    let (outcome, summary, reason, error_facts) = loop {
                        attempts = attempts.saturating_add(1);
                        let run_result = {
                            let mut sink = progress_adapter(step.index, frontend);
                            action
                                .run(
                                    run_context.runtime,
                                    &run_inputs_map,
                                    &mut sink,
                                    false, // not dry_run
                                )
                                .await
                        };
                        let (attempt_error, attempt_stage) = match run_result {
                            Ok(body) => {
                                ctx.store_local_step_body(&step.id, body);
                                let body_json = ctx.step_body_json(&step.id);
                                match ctx.bind_step_outputs(
                                    &step.id,
                                    &step.outputs,
                                    body_json.as_ref(),
                                ) {
                                    Ok(()) => {
                                        break (
                                            StepOutcome::Success,
                                            format!("{} ok", step.id),
                                            None,
                                            None,
                                        );
                                    }
                                    Err(error) => {
                                        (error, ags_protocol::workflow::StepOutcomeReason::Capture)
                                    }
                                }
                            }
                            Err(error) => {
                                (error, ags_protocol::workflow::StepOutcomeReason::Dispatch)
                            }
                        };

                        match decide_step_failure(
                            step,
                            &attempt_error,
                            run_context.options.no_input,
                            frontend,
                            attempt_stage,
                        )? {
                            FailureDisposition::Skip {
                                reason,
                                summary_tail,
                            } => {
                                let skip_facts = ags_protocol::workflow::StepErrorFacts::from_error(
                                    &attempt_error,
                                );
                                skip_step(
                                    &mut ctx,
                                    frontend,
                                    &mut step_summaries,
                                    step,
                                    reason,
                                    summary_tail.as_deref(),
                                    attempts,
                                    step_started_at,
                                    Some(skip_facts),
                                    &mut run_facts,
                                )?;
                                continue 'step_loop;
                            }
                            FailureDisposition::Retry => continue,
                            FailureDisposition::Fatal { reason } => {
                                let s = format!("{} failed", step.id);
                                let facts = ags_protocol::workflow::StepErrorFacts::from_error(
                                    &attempt_error,
                                );
                                pending_error = Some(attempt_error);
                                break (StepOutcome::Failed, s, Some(reason), Some(facts));
                            }
                        }
                    };

                    step_summaries.push(summary.clone());
                    tally_step_outcome(&mut run_facts, outcome);
                    frontend.on_event(&WorkflowEvent::StepFinished {
                        index: step.index,
                        id: step.id.clone(),
                        summary,
                        captures: Vec::new(),
                        outcome,
                        reason,
                        attempts,
                        duration_ms: step_duration_ms(step_started_at),
                        error: error_facts,
                    });

                    if outcome == StepOutcome::Failed {
                        run_outcome = RunOutcome::Failed;
                        break;
                    }
                    continue 'step_loop;
                }

                // 1. Load the service schema once. Both the per-step review plan
                // (when active) and request assembly below need it, so it is
                // hoisted above the input-collection branch.
                let op_ref = step.operation.as_ref().ok_or_else(|| {
                    RuntimeError::internal(format!(
                        "step '{}': API step reached executor without an operation",
                        step.id
                    ))
                })?;
                let service_schema = match run_context
                    .runtime
                    .catalogue_mut()
                    .get_or_load(op_ref.service.as_str())
                {
                    Ok(schema) => schema.clone(),
                    Err(error) => {
                        let facts = ags_protocol::workflow::StepErrorFacts::from_error(&error);
                        finish_step_terminal(
                            frontend,
                            &mut step_summaries,
                            step,
                            format!("{} failed", step.id),
                            StepOutcome::Failed,
                            Some(ags_protocol::workflow::StepOutcomeReason::SchemaLoad),
                            0,
                            step_started_at,
                            Some(facts),
                            &mut run_facts,
                        );
                        pending_error = Some(error);
                        run_outcome = RunOutcome::Failed;
                        break;
                    }
                };

                // 2. Collect this step's inputs. When `take_review_path` is
                // true (fullscreen interactive, step opted-in, and mode is not
                // RunWithoutStopping) the frontend reviews/edits the step's
                // complete request; otherwise the existing path gathers only
                // missing inputs.
                let take_review_path = run_context.options.review_steps
                    && (resolved_step_review(step, compiled)
                        || (step.is_optional && !step.confirm))
                    && run_mode != ags_protocol::workflow::RunMode::RunWithoutStopping;
                if take_review_path {
                    let plan = resolve_step_fields(
                        step,
                        &ctx,
                        &workflow_supplied,
                        &step_local,
                        &compiled.inputs,
                        &service_schema,
                        &default_added,
                    )?;
                    // Pause for review only when the mode and the plan warrant
                    // it. RunWithoutStopping is already excluded by
                    // take_review_path; passing true/true here lets
                    // should_pause_for_review apply its ReviewInputSteps /
                    // ReviewEveryStep logic.
                    if should_pause_for_review(
                        run_mode,
                        true,
                        true,
                        step.is_optional && !step.confirm,
                        &plan,
                    ) {
                        match frontend.review_step(&plan) {
                            Ok(ags_protocol::workflow::StepReviewOutcome::Proceed(edits)) => {
                                // The plan is deduped by binding key, so each id maps to
                                // exactly one destination field.
                                apply_step_review_edits(&plan, edits, &mut step_local);
                            }
                            Ok(ags_protocol::workflow::StepReviewOutcome::Skip)
                                if step.is_optional =>
                            {
                                skip_step(
                                    &mut ctx,
                                    frontend,
                                    &mut step_summaries,
                                    step,
                                    ags_protocol::workflow::StepOutcomeReason::DeclinedAtReview,
                                    None,
                                    0,
                                    step_started_at,
                                    // A user-initiated skip at the review gate:
                                    // nothing failed, so there is no error triple.
                                    None,
                                    &mut run_facts,
                                )?;
                                continue;
                            }
                            Ok(ags_protocol::workflow::StepReviewOutcome::Skip) => {
                                // Defensive: same guard as the confirm gate — a
                                // Skip for a non-optional step is a frontend bug.
                                let error = ags_protocol::error::RuntimeError::internal(format!(
                                    "frontend returned Skip for non-optional step '{}'",
                                    step.id
                                ));
                                let mut facts =
                                    ags_protocol::workflow::StepErrorFacts::from_error(&error);
                                facts.input_fields = step_input_fields_on_failure(
                                    step,
                                    &ctx,
                                    &workflow_supplied,
                                    &step_local,
                                    &compiled.inputs,
                                    &service_schema,
                                    &default_added,
                                    &flag_names,
                                    run_context.options.is_bundled_workflow,
                                );
                                finish_step_terminal(
                                    frontend,
                                    &mut step_summaries,
                                    step,
                                    format!("{} failed", step.id),
                                    StepOutcome::Failed,
                                    Some(
                                        ags_protocol::workflow::StepOutcomeReason::FrontendContract,
                                    ),
                                    0,
                                    step_started_at,
                                    Some(facts),
                                    &mut run_facts,
                                );
                                pending_error = Some(error);
                                run_outcome = RunOutcome::Failed;
                                break;
                            }
                            Ok(ags_protocol::workflow::StepReviewOutcome::Cancel) => {
                                finish_step_terminal(
                                    frontend,
                                    &mut step_summaries,
                                    step,
                                    format!("{} cancelled", step.id),
                                    StepOutcome::Cancelled,
                                    Some(ags_protocol::workflow::StepOutcomeReason::AtReview),
                                    0,
                                    step_started_at,
                                    None,
                                    &mut run_facts,
                                );
                                run_outcome = RunOutcome::Cancelled;
                                break;
                            }
                            Err(error) => {
                                let mut facts =
                                    ags_protocol::workflow::StepErrorFacts::from_error(&error);
                                facts.input_fields = step_input_fields_on_failure(
                                    step,
                                    &ctx,
                                    &workflow_supplied,
                                    &step_local,
                                    &compiled.inputs,
                                    &service_schema,
                                    &default_added,
                                    &flag_names,
                                    run_context.options.is_bundled_workflow,
                                );
                                finish_step_terminal(
                                    frontend,
                                    &mut step_summaries,
                                    step,
                                    format!("{} failed", step.id),
                                    StepOutcome::Failed,
                                    Some(ags_protocol::workflow::StepOutcomeReason::Gather),
                                    0,
                                    step_started_at,
                                    Some(facts),
                                    &mut run_facts,
                                );
                                pending_error = Some(error);
                                run_outcome = RunOutcome::Failed;
                                break;
                            }
                        }
                    }
                    // else: fall through to assemble/dispatch (no pause)
                } else {
                    // Existing path: compute the gather list, then gather missing inputs.
                    let needed = compute_needed_inputs(step, &workflow_supplied, &compiled.inputs);
                    if !needed.is_empty() {
                        // Build the supplied-inputs view from the current
                        // workflow_supplied map (which prior steps may have updated).
                        let supplied = build_supplied_views(
                            compiled,
                            &workflow_supplied,
                            &flag_names,
                            &default_added,
                        );
                        match frontend.gather_workflow_inputs(&needed, step, &supplied) {
                            Ok(gathered) => {
                                for entry in &needed {
                                    let Some(value) = gathered.slot_values.get(&entry.id) else {
                                        continue;
                                    };
                                    match &entry.scope {
                                    ags_protocol::workflow::AutoDeriveScope::WorkflowInput {
                                        name,
                                    } => {
                                        workflow_supplied.insert(name.clone(), value.clone());
                                    }
                                    ags_protocol::workflow::AutoDeriveScope::StepLocal {
                                        field_name,
                                    } => {
                                        step_local.insert(field_name.clone(), value.clone());
                                    }
                                }
                                }
                                // Apply any edits the frontend made to already-supplied inputs.
                                for (k, v) in gathered.input_overrides {
                                    workflow_supplied.insert(k, v);
                                }
                            }
                            Err(error) => {
                                let mut facts =
                                    ags_protocol::workflow::StepErrorFacts::from_error(&error);
                                facts.input_fields = step_input_fields_on_failure(
                                    step,
                                    &ctx,
                                    &workflow_supplied,
                                    &step_local,
                                    &compiled.inputs,
                                    &service_schema,
                                    &default_added,
                                    &flag_names,
                                    run_context.options.is_bundled_workflow,
                                );
                                finish_step_terminal(
                                    frontend,
                                    &mut step_summaries,
                                    step,
                                    format!("{} failed", step.id),
                                    StepOutcome::Failed,
                                    Some(ags_protocol::workflow::StepOutcomeReason::Gather),
                                    0,
                                    step_started_at,
                                    Some(facts),
                                    &mut run_facts,
                                );
                                pending_error = Some(error);
                                run_outcome = RunOutcome::Failed;
                                break;
                            }
                        }
                    }
                }

                // 3. Assemble the dispatch request.
                let namespace = run_context.runtime.context().namespace.clone();
                let request = match assemble_command_request(
                    step,
                    &ctx,
                    &workflow_supplied,
                    &step_local,
                    &compiled.inputs,
                    &service_schema,
                    namespace,
                    run_context.options,
                ) {
                    Ok(req) => req,
                    Err(error) => {
                        let mut facts = ags_protocol::workflow::StepErrorFacts::from_error(&error);
                        facts.input_fields = step_input_fields_on_failure(
                            step,
                            &ctx,
                            &workflow_supplied,
                            &step_local,
                            &compiled.inputs,
                            &service_schema,
                            &default_added,
                            &flag_names,
                            run_context.options.is_bundled_workflow,
                        );
                        finish_step_terminal(
                            frontend,
                            &mut step_summaries,
                            step,
                            format!("{} failed", step.id),
                            StepOutcome::Failed,
                            Some(ags_protocol::workflow::StepOutcomeReason::Assembly),
                            0,
                            step_started_at,
                            Some(facts),
                            &mut run_facts,
                        );
                        pending_error = Some(error);
                        run_outcome = RunOutcome::Failed;
                        break;
                    }
                };

                // 4a. Confirmation gate: if the step declares `confirm: true` and
                // neither --yes nor --dry-run is active, ask the frontend before
                // proceeding. Decline → Cancelled; I/O error → Failed.
                if step.confirm && !run_context.options.assume_yes && !run_context.options.dry_run {
                    let preview =
                        match build_step_preview(compiled, step, &request, run_context.runtime) {
                            Ok(p) => p,
                            Err(error) => {
                                let mut facts =
                                    ags_protocol::workflow::StepErrorFacts::from_error(&error);
                                facts.input_fields = step_input_fields_on_failure(
                                    step,
                                    &ctx,
                                    &workflow_supplied,
                                    &step_local,
                                    &compiled.inputs,
                                    &service_schema,
                                    &default_added,
                                    &flag_names,
                                    run_context.options.is_bundled_workflow,
                                );
                                finish_step_terminal(
                                    frontend,
                                    &mut step_summaries,
                                    step,
                                    format!("{} failed", step.id),
                                    StepOutcome::Failed,
                                    // Building the confirm gate's preview
                                    // failed: a preview-stage breakage, not the
                                    // user abandoning at the gate.
                                    Some(ags_protocol::workflow::StepOutcomeReason::Preview),
                                    0,
                                    step_started_at,
                                    Some(facts),
                                    &mut run_facts,
                                );
                                pending_error = Some(error);
                                run_outcome = RunOutcome::Failed;
                                break;
                            }
                        };
                    match frontend.confirm_step(step, &preview) {
                        Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed) => { /* proceed */ }
                        Ok(ags_protocol::workflow::StepConfirmOutcome::Skip)
                            if step.is_optional =>
                        {
                            skip_step(
                                &mut ctx,
                                frontend,
                                &mut step_summaries,
                                step,
                                ags_protocol::workflow::StepOutcomeReason::DeclinedAtConfirm,
                                None,
                                0,
                                step_started_at,
                                // As at the review gate: a user skip, no error.
                                None,
                                &mut run_facts,
                            )?;
                            continue;
                        }
                        Ok(ags_protocol::workflow::StepConfirmOutcome::Skip) => {
                            // Defensive: a frontend must only return Skip for an
                            // optional step. Skipping a non-optional step could
                            // strand a downstream reference (the compile rule only
                            // guarantees defaults for skippable steps). Treat as an
                            // internal-invariant failure, not a silent skip.
                            let error = ags_protocol::error::RuntimeError::internal(format!(
                                "frontend returned Skip for non-optional step '{}'",
                                step.id
                            ));
                            let mut facts =
                                ags_protocol::workflow::StepErrorFacts::from_error(&error);
                            facts.input_fields = step_input_fields_on_failure(
                                step,
                                &ctx,
                                &workflow_supplied,
                                &step_local,
                                &compiled.inputs,
                                &service_schema,
                                &default_added,
                                &flag_names,
                                run_context.options.is_bundled_workflow,
                            );
                            finish_step_terminal(
                                frontend,
                                &mut step_summaries,
                                step,
                                format!("{} failed", step.id),
                                StepOutcome::Failed,
                                Some(ags_protocol::workflow::StepOutcomeReason::FrontendContract),
                                0,
                                step_started_at,
                                Some(facts),
                                &mut run_facts,
                            );
                            pending_error = Some(error);
                            run_outcome = RunOutcome::Failed;
                            break;
                        }
                        Ok(ags_protocol::workflow::StepConfirmOutcome::Cancel) => {
                            finish_step_terminal(
                                frontend,
                                &mut step_summaries,
                                step,
                                format!("{} cancelled", step.id),
                                StepOutcome::Cancelled,
                                Some(ags_protocol::workflow::StepOutcomeReason::AtConfirm),
                                0,
                                step_started_at,
                                None,
                                &mut run_facts,
                            );
                            run_outcome = RunOutcome::Cancelled;
                            break;
                        }
                        Err(error) => {
                            let mut facts =
                                ags_protocol::workflow::StepErrorFacts::from_error(&error);
                            facts.input_fields = step_input_fields_on_failure(
                                step,
                                &ctx,
                                &workflow_supplied,
                                &step_local,
                                &compiled.inputs,
                                &service_schema,
                                &default_added,
                                &flag_names,
                                run_context.options.is_bundled_workflow,
                            );
                            finish_step_terminal(
                                frontend,
                                &mut step_summaries,
                                step,
                                format!("{} failed", step.id),
                                StepOutcome::Failed,
                                // The gate broke as I/O — the user never got to
                                // answer. `AtConfirm` is reserved for the
                                // cancellation arm above so one label never
                                // mixes abandonment with breakage.
                                Some(ags_protocol::workflow::StepOutcomeReason::Confirm),
                                0,
                                step_started_at,
                                Some(facts),
                                &mut run_facts,
                            );
                            pending_error = Some(error);
                            run_outcome = RunOutcome::Failed;
                            break;
                        }
                    }
                }

                // 4b. Dispatch: dry-run branch synthesises placeholder outputs
                // and builds a `StepDryRunPreview`; the live branch dispatches
                // via `Runtime::run_command` and stores the real `ApiOutput`.
                let (outcome, summary, reason, error_facts) = if run_context.options.dry_run {
                    // A dry run never dispatches, but `attempts: 0` would wrongly
                    // read as "never called the API" for a step that did run its
                    // dry-run synthesis; 1 keeps the field meaningful.
                    attempts = 1;
                    match crate::runtime::workflows::dry_run::synthesise_dry_run_outputs(
                        step,
                        run_context.runtime.catalogue_mut(),
                    ) {
                        Ok(synthesised) => {
                            ctx.inject_step_captures_for_dry_run(&step.id, &synthesised);
                            match crate::runtime::workflows::dry_run::build_step_dry_run_preview(
                                step,
                                &request,
                                &synthesised,
                                run_context.runtime,
                            ) {
                                Ok(preview) => {
                                    dry_run_previews.push(preview);
                                    (
                                        StepOutcome::Success,
                                        format!("{} dry-run", step.id),
                                        None,
                                        None,
                                    )
                                }
                                Err(error) => {
                                    let s = format!("{} failed", step.id);
                                    let mut facts =
                                        ags_protocol::workflow::StepErrorFacts::from_error(&error);
                                    facts.input_fields = step_input_fields_on_failure(
                                        step,
                                        &ctx,
                                        &workflow_supplied,
                                        &step_local,
                                        &compiled.inputs,
                                        &service_schema,
                                        &default_added,
                                        &flag_names,
                                        run_context.options.is_bundled_workflow,
                                    );
                                    pending_error = Some(error);
                                    (
                                        StepOutcome::Failed,
                                        s,
                                        Some(ags_protocol::workflow::StepOutcomeReason::Preview),
                                        Some(facts),
                                    )
                                }
                            }
                        }
                        Err(error) => {
                            let s = format!("{} failed", step.id);
                            let mut facts =
                                ags_protocol::workflow::StepErrorFacts::from_error(&error);
                            facts.input_fields = step_input_fields_on_failure(
                                step,
                                &ctx,
                                &workflow_supplied,
                                &step_local,
                                &compiled.inputs,
                                &service_schema,
                                &default_added,
                                &flag_names,
                                run_context.options.is_bundled_workflow,
                            );
                            pending_error = Some(error);
                            (
                                StepOutcome::Failed,
                                s,
                                Some(ags_protocol::workflow::StepOutcomeReason::Preview),
                                Some(facts),
                            )
                        }
                    }
                } else {
                    // Retry loop: a gate `Retry` re-dispatches; auto-skip / user-skip
                    // `continue 'step_loop` (skip_step emits its own StepFinished);
                    // success / cancel `break` with the (outcome, summary).
                    loop {
                        attempts = attempts.saturating_add(1);
                        // Progress events from dispatch are sunk into a tiny adapter
                        // that forwards them as WorkflowEvent::Progress with the step
                        // index attached. The block scope releases the &mut frontend
                        // borrow held by the adapter before the failure gate below.
                        let dispatch_result = {
                            let mut sink = crate::runtime::workflows::executor::progress_adapter(
                                step.index, frontend,
                            );
                            run_context.runtime.run_command(&request, &mut sink).await
                        };

                        // A success path breaks out with its (outcome, summary,
                        // reason, error_facts). Dispatch, output-binding, and
                        // unexpected-envelope failures all funnel into
                        // `attempt_error` (paired with the stage it failed at)
                        // so every failure pauses at the same gate — not just
                        // the ones that fail at the HTTP layer.
                        let (attempt_error, attempt_stage) = match dispatch_result {
                            Ok(CommandOutput::Service(api_output)) => {
                                ctx.store_step_output(&step.id, *api_output);
                                let body_json = ctx.step_body_json(&step.id);
                                match ctx.bind_step_outputs(
                                    &step.id,
                                    &step.outputs,
                                    body_json.as_ref(),
                                ) {
                                    Ok(()) => {
                                        break (
                                            StepOutcome::Success,
                                            format_step_summary(step, body_json.as_ref()),
                                            None,
                                            None,
                                        );
                                    }
                                    Err(error) => {
                                        (error, ags_protocol::workflow::StepOutcomeReason::Capture)
                                    }
                                }
                            }
                            Ok(CommandOutput::BinaryWritten(binary)) => {
                                // A binary response body, or `--output` to a file/
                                // stdout, was written by `run_command`.
                                ctx.store_binary_output(&step.id, binary);
                                break (
                                    StepOutcome::Success,
                                    format!("{} ok", step.id),
                                    None,
                                    None,
                                );
                            }
                            // Dispatch returned a non-Service envelope.
                            Ok(other) => (
                                RuntimeError::internal(format!(
                                    "step '{}' dispatch returned unexpected envelope {:?}",
                                    step.id,
                                    std::mem::discriminant(&other)
                                )),
                                ags_protocol::workflow::StepOutcomeReason::FrontendContract,
                            ),
                            Err(error) => {
                                (error, ags_protocol::workflow::StepOutcomeReason::Dispatch)
                            }
                        };

                        match decide_step_failure(
                            step,
                            &attempt_error,
                            run_context.options.no_input,
                            frontend,
                            attempt_stage,
                        )? {
                            FailureDisposition::Skip {
                                reason,
                                summary_tail,
                            } => {
                                // Every skip decided here answers a real
                                // failure (`already_exists`,
                                // `tolerated_failure`, or the user
                                // declining at the failure gate), so the
                                // skip carries the same error triple (and
                                // resolved input fields) a fatal outcome
                                // would have.
                                let mut skip_facts =
                                    ags_protocol::workflow::StepErrorFacts::from_error(
                                        &attempt_error,
                                    );
                                skip_facts.input_fields = step_input_fields_on_failure(
                                    step,
                                    &ctx,
                                    &workflow_supplied,
                                    &step_local,
                                    &compiled.inputs,
                                    &service_schema,
                                    &default_added,
                                    &flag_names,
                                    run_context.options.is_bundled_workflow,
                                );
                                skip_step(
                                    &mut ctx,
                                    frontend,
                                    &mut step_summaries,
                                    step,
                                    reason,
                                    summary_tail.as_deref(),
                                    attempts,
                                    step_started_at,
                                    Some(skip_facts),
                                    &mut run_facts,
                                )?;
                                continue 'step_loop;
                            }
                            FailureDisposition::Retry => continue,
                            FailureDisposition::Fatal { reason } => {
                                let s = format!("{} failed", step.id);
                                let mut facts = ags_protocol::workflow::StepErrorFacts::from_error(
                                    &attempt_error,
                                );
                                facts.input_fields = step_input_fields_on_failure(
                                    step,
                                    &ctx,
                                    &workflow_supplied,
                                    &step_local,
                                    &compiled.inputs,
                                    &service_schema,
                                    &default_added,
                                    &flag_names,
                                    run_context.options.is_bundled_workflow,
                                );
                                pending_error = Some(attempt_error);
                                break (StepOutcome::Failed, s, Some(reason), Some(facts));
                            }
                        }
                    }
                };

                step_summaries.push(summary.clone());
                let captures = if matches!(outcome, StepOutcome::Success) {
                    build_step_captures(
                        step,
                        &ctx,
                        &workflow_supplied,
                        &step_local,
                        &compiled.inputs,
                        &service_schema,
                        &default_added,
                    )
                } else {
                    Vec::new()
                };
                tally_step_outcome(&mut run_facts, outcome);
                frontend.on_event(&WorkflowEvent::StepFinished {
                    index: step.index,
                    id: step.id.clone(),
                    summary,
                    captures,
                    outcome,
                    reason,
                    attempts,
                    duration_ms: step_duration_ms(step_started_at),
                    error: error_facts,
                });

                if outcome == StepOutcome::Failed {
                    run_outcome = RunOutcome::Failed;
                    break;
                }
            }
        } // end if run_outcome == RunOutcome::Success (Phase-1 guard)

        // Finalisation.
        let final_output = if run_outcome == RunOutcome::Success {
            build_final_output(
                compiled,
                &ctx,
                &step_summaries,
                &dry_run_previews,
                run_context.options,
                &workflow_supplied,
            )
        } else {
            None
        };

        run_facts.duration_ms = step_duration_ms(run_started_at);
        frontend.on_event(&WorkflowEvent::WorkflowFinished {
            outcome: run_outcome,
            facts: run_facts,
        });
        Ok((run_outcome, final_output, pending_error))
    }
}

/// Elapsed milliseconds since `started_at`, saturating rather than wrapping on
/// an implausibly long run.
fn step_duration_ms(started_at: std::time::Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Build a failed step's `input_fields` telemetry facts by resolving its
/// field plan lazily — called only once a step has already failed and a
/// `StepErrorFacts` is being built for it, so the success path never pays
/// this cost. Degrades to an empty vector if `resolve_step_fields` itself
/// errors: this is telemetry and must never affect the run outcome.
#[allow(clippy::too_many_arguments)]
fn step_input_fields_on_failure(
    step: &ags_protocol::workflow::CompiledStep,
    ctx: &crate::runtime::workflows::WorkflowContext,
    workflow_supplied: &std::collections::BTreeMap<String, serde_json::Value>,
    step_local: &std::collections::BTreeMap<String, serde_json::Value>,
    workflow_input_specs: &[ags_protocol::workflow::WorkflowInputSpec],
    service_schema: &ags_protocol::catalogue::ServiceSchema,
    default_names: &std::collections::BTreeSet<String>,
    flag_names: &std::collections::BTreeSet<String>,
    bundled: bool,
) -> Vec<ags_protocol::workflow::StepInputField> {
    let plan = match crate::runtime::workflows::resolve::resolve_step_fields(
        step,
        ctx,
        workflow_supplied,
        step_local,
        workflow_input_specs,
        service_schema,
        default_names,
    ) {
        Ok(plan) => plan,
        Err(_) => return Vec::new(),
    };
    crate::runtime::workflows::telemetry_fields::build_step_input_fields(
        &plan.fields,
        flag_names,
        bundled,
    )
}

/// Tally one terminal step outcome into the run aggregate.
fn tally_step_outcome(facts: &mut ags_protocol::workflow::RunFacts, outcome: StepOutcome) {
    match outcome {
        StepOutcome::Success => facts.steps_succeeded += 1,
        StepOutcome::Failed => facts.steps_failed += 1,
        StepOutcome::Skipped => facts.steps_skipped += 1,
        StepOutcome::Cancelled => facts.steps_cancelled += 1,
    }
}

/// Emit a terminal `StepFinished` event (no captures), record its summary, and
/// tally its outcome into the run aggregate. Shared by every per-step early
/// exit — schema load, review, gather, assembly, confirm, and preview failures
/// or cancellations — so the bookkeeping stays in one place. The success path
/// emits its own `StepFinished` with real captures and tallies separately.
#[allow(clippy::too_many_arguments)]
fn finish_step_terminal(
    frontend: &mut dyn WorkflowFrontend,
    step_summaries: &mut Vec<String>,
    step: &ags_protocol::workflow::CompiledStep,
    summary: String,
    outcome: StepOutcome,
    reason: Option<ags_protocol::workflow::StepOutcomeReason>,
    attempts: u32,
    started_at: std::time::Instant,
    error: Option<ags_protocol::workflow::StepErrorFacts>,
    facts: &mut ags_protocol::workflow::RunFacts,
) {
    step_summaries.push(summary.clone());
    tally_step_outcome(facts, outcome);
    frontend.on_event(&WorkflowEvent::StepFinished {
        index: step.index,
        id: step.id.clone(),
        summary,
        captures: Vec::new(),
        outcome,
        reason,
        attempts,
        duration_ms: step_duration_ms(started_at),
        error,
    });
}

/// Bind a skipped step's outputs to their defaults and emit its terminal
/// `StepFinished(Skipped)`. Callers `continue` the step loop afterwards.
/// `reason` is the typed telemetry label for why the step was skipped;
/// `summary_tail` is the prose the human summary appends (kept separate
/// because it can embed raw API error text and must never be transmitted).
/// `error` carries the transmittable facts of the failure the skip was a
/// response to — `Some` for the three failure-driven skips (`already_exists`,
/// `tolerated_failure`, `declined_after_failure`), `None` for the two
/// user-initiated ones (review/confirm), which have no error in hand.
/// Tallying happens inside `finish_step_terminal`, not here — this delegates
/// to it and must not tally on its own, or every skip would be double-counted.
#[allow(clippy::too_many_arguments)]
fn skip_step(
    ctx: &mut crate::runtime::workflows::WorkflowContext,
    frontend: &mut dyn WorkflowFrontend,
    step_summaries: &mut Vec<String>,
    step: &ags_protocol::workflow::CompiledStep,
    reason: ags_protocol::workflow::StepOutcomeReason,
    summary_tail: Option<&str>,
    attempts: u32,
    started_at: std::time::Instant,
    error: Option<ags_protocol::workflow::StepErrorFacts>,
    facts: &mut ags_protocol::workflow::RunFacts,
) -> Result<(), ags_protocol::error::RuntimeError> {
    ctx.bind_skipped_outputs(&step.id, &step.outputs)?;
    let summary = match summary_tail {
        Some(tail) => format!("{} skipped — {}", step.id, tail),
        None => format!("{} skipped", step.id),
    };
    finish_step_terminal(
        frontend,
        step_summaries,
        step,
        summary,
        StepOutcome::Skipped,
        Some(reason),
        attempts,
        started_at,
        error,
        facts,
    );
    Ok(())
}

/// Build the editable supplied-input views shown to the frontend during gather:
/// one per declared input that currently holds a non-null, flag- or default-
/// sourced value.
///
/// An input produces no view (the three cases are indistinguishable to callers)
/// when it: has no supplied value; holds a null value; or was written by a prior
/// step's gather (prior-step captures are not editable pre-fills).
fn build_supplied_views(
    compiled: &CompiledWorkflow,
    workflow_supplied: &std::collections::BTreeMap<String, serde_json::Value>,
    flag_names: &std::collections::BTreeSet<String>,
    default_added: &std::collections::BTreeSet<String>,
) -> Vec<ags_protocol::workflow::SuppliedInputView> {
    compiled
        .inputs
        .iter()
        .filter_map(|spec| {
            let value = workflow_supplied.get(&spec.name)?.clone();
            // A null value is never a useful pre-fill (this also covers the
            // service route's synthetic `--json` body markers, which are
            // flag-sourced Nulls).
            if value.is_null() {
                return None;
            }
            let source = if flag_names.contains(&spec.name) {
                ags_protocol::workflow::SuppliedSource::FromFlag
            } else if default_added.contains(&spec.name) {
                ags_protocol::workflow::SuppliedSource::Default
            } else {
                return None;
            };
            Some(ags_protocol::workflow::SuppliedInputView {
                label: spec.name.clone(),
                value,
                schema: spec.schema.clone().unwrap_or(serde_json::Value::Null),
                description: spec.description.clone(),
                source,
                location: spec.location,
            })
        })
        .collect()
}

/// Apply the user's per-step review edits to `step_local`. The body-overflow
/// field carries a JSON object whose keys are body field names — each is merged
/// in individually; every other edit is a single step-local field value.
/// Input-backed and prior-output fields are read-only and never appear here.
fn apply_step_review_edits(
    plan: &ags_protocol::workflow::StepFieldPlan,
    edits: ags_protocol::workflow::StepFieldEdits,
    step_local: &mut std::collections::BTreeMap<String, serde_json::Value>,
) {
    let by_id: std::collections::BTreeMap<_, _> = plan.fields.iter().map(|f| (f.id, f)).collect();
    for (id, value) in edits.values {
        let Some(field) = by_id.get(&id) else {
            continue;
        };
        if field.body_overflow {
            if let Some(object) = value.as_object() {
                for (key, sub_value) in object {
                    step_local.insert(key.clone(), sub_value.clone());
                }
            }
            continue;
        }
        step_local.insert(field.field.clone(), value);
    }
}

/// Build the per-step Summary-panel captures from the step's resolved field
/// plan. Includes workflow inputs the step actually consumes (incl. derived
/// values) plus step-local options the workflow marked `show_in_review`;
/// omits read-only references to prior step outputs (already visible as the
/// source step's own captures) and the synthetic body-overflow field.
/// Returns an empty vec when the plan can't be resolved — the rest of the
/// run still has a useful summary even without captures.
pub fn build_step_captures(
    step: &ags_protocol::workflow::CompiledStep,
    ctx: &crate::runtime::workflows::WorkflowContext,
    workflow_supplied: &std::collections::BTreeMap<String, serde_json::Value>,
    step_local: &std::collections::BTreeMap<String, serde_json::Value>,
    workflow_input_specs: &[ags_protocol::workflow::WorkflowInputSpec],
    service_schema: &ags_protocol::catalogue::ServiceSchema,
    default_names: &std::collections::BTreeSet<String>,
) -> Vec<(String, String)> {
    use ags_protocol::workflow::StepFieldSource;
    let plan = match crate::runtime::workflows::resolve::resolve_step_fields(
        step,
        ctx,
        workflow_supplied,
        step_local,
        workflow_input_specs,
        service_schema,
        default_names,
    ) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    plan.fields
        .into_iter()
        .filter(|f| !f.body_overflow)
        .filter(|f| match &f.source {
            // Prior-step outputs are already shown as the source step's own
            // captures — repeating them on every downstream consumer adds
            // noise without information.
            StepFieldSource::PriorOutput => false,
            // Literal step-local values only surface when the workflow
            // explicitly marks them reviewable; treat the captures panel as
            // an extension of the same review surface.
            StepFieldSource::Literal => f.show_in_review,
            // Workflow inputs (supplied, defaulted, or derived) are always
            // worth showing — they're how the user parameterised the run.
            StepFieldSource::WorkflowInput { .. }
            | StepFieldSource::Default { .. }
            | StepFieldSource::Derived { .. } => true,
            StepFieldSource::Unset => false,
        })
        .map(|f| (f.label, format_capture_value(&f.value)))
        .collect()
}

/// Render a captured field value for the Summary panel: scalars verbatim
/// (with a single-line truncation cap), bool/number as their stringification,
/// `null` as an em-dash, and containers as compact JSON truncated to the
/// same width budget. The cap keeps each line readable inside the panel.
fn format_capture_value(v: &serde_json::Value) -> String {
    const MAX: usize = 40;
    let raw = match v {
        serde_json::Value::Null => "—".to_string(),
        // Capture values derive from API response bodies; strip terminal control
        // sequences before they reach the Summary panel (CONTRIBUTING § Security:
        // all API-sourced strings are sanitised before terminal rendering). The
        // container arm is already safe — serde_json escapes control chars.
        serde_json::Value::String(s) => {
            crate::support::strings::strip_terminal_control_sequences(s)
        }
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    if raw.chars().count() <= MAX {
        raw
    } else {
        let mut truncated: String = raw.chars().take(MAX.saturating_sub(1)).collect();
        truncated.push('\u{2026}');
        truncated
    }
}

/// One-line success summary for a step: currently just `"{step_id}: ok"`.
/// `body_json` is reserved for a future enriched summary (a shape hint derived
/// from the response body) and is unused today. Frontends are free to re-format.
pub fn format_step_summary(
    step: &ags_protocol::workflow::CompiledStep,
    _body_json: Option<&serde_json::Value>,
) -> String {
    format!("{} ok", step.id)
}

/// The specific failure tail for a step summary: the error's `reason` when it is
/// present AND non-empty (e.g. the server's own message), else the headline
/// `message`. Keeps a summary from reading `… failed — … failed` once error
/// headlines became "<operation> failed", and from reading `… failed — ` when the
/// curated path preserved an empty server message as `Some("")`.
fn error_reason_tail(error: &RuntimeError) -> String {
    error
        .reason_detail()
        .map(String::from)
        .unwrap_or_else(|| error.message.clone())
}

/// True when `error` is an HTTP 409 whose text identifies it as an
/// "already exists" conflict. Combined with a step's `skip_if_exists` opt-in to
/// auto-skip already-exists creates.
///
/// The status alone is not enough: AccelByte returns 409 for other conflicts too
/// (optimistic concurrency, "resource has changed"). Auto-skipping bypasses the
/// interactive gate, so a spurious 409 on a marked create would be silently
/// swallowed. We therefore require the "already exists" wording, which the
/// curated per-service code mappings put in the error's message, reason, or
/// detail (`error_codes/*.rs`, e.g. "Currency already exists",
/// "Category already exists"). An uncatalogued / differently-worded 409 does not
/// match and falls through to the gate — the safe direction.
pub(crate) fn is_conflict(error: &RuntimeError) -> bool {
    if !matches!(error.kind, RuntimeErrorKind::Upstream { status: 409, .. }) {
        return false;
    }
    let has_phrase = |s: &str| s.to_ascii_lowercase().contains("already exists");
    if has_phrase(&error.message) {
        return true;
    }
    error
        .details
        .as_ref()
        .map(|d| {
            d.reason.as_deref().is_some_and(has_phrase)
                || d.detail.as_deref().is_some_and(has_phrase)
        })
        .unwrap_or(false)
}

/// A step is safe to skip when every captured output has a default, so skipping
/// leaves no downstream binding unresolved. A step that captures nothing is
/// vacuously safe. Gates the failure gate's Skip action.
pub(crate) fn is_safely_skippable(step: &ags_protocol::workflow::CompiledStep) -> bool {
    step.outputs.iter().all(|o| o.default.is_some())
}

/// Outcome of deciding what to do with a failed step (no I/O).
#[derive(Debug)]
pub(crate) enum FailureDisposition {
    /// Skip the step and continue. `reason` is the typed telemetry label;
    /// `summary_tail` is the prose the human summary appends, kept separate
    /// because it can embed raw API error text and must never be transmitted.
    Skip {
        reason: ags_protocol::workflow::StepOutcomeReason,
        summary_tail: Option<String>,
    },
    /// Re-dispatch the same step.
    Retry,
    /// Stop the run; the error stands. `reason` names the stage the failure is
    /// attributed to: the attempt's own failing stage when the run gave up
    /// unattended (`--no-input`), or the gate the user was at when they either
    /// declined an unsafe skip (`FrontendContract`) or chose to cancel
    /// (`AtFailureGate`).
    Fatal {
        reason: ags_protocol::workflow::StepOutcomeReason,
    },
}

/// Decide what to do when `step`'s dispatch returns `error`. Pure: consults the
/// step flags, the error, `no_input`, and — only when interactive — the frontend
/// gate. Order matches the design's precedence. `attempt_stage` is the stage the
/// current attempt actually failed at (dispatch, capture, or a frontend-contract
/// violation); it becomes the `Fatal` reason only on the `--no-input` short
/// circuit, where there is no interactive gate to attribute the failure to.
pub(crate) fn decide_step_failure(
    step: &ags_protocol::workflow::CompiledStep,
    error: &RuntimeError,
    no_input: bool,
    frontend: &mut dyn WorkflowFrontend,
    attempt_stage: ags_protocol::workflow::StepOutcomeReason,
) -> Result<FailureDisposition, RuntimeError> {
    if step.continue_on_failure {
        return Ok(FailureDisposition::Skip {
            reason: ags_protocol::workflow::StepOutcomeReason::ToleratedFailure,
            summary_tail: Some(error_reason_tail(error)),
        });
    }
    if step.skip_if_exists && is_conflict(error) {
        return Ok(FailureDisposition::Skip {
            reason: ags_protocol::workflow::StepOutcomeReason::AlreadyExists,
            summary_tail: Some("already exists".to_string()),
        });
    }
    if no_input {
        return Ok(FailureDisposition::Fatal {
            reason: attempt_stage,
        });
    }
    let safe = is_safely_skippable(step);
    match frontend.resolve_step_failure(step, error, safe)? {
        ags_protocol::workflow::StepFailureAction::Retry => Ok(FailureDisposition::Retry),
        // Defensive: surfaces are told to hide Skip when `safe` is false, but a
        // buggy/default/test frontend could still return Skip. Skipping an unsafe
        // step would bind a null into a needed downstream reference, so treat it
        // as Fatal — never skip an unsafe step regardless of the frontend.
        ags_protocol::workflow::StepFailureAction::Skip if safe => Ok(FailureDisposition::Skip {
            reason: ags_protocol::workflow::StepOutcomeReason::DeclinedAfterFailure,
            summary_tail: None,
        }),
        ags_protocol::workflow::StepFailureAction::Skip => Ok(FailureDisposition::Fatal {
            reason: ags_protocol::workflow::StepOutcomeReason::FrontendContract,
        }),
        ags_protocol::workflow::StepFailureAction::Cancel => Ok(FailureDisposition::Fatal {
            reason: ags_protocol::workflow::StepOutcomeReason::AtFailureGate,
        }),
    }
}

/// Construct a `StepPreview` to feed `WorkflowFrontend::confirm_step`. Calls
/// `Runtime::preview_command` for the embedded `CommandPreview`. Errors
/// (operation lookup, URL templating) flow back so the executor can route
/// through the standard step-failure path.
pub fn build_step_preview(
    compiled: &ags_protocol::workflow::CompiledWorkflow,
    step: &ags_protocol::workflow::CompiledStep,
    final_request: &ags_protocol::request::CommandRequest,
    runtime: &mut Runtime,
) -> Result<ags_protocol::workflow::StepPreview, RuntimeError> {
    let command = runtime.preview_command(final_request)?;
    Ok(ags_protocol::workflow::StepPreview {
        workflow_name: compiled.name.clone(),
        step_id: step.id.clone(),
        step_label: step.description.clone().unwrap_or_else(|| step.id.clone()),
        step_index: step.index,
        step_total: compiled.steps.len(),
        command,
    })
}

/// Envelope selection per the per-step output handling rules: routes to the
/// correct `CommandOutput` variant based on dry-run mode, step count, and
/// whether the workflow declares output aliases.
pub fn build_final_output(
    compiled: &ags_protocol::workflow::CompiledWorkflow,
    ctx: &crate::runtime::workflows::WorkflowContext,
    step_summaries: &[String],
    dry_run_previews: &[ags_protocol::workflow::StepDryRunPreview],
    options: &RunOptions,
    workflow_supplied: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Option<CommandOutput> {
    if options.dry_run {
        // A synthesised single command reports the bare request, so `--dry-run`
        // on `ags <service> <resource> <method>` looks the same as it always
        // has. A lone local-action step has no request and falls through to the
        // multi-step envelope.
        if compiled.steps.len() == 1 {
            if let Some(ags_protocol::workflow::StepDryRunAction::Request(command)) =
                dry_run_previews.first().map(|p| &p.action)
            {
                return Some(CommandOutput::DryRun(command.clone()));
            }
        }
        return Some(CommandOutput::WorkflowDryRun {
            workflow_id: compiled.id.clone(),
            step_previews: dry_run_previews.to_vec(),
        });
    }
    if compiled.steps.is_empty() {
        return None;
    }
    if compiled.steps.len() == 1 && compiled.outputs.is_empty() {
        let step_id = &compiled.steps[0].id;
        if let Some(binary) = ctx.binary_output(step_id) {
            return Some(CommandOutput::BinaryWritten(binary.clone()));
        }
        return ctx
            .step_output(step_id)
            .cloned()
            .map(|api| CommandOutput::Service(Box::new(api)));
    }
    Some(CommandOutput::Workflow {
        workflow_id: compiled.id.clone(),
        outputs: ctx.resolve_workflow_outputs(&compiled.outputs),
        step_summaries: step_summaries.to_vec(),
        completion: crate::runtime::workflows::resolve::resolve_completion(
            &compiled.completion,
            workflow_supplied,
        ),
        output_view: {
            let has_view = compiled
                .outputs
                .iter()
                .any(|a| a.section.is_some() || a.label.is_some());
            if has_view {
                Some(ctx.resolve_workflow_output_view(&compiled.outputs))
            } else {
                None
            }
        },
    })
}

/// One reason a workflow cannot run under `--no-input`.
#[derive(Debug, Clone, PartialEq)]
pub enum NoInputViolation {
    /// A required (or referenced) workflow input has no value supplied.
    MissingInput {
        /// Name of the unsupplied workflow input.
        name: String,
        /// Step id where the executor first noticed.
        first_seen_at: String,
    },
    /// A step has `confirm: true` and dry-run is not active.
    ConfirmRequired {
        /// Step id.
        step: String,
    },
    /// A step has a step-local field that would trigger gather.
    StepLocalGather {
        /// Step id.
        step: String,
        /// Operation field name.
        field: String,
    },
}

/// Simulate the executor's gather-detection logic against the compiled
/// workflow, returning every reason it cannot proceed non-interactively.
///
/// `assume_yes` (the `--yes` flag) pre-approves every confirmation, so a
/// `confirm: true` step is not a blocker when it is set — only a confirmation
/// that would otherwise need an interactive prompt counts as a violation.
pub fn no_input_precheck(
    compiled: &ags_protocol::workflow::CompiledWorkflow,
    workflow_supplied: &std::collections::BTreeMap<String, serde_json::Value>,
    dry_run: bool,
    assume_yes: bool,
) -> Vec<NoInputViolation> {
    use crate::runtime::workflows::resolve::compute_needed_inputs;

    let mut issues: Vec<NoInputViolation> = Vec::new();
    let mut sim_supplied = workflow_supplied.clone();
    let sentinel = serde_json::Value::Null;
    for step in &compiled.steps {
        let needed = compute_needed_inputs(step, &sim_supplied, &compiled.inputs);
        for entry in &needed {
            match &entry.scope {
                ags_protocol::workflow::AutoDeriveScope::WorkflowInput { name } => {
                    issues.push(NoInputViolation::MissingInput {
                        name: name.clone(),
                        first_seen_at: step.id.clone(),
                    });
                    sim_supplied.insert(name.clone(), sentinel.clone());
                }
                ags_protocol::workflow::AutoDeriveScope::StepLocal { field_name } => {
                    issues.push(NoInputViolation::StepLocalGather {
                        step: step.id.clone(),
                        field: field_name.clone(),
                    });
                }
            }
        }
        if step.confirm && !dry_run && !assume_yes {
            issues.push(NoInputViolation::ConfirmRequired {
                step: step.id.clone(),
            });
        }
    }
    issues
}

/// Machine-readable code for the first violation's kind. Only the *kind* is
/// encoded — never an input, step, or field name, which are user-authored for
/// an external workflow and must not be transmitted.
fn no_input_violation_code(violation: &NoInputViolation) -> &'static str {
    match violation {
        NoInputViolation::MissingInput { .. } => "no_input.missing_input",
        NoInputViolation::ConfirmRequired { .. } => "no_input.confirm_required",
        NoInputViolation::StepLocalGather { .. } => "no_input.step_local_gather",
    }
}

/// Build an aggregated `RuntimeError` from a non-empty violation list.
pub fn no_input_violations_to_error(
    violations: &[NoInputViolation],
    single_command: bool,
) -> RuntimeError {
    let mut lines: Vec<String> = Vec::with_capacity(violations.len());
    for v in violations {
        // A synthesised single command has one step ("main") the user never
        // named, so its messages drop the "workflow"/"step" vocabulary and
        // speak of the command itself.
        let line = match (v, single_command) {
            (NoInputViolation::MissingInput { name, .. }, true) => {
                format!("input '{name}' is required")
            }
            (NoInputViolation::ConfirmRequired { .. }, true) => {
                "confirmation is required".to_string()
            }
            (NoInputViolation::StepLocalGather { field, .. }, true) => {
                format!("field '{field}' is unbound; cannot gather under --no-input")
            }
            (
                NoInputViolation::MissingInput {
                    name,
                    first_seen_at,
                },
                false,
            ) => {
                format!("workflow input '{name}' is required (first referenced by step '{first_seen_at}')")
            }
            (NoInputViolation::ConfirmRequired { step }, false) => {
                format!("step '{step}' requires confirmation; cannot run under --no-input")
            }
            (NoInputViolation::StepLocalGather { step, field }, false) => {
                format!("step '{step}' field '{field}' is unbound; cannot gather under --no-input")
            }
        };
        lines.push(line);
    }
    // Cap a long list so the error stays readable: show the first few and a
    // "(+ N more)" tail. Only truncate when it hides at least two lines.
    // Matches the service-route missing-input cap.
    const MAX_LISTED: usize = 3;
    if lines.len() > MAX_LISTED + 1 {
        let extra = lines.len() - MAX_LISTED;
        lines.truncate(MAX_LISTED);
        lines.push(format!("(+ {extra} more)"));
    }
    // A non-interactive rejection is a usage error (the caller did not supply
    // enough on the command line), not an internal bug — `Validation` kind maps
    // to `CliError::Usage` → exit 1. Structured as headline / Reason / Fix to
    // match the other "interactive input unavailable" errors.
    RuntimeError {
        kind: RuntimeErrorKind::Validation,
        message: format!(
            "{}\n  {}",
            if single_command {
                "This command cannot run non-interactively:"
            } else {
                "Cannot run this workflow non-interactively:"
            },
            lines.join("\n  ")
        ),
        details: Some(Box::new(ErrorDetails {
            code: violations
                .first()
                .map(|v| no_input_violation_code(v).to_string()),
            reason: Some(
                "The run is non-interactive (--no-input, or stdin and stderr are not terminals)."
                    .to_string(),
            ),
            detail: None,
            suggestion_kind: None,
            tip: None,
        })),
        hint: Some(
            "Pass the missing inputs as --<name> flags (use --yes to skip confirmations), or \
             run interactively with stdin and stderr attached to a terminal."
                .to_string(),
        ),
        trace: None,
    }
}

/// Adapter that turns a `&mut dyn WorkflowFrontend` into a
/// `ags_protocol::event::ProgressSink` (the trait `Runtime::run_command`
/// expects), attaching the step index to every forwarded event.
pub(crate) fn progress_adapter<'a>(
    step_index: usize,
    frontend: &'a mut dyn WorkflowFrontend,
) -> ProgressAdapter<'a> {
    ProgressAdapter {
        step_index,
        frontend,
    }
}

/// Lifetime-bound progress sink used by `progress_adapter`.
pub(crate) struct ProgressAdapter<'a> {
    step_index: usize,
    frontend: &'a mut dyn WorkflowFrontend,
}

impl<'a> ags_protocol::event::ProgressSink for ProgressAdapter<'a> {
    fn on_event(&mut self, event: ags_protocol::event::ProgressEvent) {
        self.frontend
            .on_event(&crate::runtime::workflows::WorkflowEvent::Progress {
                step_index: Some(self.step_index),
                event,
            });
    }
}

/// Reorder declared workflow inputs by the index of the step that first
/// references them, so the gather-inputs form lists inputs in the order the
/// workflow will actually use them. Stable for inputs used in the same step;
/// any input never referenced is placed at the end in its declared order.
///
/// An input that feeds another input's dynamic-enum picker (a `FromInput`
/// `options_source` dependency, e.g. a search query backing a user-id picker)
/// is pulled up to its picker's first use, so the dependency appears before the
/// picker rather than sinking to the end as an input no step references.
fn order_inputs_by_first_use(
    inputs: &[ags_protocol::workflow::WorkflowInputSpec],
    steps: &[ags_protocol::workflow::CompiledStep],
) -> Vec<ags_protocol::workflow::WorkflowInputSpec> {
    use crate::runtime::workflows::compile::visit_binding_workflow_inputs;
    use ags_protocol::workflow::{AutoDeriveScope, OptionParameterBinding};
    let first_use = |name: &str| -> usize {
        for (i, step) in steps.iter().enumerate() {
            // Binding references (target / arithmetic operand / format
            // placeholder) — shared with `compute_needed_inputs`.
            let mut referenced = false;
            visit_binding_workflow_inputs(step, |n| {
                if n == name {
                    referenced = true;
                }
            });
            if referenced {
                return i;
            }
            // Auto-derived workflow-input scopes are an input use too.
            for field in &step.auto_derived {
                if let AutoDeriveScope::WorkflowInput { name: scope_name } = &field.scope {
                    if scope_name == name {
                        return i;
                    }
                }
            }
        }
        usize::MAX
    };

    // Base first-use per input, then pull picker dependencies up to (no later
    // than) the picker that consumes them.
    let mut key: std::collections::HashMap<String, usize> = inputs
        .iter()
        .map(|spec| (spec.name.clone(), first_use(&spec.name)))
        .collect();
    for spec in inputs {
        if let Some(source) = &spec.options_source {
            let picker_use = key.get(&spec.name).copied().unwrap_or(usize::MAX);
            for binding in source.parameters.values() {
                if let OptionParameterBinding::FromInput(dep)
                | OptionParameterBinding::FromInputOptional(dep) = binding
                {
                    if let Some(slot) = key.get_mut(dep) {
                        *slot = (*slot).min(picker_use);
                    }
                }
            }
        }
    }

    let mut indexed: Vec<(usize, ags_protocol::workflow::WorkflowInputSpec)> = inputs
        .iter()
        .map(|spec| {
            (
                key.get(&spec.name).copied().unwrap_or(usize::MAX),
                spec.clone(),
            )
        })
        .collect();
    indexed.sort_by_key(|(idx, _)| *idx);
    indexed.into_iter().map(|(_, s)| s).collect()
}

/// The names of "picker-support" workflow inputs: those referenced by another
/// input's `options_source` (as a `FromInput` parameter binding or a
/// `LabelDetail::ByInput`) but not bound by any step. These exist only to
/// parameterise a dynamic-enum picker's search, so on a surface with no picker
/// they carry no meaning — the picker-backed input is typed directly instead.
///
/// An input referenced by both a picker and a step (e.g. `namespace`) is NOT
/// support: it is needed for the step regardless. A picker-backed input (e.g.
/// `userId`) is bound by steps, so it is likewise excluded.
fn picker_support_input_names(
    inputs: &[ags_protocol::workflow::WorkflowInputSpec],
    steps: &[ags_protocol::workflow::CompiledStep],
) -> std::collections::BTreeSet<String> {
    use crate::runtime::workflows::compile::visit_binding_workflow_inputs;
    use ags_protocol::workflow::{LabelDetail, OptionParameterBinding};

    // Names any step references (binding targets, transform operands, format
    // placeholders, and auto-derived workflow-input scopes).
    let mut step_referenced: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for step in steps {
        visit_binding_workflow_inputs(step, |name| {
            step_referenced.insert(name.to_string());
        });
        for field in &step.auto_derived {
            if let ags_protocol::workflow::AutoDeriveScope::WorkflowInput { name } = &field.scope {
                step_referenced.insert(name.clone());
            }
        }
    }

    // Names any picker (another input's options_source) draws from.
    let mut picker_referenced: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    for spec in inputs {
        if let Some(source) = &spec.options_source {
            for binding in source.parameters.values() {
                if let OptionParameterBinding::FromInput(name)
                | OptionParameterBinding::FromInputOptional(name) = binding
                {
                    picker_referenced.insert(name.clone());
                }
            }
            if let Some(LabelDetail::ByInput { input, .. }) = &source.label_detail {
                picker_referenced.insert(input.clone());
            }
        }
    }

    // Picker-support = picker-referenced minus anything a step needs.
    picker_referenced
        .into_iter()
        .filter(|name| !step_referenced.contains(name))
        .collect()
}

/// The declared inputs to gather up front, in first-use order, minus any
/// picker-support input when the active surface has no picker. With a picker
/// available (fullscreen), or when no input is picker-support, the first-use
/// ordering is returned unchanged.
fn gather_inputs_for_surface(
    inputs: &[ags_protocol::workflow::WorkflowInputSpec],
    steps: &[ags_protocol::workflow::CompiledStep],
    pickers_available: bool,
) -> Vec<ags_protocol::workflow::WorkflowInputSpec> {
    let ordered = order_inputs_by_first_use(inputs, steps);
    if pickers_available {
        return ordered;
    }
    let support = picker_support_input_names(inputs, steps);
    ordered
        .into_iter()
        .filter(|spec| !support.contains(&spec.name))
        .map(|mut spec| {
            // A picker-backed input renders as a plain text field here (no
            // picker), so swap in the author's no-picker copy when supplied.
            if let Some(fallback) = spec
                .options_source
                .as_ref()
                .and_then(|source| source.fallback_description.clone())
            {
                spec.description = Some(fallback);
            }
            spec
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_has_reviewable_fields() {
        use ags_protocol::workflow::{
            StepField, StepFieldId, StepFieldLocation, StepFieldPlan, StepFieldSource,
        };
        let field = |show_in_review: bool, required: bool, source: StepFieldSource| StepField {
            id: StepFieldId(0),
            field: "f".into(),
            label: "f".into(),
            description: None,
            location: StepFieldLocation::Body,
            schema: serde_json::json!({"type": "string"}),
            value: serde_json::Value::Null,
            source,
            required,
            workflow_input: None,
            body_overflow: false,
            show_in_review,
        };
        let plan = |fields: Vec<StepField>| StepFieldPlan {
            step_index: 0,
            step_label: "s".into(),
            step_description: None,
            optional: false,
            fields,
        };
        // Only auto-bound params, a hidden literal, and a prior-output ref →
        // nothing to review.
        assert!(!plan_has_reviewable_fields(&plan(vec![
            field(
                false,
                true,
                StepFieldSource::WorkflowInput {
                    name: "namespace".into()
                }
            ),
            field(false, false, StepFieldSource::Literal),
            field(false, true, StepFieldSource::PriorOutput),
        ])));
        // A reviewable literal → review.
        assert!(plan_has_reviewable_fields(&plan(vec![field(
            true,
            false,
            StepFieldSource::Literal
        )])));
        // A required field still awaiting input → review (must gather).
        assert!(plan_has_reviewable_fields(&plan(vec![field(
            false,
            true,
            StepFieldSource::Unset
        )])));
        // Unset but optional → nothing the user must provide.
        assert!(!plan_has_reviewable_fields(&plan(vec![field(
            false,
            false,
            StepFieldSource::Unset
        )])));
    }

    // --- shared plan/field helpers used by run-mode gate tests ---------------

    fn reviewable_field() -> ags_protocol::workflow::StepField {
        use ags_protocol::workflow::{StepFieldId, StepFieldLocation, StepFieldSource};
        ags_protocol::workflow::StepField {
            id: StepFieldId(0),
            field: "f".into(),
            label: "f".into(),
            description: None,
            location: StepFieldLocation::Body,
            schema: serde_json::json!({"type": "string"}),
            value: serde_json::Value::Null,
            source: StepFieldSource::Literal,
            required: false,
            workflow_input: None,
            body_overflow: false,
            show_in_review: true,
        }
    }

    fn auto_bound_field() -> ags_protocol::workflow::StepField {
        use ags_protocol::workflow::{StepFieldId, StepFieldLocation, StepFieldSource};
        ags_protocol::workflow::StepField {
            id: StepFieldId(1),
            field: "g".into(),
            label: "g".into(),
            description: None,
            location: StepFieldLocation::Body,
            schema: serde_json::json!({"type": "string"}),
            value: serde_json::json!("ns"),
            source: StepFieldSource::WorkflowInput {
                name: "namespace".into(),
            },
            required: true,
            workflow_input: Some("namespace".into()),
            body_overflow: false,
            show_in_review: false,
        }
    }

    fn plan_with_fields(
        fields: Vec<ags_protocol::workflow::StepField>,
    ) -> ags_protocol::workflow::StepFieldPlan {
        ags_protocol::workflow::StepFieldPlan {
            step_index: 0,
            step_label: "s".into(),
            step_description: None,
            optional: false,
            fields,
        }
    }

    #[test]
    fn test_run_mode_gates_review_pause() {
        use ags_protocol::workflow::RunMode;
        // Build a plan with a reviewable field and one with none, reusing the
        // StepFieldPlan builder from the plan_has_reviewable_fields test above.
        let reviewable = plan_with_fields(vec![reviewable_field()]);
        let auto_bound = plan_with_fields(vec![auto_bound_field()]);

        // ReviewInputSteps: pause only when there is a reviewable field.
        assert!(should_pause_for_review(
            RunMode::ReviewInputSteps,
            true,
            true,
            false,
            &reviewable
        ));
        assert!(!should_pause_for_review(
            RunMode::ReviewInputSteps,
            true,
            true,
            false,
            &auto_bound
        ));
        // ReviewEveryStep: pause on both (step is reviewed).
        assert!(should_pause_for_review(
            RunMode::ReviewEveryStep,
            true,
            true,
            false,
            &auto_bound
        ));
        // RunWithoutStopping: never pause.
        assert!(!should_pause_for_review(
            RunMode::RunWithoutStopping,
            true,
            true,
            false,
            &reviewable
        ));
        // review_steps false (plain/json/-y): never pause regardless of mode.
        assert!(!should_pause_for_review(
            RunMode::ReviewEveryStep,
            false,
            true,
            false,
            &reviewable
        ));
        // step opts out of review: never pause.
        assert!(!should_pause_for_review(
            RunMode::ReviewEveryStep,
            true,
            false,
            false,
            &reviewable
        ));
    }

    use ags_protocol::catalogue::{OperationId, ServiceId};
    use ags_protocol::workflow::{
        ArithmeticOp, ArithmeticOperand, ArithmeticTransform, BindingSource, CompiledStep,
        CompiledWorkflow, CompletionResource, FormatBinding, OperationReference, ReferenceBinding,
        ReferenceTarget, StepInputBinding, TransformKind, WorkflowCompletion, WorkflowId,
        WorkflowInputSpec,
    };
    use std::collections::BTreeMap;

    /// Build a `CompiledStep` fixture.
    fn compiled_step(id: &str, index: usize) -> CompiledStep {
        CompiledStep {
            id: id.into(),
            index,
            description: None,
            kind: ags_protocol::workflow::StepKind::default(),
            action: None,
            operation: Some(OperationReference {
                service: ServiceId::new("svc"),
                operation: OperationId::new("op"),
            }),
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
    fn test_is_conflict_requires_409_and_already_exists_text() {
        use ags_protocol::error::{ErrorDetails, RuntimeError, RuntimeErrorKind};
        let mk = |kind, message: &str, reason: Option<&str>, detail: Option<&str>| RuntimeError {
            kind,
            message: message.into(),
            details: Some(Box::new(ErrorDetails {
                code: None,
                reason: reason.map(String::from),
                detail: detail.map(String::from),
                suggestion_kind: None,
                tip: None,
            })),
            hint: None,
            trace: None,
        };
        let up409 = || RuntimeErrorKind::Upstream {
            status: 409,
            code: None,
        };

        // 409 with the phrase in message / reason / detail → conflict.
        assert!(is_conflict(&mk(
            up409(),
            "Currency already exists",
            None,
            None
        )));
        assert!(is_conflict(&mk(
            up409(),
            "Create failed",
            Some("Store already exists"),
            None
        )));
        assert!(is_conflict(&mk(
            up409(),
            "Create failed",
            None,
            Some("(Category already exists)")
        )));

        // 409 without the phrase (e.g. optimistic concurrency) → NOT a conflict,
        // so it falls through to the gate rather than being silently skipped.
        assert!(!is_conflict(&mk(
            up409(),
            "Update rejected — resource has changed",
            None,
            None
        )));

        // Right phrase, wrong status → not a conflict.
        assert!(!is_conflict(&mk(
            RuntimeErrorKind::Upstream {
                status: 500,
                code: None
            },
            "already exists",
            None,
            None
        )));
        assert!(!is_conflict(&mk(
            RuntimeErrorKind::NotFound,
            "already exists",
            None,
            None
        )));
    }

    /// Footgun guard: `skip_if_exists` auto-skip keys off the "already exists"
    /// wording, which the curated error-code table supplies (via the message,
    /// reason, or the `Error code N (…)` detail). This pins the real
    /// `classify_to_runtime_error` → `is_conflict` path for the exact codes the
    /// in-game-store create steps rely on for idempotent re-runs. If a curated
    /// conflict message is reworded to drop "already exists" (or the detail
    /// format changes), auto-skip silently breaks with no other test catching
    /// it — this one fails instead. The server message is deliberately neutral
    /// ("conflict") so the phrase can only come from the curated table, not a
    /// server echo.
    #[test]
    fn test_curated_conflict_codes_stay_recognised_by_is_conflict() {
        use crate::runtime::dispatch::classify::classify_to_runtime_error;
        // (service, errorCode, resource, method) for the platform create
        // operations behind the workflow's skip_if_exists steps.
        let cases = [
            ("platform", 36171, "currencies", "create"), // Currency already exists
            ("platform", 30271, "categories", "create"), // Category already exists
            ("platform", 30374, "items", "create"),      // Item SKU already exists
        ];
        for (service, code, resource, method) in cases {
            let body = serde_json::json!({ "errorCode": code, "errorMessage": "conflict" });
            let error = classify_to_runtime_error(409, &body, service, resource, method);
            assert!(
                is_conflict(&error),
                "{service} code {code} must classify as an already-exists conflict so \
                 skip_if_exists auto-skips on re-run; a reworded curated message broke it"
            );
        }
    }

    #[test]
    fn test_is_safely_skippable_by_output_defaults() {
        use ags_protocol::workflow::{CaptureSource, StepOutputCapture};
        let mut step = compiled_step("s", 0); // outputs default to vec![] → safe
        assert!(is_safely_skippable(&step));
        step.outputs = vec![StepOutputCapture {
            name: "id".into(),
            source: CaptureSource::ResponseBody {
                path: "$.id".into(),
            },
            default: Some(serde_json::json!(null)),
            sensitive: false,
        }];
        assert!(is_safely_skippable(&step));
        step.outputs[0].default = None;
        assert!(!is_safely_skippable(&step));
    }

    struct FailureFrontend {
        action: ags_protocol::workflow::StepFailureAction,
        calls: std::cell::Cell<usize>,
        last_allow_skip: std::cell::Cell<bool>,
    }
    impl FailureFrontend {
        fn new(action: ags_protocol::workflow::StepFailureAction) -> Self {
            Self {
                action,
                calls: std::cell::Cell::new(0),
                last_allow_skip: std::cell::Cell::new(false),
            }
        }
    }
    impl WorkflowFrontend for FailureFrontend {
        fn gather_workflow_inputs(
            &mut self,
            _n: &[ags_protocol::workflow::WorkflowInputNeeded],
            _s: &CompiledStep,
            _v: &[ags_protocol::workflow::SuppliedInputView],
        ) -> Result<ags_protocol::workflow::GatherResult, RuntimeError> {
            unreachable!("not used by decide_step_failure")
        }
        fn confirm_step(
            &mut self,
            _s: &CompiledStep,
            _p: &ags_protocol::workflow::StepPreview,
        ) -> Result<ags_protocol::workflow::StepConfirmOutcome, RuntimeError> {
            unreachable!("not used by decide_step_failure")
        }
        fn resolve_step_failure(
            &mut self,
            _step: &CompiledStep,
            _error: &RuntimeError,
            allow_skip: bool,
        ) -> Result<ags_protocol::workflow::StepFailureAction, RuntimeError> {
            self.calls.set(self.calls.get() + 1);
            self.last_allow_skip.set(allow_skip);
            Ok(self.action)
        }
    }

    /// Arbitrary attempt-stage fixture for `decide_step_failure` tests that
    /// don't care which stage the attempt failed at.
    fn dispatch_stage() -> ags_protocol::workflow::StepOutcomeReason {
        ags_protocol::workflow::StepOutcomeReason::Dispatch
    }

    fn upstream(status: u16) -> RuntimeError {
        RuntimeError {
            kind: ags_protocol::error::RuntimeErrorKind::Upstream { status, code: None },
            message: "m".into(),
            details: None,
            hint: None,
            trace: None,
        }
    }

    /// A 409 whose message identifies it as an already-exists conflict.
    fn already_exists_409() -> RuntimeError {
        RuntimeError {
            kind: ags_protocol::error::RuntimeErrorKind::Upstream {
                status: 409,
                code: None,
            },
            message: "Currency already exists".into(),
            details: None,
            hint: None,
            trace: None,
        }
    }

    fn defaultless_capture_step() -> CompiledStep {
        use ags_protocol::workflow::{CaptureSource, StepOutputCapture};
        let mut step = compiled_step("s", 0);
        step.outputs = vec![StepOutputCapture {
            name: "id".into(),
            source: CaptureSource::ResponseBody {
                path: "$.id".into(),
            },
            default: None,
            sensitive: false,
        }];
        step
    }

    #[test]
    fn test_decide_continue_on_failure_skips_any_error() {
        let mut step = compiled_step("s", 0);
        step.continue_on_failure = true;
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Cancel);
        let d =
            decide_step_failure(&step, &upstream(500), false, &mut fe, dispatch_stage()).unwrap();
        assert!(matches!(d, FailureDisposition::Skip { .. }));
        assert_eq!(
            fe.calls.get(),
            0,
            "gate not consulted for continue_on_failure"
        );
    }

    #[test]
    fn test_decide_skip_if_exists_already_exists_409_auto_skips() {
        let mut step = compiled_step("s", 0);
        step.skip_if_exists = true;
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Cancel);
        let d = decide_step_failure(
            &step,
            &already_exists_409(),
            false,
            &mut fe,
            dispatch_stage(),
        )
        .unwrap();
        match d {
            FailureDisposition::Skip {
                reason,
                summary_tail,
            } => {
                assert_eq!(
                    reason,
                    ags_protocol::workflow::StepOutcomeReason::AlreadyExists
                );
                assert_eq!(summary_tail.as_deref(), Some("already exists"));
            }
            _ => panic!("expected Skip"),
        }
        assert_eq!(fe.calls.get(), 0, "no gate for auto-skip");
    }

    #[test]
    fn test_decide_step_failure_labels_skip_if_exists_as_already_exists() {
        let mut step = compiled_step("s", 0);
        step.skip_if_exists = true;
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Cancel);
        match decide_step_failure(
            &step,
            &already_exists_409(),
            false,
            &mut fe,
            dispatch_stage(),
        )
        .unwrap()
        {
            FailureDisposition::Skip { reason, .. } => {
                assert_eq!(
                    reason,
                    ags_protocol::workflow::StepOutcomeReason::AlreadyExists
                );
            }
            other => panic!("expected Skip, got {other:?}"),
        }
    }

    #[test]
    fn test_decide_step_failure_labels_continue_on_failure_as_tolerated() {
        let mut step = compiled_step("s", 0);
        step.continue_on_failure = true;
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Cancel);
        match decide_step_failure(
            &step,
            &already_exists_409(),
            false,
            &mut fe,
            dispatch_stage(),
        )
        .unwrap()
        {
            FailureDisposition::Skip { reason, .. } => {
                assert_eq!(
                    reason,
                    ags_protocol::workflow::StepOutcomeReason::ToleratedFailure
                );
            }
            other => panic!("expected Skip, got {other:?}"),
        }
    }

    #[test]
    fn test_decide_step_failure_labels_gate_skip_as_declined_after_failure() {
        let step = compiled_step("s", 0); // no captures → safely skippable
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Skip);
        match decide_step_failure(&step, &upstream(500), false, &mut fe, dispatch_stage()).unwrap()
        {
            FailureDisposition::Skip { reason, .. } => {
                assert_eq!(
                    reason,
                    ags_protocol::workflow::StepOutcomeReason::DeclinedAfterFailure
                );
            }
            other => panic!("expected Skip, got {other:?}"),
        }
    }

    #[test]
    fn test_decide_skip_if_exists_bare_409_goes_to_gate() {
        // A 409 that is NOT "already exists" (e.g. optimistic concurrency) must
        // NOT auto-skip even on a skip_if_exists step — it goes to the gate.
        let mut step = compiled_step("s", 0);
        step.skip_if_exists = true;
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Retry);
        let d =
            decide_step_failure(&step, &upstream(409), false, &mut fe, dispatch_stage()).unwrap();
        assert!(matches!(d, FailureDisposition::Retry));
        assert_eq!(fe.calls.get(), 1, "bare 409 is not auto-skipped");
    }

    #[test]
    fn test_decide_skip_if_exists_non_409_goes_to_gate() {
        let mut step = compiled_step("s", 0);
        step.skip_if_exists = true;
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Retry);
        let d =
            decide_step_failure(&step, &upstream(500), false, &mut fe, dispatch_stage()).unwrap();
        assert!(matches!(d, FailureDisposition::Retry));
        assert_eq!(fe.calls.get(), 1);
    }

    #[test]
    fn test_decide_unmarked_409_goes_to_gate_with_allow_skip_flag() {
        let step = compiled_step("s", 0); // no captures → safely skippable
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Skip);
        let d =
            decide_step_failure(&step, &upstream(409), false, &mut fe, dispatch_stage()).unwrap();
        assert!(matches!(
            d,
            FailureDisposition::Skip {
                reason: ags_protocol::workflow::StepOutcomeReason::DeclinedAfterFailure,
                summary_tail: None
            }
        ));
        assert!(
            fe.last_allow_skip.get(),
            "no-capture step is safely skippable"
        );
    }

    #[test]
    fn test_decide_no_input_is_fatal_without_consulting_gate() {
        // Machine surfaces (`--format json`) run with `no_input = true`, which
        // becomes `Fatal` before the frontend gate is reached. This guards the
        // `JsonInteraction` seam: it inherits the default `resolve_step_failure`
        // (Cancel), and this short-circuit is the sole reason that default is
        // never exercised. If the coupling ever loosens, this test fails.
        let step = compiled_step("s", 0);
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Retry);
        let d =
            decide_step_failure(&step, &upstream(500), true, &mut fe, dispatch_stage()).unwrap();
        assert!(matches!(d, FailureDisposition::Fatal { .. }));
        assert_eq!(
            fe.calls.get(),
            0,
            "no_input must not consult the frontend gate"
        );
    }

    #[test]
    fn test_decide_unsafe_step_gate_allow_skip_false() {
        let step = defaultless_capture_step();
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Cancel);
        let d =
            decide_step_failure(&step, &upstream(500), false, &mut fe, dispatch_stage()).unwrap();
        assert!(matches!(d, FailureDisposition::Fatal { .. }));
        assert!(
            !fe.last_allow_skip.get(),
            "defaultless capture is not safely skippable"
        );
    }

    #[test]
    fn test_decide_unsafe_step_skip_from_frontend_is_fatal() {
        // Defensive guard: even if a frontend returns Skip for an unsafe step,
        // the decision must NOT skip — that would break downstream.
        let step = defaultless_capture_step();
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Skip);
        let d =
            decide_step_failure(&step, &upstream(500), false, &mut fe, dispatch_stage()).unwrap();
        assert!(
            matches!(d, FailureDisposition::Fatal { .. }),
            "unsafe + Skip must be Fatal"
        );
        assert!(!fe.last_allow_skip.get(), "gate was told allow_skip=false");
    }

    #[test]
    fn test_decide_no_input_is_fatal_without_gate() {
        let step = compiled_step("s", 0);
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Skip);
        let d =
            decide_step_failure(&step, &upstream(500), true, &mut fe, dispatch_stage()).unwrap();
        assert!(matches!(d, FailureDisposition::Fatal { .. }));
        assert_eq!(fe.calls.get(), 0, "no gate in no_input mode");
    }

    /// The `--no-input` short circuit attributes the Fatal reason to the
    /// stage the attempt actually failed at, not a fixed label.
    #[test]
    fn test_decide_no_input_fatal_carries_attempt_stage() {
        let step = compiled_step("s", 0);
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Retry);
        let d = decide_step_failure(
            &step,
            &upstream(500),
            true,
            &mut fe,
            ags_protocol::workflow::StepOutcomeReason::Capture,
        )
        .unwrap();
        assert!(matches!(
            d,
            FailureDisposition::Fatal {
                reason: ags_protocol::workflow::StepOutcomeReason::Capture
            }
        ));
    }

    /// The defensive downgrade — a frontend returning `Skip` for an unsafe
    /// step — must be labelled `FrontendContract`, distinguishing "the
    /// frontend broke the contract" from every other Fatal cause.
    #[test]
    fn test_decide_unsafe_step_skip_from_frontend_labels_frontend_contract() {
        let step = defaultless_capture_step();
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Skip);
        let d =
            decide_step_failure(&step, &upstream(500), false, &mut fe, dispatch_stage()).unwrap();
        assert!(matches!(
            d,
            FailureDisposition::Fatal {
                reason: ags_protocol::workflow::StepOutcomeReason::FrontendContract
            }
        ));
    }

    /// A user-chosen Cancel at the interactive failure gate must be labelled
    /// `AtFailureGate`, distinguishing "the user gave up" from an unattended
    /// failure.
    #[test]
    fn test_decide_cancel_from_frontend_labels_at_failure_gate() {
        let step = compiled_step("s", 0);
        let mut fe = FailureFrontend::new(ags_protocol::workflow::StepFailureAction::Cancel);
        let d =
            decide_step_failure(&step, &upstream(500), false, &mut fe, dispatch_stage()).unwrap();
        assert!(matches!(
            d,
            FailureDisposition::Fatal {
                reason: ags_protocol::workflow::StepOutcomeReason::AtFailureGate
            }
        ));
    }

    /// Build a `WorkflowInputSpec` fixture.
    fn input_spec(name: &str) -> WorkflowInputSpec {
        WorkflowInputSpec {
            name: name.into(),
            description: None,
            schema: None,
            required: false,
            default: None,
            sensitive: false,
            options_source: None,
            location: Default::default(),
            file_picker: None,
        }
    }

    /// Build a `StepInputBinding` fixture.
    fn binding(source: BindingSource) -> StepInputBinding {
        StepInputBinding {
            field: "f".into(),
            source,
            show_in_review: false,
            description: None,
        }
    }

    #[test]
    fn test_order_inputs_by_first_use_counts_format_placeholders() {
        // `early` is used ONLY through a Format template on step 0; `late` is a
        // workflow-input Reference on step 1. Despite being declared last,
        // `early` must sort first because step 0 uses it — a regression guard
        // for Format bindings being skipped by the first-use walk.
        let mut step0 = compiled_step("s0", 0);
        step0.inputs = vec![binding(BindingSource::Format(FormatBinding {
            template: "{early}-suffix".into(),
        }))];
        let mut step1 = compiled_step("s1", 1);
        step1.inputs = vec![binding(BindingSource::Reference(ReferenceBinding {
            from: ReferenceTarget::Workflow {
                input: "late".into(),
            },
            output: None,
            transform: None,
        }))];

        let inputs = vec![input_spec("late"), input_spec("early")];
        let ordered = order_inputs_by_first_use(&inputs, &[step0, step1]);

        let names: Vec<&str> = ordered.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["early", "late"]);
    }

    #[test]
    fn test_order_inputs_by_first_use_counts_arithmetic_operands() {
        // `factor` is used ONLY as an arithmetic transform operand on step 0
        // (e.g. count * factor); `late` is a Reference on step 1. `factor` must
        // sort first despite being declared last — regression guard for the
        // transform operand walk.
        let mut step0 = compiled_step("s0", 0);
        step0.inputs = vec![binding(BindingSource::Reference(ReferenceBinding {
            from: ReferenceTarget::Workflow {
                input: "count".into(),
            },
            output: None,
            transform: Some(TransformKind::Arithmetic(ArithmeticTransform {
                op: ArithmeticOp::Mul,
                operand: ArithmeticOperand::WorkflowInput("factor".into()),
            })),
        }))];
        let mut step1 = compiled_step("s1", 1);
        step1.inputs = vec![binding(BindingSource::Reference(ReferenceBinding {
            from: ReferenceTarget::Workflow {
                input: "late".into(),
            },
            output: None,
            transform: None,
        }))];

        let inputs = vec![input_spec("late"), input_spec("factor")];
        let ordered = order_inputs_by_first_use(&inputs, &[step0, step1]);

        let names: Vec<&str> = ordered.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["factor", "late"]);
    }

    #[test]
    fn test_order_inputs_pulls_picker_deps_before_the_picker() {
        // `userId` is a picker whose `options_source` depends on `by` and
        // `query`, which no step references; `namespace` and `userId` are bound
        // by step 0. The picker's dependency inputs must sort before the picker,
        // not sink to the end (the player-overview search-then-pick shape).
        use ags_protocol::workflow::{OptionParameterBinding, OptionsSource};
        let mut step0 = compiled_step("s0", 0);
        step0.inputs = vec![
            binding(BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: "namespace".into(),
                },
                output: None,
                transform: None,
            })),
            binding(BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: "userId".into(),
                },
                output: None,
                transform: None,
            })),
        ];

        let mut user_id = input_spec("userId");
        user_id.options_source = Some(OptionsSource {
            operation: OperationReference {
                service: ServiceId::new("iam"),
                operation: OperationId::new("iam/admin/users/v3/search"),
            },
            parameters: BTreeMap::from([
                (
                    "namespace".to_string(),
                    OptionParameterBinding::FromInput("namespace".into()),
                ),
                (
                    "query".to_string(),
                    OptionParameterBinding::FromInput("query".into()),
                ),
                (
                    "by".to_string(),
                    OptionParameterBinding::FromInput("by".into()),
                ),
            ]),
            items_path: "$.data".into(),
            value: "$.userId".into(),
            label: None,
            label_detail: None,
            fallback_description: None,
            filter: None,
        });

        // Declared order mirrors player-overview: the search fields are
        // declared before the picker.
        let inputs = vec![
            input_spec("namespace"),
            input_spec("by"),
            input_spec("query"),
            user_id,
        ];
        let ordered = order_inputs_by_first_use(&inputs, &[step0]);
        let names: Vec<&str> = ordered.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["namespace", "by", "query", "userId"]);
    }

    #[test]
    fn test_order_inputs_pulls_optional_picker_deps_before_the_picker() {
        // Same shape, but `query`/`by` are FromInputOptional. Optional deps must
        // still order before the picker even though they do not gate it.
        use ags_protocol::workflow::{OptionParameterBinding, OptionsSource};
        let mut step0 = compiled_step("s0", 0);
        step0.inputs = vec![
            binding(BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: "namespace".into(),
                },
                output: None,
                transform: None,
            })),
            binding(BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: "userId".into(),
                },
                output: None,
                transform: None,
            })),
        ];
        let mut user_id = input_spec("userId");
        user_id.options_source = Some(OptionsSource {
            operation: OperationReference {
                service: ServiceId::new("iam"),
                operation: OperationId::new("iam/admin/users/v3/search"),
            },
            parameters: BTreeMap::from([
                (
                    "namespace".to_string(),
                    OptionParameterBinding::FromInput("namespace".into()),
                ),
                (
                    "query".to_string(),
                    OptionParameterBinding::FromInputOptional("query".into()),
                ),
                (
                    "by".to_string(),
                    OptionParameterBinding::FromInputOptional("by".into()),
                ),
            ]),
            items_path: "$.data".into(),
            value: "$.userId".into(),
            label: None,
            label_detail: None,
            fallback_description: None,
            filter: None,
        });
        let inputs = vec![
            input_spec("namespace"),
            input_spec("by"),
            input_spec("query"),
            user_id,
        ];
        let ordered = order_inputs_by_first_use(&inputs, &[step0]);
        let names: Vec<&str> = ordered.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["namespace", "by", "query", "userId"]);
    }

    #[test]
    fn test_optional_picker_dep_hidden_on_no_picker_surface() {
        // With pickers_available = false, an optional picker-support input
        // (`query`) is stripped from the gathered inputs just like a FromInput one.
        use ags_protocol::workflow::{OptionParameterBinding, OptionsSource};
        let mut step0 = compiled_step("s0", 0);
        step0.inputs = vec![binding(BindingSource::Reference(ReferenceBinding {
            from: ReferenceTarget::Workflow {
                input: "userId".into(),
            },
            output: None,
            transform: None,
        }))];
        let mut user_id = input_spec("userId");
        user_id.options_source = Some(OptionsSource {
            operation: OperationReference {
                service: ServiceId::new("iam"),
                operation: OperationId::new("iam/admin/users/v3/search"),
            },
            parameters: BTreeMap::from([(
                "query".to_string(),
                OptionParameterBinding::FromInputOptional("query".into()),
            )]),
            items_path: "$.data".into(),
            value: "$.userId".into(),
            label: None,
            label_detail: None,
            fallback_description: None,
            filter: None,
        });
        let inputs = vec![input_spec("query"), user_id];
        let gathered = gather_inputs_for_surface(&inputs, &[step0], false);
        let names: Vec<&str> = gathered.iter().map(|s| s.name.as_str()).collect();
        assert!(
            !names.contains(&"query"),
            "optional picker dep must be hidden: {names:?}"
        );
        assert!(
            names.contains(&"userId"),
            "picker-backed input stays: {names:?}"
        );
    }

    #[test]
    fn test_format_capture_value_strips_control_sequences() {
        // Capture values can be API-response-derived; terminal control
        // sequences must be stripped before they reach the Summary panel.
        let v = serde_json::Value::String("\x1b[31mred-id\x1b[0m".to_string());
        let result = format_capture_value(&v);
        assert!(
            !result.contains('\x1b'),
            "control sequences must be stripped: {result:?}"
        );
        assert!(
            result.contains("red-id"),
            "visible content must be preserved: {result:?}"
        );
    }

    /// Build a two-step compiled workflow fixture with a completion message.
    fn two_step_with_completion() -> CompiledWorkflow {
        CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "WF".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![compiled_step("s1", 0), compiled_step("s2", 1)],
            outputs: vec![],
            completion: Some(WorkflowCompletion {
                created: vec![CompletionResource {
                    label: "X".into(),
                    value: "x".into(),
                }],
                next_steps: vec![],
            }),
        }
    }

    #[test]
    fn test_build_final_output_attaches_completion_on_multi_step_success() {
        let compiled = two_step_with_completion();
        let ctx = crate::runtime::workflows::WorkflowContext::new();
        let supplied = BTreeMap::new();
        let opts = RunOptions::default(); // dry_run = false
        let out = build_final_output(&compiled, &ctx, &[], &[], &opts, &supplied).unwrap();
        match out {
            CommandOutput::Workflow { completion, .. } => assert!(completion.is_some()),
            other => panic!(
                "expected Workflow, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn test_build_final_output_no_completion_on_dry_run() {
        let compiled = two_step_with_completion();
        let ctx = crate::runtime::workflows::WorkflowContext::new();
        let supplied = BTreeMap::new();
        let opts = RunOptions {
            dry_run: true,
            ..Default::default()
        };
        let out = build_final_output(&compiled, &ctx, &[], &[], &opts, &supplied).unwrap();
        assert!(matches!(out, CommandOutput::WorkflowDryRun { .. }));
    }

    #[test]
    fn test_no_input_precheck_confirm_blocks_live_but_exempt_under_dry_run() {
        let mut step = compiled_step("s0", 0);
        step.confirm = true;
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "WF".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![step],
            outputs: vec![],
            completion: None,
        };
        let supplied = BTreeMap::new();

        // Live run, no --yes: the confirm step is a blocker.
        let live = no_input_precheck(&compiled, &supplied, false, false);
        assert!(
            live.iter()
                .any(|v| matches!(v, NoInputViolation::ConfirmRequired { .. })),
            "live confirm step must be a no-input violation: {live:?}"
        );

        // --yes pre-approves: no confirm violation.
        let yes = no_input_precheck(&compiled, &supplied, false, true);
        assert!(
            !yes.iter()
                .any(|v| matches!(v, NoInputViolation::ConfirmRequired { .. })),
            "--yes must clear the confirm violation: {yes:?}"
        );

        // Dry-run is exempt: nothing is dispatched, so nothing to confirm.
        let dry = no_input_precheck(&compiled, &supplied, true, false);
        assert!(
            !dry.iter()
                .any(|v| matches!(v, NoInputViolation::ConfirmRequired { .. })),
            "dry-run must be exempt from the confirm violation: {dry:?}"
        );
    }

    #[test]
    fn test_error_reason_tail_uses_reason_when_present() {
        use ags_protocol::error::{ErrorDetails, RuntimeError, RuntimeErrorKind};
        let error = RuntimeError {
            kind: RuntimeErrorKind::Upstream {
                status: 409,
                code: Some("30122".into()),
            },
            message: "Publish all failed".to_string(),
            details: Some(Box::new(ErrorDetails {
                code: Some("30122".into()),
                reason: Some("Language/Region does not match".to_string()),
                detail: None,
                suggestion_kind: None,
                tip: None,
            })),
            hint: None,
            trace: None,
        };
        // The specific reason wins over the headline message when present. Used
        // by the optional-skip summary (skipped — <reason>).
        assert_eq!(error_reason_tail(&error), "Language/Region does not match");
    }

    #[test]
    fn test_error_reason_tail_falls_back_to_message() {
        use ags_protocol::error::{RuntimeError, RuntimeErrorKind};
        let error = RuntimeError {
            kind: RuntimeErrorKind::Network,
            message: "Network error".to_string(),
            details: None,
            hint: None,
            trace: None,
        };
        assert_eq!(error_reason_tail(&error), "Network error");
    }

    #[test]
    fn test_error_reason_tail_ignores_empty_reason() {
        use ags_protocol::error::{ErrorDetails, RuntimeError, RuntimeErrorKind};
        let error = RuntimeError {
            kind: RuntimeErrorKind::Upstream {
                status: 400,
                code: None,
            },
            message: "Create store failed".to_string(),
            details: Some(Box::new(ErrorDetails {
                code: None,
                reason: Some("   ".to_string()), // empty/whitespace reason
                detail: None,
                suggestion_kind: None,
                tip: None,
            })),
            hint: None,
            trace: None,
        };
        // An empty/whitespace reason must fall back to the message — never
        // produce "… failed — " with a blank tail.
        assert_eq!(error_reason_tail(&error), "Create store failed");
    }

    /// Build a player-overview-shaped input set: `namespace` + `userId` are bound
    /// by step 0; `userId` is a picker whose options_source draws
    /// `searchQuery`/`searchBy` from workflow inputs that no step binds. So
    /// `searchQuery`/`searchBy` are picker-support: they exist only to
    /// parameterise the picker.
    fn picker_fixture() -> (Vec<WorkflowInputSpec>, Vec<CompiledStep>) {
        use ags_protocol::workflow::{LabelDetail, OptionParameterBinding, OptionsSource};
        let mut step0 = compiled_step("account", 0);
        step0.inputs = vec![
            binding(BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: "namespace".into(),
                },
                output: None,
                transform: None,
            })),
            binding(BindingSource::Reference(ReferenceBinding {
                from: ReferenceTarget::Workflow {
                    input: "userId".into(),
                },
                output: None,
                transform: None,
            })),
        ];

        let mut user_id = input_spec("userId");
        user_id.options_source = Some(OptionsSource {
            operation: OperationReference {
                service: ServiceId::new("iam"),
                operation: OperationId::new("iam/admin/users/v3/search"),
            },
            parameters: BTreeMap::from([
                (
                    "namespace".to_string(),
                    OptionParameterBinding::FromInput("namespace".into()),
                ),
                (
                    "query".to_string(),
                    OptionParameterBinding::FromInput("searchQuery".into()),
                ),
                (
                    "by".to_string(),
                    OptionParameterBinding::FromInput("searchBy".into()),
                ),
            ]),
            items_path: "$.data".into(),
            value: "$.userId".into(),
            label: None,
            label_detail: Some(LabelDetail::ByInput {
                input: "searchBy".into(),
                paths: BTreeMap::from([("emailAddress".into(), "$.emailAddress".into())]),
            }),
            fallback_description: Some("Enter the player's user id".into()),
            filter: None,
        });
        user_id.description = Some("Pick the player (opens the picker)".into());

        let inputs = vec![
            input_spec("namespace"),
            input_spec("searchBy"),
            input_spec("searchQuery"),
            user_id,
        ];
        (inputs, vec![step0])
    }

    #[test]
    fn test_picker_support_names_are_search_inputs_only() {
        let (inputs, steps) = picker_fixture();
        let support = picker_support_input_names(&inputs, &steps);
        // searchBy + searchQuery feed the picker and no step binds them.
        assert!(
            support.contains("searchBy"),
            "searchBy is picker-support: {support:?}"
        );
        assert!(
            support.contains("searchQuery"),
            "searchQuery is picker-support: {support:?}"
        );
        // namespace feeds the picker too but is bound by a step → not support.
        assert!(
            !support.contains("namespace"),
            "namespace is step-bound: {support:?}"
        );
        // userId is the picker itself, bound by a step → not support.
        assert!(
            !support.contains("userId"),
            "userId is step-bound: {support:?}"
        );
        assert_eq!(
            support.len(),
            2,
            "exactly the two search inputs: {support:?}"
        );
    }

    #[test]
    fn test_gather_inputs_drops_picker_support_when_no_picker() {
        let (inputs, steps) = picker_fixture();
        let gathered = gather_inputs_for_surface(&inputs, &steps, false);
        let names: Vec<&str> = gathered.iter().map(|s| s.name.as_str()).collect();
        // First-use order among the survivors: namespace then userId (both step 0).
        assert_eq!(names, vec!["namespace", "userId"]);
        // The surviving picker input renders as plain text here, so its
        // description is swapped for the picker's fallback copy.
        let user_id = gathered.iter().find(|s| s.name == "userId").unwrap();
        assert_eq!(
            user_id.description.as_deref(),
            Some("Enter the player's user id")
        );
    }

    #[test]
    fn test_gather_inputs_keeps_all_when_picker_available() {
        let (inputs, steps) = picker_fixture();
        let gathered = gather_inputs_for_surface(&inputs, &steps, true);
        let names: Vec<&str> = gathered.iter().map(|s| s.name.as_str()).collect();
        // Unchanged from order_inputs_by_first_use: picker deps pulled up before userId.
        assert_eq!(
            names,
            vec!["namespace", "searchBy", "searchQuery", "userId"]
        );
        // With a picker available the description is untouched (picker copy).
        let user_id = gathered.iter().find(|s| s.name == "userId").unwrap();
        assert_eq!(
            user_id.description.as_deref(),
            Some("Pick the player (opens the picker)")
        );
    }

    /// A plan with no reviewable or missing fields, marked optional — used by
    /// the force-pause unit test.
    fn empty_field_plan() -> ags_protocol::workflow::StepFieldPlan {
        ags_protocol::workflow::StepFieldPlan {
            step_index: 0,
            step_label: "s".into(),
            step_description: None,
            optional: true,
            fields: vec![],
        }
    }

    #[test]
    fn test_force_pause_pauses_zero_field_plan() {
        use ags_protocol::workflow::RunMode;
        let plan = empty_field_plan(); // no reviewable/missing fields
        assert!(should_pause_for_review(
            RunMode::ReviewInputSteps,
            true,
            true,
            /* force_pause */ true,
            &plan
        ));
        // Not forced → keeps today's behaviour (no reviewable fields → no pause).
        assert!(!should_pause_for_review(
            RunMode::ReviewInputSteps,
            true,
            true,
            /* force_pause */ false,
            &plan
        ));
        // RunWithoutStopping still never pauses, even forced.
        assert!(!should_pause_for_review(
            RunMode::RunWithoutStopping,
            true,
            true,
            /* force_pause */ true,
            &plan
        ));
    }

    // --- input-provenance counting -----------------------------------------

    /// Build a runtime whose HTTP client panics if dispatched; the
    /// provenance fixtures declare zero steps, so dispatch is never reached.
    fn provenance_test_runtime() -> crate::runtime::Runtime {
        use crate::runtime::dispatch::http::{HttpClient, HttpRequest, HttpResponse};

        struct NeverClient;
        #[async_trait::async_trait]
        impl HttpClient for NeverClient {
            async fn send(&self, _: HttpRequest) -> Result<HttpResponse, RuntimeError> {
                unreachable!("provenance fixtures declare no steps to dispatch")
            }
        }

        crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext::default(),
            Box::new(NeverClient),
            reqwest::Client::new(),
        )
    }

    /// Zero-step workflow declaring two inputs: `flagged` (no default, meant
    /// to be pre-supplied) and `defaulted` (carries a declared default).
    fn provenance_workflow() -> CompiledWorkflow {
        let mut defaulted = input_spec("defaulted");
        defaulted.default = Some(serde_json::json!("dv"));
        CompiledWorkflow {
            id: WorkflowId::new("wf"),
            name: "WF".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![input_spec("flagged"), defaulted],
            is_reviewed_by_default: true,
            steps: vec![],
            outputs: vec![],
            completion: None,
        }
    }

    /// Frontend for provenance fixtures: records the `RunFacts` carried by
    /// `WorkflowFinished`, and lets a test simulate a prompted value by
    /// returning it from `collect_workflow_inputs`.
    #[derive(Default)]
    struct ProvenanceFrontend {
        finished_facts: Option<ags_protocol::workflow::RunFacts>,
        prompted_inputs: BTreeMap<String, serde_json::Value>,
    }

    impl WorkflowFrontend for ProvenanceFrontend {
        fn on_event(&mut self, event: &WorkflowEvent) {
            if let WorkflowEvent::WorkflowFinished { facts, .. } = event {
                self.finished_facts = Some(facts.clone());
            }
        }

        fn gather_workflow_inputs(
            &mut self,
            _needed: &[ags_protocol::workflow::WorkflowInputNeeded],
            _step_context: &CompiledStep,
            _supplied: &[ags_protocol::workflow::SuppliedInputView],
        ) -> Result<ags_protocol::workflow::GatherResult, RuntimeError> {
            unreachable!("provenance fixtures declare no steps to gather inputs for")
        }

        fn confirm_step(
            &mut self,
            _step: &CompiledStep,
            _preview: &ags_protocol::workflow::StepPreview,
        ) -> Result<ags_protocol::workflow::StepConfirmOutcome, RuntimeError> {
            unreachable!("provenance fixtures declare no steps to confirm")
        }

        fn collect_workflow_inputs(
            &mut self,
            _specs: &[WorkflowInputSpec],
            current: &BTreeMap<String, serde_json::Value>,
        ) -> Result<Option<ags_protocol::workflow::CollectOutcome>, RuntimeError> {
            let mut inputs = current.clone();
            for (name, value) in &self.prompted_inputs {
                inputs.insert(name.clone(), value.clone());
            }
            Ok(Some(ags_protocol::workflow::CollectOutcome {
                inputs,
                run_mode: ags_protocol::workflow::RunMode::ReviewInputSteps,
            }))
        }
    }

    /// Run `provenance_workflow` with `flagged` pre-supplied (as a CLI flag
    /// would) and `defaulted` left unset so the default-layering loop fills
    /// it. `review_steps` stays false, so Phase 1 never runs.
    async fn run_workflow_with_one_flag_and_one_default(frontend: &mut ProvenanceFrontend) {
        let compiled = provenance_workflow();
        let mut pre_supplied = BTreeMap::new();
        pre_supplied.insert("flagged".to_string(), serde_json::json!("fv"));
        let options = RunOptions::default();
        let mut runtime = provenance_test_runtime();
        let mut run_context = RunContext::new(&mut runtime, &options);
        let _ = Executor::execute(&compiled, pre_supplied, frontend, &mut run_context).await;
    }

    #[tokio::test]
    async fn test_run_facts_count_flag_and_default_provenance() {
        let mut frontend = ProvenanceFrontend::default();
        run_workflow_with_one_flag_and_one_default(&mut frontend).await;
        let facts = frontend
            .finished_facts
            .expect("WorkflowFinished must be emitted");
        assert_eq!(facts.inputs_from_flag, 1);
        assert_eq!(facts.inputs_from_default, 1);
        assert_eq!(facts.inputs_from_prompt, 0);
        assert_eq!(facts.inputs_edited_in_form, 0);
    }

    #[tokio::test]
    async fn test_run_facts_count_prompted_input_provenance() {
        // `review_steps: true` so Phase 1 runs; nothing is pre-supplied, so
        // `flagged` enters Phase 1 with no value at all and `defaulted`
        // enters holding its declared default. The frontend's
        // `collect_workflow_inputs` fills `flagged` (present in neither
        // `flag_names` nor `default_added` — the prompt branch) and changes
        // `defaulted` away from its declared default. Both values differ from
        // their pre-Phase-1 state (`None` vs `Some`, and `Some(default)` vs
        // `Some(edited)`), so both count as edited — filling in a
        // previously-unset input is, by this diff, an edit too.
        let compiled = provenance_workflow();
        let mut frontend = ProvenanceFrontend {
            finished_facts: None,
            prompted_inputs: BTreeMap::from([
                ("flagged".to_string(), serde_json::json!("typed")),
                ("defaulted".to_string(), serde_json::json!("edited")),
            ]),
        };
        let options = RunOptions {
            review_steps: true,
            ..Default::default()
        };
        let mut runtime = provenance_test_runtime();
        let mut run_context = RunContext::new(&mut runtime, &options);
        let _ =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context).await;
        let facts = frontend
            .finished_facts
            .expect("WorkflowFinished must be emitted");
        assert_eq!(facts.inputs_from_flag, 0);
        assert_eq!(
            facts.inputs_from_default, 1,
            "defaulted is still attributed to the declared default even though the form changed its value"
        );
        assert_eq!(
            facts.inputs_from_prompt, 1,
            "flagged was supplied at the run-start form, not by flag or default"
        );
        assert_eq!(
            facts.inputs_edited_in_form, 2,
            "both flagged (filled from nothing) and defaulted (changed) differ from their pre-Phase-1 value"
        );
    }

    /// A local-action step failure must carry `StepErrorFacts` in the
    /// `StepFinished` event so that telemetry receives the error
    /// classification.
    #[test]
    fn test_local_step_failure_carries_error_facts_in_step_finished() {
        use ags_protocol::error::RuntimeError;
        use ags_protocol::workflow::{RunFacts, StepErrorFacts, WorkflowEvent, WorkflowFrontend};
        use std::sync::Mutex;

        /// Recording frontend that captures the `StepFinished` event's `error`.
        struct Recorder(Mutex<Option<Option<StepErrorFacts>>>);
        impl WorkflowFrontend for Recorder {
            fn on_event(&mut self, event: &WorkflowEvent) {
                if let WorkflowEvent::StepFinished { error, .. } = event {
                    *self.0.lock().unwrap() = Some(error.clone());
                }
            }
            fn gather_workflow_inputs(
                &mut self,
                _: &[ags_protocol::workflow::WorkflowInputNeeded],
                _: &ags_protocol::workflow::CompiledStep,
                _: &[ags_protocol::workflow::SuppliedInputView],
            ) -> Result<ags_protocol::workflow::GatherResult, RuntimeError> {
                Ok(ags_protocol::workflow::GatherResult::default())
            }
            fn confirm_step(
                &mut self,
                _: &ags_protocol::workflow::CompiledStep,
                _: &ags_protocol::workflow::StepPreview,
            ) -> Result<ags_protocol::workflow::StepConfirmOutcome, RuntimeError> {
                Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
            }
        }

        let mut step = compiled_step("upload-image", 0);
        step.operation = None;
        step.kind = ags_protocol::workflow::StepKind::Local;
        step.action = Some("ams/upload-image".to_string());

        let error = RuntimeError::internal("ams/upload-image failed — directory not found");
        let facts = StepErrorFacts::from_error(&error);

        let mut recorder = Recorder(Mutex::new(None));
        let mut summaries = Vec::new();
        let mut run_facts = RunFacts::default();

        finish_step_terminal(
            &mut recorder,
            &mut summaries,
            &step,
            "upload-image failed".into(),
            StepOutcome::Failed,
            None,
            1,
            std::time::Instant::now(),
            Some(facts),
            &mut run_facts,
        );

        let recorded = recorder
            .0
            .lock()
            .unwrap()
            .take()
            .expect("StepFinished event must be emitted");
        let error_facts = recorded.expect("error must be Some for a local step failure");
        assert_eq!(
            error_facts.class, "internal",
            "class must reflect the underlying RuntimeError kind"
        );
    }

    // -----------------------------------------------------------------
    // FIX 2: A local action that fails during dry-run must produce a
    // StepFinished(Failed) event and increment steps_failed, rather
    // than escaping the executor via bare `?`.
    // -----------------------------------------------------------------
    #[tokio::test]
    async fn test_dry_run_local_action_failure_uses_step_bookkeeping() {
        use std::sync::Mutex;

        /// Captures StepFinished events emitted by the executor.
        struct EventCapture {
            finished: Mutex<Vec<(String, StepOutcome)>>,
        }
        impl EventCapture {
            fn new() -> Self {
                Self {
                    finished: Mutex::new(Vec::new()),
                }
            }
        }
        impl WorkflowFrontend for EventCapture {
            fn on_event(&mut self, event: &WorkflowEvent) {
                if let WorkflowEvent::StepFinished { id, outcome, .. } = event {
                    self.finished.lock().unwrap().push((id.clone(), *outcome));
                }
            }
            fn gather_workflow_inputs(
                &mut self,
                _: &[ags_protocol::workflow::WorkflowInputNeeded],
                _: &CompiledStep,
                _: &[ags_protocol::workflow::SuppliedInputView],
            ) -> Result<ags_protocol::workflow::GatherResult, RuntimeError> {
                unreachable!("dry_run + assume_yes should not gather")
            }
            fn confirm_step(
                &mut self,
                _: &CompiledStep,
                _: &ags_protocol::workflow::StepPreview,
            ) -> Result<ags_protocol::workflow::StepConfirmOutcome, RuntimeError> {
                unreachable!("dry_run should not confirm")
            }
        }

        struct NeverClient;
        #[async_trait::async_trait]
        impl crate::runtime::dispatch::http::HttpClient for NeverClient {
            async fn send(
                &self,
                _: crate::runtime::dispatch::http::HttpRequest,
            ) -> Result<crate::runtime::dispatch::http::HttpResponse, RuntimeError> {
                unreachable!("dry_run + local action should not dispatch HTTP")
            }
        }

        let mut step = compiled_step("fail-step", 0);
        step.kind = ags_protocol::workflow::StepKind::Local;
        step.action = Some("test-fail".into());
        step.operation = None;

        let compiled = CompiledWorkflow {
            id: WorkflowId::new("test-wf"),
            name: "Test".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: false,
            steps: vec![step],
            outputs: vec![],
            completion: None,
        };

        let mut runtime = crate::runtime::Runtime::new(
            crate::runtime::execution::ExecutionContext::default(),
            Box::new(NeverClient),
            reqwest::Client::new(),
        );
        let options = RunOptions {
            dry_run: true,
            assume_yes: true,
            no_input: true,
            ..Default::default()
        };
        let mut run_context = RunContext::new(&mut runtime, &options);
        let mut frontend = EventCapture::new();

        let result =
            Executor::execute(&compiled, BTreeMap::new(), &mut frontend, &mut run_context).await;

        // With the fix: the error is captured in step bookkeeping, not
        // propagated via bare `?`. The executor returns Ok with a Failed
        // outcome and the error in pending_error.
        let (outcome, _, pending_error) = result.expect(
            "dry-run action failure must be captured in step bookkeeping, \
             not escape the executor as bare Err",
        );
        assert_eq!(
            outcome,
            RunOutcome::Failed,
            "run outcome must be Failed when the local action fails"
        );
        assert!(
            pending_error.is_some(),
            "pending_error must carry the action's error"
        );

        // StepFinished must have been emitted with Failed outcome.
        let finished = frontend.finished.lock().unwrap();
        assert_eq!(finished.len(), 1, "exactly one StepFinished expected");
        assert_eq!(finished[0].0, "fail-step");
        assert_eq!(
            finished[0].1,
            StepOutcome::Failed,
            "StepFinished must carry Failed outcome"
        );
    }
}
