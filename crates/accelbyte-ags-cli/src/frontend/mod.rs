//! Presentation layer: format and render all CLI output.

pub mod dynamic_options;
pub mod event;
pub mod output;
mod presenters;
pub mod sink;
pub mod streams;
pub mod style;
pub mod terminal;

use crate::errors::CliError;
use ags_protocol::output::CommandOutput;
use ags_protocol::workflow::{
    CompiledStep, GatherResult, StepPreview, SuppliedInputView, WorkflowInputNeeded,
};
use std::cell::RefCell;
use std::rc::Rc;

/// Pagination metadata for display in list output.
#[derive(Debug, Clone)]
pub struct PaginationHint {
    pub total: Option<u64>,
    pub has_next: bool,
}

/// Text-family selector used by the pure formatting helpers in `output/render.rs`.
/// Decoupled from `PhaseBackend` so the inline-surface→plain mapping happens
/// once at inline-surface construction, not in every per-output match arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderFormat {
    Human,
    Json,
}

/// Final rendered text ready for emission to stdout and/or stderr
#[derive(Debug, Clone, Default)]
pub struct RenderedOutput {
    pub stdout: Option<String>,
    pub stderr: Option<String>,
    /// When true, print stdout before stderr (e.g. status headline first).
    /// When false (default), print stderr before stdout (e.g. verbose trace first).
    pub is_stdout_first: bool,
}

/// Emit rendered output honouring the caller's `--output` flag. When
/// `output` is `Some(path)` or `Some("-")`, the stdout portion goes
/// through `OutputSink` instead of `println!`. The stderr portion
/// always goes to stderr (spinners, confirmation lines, warnings).
pub fn emit_with_options(
    rendered: RenderedOutput,
    options: &RenderOptions,
) -> Result<(), crate::errors::CliError> {
    // stderr is unaffected by --output — always goes to real stderr.
    // stdout goes to OutputSink when --output is set.
    if rendered.is_stdout_first {
        if let Some(stdout) = rendered.stdout.filter(|s| !s.is_empty()) {
            write_stdout(&stdout, options)?;
        }
        if let Some(stderr) = rendered.stderr.filter(|s| !s.is_empty()) {
            write_stderr_line(&stderr);
        }
    } else {
        if let Some(stderr) = rendered.stderr.filter(|s| !s.is_empty()) {
            write_stderr_line(&stderr);
        }
        if let Some(stdout) = rendered.stdout.filter(|s| !s.is_empty()) {
            write_stdout(&stdout, options)?;
        }
    }
    Ok(())
}

pub use event::{FrontendEvent, RunOutcome};
pub(crate) use output::render::render_output;
pub use sink::FrontendSink;

/// Route stdout text either to the console or to the `--output` destination resolved by `OutputSink`.
fn write_stdout(text: &str, options: &RenderOptions) -> Result<(), crate::errors::CliError> {
    use ags_runtime::support::output_sink::OutputSink;

    match options.output.as_ref() {
        // Common case: no --output flag. Writes go through anstream::stdout(),
        // which enables Windows VT mode on first use and surfaces a closed
        // pipe as ErrorKind::BrokenPipe instead of panicking. On Unix,
        // reset_sigpipe() still drives the SIGPIPE-based exit-141 path.
        None => write_stdout_line(text),
        Some(destination) => {
            let sink = OutputSink::resolve(Some(destination), false)
                .map_err(map_output_sink_error_to_cli_error)?;
            // Append a trailing newline to match println! semantics — users
            // expect text files to end with a newline.
            //
            // This is intentionally asymmetric with the binary --output path
            // in runtime::dispatch, which writes raw bytes verbatim.
            let mut bytes = text.as_bytes().to_vec();
            if !bytes.ends_with(b"\n") {
                bytes.push(b'\n');
            }
            sink.write(&bytes)
                .map_err(map_output_sink_error_to_cli_error)
        }
    }
}

/// Convert an output-sink error into the CLI's top-level error type.
pub(crate) fn map_output_sink_error_to_cli_error(
    err: ags_runtime::support::output_sink::OutputSinkError,
) -> crate::errors::CliError {
    use ags_runtime::support::output_sink::OutputSinkError;
    match err {
        OutputSinkError::Usage(message) => crate::errors::CliError::Usage {
            message,
            metadata: None,
        },
        OutputSinkError::Internal(inner) => crate::errors::CliError::Internal(inner),
    }
}

