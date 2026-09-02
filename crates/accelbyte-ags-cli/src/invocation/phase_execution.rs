//! Shared post-prologue execution lifecycle for the self-owned, phase-aware
//! command families (`workflow run` and synthesised `ags <service> <op>`).
//!
//! The caller does all pre-execution work (parse/help/prologue) with NO
//! presentation surfaces, constructs the phase-resolved
//! [`ExecutionPhaseSurfaces`], then hands them here. This helper owns the
//! lifecycle from `RunStarted` through `final_frontend.finish()`.

use std::collections::BTreeMap;

use ags_protocol::workflow::CompiledWorkflow;
use ags_runtime::runtime::workflows::executor::{Executor, RunContext};
use ags_runtime::runtime::workflows::{RunOptions, RunOutcome};
use ags_runtime::runtime::Runtime;

use crate::errors::CliError;
use crate::frontend::sink::{ExecutionFrontendAdapter, WorkflowStepTelemetry};
use crate::invocation::InvocationOutcome;

/// Selects how the workflow executor's lifecycle chrome is forwarded.
pub(crate) enum AdapterMode {
    /// Forward all workflow lifecycle events — registered multi-step
    /// workflows. `telemetry` is `Some` when step-level telemetry is enabled
    /// for this run (telemetry configured AND a `sub` was resolved), `None`
    /// otherwise (telemetry disabled, or `resolve_identity` failed) — in
    /// both `None` cases step events are simply not emitted, exactly like
    /// every other best-effort telemetry path in this codebase.
    ///
    /// Boxed per `clippy::large_enum_variant`: `WorkflowStepTelemetry`
    /// (which embeds a `TelemetryClient`) is far larger than the unit
    /// `SuppressedLifecycle` variant, so leaving it unboxed would size every
    /// `AdapterMode` value to the largest variant.
    FullLifecycle {
        telemetry: Option<Box<WorkflowStepTelemetry>>,
    },
    /// Suppress workflow lifecycle banners for synthesised single commands.
    SuppressedLifecycle,
}

/// Upper bound on flushing the step-telemetry client. `posthog-rs`'s default
/// client has no request-timeout override configured by
/// `TelemetryClient::from_env()`, so a black-holed connection could otherwise
/// stall a flush for a long time (its default request timeout times its
/// default retry count) — well past the point the command has already
/// finished all its real work. Telemetry must never delay CLI exit, so this
/// bound is applied to every flush of the step-telemetry client, and the
/// `Result` (timeout or completion) is discarded either way — same
/// fire-and-forget philosophy as the rest of this telemetry pipeline.
const STEP_TELEMETRY_FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Flush a step-telemetry client with [`STEP_TELEMETRY_FLUSH_TIMEOUT`] bound
/// so a stalled network connection cannot hang CLI exit indefinitely. A no-op
/// when `client` is `None` (no step telemetry was configured for this run).
pub(crate) async fn flush_step_telemetry(
    client: Option<ags_runtime::runtime::telemetry::TelemetryClient>,
) {
    if let Some(client) = client {
        let _ = tokio::time::timeout(STEP_TELEMETRY_FLUSH_TIMEOUT, client.flush()).await;
    }
}

/// Map a workflow `RunOutcome` to the CLI `InvocationOutcome`.
///
/// `Failed` with no pending error maps to `Exit(1)` — the existing behavior
/// for both families. A failure that carries a pending error instead uses the
/// error's real `exit_code()` (see the classification below).
fn run_outcome_to_invocation(run_outcome: RunOutcome) -> InvocationOutcome {
    match run_outcome {
        RunOutcome::Success => InvocationOutcome::Complete,
        RunOutcome::Failed => InvocationOutcome::Exit(1),
        RunOutcome::Cancelled => InvocationOutcome::Cancelled,
    }
}

