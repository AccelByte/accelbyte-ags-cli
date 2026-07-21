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
use crate::frontend::sink::ExecutionFrontendAdapter;
use crate::invocation::InvocationOutcome;

/// Selects how the workflow executor's lifecycle chrome is forwarded.
pub(crate) enum AdapterMode {
    /// Forward all workflow lifecycle events — registered multi-step workflows.
    FullLifecycle,
    /// Suppress workflow lifecycle banners for synthesised single commands.
    SuppressedLifecycle,
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

/// Drives `RunStarted → execute → classify → RunFinished → resolution_trace`
/// on the given progress surface.
///
/// Returns `(outcome, pending_failure, final_output)`. Does NOT call
/// `finish()` on the surface and does NOT render the final result — the
/// caller does that differently for `Split` vs `Unified`.
async fn drive_run(
    progress: &mut dyn crate::frontend::Frontend,
    interaction: &mut dyn crate::frontend::ExecutionInteraction,
    pre_supplied: BTreeMap<String, serde_json::Value>,
    ctx: DriveRunContext<'_>,
) -> (
    InvocationOutcome,
    Option<CliError>,
    Option<ags_protocol::output::CommandOutput>,
) {
    progress.on_event(&crate::frontend::FrontendEvent::RunStarted {
        workflow_banner: None,
    });

    let execution = {
        let mut run_ctx = RunContext::new(ctx.runtime, ctx.options);
        let mut adapter = match ctx.adapter_mode {
            AdapterMode::FullLifecycle => ExecutionFrontendAdapter::new(progress, interaction),
            AdapterMode::SuppressedLifecycle => {
                ExecutionFrontendAdapter::new_for_synthesised_command(progress, interaction)
            }
        };
        Executor::execute(ctx.compiled, pre_supplied, &mut adapter, &mut run_ctx).await
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

    // `RunFinished` reflects the executor outcome, not the later render result.
    progress.on_event(&crate::frontend::FrontendEvent::RunFinished {
        outcome: after_run_outcome,
    });

    // Resolution traces belong to the progress surface and render before teardown.
    if let Some(trace) = ctx.resolution_trace {
        progress.render_resolution_trace(&trace);
    }

    (outcome, pending_failure, final_output)
}

/// Execution plumbing threaded through [`drive_run`] to the executor and the
/// resolution-trace render. Bundled so `drive_run`'s argument list stays small
/// (the load-bearing parameters are `progress` + `interaction`).
struct DriveRunContext<'a> {
    compiled: &'a CompiledWorkflow,
    runtime: &'a mut Runtime,
    options: &'a RunOptions,
    adapter_mode: &'a AdapterMode,
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
) -> Result<InvocationOutcome, CliError> {
    match surfaces {
        crate::frontend::ExecutionPhaseSurfaces::Split {
            mut progress_frontend,
            mut final_frontend,
            mut interaction,
        } => {
            let (outcome, pending_failure, final_output) = drive_run(
                progress_frontend.as_mut(),
                interaction.as_mut(),
                pre_supplied,
                DriveRunContext {
                    compiled,
                    runtime,
                    options,
                    adapter_mode: &adapter_mode,
                    resolution_trace,
                },
            )
            .await;

            // Tear down the progress surface before final rendering restores stdout/stderr order.
            let mut outcome = match outcome {
                InvocationOutcome::Complete => {
                    progress_frontend.finish()?;
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
            Ok(outcome)
        }
        crate::frontend::ExecutionPhaseSurfaces::Unified {
            mut surface,
            mut interaction,
        } => {
            let (mut outcome, pending_failure, final_output) = drive_run(
                surface.as_mut(),
                interaction.as_mut(),
                pre_supplied,
                DriveRunContext {
                    compiled,
                    runtime,
                    options,
                    adapter_mode: &adapter_mode,
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
                    surface.finish()?;
                    Ok(InvocationOutcome::Complete)
                }
                other => {
                    let _ = surface.finish();
                    Ok(other)
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

        let outcome = run_phase_owned_execution(
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

        let outcome = run_phase_owned_execution(
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