/// Interaction methods for execution: gathering inputs and confirming
/// steps. Each presentation surface provides its own implementation.
///
/// This trait is separate from [`Frontend`] so that the interaction logic can
/// be unit-tested in isolation and later composed independently of rendering.
pub trait ExecutionInteraction {
    /// Show the workflow's long-form briefing before any inputs are
    /// gathered. `Ok(true)` proceeds, `Ok(false)` cancels the run.
    /// Default: `Ok(true)` — only `FullscreenInteraction` overrides.
    fn present_briefing(
        &mut self,
        _briefing: &ags_protocol::workflow::WorkflowBriefing,
        _workflow_name: &str,
    ) -> Result<bool, CliError> {
        Ok(true)
    }

    /// Gather values for the listed workflow input slots. `supplied` carries
    /// the already-resolved inputs (with provenance) the form may pre-fill.
    /// Returns a `GatherResult` with values for the missing slots and any
    /// edited overrides for already-supplied inputs.
    fn gather_workflow_inputs(
        &mut self,
        needed: &[WorkflowInputNeeded],
        step_context: &CompiledStep,
        supplied: &[SuppliedInputView],
    ) -> Result<GatherResult, CliError>;

    /// Per-step confirmation prompt during a workflow run. Returns
    /// `Ok(StepConfirmOutcome::Proceed)` to proceed, `Skip` to skip an optional
    /// step and continue the run, `Cancel` to cancel the workflow.
    fn confirm_step(
        &mut self,
        step: &CompiledStep,
        preview: &StepPreview,
    ) -> Result<ags_protocol::workflow::StepConfirmOutcome, CliError>;

    /// Review/edit a step's full request before it runs. Default: proceed with
    /// no edits — only `FullscreenInteraction` overrides this.
    fn review_step(
        &mut self,
        _plan: &ags_protocol::workflow::StepFieldPlan,
    ) -> Result<ags_protocol::workflow::StepReviewOutcome, CliError> {
        Ok(ags_protocol::workflow::StepReviewOutcome::Proceed(
            ags_protocol::workflow::StepFieldEdits::default(),
        ))
    }

    /// Interactive failure gate: Retry / Skip / Cancel. `allow_skip` is false
    /// when skipping would break downstream. Default: `Cancel` — `machine_json`
    /// and any surface that does not render a gate keep today's fail-fast.
    fn resolve_step_failure(
        &mut self,
        _step: &CompiledStep,
        _error: &ags_protocol::error::RuntimeError,
        _allow_skip: bool,
    ) -> Result<ags_protocol::workflow::StepFailureAction, CliError> {
        Ok(ags_protocol::workflow::StepFailureAction::Cancel)
    }