/// Drives `RunStarted → run_started → execute → classify → run_completed →
/// RunFinished → resolution_trace` on the given progress surface.
///
/// Returns `(outcome, pending_failure, final_output, telemetry_client)`. Does
/// NOT call `finish()` on the surface and does NOT render the final result —
/// the caller does that differently for `Split` vs `Unified`.
/// `telemetry_client` is the step-telemetry client reclaimed from the
/// adapter (if any was configured), so the caller can flush the exact
/// instance that queued `cli.workflow.step_*` events.
///
/// `cli.workflow.run_started` is emitted before `Executor::execute` is even
/// called — a `--no-input` rejection returns `Err` from the executor before
/// any workflow event fires, so the run funnel would otherwise never see
/// these runs at all. `cli.workflow.run_completed` is emitted after the
/// executor's result is classified (so `pending_failure` is known) and
/// before `RunFinished`, so both run events fire on every exit path: success,
/// failure, cancellation, the `--no-input` precheck rejection, and a
/// declined briefing. Both events are only emitted for a registered workflow
/// run with telemetry enabled (`AdapterMode::FullLifecycle { telemetry: Some }`)
/// — a synthesised single command never gets run events, since
/// `cli.command.invoked` already covers it.
async fn drive_run(
    progress: &mut dyn crate::frontend::Frontend,
    interaction: &mut dyn crate::frontend::ExecutionInteraction,
    pre_supplied: BTreeMap<String, serde_json::Value>,
    ctx: DriveRunContext<'_>,
) -> (
    InvocationOutcome,
    Option<CliError>,
    Option<ags_protocol::output::CommandOutput>,
    Option<ags_runtime::runtime::telemetry::TelemetryClient>,
) {
    progress.on_event(&crate::frontend::FrontendEvent::RunStarted {
        workflow_banner: None,
    });

    let (execution, telemetry, run_facts) = {
        let mut run_ctx = RunContext::new(ctx.runtime, ctx.options);
        let mut adapter = match ctx.adapter_mode {
            AdapterMode::FullLifecycle { telemetry: Some(t) } => {
                // Emit `run_started` before the executor runs, so a
                // `--no-input` rejection — which returns `Err` before any
                // workflow event fires — still produces a run funnel entry.
                ags_runtime::runtime::telemetry::capture_workflow_run_started(
                    &t.client,
                    &t.sub,
                    &t.context,
                    ctx.options.assume_yes,
                    ctx.options.no_input,
                );
                ExecutionFrontendAdapter::new_with_telemetry(progress, interaction, *t)
            }
            AdapterMode::FullLifecycle { telemetry: None } => {
                ExecutionFrontendAdapter::new(progress, interaction)
            }
            AdapterMode::SuppressedLifecycle => {
                ExecutionFrontendAdapter::new_for_synthesised_command(progress, interaction)
            }
        };
        let execution =
            Executor::execute(ctx.compiled, pre_supplied, &mut adapter, &mut run_ctx).await;
        let (telemetry, run_facts) = adapter.into_telemetry_parts();
        (execution, telemetry, run_facts)
    };

    // Classify the executor result before any final rendering.
    let mut pending_failure: Option<CliError> = None;
    let mut final_output: Option<ags_protocol::output::CommandOutput> = None;
    let (outcome, after_run_outcome) = match execution {
        Ok((run_outcome, output, pending_error)) => match pending_error {
            Some(error) => {
                let error = CliError::from(error);
                let exit_code = error.exit_code();
                pending_failure = Some(error);
                (
                    InvocationOutcome::Exit(exit_code),
                    crate::frontend::RunOutcome::Failed,
                )
            }
            None => {
                final_output = output;
                let after = match run_outcome {
                    RunOutcome::Cancelled => crate::frontend::RunOutcome::Cancelled,
                    RunOutcome::Failed => crate::frontend::RunOutcome::Failed,
                    RunOutcome::Success => crate::frontend::RunOutcome::Success,
                };
                (run_outcome_to_invocation(run_outcome), after)
            }
        },
        Err(error) => {
            let error = CliError::from(error);
            let exit_code = error.exit_code();
            pending_failure = Some(error);
            (
                InvocationOutcome::Exit(exit_code),
                crate::frontend::RunOutcome::Failed,
            )
        }
    };

    // Emit `run_completed` for every exit path — success, failure,
    // cancellation, the `--no-input` precheck rejection, and a declined
    // briefing — now that `pending_failure` and `after_run_outcome` are both
    // known. Only present when telemetry was configured for this run.
    if let Some(t) = &telemetry {
        let facts = run_completed_facts(&run_facts, after_run_outcome, pending_failure.as_ref());
        ags_runtime::runtime::telemetry::capture_workflow_run_completed(
            &t.client,
            &t.sub,
            &t.context,
            &facts,
            ctx.options.assume_yes,
            ctx.options.no_input,
        );
    }
    let telemetry_client = telemetry.map(|t| t.client);

    // `RunFinished` reflects the executor outcome, not the later render result.
    progress.on_event(&crate::frontend::FrontendEvent::RunFinished {
        outcome: after_run_outcome,
    });

    // Resolution traces belong to the progress surface and render before teardown.
    if let Some(trace) = ctx.resolution_trace {
        progress.render_resolution_trace(&trace);
    }

    (outcome, pending_failure, final_output, telemetry_client)
}