    /// Phase 1: collect declared workflow inputs. `Ok(Some(outcome))` = proceed
    /// with the declared-input map plus the chosen run stop-mode, `Ok(None)` =
    /// user cancelled (clean cancel), `Err` = I/O failure. Default:
    /// `Ok(Some(CollectOutcome { inputs: current.clone(), run_mode:
    /// RunMode::ReviewInputSteps }))` — only `FullscreenInteraction` and
    /// `InlineInteraction` override this.
    fn collect_workflow_inputs(
        &mut self,
        _specs: &[ags_protocol::workflow::WorkflowInputSpec],
        current: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Option<ags_protocol::workflow::CollectOutcome>, CliError> {
        Ok(Some(ags_protocol::workflow::CollectOutcome {
            inputs: current.clone(),
            run_mode: ags_protocol::workflow::RunMode::ReviewInputSteps,
        }))
    }
}

/// The frontend abstraction: renders output and errors to the appropriate medium,
/// and consumes structured events emitted by the CLI invocation layer (lifecycle)
/// and by the runtime (progress, via `FrontendSink`).
pub trait Frontend {
    /// Consume a structured event (lifecycle or progress). Default: no-op.
    fn on_event(&mut self, _event: &crate::frontend::event::FrontendEvent) {}
    /// Render a command's structured output in this frontend's format.
    /// `RenderOptions` is held on `self` (captured at construction); the trait
    /// method does not take it as a parameter.
    fn render(&mut self, output: &CommandOutput) -> Result<(), CliError>;
    /// Render a fatal error in this frontend's format.
    fn render_error(&mut self, err: &CliError);
    /// Emit a non-fatal warning to stderr.
    fn render_warning(&mut self, message: &str, reason: Option<&str>, tip: Option<&str>);
    /// Emit the verbose resolution trace shown for `--dry-run --verbose`.
    fn render_resolution_trace(&mut self, trace: &ags_protocol::output::ResolutionTrace);
    /// Explicit happy-path teardown. May surface terminal-restore failures
    /// to the user. A `Drop` impl on the concrete frontend type provides
    /// the panic-safety fallback.
    fn finish(self: Box<Self>) -> Result<(), CliError>;
}

/// Phase-resolved presentation surfaces for one self-owned invocation
/// (`workflow run` or a synthesised service command).
///
/// `Split` keeps progress and final rendering on separate surfaces (the
/// existing path for all inline and plain-terminal runs). `Unified` routes
/// both progress events and the final result through the same surface, used
/// by the fullscreen TUI so teardown and result emission happen as one unit.
pub enum ExecutionPhaseSurfaces {
    /// Separate surfaces for progress and final rendering (existing path).
    Split {
        /// Drives lifecycle/progress events during the run.
        progress_frontend: Box<dyn Frontend>,
        /// Renders the final result (`render`) or failure (`render_error`).
        final_frontend: Box<dyn Frontend>,
        /// Drives input gathering and per-step confirmation.
        interaction: Box<dyn ExecutionInteraction>,
    },
    /// Single surface drives both progress events and the final render, with
    /// one teardown at the end. Used by the fullscreen TUI.
    Unified {
        /// Drives lifecycle/progress events AND renders the final result.
        surface: Box<dyn Frontend>,
        /// Drives input gathering and per-step confirmation.
        interaction: Box<dyn ExecutionInteraction>,
    },
}

/// Assemble fullscreen `Unified` surfaces from a built surface: the
/// `FullscreenFrontend` is the progress + final + dismiss surface, and the
/// `FullscreenInteraction` drives gather/confirm — both share one
/// [`FullscreenSurface`], so every phase draws through the identical layout.
fn fullscreen_phase_surfaces_from_surface(
    surface: Rc<RefCell<crate::frontend::terminal::fullscreen::surface::FullscreenSurface>>,
    resolver: Option<Box<dyn crate::frontend::dynamic_options::DynamicOptionResolver>>,
) -> ExecutionPhaseSurfaces {
    let frontend =
        crate::frontend::terminal::fullscreen::frontend::FullscreenFrontend::from_surface(
            Rc::clone(&surface),
        );
    let mut interaction =
        crate::frontend::terminal::fullscreen::interaction::FullscreenInteraction::new(surface);
    if let Some(resolver) = resolver {
        interaction = interaction.with_resolver(resolver);
    }
    ExecutionPhaseSurfaces::Unified {
        surface: Box::new(frontend),
        interaction: Box::new(interaction),
    }
}

/// Construct fullscreen phase surfaces for a workflow run. One
/// [`FullscreenSurface`] owns the alt-screen terminal **and** the render
/// model, shared by the unified [`FullscreenFrontend`] (progress + final +
/// dismiss) and [`FullscreenInteraction`] (gather/confirm in-surface).
///
/// Workflow routes that want the fullscreen surface call this directly
/// with the compiled step list.
pub fn select_fullscreen_workflow_surfaces(
    ctx: &crate::invocation::context::FrontendContext,
    options: RenderOptions,
    workflow_title: String,
    header_kind: crate::frontend::terminal::fullscreen::step_strip::HeaderKind,
    steps: Vec<crate::frontend::terminal::fullscreen::step_strip::Step>,
    workflow_description: Option<String>,
    resolver_factory: impl FnOnce(
        Rc<RefCell<crate::frontend::terminal::fullscreen::surface::FullscreenSurface>>,
    ) -> Option<
        Box<dyn crate::frontend::dynamic_options::DynamicOptionResolver>,
    >,
) -> Result<ExecutionPhaseSurfaces, CliError> {
    let mut surface_inner = crate::frontend::terminal::fullscreen::surface::FullscreenSurface::new(
        options,
        workflow_title,
        steps,
        ctx.terminal.stdout_is_tty,
    )?;
    surface_inner.header_kind = header_kind;
    surface_inner.workflow_description = workflow_description;
    let surface = Rc::new(RefCell::new(surface_inner));
    let resolver = resolver_factory(Rc::clone(&surface));
    Ok(fullscreen_phase_surfaces_from_surface(surface, resolver))
}

/// Construct the phase-resolved [`ExecutionPhaseSurfaces`] for a workflow run.
pub(crate) fn select_workflow_phase_surfaces(
    ctx: &crate::invocation::context::FrontendContext,
    options: RenderOptions,
    full_surface_inputs: Option<Vec<ags_protocol::workflow::WorkflowInputSpec>>,
    options_fetch: Option<Box<dyn crate::frontend::dynamic_options::OptionsFetch>>,
) -> Result<ExecutionPhaseSurfaces, CliError> {
    use crate::invocation::context::{InteractionPhase, PhaseBackend};

    let progress_backend = ctx.backend_for_phase(InteractionPhase::Progress);
    let final_backend = ctx.backend_for_phase(InteractionPhase::Result);
    debug_assert_eq!(
        final_backend,
        ctx.backend_for_phase(InteractionPhase::Error),
        "Result and Error must share the final frontend"
    );
    let interaction_backend = ctx.backend_for_phase(InteractionPhase::Input);
    debug_assert_eq!(
        interaction_backend,
        ctx.backend_for_phase(InteractionPhase::Confirmation),
        "Input and Confirmation must share the interaction surface"
    );

    // Inline runs share one terminal session between progress and interaction.
    if progress_backend == PhaseBackend::InlineTerminalUi
        || interaction_backend == PhaseBackend::InlineTerminalUi
    {
        debug_assert_eq!(
            progress_backend,
            PhaseBackend::InlineTerminalUi,
            "inline workflows route progress through the inline terminal"
        );
        debug_assert_eq!(
            interaction_backend,
            PhaseBackend::InlineTerminalUi,
            "inline workflows route interaction through the inline terminal"
        );
        // One terminal acquisition, shared by progress + interaction.
        let session = Rc::new(RefCell::new(
            crate::frontend::terminal::inline::session::InlineSession::new()?,
        ));
        return inline_phase_surfaces_from_session(
            session,
            final_backend,
            options,
            full_surface_inputs,
            options_fetch,
        );
    }

    Ok(ExecutionPhaseSurfaces::Split {
        progress_frontend: frontend_for_surface(progress_backend, options.clone())?,
        final_frontend: frontend_for_surface(final_backend, options)?,
        interaction: interaction_for_surface(interaction_backend),
    })
}

/// Build a standalone interaction surface for a phase backend.
fn interaction_for_surface(
    backend: crate::invocation::context::PhaseBackend,
) -> Box<dyn ExecutionInteraction> {
    use crate::invocation::context::PhaseBackend;
    match backend {
        PhaseBackend::PlainTerminal => {
            Box::new(crate::frontend::terminal::plain::interaction::PlainInteraction)
        }
        PhaseBackend::StructuredJson => {
            Box::new(crate::frontend::terminal::machine_json::JsonInteraction)
        }
        PhaseBackend::InlineTerminalUi => {
            unreachable!("InlineTerminalUi interaction is built via the shared-session path")
        }
        PhaseBackend::FullscreenTerminalUi => {
            unreachable!("FullscreenTerminalUi interaction is built via the shared-session path")
        }
    }
}

/// Assemble inline-terminal [`ExecutionPhaseSurfaces`] from an acquired session.
fn inline_phase_surfaces_from_session(
    session: Rc<RefCell<crate::frontend::terminal::inline::session::InlineSession>>,
    final_backend: crate::invocation::context::PhaseBackend,
    options: RenderOptions,
    full_surface_inputs: Option<Vec<ags_protocol::workflow::WorkflowInputSpec>>,
    options_fetch: Option<Box<dyn crate::frontend::dynamic_options::OptionsFetch>>,
) -> Result<ExecutionPhaseSurfaces, CliError> {
    let progress_frontend =
        crate::frontend::terminal::inline::frontend::InlineFrontend::from_session(
            Rc::clone(&session),
            options.clone(),
        );
    let mut interaction = crate::frontend::terminal::inline::interaction::InlineInteraction::new(
        session,
        full_surface_inputs,
    );
    if let Some(fetch) = options_fetch {
        interaction = interaction.with_options_fetch(fetch);
    }
    Ok(ExecutionPhaseSurfaces::Split {
        progress_frontend: Box::new(progress_frontend),
        final_frontend: frontend_for_surface(final_backend, options)?,
        interaction: Box::new(interaction),
    })
}

/// Whether `frontend_for_surface` builds a PLAIN-text frontend for this
/// backend. True for `PlainTerminal` and for `FullscreenTerminalUi` — the
/// latter degrades to plain *here* because `frontend_for_surface` is the
/// single-shot path with no step list; real fullscreen workflow runs use
/// `select_fullscreen_workflow_surfaces` instead. The lock-contention reporter
/// gate keys off this so it never registers for a real TUI surface (inline) or JSON.
pub(crate) fn factory_renders_plain(backend: crate::invocation::context::PhaseBackend) -> bool {
    use crate::invocation::context::PhaseBackend;
    matches!(
        backend,
        PhaseBackend::PlainTerminal | PhaseBackend::FullscreenTerminalUi
    )
}

/// Construct the frontend for a presentation surface.
pub fn frontend_for_surface(
    backend: crate::invocation::context::PhaseBackend,
    options: RenderOptions,
) -> Result<Box<dyn Frontend>, CliError> {
    use crate::invocation::context::PhaseBackend;
    // `factory_renders_plain` and this factory stay in sync: both
    // `PlainTerminal` and `FullscreenTerminalUi` build a `PlainFrontend`.
    // `FullscreenTerminalUi` degrades because `frontend_for_surface` returns
    // surfaces that don't need workflow metadata. Fullscreen needs a step list
    // to render the step strip — workflow routes use
    // `select_fullscreen_workflow_surfaces` instead, which threads the compiled
    // steps in. Reaching this arm means an unsupported single-shot route forced
    // `--ui=fullscreen`; fall back to plain for now.
    // TODO: wire the single-shot fullscreen fallback through the frontend
    // factory. The layout layer already supports it; only this factory arm
    // still degrades to plain.
    if factory_renders_plain(backend) {
        return Ok(Box::new(
            crate::frontend::terminal::plain::frontend::PlainFrontend::new(options),
        ));
    }
    match backend {
        PhaseBackend::StructuredJson => Ok(Box::new(
            crate::frontend::output::json::frontend::JsonFrontend::new(options),
        )),
        PhaseBackend::InlineTerminalUi => {
            let inline = crate::frontend::terminal::inline::frontend::InlineFrontend::new(options)?;
            Ok(Box::new(inline))
        }
        PhaseBackend::PlainTerminal | PhaseBackend::FullscreenTerminalUi => {
            unreachable!(
                "handled by the factory_renders_plain early return: PlainTerminal and \
                 FullscreenTerminalUi both build PlainFrontend"
            )
        }
    }
}

/// Render-layer options extracted from global CLI flags.
///
/// Only the fields the render layer actually needs — keeps the render module
/// independent of invocation types. Format is not here: each frontend
/// already embodies its own format.
#[derive(Debug, Default, Clone)]
pub struct RenderOptions {
    pub verbosity: ags_protocol::request::Verbosity,
    pub is_page_all: bool,
    pub output: Option<ags_protocol::request::OutputDestination>,
}

/// Internal stdout helper parameterised by writer for unit testing.
/// Translates `BrokenPipe` to `Ok(())` (matches Unix SIGPIPE-driven exit
/// semantics — a downstream consumer closing the pipe is a stop-signal,
/// not an error). All other I/O errors are mapped to `CliError::Usage`.
/// Flushes after a successful write so downstream consumers see the
/// bytes without depending on platform-specific stream-buffering
/// behaviour. On a `BrokenPipe` from either `writeln!` or the trailing
/// `flush`, the helper returns `Ok(())` without retrying — the stream
/// is broken and any further write would also fail.
fn write_stdout_line_into<W: std::io::Write>(
    mut w: W,
    text: &str,
) -> Result<(), crate::errors::CliError> {
    match writeln!(w, "{text}").and_then(|_| w.flush()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(crate::errors::CliError::Usage {
            message: format!("Cannot write to stdout: {e}."),
            metadata: None,
        }),
    }
}

/// Internal no-newline stderr helper parameterised by writer for unit testing.
/// Used for inline prompts (e.g. "Continue? [y/N] ") where a trailing newline
/// would break the input flow. Always flushes after writing so the prompt
/// is visible before the program blocks on stdin. Same best-effort error
/// policy as the line variant.
fn write_stderr_into<W: std::io::Write>(mut w: W, text: &str) {
    let _ = w.write_all(text.as_bytes()).and_then(|_| w.flush());
}

/// Write `text` followed by a newline to stdout via `anstream::stdout()`.
/// On Windows, `anstream` enables the console's virtual terminal mode on
/// first use, so escape codes produced upstream render correctly.
pub(crate) fn write_stdout_line(text: &str) -> Result<(), crate::errors::CliError> {
    write_stdout_line_into(anstream::stdout().lock(), text)
}

/// Write `text` followed by a newline to stderr via `UiSink`.
/// Errors are silently ignored — stderr is best-effort.
pub(crate) fn write_stderr_line(text: &str) {
    let _ = crate::frontend::streams::UiSink.write_line(text);
}

/// Write `text` to stderr without a trailing newline. Used for inline
/// prompts where the next character on the line is user input.
pub(crate) fn write_stderr(text: &str) {
    write_stderr_into(anstream::stderr().lock(), text);
}

/// Coerce a raw string to the JSON type indicated by `schema["type"]`, using
/// the soft-coercion policy: on parse failure preserve as `Value::String` and
/// let the executor surface the validation error rather than re-prompting.
///
/// - `"string"` → passthrough as `Value::String`.
/// - `"integer"` → parse as `i64`; fall back to `Value::String` on failure.
/// - `"number"` → parse as `f64`; fall back to `Value::String` on failure.
/// - `"boolean"` → accept `"true"` / `"false"` (case-insensitive); fall back to `Value::String`.
/// - `"array"` / `"object"` → parse as JSON; fall back to `Value::String`.
/// - `"null"` → `Value::Null` (input is ignored).
/// - Unknown or absent type → `Value::String` fallback.
pub(crate) fn coerce_to_schema(raw: &str, schema: &serde_json::Value) -> serde_json::Value {
    let type_hint = schema
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("string");

    match type_hint {
        "string" => serde_json::Value::String(raw.to_string()),
        "integer" => raw
            .parse::<i64>()
            .map(|n| serde_json::json!(n))
            .unwrap_or_else(|_| serde_json::Value::String(raw.to_string())),
        "number" => raw
            .parse::<f64>()
            .map(|n| serde_json::json!(n))
            .unwrap_or_else(|_| serde_json::Value::String(raw.to_string())),
        "boolean" => match raw.to_lowercase().as_str() {
            "true" => serde_json::json!(true),
            "false" => serde_json::json!(false),
            _ => serde_json::Value::String(raw.to_string()),
        },
        "array" | "object" => {
            serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_string()))
        }
        "null" => serde_json::Value::Null,
        _ => serde_json::Value::String(raw.to_string()),
    }
}