/// Project the executor's run aggregate plus the classified failure into the
/// transmittable `RunCompletedFacts`. `RunFacts` defaults to zeroed counts
/// when the executor returned before emitting `WorkflowFinished` — a
/// `--no-input` precheck rejection, or a `?` that propagated out of
/// `skip_step` (via `bind_skipped_outputs`) or `decide_step_failure` before
/// the final event fired.
fn run_completed_facts(
    run_facts: &Option<ags_protocol::workflow::RunFacts>,
    outcome: crate::frontend::RunOutcome,
    failure: Option<&CliError>,
) -> ags_runtime::runtime::telemetry::RunCompletedFacts {
    let facts = run_facts.clone().unwrap_or_default();
    let metadata = failure.and_then(CliError::metadata);
    ags_runtime::runtime::telemetry::RunCompletedFacts {
        outcome: match outcome {
            crate::frontend::RunOutcome::Success => "completed",
            crate::frontend::RunOutcome::Failed => "failed",
            crate::frontend::RunOutcome::Cancelled => "cancelled",
        },
        reason: run_reason_label(&facts, failure),
        duration_ms: facts.duration_ms,
        run_mode: facts.run_mode.map(run_mode_label),
        steps_started: facts.steps_started,
        steps_succeeded: facts.steps_succeeded,
        steps_failed: facts.steps_failed,
        steps_skipped: facts.steps_skipped,
        steps_cancelled: facts.steps_cancelled,
        last_step_index: facts.last_step_index,
        error_class: failure.map(CliError::telemetry_class),
        http_status: metadata.and_then(|m| m.http_status),
        error_code: metadata.and_then(|m| m.code.clone()),
        inputs_from_flag: facts.inputs_from_flag,
        inputs_from_prompt: facts.inputs_from_prompt,
        inputs_from_default: facts.inputs_from_default,
        inputs_edited_in_form: facts.inputs_edited_in_form,
    }
}

/// Stable telemetry label for a run stop-mode.
fn run_mode_label(mode: ags_protocol::workflow::RunMode) -> &'static str {
    match mode {
        ags_protocol::workflow::RunMode::ReviewInputSteps => "review_input_steps",
        ags_protocol::workflow::RunMode::ReviewEveryStep => "review_every_step",
        ags_protocol::workflow::RunMode::RunWithoutStopping => "run_without_stopping",
    }
}

/// Run-level reason, in precedence order:
///
/// 1. The executor's own `RunFacts.reason`, when it set one. Only the
///    pre-step cancellation stages do — `at_briefing` and `at_input_gather` —
///    and nothing else can report them: no step ever starts, so there is no
///    step event to carry the stage, and a declined briefing would otherwise
///    emit `cancelled` with no reason at all.
/// 2. `no_input` when the classified failure's own machine error code marks a
///    `--no-input` rejection (a `no_input.`-prefixed code, populated by the
///    executor's non-interactive precheck), which happens *before*
///    `WorkflowFinished` and so never arrives with a `RunFacts` at all.
///
/// The fallback is deliberately keyed off the error's code rather than
/// `run_facts.is_none()`: `Executor::execute` also returns `Err` without ever
/// emitting `WorkflowFinished` when `?` propagates out of `skip_step` (via
/// `bind_skipped_outputs`) or out of `decide_step_failure` — an
/// `run_facts.is_none()` heuristic would mislabel those unrelated failures as
/// `no_input` too.
fn run_reason_label(
    facts: &ags_protocol::workflow::RunFacts,
    failure: Option<&CliError>,
) -> Option<&'static str> {
    if let Some(reason) = facts.reason {
        return Some(reason.as_label());
    }
    let code = failure
        .and_then(CliError::metadata)
        .and_then(|metadata| metadata.code.as_deref())?;
    code.starts_with("no_input.")
        .then(|| ags_protocol::workflow::StepOutcomeReason::NoInput.as_label())
}

/// Execution plumbing threaded through [`drive_run`] to the executor and the
/// resolution-trace render. Bundled so `drive_run`'s argument list stays small
/// (the load-bearing parameters are `progress` + `interaction`).
struct DriveRunContext<'a> {
    compiled: &'a CompiledWorkflow,
    runtime: &'a mut Runtime,
    options: &'a RunOptions,
    adapter_mode: AdapterMode,
    resolution_trace: Option<ags_protocol::output::ResolutionTrace>,
}

/// Own the post-prologue execution lifecycle and preserve CLI exit codes.
pub(crate) async fn run_phase_owned_execution(
    surfaces: crate::frontend::ExecutionPhaseSurfaces,
    compiled: &CompiledWorkflow,
    pre_supplied: BTreeMap<String, serde_json::Value>,
    runtime: &mut Runtime,
    options: &RunOptions,
    adapter_mode: AdapterMode,
    resolution_trace: Option<ags_protocol::output::ResolutionTrace>,
) -> Result<
    (
        InvocationOutcome,
        Option<ags_runtime::runtime::telemetry::TelemetryClient>,
    ),
    CliError,