#[cfg(test)]
mod surface_tests {
    use super::{
        factory_renders_plain, inline_phase_surfaces_from_session, select_workflow_phase_surfaces,
        RenderOptions,
    };
    use crate::frontend::terminal::inline::session::InlineSession;
    use crate::invocation::context::{
        ConsumerKind, FrontendContext, PhaseBackend, TerminalCapabilities,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Build a `FrontendContext` through the real resolver so it cannot
    /// fabricate a context the resolver never produces. Automation implies
    /// no input; rich UI implies an explicit `--ui=inline` intent.
    fn ctx(consumer: ConsumerKind, prefer_rich_ui: bool) -> FrontendContext {
        use crate::invocation::context::resolve_with_terminal;
        use crate::invocation::flags::{GlobalFlags, UiFlag};

        let caps = TerminalCapabilities {
            stdin_is_tty: true,
            stdout_is_tty: true,
            stderr_is_tty: true,
            color_force_off: true,
        };

        let flags = match consumer {
            ConsumerKind::Human => {
                let ui = if prefer_rich_ui {
                    Some(UiFlag::Inline)
                } else {
                    None
                };
                GlobalFlags {
                    ui,
                    ..GlobalFlags::default()
                }
            }
            ConsumerKind::Automation => {
                assert!(!prefer_rich_ui, "automation consumer cannot prefer rich UI");
                GlobalFlags {
                    format: Some(ags_protocol::request::OutputFormat::Json),
                    ..GlobalFlags::default()
                }
            }
        };

        resolve_with_terminal(&flags, caps).expect("test context must resolve")
    }

    /// Plain-terminal surfaces construct without acquiring an inline session.
    #[test]
    fn test_phase_surfaces_human_constructs_without_terminal_acquisition() {
        let surfaces = select_workflow_phase_surfaces(
            &ctx(ConsumerKind::Human, false),
            RenderOptions::default(),
            None,
            None,
        )
        .expect("human phase surfaces");
        drop(surfaces);
    }

    /// Structured-JSON surfaces construct without acquiring an inline session.
    #[test]
    fn test_phase_surfaces_json_constructs_without_terminal_acquisition() {
        let surfaces = select_workflow_phase_surfaces(
            &ctx(ConsumerKind::Automation, false),
            RenderOptions::default(),
            None,
            None,
        )
        .expect("json phase surfaces");
        drop(surfaces);
    }

    /// Inline progress and interaction share one session; final rendering does not.
    #[test]
    fn test_phase_surfaces_inline_progress_and_interaction_share_one_session() {
        let session = Rc::new(RefCell::new(InlineSession::without_terminal()));
        let surfaces = inline_phase_surfaces_from_session(
            session.clone(),
            PhaseBackend::PlainTerminal,
            RenderOptions::default(),
            None,
            None,
        )
        .expect("inline phase surfaces");
        // test's ref + progress_frontend's clone + interaction's clone.
        // The human `final_frontend` holds no session reference.
        assert_eq!(Rc::strong_count(&session), 3);
        drop(surfaces);
        assert_eq!(Rc::strong_count(&session), 1);
    }

    /// An automation consumer must never allow input — the resolver enforces
    /// this invariant, and the test helper must not fabricate a context that
    /// violates it. Before the fix, `ctx(Automation, false)` hard-coded
    /// `allow_input: true`, which `resolve_frontend_context` never produces.
    #[test]
    fn test_automation_ctx_disallows_input() {
        let automation = ctx(ConsumerKind::Automation, false);
        assert!(
            !automation.interaction.allow_input,
            "automation consumer must not allow input; ctx() must not fabricate an impossible context"
        );
    }

    /// `factory_renders_plain` is the single source of truth for the lock-
    /// reporter gate; it must agree with which backends `frontend_for_surface`
    /// builds a plain frontend for. Plain + fullscreen (degrades to plain) →
    /// true; inline (real TUI) + json → false.
    #[test]
    fn test_factory_renders_plain_truth_table() {
        assert!(factory_renders_plain(PhaseBackend::PlainTerminal));
        assert!(factory_renders_plain(PhaseBackend::FullscreenTerminalUi));
        assert!(!factory_renders_plain(PhaseBackend::InlineTerminalUi));
        assert!(!factory_renders_plain(PhaseBackend::StructuredJson));
    }
}

#[cfg(test)]
mod coerce_tests {
    use super::coerce_to_schema;

    #[test]
    fn test_coerce_inline_string_passthrough() {
        let schema = serde_json::json!({"type": "string"});
        assert_eq!(
            coerce_to_schema("hello", &schema),
            serde_json::Value::String("hello".to_string())
        );
    }

    #[test]
    fn test_coerce_inline_integer_valid() {
        let schema = serde_json::json!({"type": "integer"});
        assert_eq!(coerce_to_schema("42", &schema), serde_json::json!(42i64));
    }

    #[test]
    fn test_coerce_inline_integer_invalid_falls_back_to_string() {
        let schema = serde_json::json!({"type": "integer"});
        assert_eq!(
            coerce_to_schema("abc", &schema),
            serde_json::Value::String("abc".to_string())
        );
    }

    #[test]
    fn test_coerce_inline_number_valid() {
        let schema = serde_json::json!({"type": "number"});
        assert_eq!(coerce_to_schema("2.5", &schema), serde_json::json!(2.5f64));
    }

    #[test]
    fn test_coerce_inline_number_invalid_falls_back_to_string() {
        let schema = serde_json::json!({"type": "number"});
        assert_eq!(
            coerce_to_schema("abc", &schema),
            serde_json::Value::String("abc".to_string())
        );
    }

    #[test]
    fn test_coerce_inline_boolean_true() {
        let schema = serde_json::json!({"type": "boolean"});
        assert_eq!(coerce_to_schema("true", &schema), serde_json::json!(true));
        assert_eq!(coerce_to_schema("True", &schema), serde_json::json!(true));
    }