> {
    match surfaces {
        crate::frontend::ExecutionPhaseSurfaces::Split {
            mut progress_frontend,
            mut final_frontend,
            mut interaction,
        } => {
            let (outcome, pending_failure, final_output, telemetry_client) = drive_run(
                progress_frontend.as_mut(),
                interaction.as_mut(),
                pre_supplied,
                DriveRunContext {
                    compiled,
                    runtime,
                    options,
                    adapter_mode,
                    resolution_trace,
                },
            )
            .await;

            // Tear down the progress surface before final rendering restores stdout/stderr order.
            let mut outcome = match outcome {
                InvocationOutcome::Complete => {
                    if let Err(err) = progress_frontend.finish() {
                        // Teardown failed: flush the already-reclaimed
                        // step-telemetry client before propagating the error,
                        // otherwise it is dropped unflushed by the early
                        // return (see Finding 2 in the final review — the
                        // process may later `std::process::exit`, so `Drop`
                        // alone would not save queued events).
                        flush_step_telemetry(telemetry_client).await;
                        return Err(err);
                    }
                    InvocationOutcome::Complete
                }
                other => {
                    let _ = progress_frontend.finish();
                    other
                }
            };

            // Then render the final result or error on the final surface.
            if let Some(error) = pending_failure {
                final_frontend.render_error(&error);
            } else if let Some(output) = final_output {
                if let Err(error) = final_frontend.render(&output) {
                    // Final-render failure does not change the recorded run outcome.
                    let exit_code = error.exit_code();
                    final_frontend.render_error(&error);
                    outcome = InvocationOutcome::Exit(exit_code);
                }
            }
            let _ = final_frontend.finish();
            Ok((outcome, telemetry_client))
        }
        crate::frontend::ExecutionPhaseSurfaces::Unified {
            mut surface,
            mut interaction,
        } => {
            let (mut outcome, pending_failure, final_output, telemetry_client) = drive_run(
                surface.as_mut(),
                interaction.as_mut(),
                pre_supplied,
                DriveRunContext {
                    compiled,
                    runtime,
                    options,
                    adapter_mode,
                    resolution_trace,
                },
            )
            .await;

            // No finish() between progress and final render — the same surface
            // handles both without intermediate teardown.
            if let Some(error) = pending_failure {
                surface.render_error(&error);
            } else if let Some(output) = final_output {
                if let Err(error) = surface.render(&output) {
                    // `RunFinished` already fired with the executor
                    // outcome (Success) before this final render. A render
                    // failure means the run did not actually complete, so flip
                    // the recorded outcome by re-emitting `RunFinished { Failed }`
                    // on the still-live unified surface. (The split branch cannot
                    // do this — its progress surface is torn down before the
                    // separate final render — but its exit code already reflects
                    // the failure, which is what plain/JSON consumers key on.)
                    let exit_code = error.exit_code();
                    surface.render_error(&error);
                    surface.on_event(&crate::frontend::FrontendEvent::RunFinished {
                        outcome: crate::frontend::RunOutcome::Failed,
                    });
                    outcome = InvocationOutcome::Exit(exit_code);
                }
            }

            // Single teardown for the unified surface.
            match outcome {
                InvocationOutcome::Complete => {
                    if let Err(err) = surface.finish() {
                        // Teardown failed: flush the already-reclaimed
                        // step-telemetry client before propagating the error
                        // (see the matching comment in the `Split` arm above).
                        flush_step_telemetry(telemetry_client).await;
                        return Err(err);
                    }
                    Ok((InvocationOutcome::Complete, telemetry_client))
                }
                other => {
                    let _ = surface.finish();
                    Ok((other, telemetry_client))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::run_outcome_to_invocation;
    use crate::invocation::InvocationOutcome;
    use ags_runtime::runtime::workflows::RunOutcome;

    #[test]
    fn test_run_outcome_to_invocation_success_is_complete() {
        assert!(matches!(
            run_outcome_to_invocation(RunOutcome::Success),
            InvocationOutcome::Complete
        ));
    }

    #[test]
    fn test_run_outcome_to_invocation_failed_is_exit_1() {
        // A bare `Failed` maps to `Exit(1)`; pending errors override that upstream.
        assert!(matches!(
            run_outcome_to_invocation(RunOutcome::Failed),
            InvocationOutcome::Exit(1)
        ));
    }

    #[test]
    fn test_run_outcome_to_invocation_cancelled_is_cancelled() {
        assert!(matches!(
            run_outcome_to_invocation(RunOutcome::Cancelled),
            InvocationOutcome::Cancelled
        ));
    }

    // ------------------------------------------------------------------ //
    // run_reason_label / run_mode_label / run_completed_facts             //
    // ------------------------------------------------------------------ //

    use super::{run_completed_facts, run_mode_label, run_reason_label};
    use crate::errors::{CliError, ErrorMetadata};

    /// Build a `CliError::Usage` carrying only the given machine code, for
    /// testing the `--no-input` detection path in isolation.
    fn usage_error_with_code(code: &str) -> CliError {
        CliError::Usage {
            message: "irrelevant".into(),
            metadata: Some(Box::new(ErrorMetadata {
                code: Some(code.to_string()),
                ..Default::default()
            })),
        }
    }

    /// A zeroed `RunFacts`: the executor set no run-level stage, so
    /// `run_reason_label` must fall through to its error-code sniff.
    fn no_run_reason() -> ags_protocol::workflow::RunFacts {
        ags_protocol::workflow::RunFacts::default()
    }

    #[test]
    fn test_run_reason_label_none_when_no_failure() {
        assert_eq!(run_reason_label(&no_run_reason(), None), None);
    }

    /// A failure with no metadata at all (e.g. `CliError::Internal`) must not
    /// be mislabeled `no_input`.
    #[test]
    fn test_run_reason_label_none_when_no_metadata() {
        let err = CliError::Internal(anyhow::anyhow!("boom"));
        assert_eq!(run_reason_label(&no_run_reason(), Some(&err)), None);
    }

    /// An ordinary failure whose code does not start with `no_input.` must
    /// report no run-level reason — the per-step events carry that detail.
    /// This is the case the brief's original heuristic (`run_facts.is_none()
    /// && failure.is_some()`) would have wrongly stamped `no_input` on, since
    /// `skip_step`/`decide_step_failure` can also return `Err` before
    /// `WorkflowFinished` fires.
    #[test]
    fn test_run_reason_label_none_for_unrelated_error_code() {
        let err = usage_error_with_code("validation.bad_input");
        assert_eq!(run_reason_label(&no_run_reason(), Some(&err)), None);
    }

    /// The one case that must report `no_input`: the failure's own machine
    /// code is `no_input`-prefixed, populated by the executor's
    /// non-interactive precheck.
    #[test]
    fn test_run_reason_label_no_input_when_code_prefixed() {
        let err = usage_error_with_code("no_input.missing_input");
        assert_eq!(
            run_reason_label(&no_run_reason(), Some(&err)),
            Some("no_input")
        );
    }

    /// The two pre-step cancellation stages: the executor's own
    /// `RunFacts.reason` is the only source for them, and a run that cancels
    /// at the briefing or the run-start gather must report the stage rather
    /// than an unexplained bare `cancelled`. Both go end to end through
    /// `run_completed_facts`, which is what feeds the emitted event.
    #[test]
    fn test_run_completed_facts_reports_each_pre_step_cancellation_stage() {
        for (reason, expected) in [
            (
                ags_protocol::workflow::StepOutcomeReason::AtBriefing,
                "at_briefing",
            ),
            (
                ags_protocol::workflow::StepOutcomeReason::AtInputGather,
                "at_input_gather",
            ),
        ] {
            let run_facts = ags_protocol::workflow::RunFacts {
                reason: Some(reason),
                ..Default::default()
            };
            let facts = run_completed_facts(
                &Some(run_facts),
                crate::frontend::RunOutcome::Cancelled,
                None,
            );
            assert_eq!(facts.outcome, "cancelled");
            assert_eq!(
                facts.reason,
                Some(expected),
                "cancellation stage {reason:?} must reach outcome_reason"
            );
        }
    }

    /// The executor's stage wins over the error-code sniff: a run that
    /// cancelled at the briefing keeps `at_briefing` even if some unrelated
    /// failure is also in hand.
    #[test]
    fn test_run_reason_label_prefers_the_executor_stage_over_the_code_sniff() {
        let run_facts = ags_protocol::workflow::RunFacts {
            reason: Some(ags_protocol::workflow::StepOutcomeReason::AtBriefing),
            ..Default::default()
        };
        let err = usage_error_with_code("no_input.missing_input");
        assert_eq!(
            run_reason_label(&run_facts, Some(&err)),
            Some("at_briefing")
        );
    }

    #[test]
    fn test_run_mode_label_maps_all_variants() {
        assert_eq!(
            run_mode_label(ags_protocol::workflow::RunMode::ReviewInputSteps),
            "review_input_steps"
        );
        assert_eq!(
            run_mode_label(ags_protocol::workflow::RunMode::ReviewEveryStep),
            "review_every_step"
        );
        assert_eq!(
            run_mode_label(ags_protocol::workflow::RunMode::RunWithoutStopping),
            "run_without_stopping"
        );
    }

    /// A `--no-input` precheck rejection never reaches `WorkflowFinished`, so
    /// `run_facts` is `None`; the mapped `RunCompletedFacts` must still carry
    /// zeroed counts (never a panic or garbage) plus the `no_input` reason
    /// and the failure's real error class/code.
    #[test]
    fn test_run_completed_facts_zeroed_when_run_facts_none_precheck_rejection() {
        let err = usage_error_with_code("no_input.missing_input");
        let facts = run_completed_facts(&None, crate::frontend::RunOutcome::Failed, Some(&err));
        assert_eq!(facts.outcome, "failed");
        assert_eq!(facts.reason, Some("no_input"));
        assert_eq!(facts.duration_ms, 0);
        assert_eq!(facts.steps_started, 0);
        assert_eq!(facts.error_class, Some("usage"));
        assert_eq!(facts.error_code.as_deref(), Some("no_input.missing_input"));
    }

    /// A plain success carries no reason, no error class, and reflects the
    /// executor's real aggregate counts.
    #[test]
    fn test_run_completed_facts_success_has_no_reason_or_error_class() {
        let run_facts = ags_protocol::workflow::RunFacts {
            steps_started: 3,
            steps_succeeded: 3,
            duration_ms: 1234,
            ..Default::default()
        };
        let facts =
            run_completed_facts(&Some(run_facts), crate::frontend::RunOutcome::Success, None);
        assert_eq!(facts.outcome, "completed");
        assert_eq!(facts.reason, None);
        assert_eq!(facts.error_class, None);
        assert_eq!(facts.steps_started, 3);
        assert_eq!(facts.steps_succeeded, 3);
        assert_eq!(facts.duration_ms, 1234);
    }

    // ------------------------------------------------------------------ //
    // Unified surface call-order test                                      //
    // ------------------------------------------------------------------ //

    /// Prove that the `Unified` branch drives ONE surface for both progress
    /// and the final render, with exactly ONE `finish()` call at the very
    /// end — no `finish()` between the progress phase and the final render.
    #[tokio::test]
    async fn test_unified_surfaces_single_finish_after_render() {
        use super::{run_phase_owned_execution, AdapterMode};
        use crate::errors::CliError;
        use crate::frontend::{ExecutionInteraction, ExecutionPhaseSurfaces, Frontend};
        use ags_protocol::output::CommandOutput;
        use ags_protocol::workflow::{
            CompiledStep, CompiledWorkflow, GatherResult, StepPreview, SuppliedInputView,
            WorkflowId, WorkflowInputNeeded,
        };
        use ags_runtime::runtime::dispatch::http::{HttpClient, HttpRequest, HttpResponse};
        use ags_runtime::runtime::execution::ExecutionContext;
        use ags_runtime::runtime::workflows::RunOptions;
        use ags_runtime::runtime::Runtime;
        use std::cell::RefCell;
        use std::collections::BTreeMap;
        use std::rc::Rc;

        // ---------------------------------------------------------------- //
        // Recording Frontend                                                //
        // ---------------------------------------------------------------- //

        struct RecordingFrontend {
            log: Rc<RefCell<Vec<String>>>,
        }

        impl Frontend for RecordingFrontend {
            fn on_event(&mut self, event: &crate::frontend::FrontendEvent) {
                use crate::frontend::FrontendEvent;
                let label = match event {
                    FrontendEvent::RunStarted { .. } => "RunStarted",
                    FrontendEvent::RunFinished { .. } => "RunFinished",
                    FrontendEvent::Progress { .. } => "Progress",
                    FrontendEvent::StepStarted { .. } => "StepStarted",
                    FrontendEvent::StepFinished { .. } => "StepFinished",
                };
                self.log.borrow_mut().push(label.to_string());
            }

            fn render(&mut self, _output: &CommandOutput) -> Result<(), CliError> {
                self.log.borrow_mut().push("render".to_string());
                Ok(())
            }

            fn render_error(&mut self, _err: &CliError) {
                self.log.borrow_mut().push("render_error".to_string());
            }

            fn render_warning(
                &mut self,
                _message: &str,
                _reason: Option<&str>,
                _tip: Option<&str>,
            ) {
                self.log.borrow_mut().push("render_warning".to_string());
            }

            fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {
                self.log
                    .borrow_mut()
                    .push("render_resolution_trace".to_string());
            }

            fn finish(self: Box<Self>) -> Result<(), CliError> {
                self.log.borrow_mut().push("finish".to_string());
                Ok(())
            }
        }

        // ---------------------------------------------------------------- //
        // Recording ExecutionInteraction (no-op; zero-step workflow never  //
        // calls gather or confirm)                                          //
        // ---------------------------------------------------------------- //

        struct NoopInteraction;

        impl ExecutionInteraction for NoopInteraction {
            fn gather_workflow_inputs(
                &mut self,
                _needed: &[WorkflowInputNeeded],
                _step_context: &CompiledStep,
                _supplied: &[SuppliedInputView],
            ) -> Result<GatherResult, CliError> {
                Ok(GatherResult::default())
            }

            fn confirm_step(
                &mut self,
                _step: &CompiledStep,
                _preview: &StepPreview,
            ) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
                Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
            }
        }

        // ---------------------------------------------------------------- //
        // Minimal Runtime (NeverClient — zero-step workflow never dispatches)
        // ---------------------------------------------------------------- //

        struct NeverClient;

        #[async_trait::async_trait]
        impl HttpClient for NeverClient {
            async fn send(
                &self,
                _: HttpRequest,
            ) -> Result<HttpResponse, ags_protocol::error::RuntimeError> {
                unreachable!("zero-step workflow must not dispatch")
            }
        }

        let mut runtime = Runtime::new(
            ExecutionContext::default(),
            Box::new(NeverClient),
            reqwest::Client::new(),
        );

        // ---------------------------------------------------------------- //
        // Zero-step compiled workflow                                       //
        // ---------------------------------------------------------------- //

        let compiled = CompiledWorkflow {
            id: WorkflowId::new("unified-test"),
            name: "unified test".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![],
            outputs: vec![],
            completion: None,
        };

        // ---------------------------------------------------------------- //
        // Run via Unified surfaces                                          //
        // ---------------------------------------------------------------- //

        let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let recording = RecordingFrontend {
            log: Rc::clone(&log),
        };

        let surfaces = ExecutionPhaseSurfaces::Unified {
            surface: Box::new(recording),
            interaction: Box::new(NoopInteraction),
        };

        let options = RunOptions {
            dry_run: true,
            ..Default::default()
        };

        let (outcome, _telemetry_client) = run_phase_owned_execution(
            surfaces,
            &compiled,
            BTreeMap::new(),
            &mut runtime,
            &options,
            AdapterMode::SuppressedLifecycle,
            None,
        )
        .await
        .expect("run_phase_owned_execution must succeed for zero-step workflow");

        assert!(
            matches!(outcome, InvocationOutcome::Complete),
            "expected Complete, got {outcome:?}"
        );

        // ---------------------------------------------------------------- //
        // Assertions on call order                                          //
        // ---------------------------------------------------------------- //

        let observed = log.borrow().clone();

        // At least the lifecycle events must be present.
        assert!(
            observed.contains(&"RunStarted".to_string()),
            "log must contain RunStarted; got {observed:?}"
        );
        assert!(
            observed.contains(&"RunFinished".to_string()),
            "log must contain RunFinished; got {observed:?}"
        );

        // Exactly one finish, and it is the last entry.
        let finish_count = observed.iter().filter(|s| s.as_str() == "finish").count();
        assert_eq!(
            finish_count, 1,
            "Unified branch must call finish() exactly once; got {observed:?}"
        );
        assert_eq!(
            observed.last().map(String::as_str),
            Some("finish"),
            "finish() must be the very last call; got {observed:?}"
        );

        // No finish before the final render. For a zero-step dry run the
        // executor returns a `WorkflowDryRun` envelope, so the Unified branch
        // calls `render` (then `finish`) on the same surface. The key invariant:
        // finish is last and occurs exactly once, after the final render.
        //
        // Explicitly verify there is no finish anywhere before the last entry.
        let last_idx = observed.len() - 1;
        let early_finish = observed[..last_idx].iter().any(|s| s.as_str() == "finish");
        assert!(
            !early_finish,
            "no finish() must appear before the final entry; got {observed:?}"
        );
    }

    // ---------------------------------------------------------------- //
    // final-render failure flips the recorded run outcome              //
    // ---------------------------------------------------------------- //

    /// Prove that when the final render fails on a Unified surface, the run's
    /// recorded outcome is corrected: `RunFinished { Success }` fires first
    /// (before the render), then a corrective `RunFinished { Failed }` fires
    /// after the render error. Without the fix only the first event is emitted,
    /// leaving a successful-looking lifecycle on a run that did not complete.
    #[tokio::test]
    async fn test_unified_render_failure_re_emits_run_finished_failed() {
        use super::{run_phase_owned_execution, AdapterMode};
        use crate::errors::CliError;
        use crate::frontend::{
            ExecutionInteraction, ExecutionPhaseSurfaces, Frontend, FrontendEvent,
        };
        use ags_protocol::output::CommandOutput;
        use ags_protocol::workflow::{
            CompiledStep, CompiledWorkflow, GatherResult, StepPreview, SuppliedInputView,
            WorkflowId, WorkflowInputNeeded,
        };
        use ags_runtime::runtime::dispatch::http::{HttpClient, HttpRequest, HttpResponse};
        use ags_runtime::runtime::execution::ExecutionContext;
        use ags_runtime::runtime::workflows::RunOptions;
        use ags_runtime::runtime::Runtime;
        use std::cell::RefCell;
        use std::collections::BTreeMap;
        use std::rc::Rc;

        struct FailingRenderFrontend {
            log: Rc<RefCell<Vec<String>>>,
        }

        impl Frontend for FailingRenderFrontend {
            fn on_event(&mut self, event: &FrontendEvent) {
                let label = match event {
                    FrontendEvent::RunStarted { .. } => "RunStarted".to_string(),
                    FrontendEvent::RunFinished { outcome } => format!("RunFinished:{outcome:?}"),
                    FrontendEvent::Progress { .. } => "Progress".to_string(),
                    FrontendEvent::StepStarted { .. } => "StepStarted".to_string(),
                    FrontendEvent::StepFinished { .. } => "StepFinished".to_string(),
                };
                self.log.borrow_mut().push(label);
            }

            fn render(&mut self, _output: &CommandOutput) -> Result<(), CliError> {
                self.log.borrow_mut().push("render".to_string());
                Err(CliError::Internal(anyhow::anyhow!("render boom")))
            }

            fn render_error(&mut self, _err: &CliError) {
                self.log.borrow_mut().push("render_error".to_string());
            }

            fn render_warning(&mut self, _m: &str, _r: Option<&str>, _t: Option<&str>) {}

            fn render_resolution_trace(&mut self, _t: &ags_protocol::output::ResolutionTrace) {}

            fn finish(self: Box<Self>) -> Result<(), CliError> {
                self.log.borrow_mut().push("finish".to_string());
                Ok(())
            }
        }

        struct NoopInteraction;

        impl ExecutionInteraction for NoopInteraction {
            fn gather_workflow_inputs(
                &mut self,
                _needed: &[WorkflowInputNeeded],
                _step_context: &CompiledStep,
                _supplied: &[SuppliedInputView],
            ) -> Result<GatherResult, CliError> {
                Ok(GatherResult::default())
            }

            fn confirm_step(
                &mut self,
                _step: &CompiledStep,
                _preview: &StepPreview,
            ) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError> {
                Ok(ags_protocol::workflow::StepConfirmOutcome::Proceed)
            }
        }

        struct NeverClient;

        #[async_trait::async_trait]
        impl HttpClient for NeverClient {
            async fn send(
                &self,
                _: HttpRequest,
            ) -> Result<HttpResponse, ags_protocol::error::RuntimeError> {
                unreachable!("zero-step workflow must not dispatch")
            }
        }

        let mut runtime = Runtime::new(
            ExecutionContext::default(),
            Box::new(NeverClient),
            reqwest::Client::new(),
        );

        // Zero-step dry run: the executor returns a `WorkflowDryRun` envelope,
        // so the Unified branch attempts the final render (which fails here).
        let compiled = CompiledWorkflow {
            id: WorkflowId::new("render-fail-test"),
            name: "render fail test".into(),
            intent: None,
            description: None,
            briefing: None,
            inputs: vec![],
            is_reviewed_by_default: true,
            steps: vec![],
            outputs: vec![],
            completion: None,
        };

        let log: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let recording = FailingRenderFrontend {
            log: Rc::clone(&log),
        };

        let surfaces = ExecutionPhaseSurfaces::Unified {
            surface: Box::new(recording),
            interaction: Box::new(NoopInteraction),
        };

        let options = RunOptions {
            dry_run: true,
            ..Default::default()
        };

        let (outcome, _telemetry_client) = run_phase_owned_execution(
            surfaces,
            &compiled,
            BTreeMap::new(),
            &mut runtime,
            &options,
            AdapterMode::SuppressedLifecycle,
            None,
        )
        .await
        .expect("run_phase_owned_execution returns Ok even when the final render fails");

        // The render failure surfaces as a non-zero exit code.
        assert!(
            matches!(outcome, InvocationOutcome::Exit(code) if code != 0),
            "render failure must yield a non-zero exit; got {outcome:?}"
        );

        let observed = log.borrow().clone();

        // The executor's success fires first (before the final render).
        assert!(
            observed.contains(&"RunFinished:Success".to_string()),
            "executor success must be recorded before render; got {observed:?}"
        );
        // The render is attempted and fails.
        assert!(
            observed.contains(&"render".to_string()),
            "final render must be attempted; got {observed:?}"
        );
        // The corrective lifecycle event flips the outcome to Failed.
        assert!(
            observed.contains(&"RunFinished:Failed".to_string()),
            "render failure must re-emit RunFinished:Failed; got {observed:?}"
        );

        // The correction comes after the render attempt — not before.
        let render_idx = observed
            .iter()
            .position(|s| s == "render")
            .expect("render recorded");
        let failed_idx = observed
            .iter()
            .rposition(|s| s == "RunFinished:Failed")
            .expect("RunFinished:Failed recorded");
        assert!(
            failed_idx > render_idx,
            "corrective RunFinished:Failed must follow the render attempt; got {observed:?}"
        );
    }
}