    #[test]
    fn test_coerce_inline_boolean_false() {
        let schema = serde_json::json!({"type": "boolean"});
        assert_eq!(coerce_to_schema("false", &schema), serde_json::json!(false));
        assert_eq!(coerce_to_schema("FALSE", &schema), serde_json::json!(false));
    }

    #[test]
    fn test_coerce_inline_boolean_invalid_falls_back_to_string() {
        let schema = serde_json::json!({"type": "boolean"});
        assert_eq!(
            coerce_to_schema("yes", &schema),
            serde_json::Value::String("yes".to_string())
        );
    }

    #[test]
    fn test_coerce_inline_array_valid_json() {
        let schema = serde_json::json!({"type": "array"});
        assert_eq!(
            coerce_to_schema("[1,2,3]", &schema),
            serde_json::json!([1, 2, 3])
        );
    }

    #[test]
    fn test_coerce_inline_array_invalid_falls_back_to_string() {
        let schema = serde_json::json!({"type": "array"});
        assert_eq!(
            coerce_to_schema("not json", &schema),
            serde_json::Value::String("not json".to_string())
        );
    }

    #[test]
    fn test_coerce_inline_null_ignores_input() {
        let schema = serde_json::json!({"type": "null"});
        assert_eq!(
            coerce_to_schema("anything", &schema),
            serde_json::Value::Null
        );
    }

    #[test]
    fn test_coerce_inline_unknown_type_falls_back_to_string() {
        let schema = serde_json::json!({"type": "exotic"});
        assert_eq!(
            coerce_to_schema("value", &schema),
            serde_json::Value::String("value".to_string())
        );
    }

    #[test]
    fn test_coerce_inline_absent_type_falls_back_to_string() {
        let schema = serde_json::json!({});
        assert_eq!(
            coerce_to_schema("value", &schema),
            serde_json::Value::String("value".to_string())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{write_stderr_into, write_stdout_line_into};
    use std::io::{self, Write};

    /// A `Write` that records bytes, tracks flush calls, and optionally returns
    /// a configured error from `write`.
    struct Mock {
        buf: Vec<u8>,
        err: Option<io::ErrorKind>,
        flushed: usize,
    }

    impl Mock {
        /// Build a Mock writer that accepts every write — used for the success-path tests.
        fn ok() -> Self {
            Self {
                buf: Vec::new(),
                err: None,
                flushed: 0,
            }
        }

        /// Build a Mock writer that fails every write with the given `io::ErrorKind`.
        fn failing(kind: io::ErrorKind) -> Self {
            Self {
                buf: Vec::new(),
                err: Some(kind),
                flushed: 0,
            }
        }
    }

    impl Write for Mock {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            if let Some(kind) = self.err {
                return Err(io::Error::from(kind));
            }
            self.buf.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushed += 1;
            Ok(())
        }
    }

    #[test]
    fn test_write_stdout_line_into_writes_text_with_trailing_newline() {
        let mut m = Mock::ok();
        write_stdout_line_into(&mut m, "hello").unwrap();
        assert_eq!(m.buf, b"hello\n");
    }

    #[test]
    fn test_write_stdout_line_into_returns_ok_on_broken_pipe() {
        let mut m = Mock::failing(io::ErrorKind::BrokenPipe);
        assert!(write_stdout_line_into(&mut m, "x").is_ok());
    }

    #[test]
    fn test_write_stdout_line_into_returns_clierror_on_other_error() {
        let mut m = Mock::failing(io::ErrorKind::Other);
        let err = write_stdout_line_into(&mut m, "x").unwrap_err();
        match err {
            crate::errors::CliError::Usage { message, .. } => {
                assert!(message.contains("stdout"));
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn test_write_stderr_into_writes_text_without_trailing_newline() {
        let mut m = Mock::ok();
        write_stderr_into(&mut m, "Continue? [y/N] ");
        assert_eq!(m.buf, b"Continue? [y/N] ");
    }

    #[test]
    fn test_write_stderr_into_does_not_panic_on_broken_pipe() {
        let mut m = Mock::failing(io::ErrorKind::BrokenPipe);
        write_stderr_into(&mut m, "x"); // returns ()
    }

    #[test]
    fn test_write_stdout_line_into_flushes_after_write() {
        let mut m = Mock::ok();
        write_stdout_line_into(&mut m, "hello").unwrap();
        assert_eq!(m.flushed, 1);
    }

    #[test]
    fn test_write_stderr_into_flushes_after_write() {
        let mut m = Mock::ok();
        write_stderr_into(&mut m, "Continue? [y/N] ");
        assert_eq!(m.flushed, 1);
    }
}
