//! `FullscreenFrontend` — alt-screen workflow surface implementing the
//! `Frontend` trait for the fullscreen workflow surface.
//!
//! Holds a shared [`FullscreenSurface`] (terminal + render model) plus
//! output-only buffered state (deferred scrollback + resolution traces).
//! Its `Frontend` methods mutate surface state then redraw through the
//! surface's single [`render`](FullscreenSurface::render) — the identical
//! path [`FullscreenInteraction`] draws gather/confirm through, so every
//! phase keeps the four-region layout. `finish` runs the dismiss loop
//! (q/Enter to exit, ↑/↓ to scroll the summary), tears down the alt screen,
//! emits any captured failure summary on stderr, and flushes pending stdout
//! per the TTY-aware rule.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use ags_protocol::output::CommandOutput;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

use crate::errors::CliError;
use crate::frontend::event::{FrontendEvent, RunOutcome, StepOutcome};
use crate::frontend::terminal::fullscreen::surface::{FailureSummary, FullscreenSurface};
use crate::frontend::{Frontend, RenderFormat, RenderOptions};

use super::phases::{self, Phase};
use super::step_strip::{Step, StepRowKind, StepState};
use super::summary::SummaryEntry;

pub struct FullscreenFrontend {
    surface: Rc<RefCell<FullscreenSurface>>,
    /// Set once the step-failure pointer has been emitted (by `render_error`,
    /// ahead of the error block) so `finish` does not emit it a second time.
    failure_summary_emitted: bool,
    /// When stdout is a TTY, `render()` buffers the rendered
    /// output and `finish()`'s dismiss-teardown flushes it via
    /// `emit_with_options`; when stdout is piped/file, render() emits
    /// immediately so the consumer doesn't wait for the user to press q.
    pending_scrollback: Option<crate::frontend::RenderedOutput>,
    /// Resolution traces emitted while the surface is up; flushed in
    /// `finish` after teardown so stderr writes don't corrupt the frame.
    pending_traces: Vec<String>,
}

/// What the dismiss-loop driver should do after applying a key.
#[derive(Debug, PartialEq, Eq)]
pub enum DismissStep {
    /// Re-render and keep waiting.
    Continue,
    /// User pressed q/Q/Enter — tear down the surface.
    Exit,
}

/// Whether `finish` should run the dismiss loop. It runs only when
/// stdout is a TTY (piped stdout already shipped its result) AND the alt-screen
/// surface is still live. `render_error` tears the surface down to print a plain
/// error, leaving it inactive, so the loop is skipped — otherwise `finish` would
/// block on input against an already-restored terminal.
fn dismiss_loop_should_run(stdout_is_tty: bool, surface_active: bool) -> bool {
    stdout_is_tty && surface_active
}

impl FullscreenFrontend {
    /// Acquire the alt-screen terminal and return a frontend ready to
    /// drive a workflow run. Errors if the terminal cannot be acquired
    /// (raw-mode failure, no TTY, etc.).
    pub fn new(
        options: RenderOptions,
        title: impl Into<String>,
        steps: Vec<Step>,
        stdout_is_tty: bool,
    ) -> Result<Self, CliError> {
        let surface = Rc::new(RefCell::new(FullscreenSurface::new(
            options,
            title,
            steps,
            stdout_is_tty,
        )?));
        Ok(Self::from_surface(surface))
    }

    /// Construct from an already-built shared surface. The workflow factory
    /// calls this so the frontend and the interaction share one surface;
    /// tests use it with `FullscreenSurface::without_terminal()`.
    pub fn from_surface(surface: Rc<RefCell<FullscreenSurface>>) -> Self {
        Self {
            surface,
            pending_scrollback: None,
            pending_traces: Vec::new(),
            failure_summary_emitted: false,
        }
    }

    /// Format the failure summary as the one-line stderr pointer emitted on
    /// teardown: the light cross, the step position, and the step title, with
    /// the whole line in red. The reason and detail are not repeated here — the
    /// full error block is rendered separately after the alt screen closes.
    /// `None` for successful runs.
    pub fn format_failure_summary(&self, color_enabled: bool) -> Option<String> {
        let surface = self.surface.borrow();
        let f = surface.last_failure.as_ref()?;
        let line = format!(
            "{} Step {}: {} failed",
            crate::frontend::style::text::SYMBOL_ERROR,
            f.step_index + 1,
            f.step_title,
        );
        Some(crate::frontend::style::ansi::red(&line, color_enabled))
    }

    /// Write the failure summary through `UiSink` (stderr).
    /// Returns `true` if a summary was emitted; best-effort I/O.
    pub fn emit_failure_summary(&self) -> bool {
        match self.format_failure_summary(crate::frontend::style::is_stderr_enabled()) {
            Some(text) => {
                let _ = crate::frontend::streams::UiSink.write_line(&text);
                true
            }
            None => false,
        }
    }

    /// Apply one dismiss-loop key event. q/Q/Enter exits; ↑/↓ scrolls
    /// the Result panel (saturating at zero; the render-time clamp caps the
    /// upper bound so we don't need to bound it here).
    pub fn dismiss_step(&mut self, key: KeyEvent) -> DismissStep {
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Enter => DismissStep::Exit,
            KeyCode::Up => {
                let mut surface = self.surface.borrow_mut();
                surface.result_scroll = surface.result_scroll.saturating_sub(1);
                DismissStep::Continue
            }
            KeyCode::Down => {
                let mut surface = self.surface.borrow_mut();
                surface.result_scroll = surface.result_scroll.saturating_add(1);
                DismissStep::Continue
            }
            KeyCode::PageUp => {
                let mut surface = self.surface.borrow_mut();
                surface.summary_scroll = surface.summary_scroll.saturating_sub(1);
                DismissStep::Continue
            }
            KeyCode::PageDown => {
                let mut surface = self.surface.borrow_mut();
                surface.summary_scroll = surface.summary_scroll.saturating_add(1);
                DismissStep::Continue
            }
            _ => DismissStep::Continue,
        }
    }

    /// Transition the main area to the [`Result`](Phase::Result) panel.
    pub fn transition_to_result(
        &mut self,
        body: impl Into<String>,
        completion: Option<ags_protocol::output_views::WorkflowCompletionView>,
    ) {
        let mut surface = self.surface.borrow_mut();
        let title = format!("{} complete", surface.title);
        surface.current_phase = Phase::Result(phases::result::ResultPanel {
            title,
            description: String::new(),
            body: body.into(),
            completion,
            status: phases::result::ResultStatus::Success,
        });
    }

    /// Redraw the live terminal once, swallowing errors so a draw failure
    /// during event handling doesn't kill the run. The dismiss loop reports
    /// its own draw errors via [`run_dismiss_loop`].
    fn redraw(&mut self) {
        let _ = self.surface.borrow_mut().render();
    }

    /// Poll for key events and apply them via [`dismiss_step`] until
    /// the user exits. Surfaces draw failures as `CliError::Usage`.
    fn run_dismiss_loop(&mut self) -> Result<(), CliError> {
        debug_assert!(
            self.surface.borrow().is_active(),
            "dismiss loop must not run on a torn-down surface"
        );
        loop {
            self.surface.borrow_mut().render()?;
            if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                if let Ok(Event::Key(key)) = event::read() {
                    if key.kind != crossterm::event::KeyEventKind::Press {
                        continue;
                    }
                    // Raw mode swallows SIGINT; intercept Ctrl-C explicitly so
                    // it exits cleanly rather than hanging.
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        return Ok(());
                    }
                    if self.dismiss_step(key) == DismissStep::Exit {
                        return Ok(());
                    }
                }
            }
        }
    }
}

impl Drop for FullscreenFrontend {
    fn drop(&mut self) {
        // Panic-safety fallback; the normal teardown happens in `finish`.
        self.surface.borrow_mut().teardown();
    }
}

impl Frontend for FullscreenFrontend {
    fn on_event(&mut self, event: &FrontendEvent) {
        {
            let mut surface = self.surface.borrow_mut();
            match event {
                FrontendEvent::RunStarted { workflow_banner } => {
                    if let Some(name) = workflow_banner {
                        surface.title = name.clone();
                    }
                    surface.status = "running".into();
                }
                FrontendEvent::StepStarted { index, .. } => {
                    if let Some(step) = surface.steps.iter_mut().find(|s| {
                        s.kind
                            == StepRowKind::Workflow {
                                runtime_index: *index,
                            }
                    }) {
                        step.state = StepState::Current;
                    }
                }
                FrontendEvent::StepFinished {
                    index,
                    outcome,
                    summary,
                    captures,
                    ..
                } => {
                    let state = match outcome {
                        StepOutcome::Success => StepState::Complete,
                        StepOutcome::Failed => StepState::Failed,
                        StepOutcome::Cancelled => StepState::Skipped,
                        StepOutcome::Skipped => StepState::Skipped,
                    };
                    let title = surface
                        .steps
                        .iter()
                        .find(|s| {
                            s.kind
                                == StepRowKind::Workflow {
                                    runtime_index: *index,
                                }
                        })
                        .map(|s| s.title.clone())
                        .unwrap_or_else(|| format!("step {}", index + 1));
                    if let Some(step) = surface.steps.iter_mut().find(|s| {
                        s.kind
                            == StepRowKind::Workflow {
                                runtime_index: *index,
                            }
                    }) {
                        step.state = state;
                    }
                    // On success we surface the resolved inputs/options the
                    // step consumed; on failure/cancellation captures arrive
                    // empty and we fall back to the one-line summary so the
                    // Summary panel still says something useful.
                    let entry_captures = if captures.is_empty() {
                        vec![("summary".into(), summary.clone())]
                    } else {
                        captures.clone()
                    };
                    surface.summary_entries.push(SummaryEntry {
                        title: title.clone(),
                        state,
                        captures: entry_captures,
                        error_line: None,
                    });
                    // Follow the tail: pin the summary to the newest entry so
                    // recent completions stay visible during the run (the render
                    // clamps this to the panel's max offset). Manual PgUp/PgDn in
                    // the Result phase can then scroll back up.
                    surface.summary_scroll = usize::MAX;
                    if matches!(outcome, StepOutcome::Failed) {
                        surface.cleanup_required = true;
                        if surface.last_failure.is_none() {
                            surface.last_failure = Some(FailureSummary {
                                step_index: *index,
                                step_title: title,
                            });
                        }
                    }
                }
                FrontendEvent::RunFinished { outcome } => {
                    surface.status = match outcome {
                        RunOutcome::Success => "complete".into(),
                        RunOutcome::Failed => "failed".into(),
                        RunOutcome::Cancelled => "cancelled".into(),
                    };
                    // The run is over, so no step is "current" any more. Advance
                    // any row still marked Current to the terminal state for the
                    // run outcome. This is what moves a synthesised single
                    // command's `gather-inputs` row from active (cyan ●) to
                    // complete (green ✔): its per-step `StepFinished` event is
                    // suppressed, so `RunFinished` is the only signal the surface
                    // gets. Multi-step workflows resolve every step via
                    // `StepFinished`, so no Current row remains and this is a no-op.
                    let terminal_state = match outcome {
                        RunOutcome::Success => StepState::Complete,
                        RunOutcome::Failed => StepState::Failed,
                        RunOutcome::Cancelled => StepState::Skipped,
                    };
                    for step in surface.steps.iter_mut() {
                        if step.state == StepState::Current {
                            step.state = terminal_state;
                        }
                    }
                    // A cancelled run (e.g. Esc/Ctrl-C at a step review) has no
                    // result or error to render, so the main area would otherwise
                    // keep showing the interaction's restored "Starting" running
                    // placeholder. Replace it with a clear cancelled panel.
                    if matches!(outcome, RunOutcome::Cancelled) {
                        let title = format!("{} cancelled", surface.title);
                        surface.current_phase = Phase::Result(phases::result::ResultPanel {
                            title,
                            description:
                                "Run cancelled — no changes were dispatched for the current step."
                                    .into(),
                            body: String::new(),
                            completion: None,
                            status: phases::result::ResultStatus::Cancelled,
                        });
                    }
                }
                FrontendEvent::Progress { .. } => {
                    // Per-step verb/elapsed updates land here once the dispatch
                    // progress channel feeds the running panel; for now we just
                    // redraw with whatever the current phase already shows.
                }
            }
        }
        self.redraw();
    }

    fn render(&mut self, output: &CommandOutput) -> Result<(), CliError> {
        let options = self.surface.borrow().options.clone();
        let rendered = crate::frontend::render_output(RenderFormat::Human, output, &options)?;
        // Surface the result in-panel so the user sees it
        // before the dismiss loop hands control back.
        let body = rendered.stdout.clone().unwrap_or_default();
        // Render the structured completion in-panel (not via the stdout string),
        // so the Created / Next steps blocks get proper styling.
        let completion = match output {
            CommandOutput::Workflow { completion, .. } => completion.clone(),
            _ => None,
        };
        // Keep ANSI markers in the stored body — the Result panel uses the
        // dim escape (`\x1b[2m…`) to distinguish backend-styled lines (Query
        // / Headers / Body) from the bright endpoint line, then strips the
        // escapes itself before drawing. Pre-stripping here would erase the
        // signal and force a fragile indentation heuristic.
        self.transition_to_result(&body, completion);
        self.redraw();

        // TTY stdout buffers; piped stdout emits immediately.
        if self.surface.borrow().stdout_is_tty {
            self.pending_scrollback = Some(rendered);
        } else {
            crate::frontend::emit_with_options(rendered, &options)?;
        }
        Ok(())
    }

    fn render_error(&mut self, err: &CliError) {
        // Catastrophic errors tear down the
        // alt screen and land plain on stderr. After this the surface is
        // inactive, so a subsequent `finish()` skips the dismiss loop rather
        // than blocking on input against the already-restored terminal.
        let options = self.surface.borrow().options.clone();
        self.surface.borrow_mut().teardown();
        // Emit the terse step-failure pointer BEFORE the command error block, so
        // the order matches the streaming (plain/inline) surfaces: which step
        // failed, then the error detail. `finish` then skips the pointer.
        if self.emit_failure_summary() {
            self.failure_summary_emitted = true;
        }
        let mut plain = crate::frontend::terminal::plain::frontend::PlainFrontend::new(options);
        plain.render_error(err);
    }

    fn render_warning(&mut self, message: &str, reason: Option<&str>, tip: Option<&str>) {
        let options = self.surface.borrow().options.clone();
        let mut plain = crate::frontend::terminal::plain::frontend::PlainFrontend::new(options);
        plain.render_warning(message, reason, tip);
    }

    fn render_resolution_trace(&mut self, trace: &ags_protocol::output::ResolutionTrace) {
        let text =
            crate::frontend::output::human::commands::service::render_resolution_trace(trace);
        self.pending_traces.push(text);
    }

    fn finish(mut self: Box<Self>) -> Result<(), CliError> {
        // Wait for the user to dismiss the surface (only meaningful when
        // stdout is a TTY — for piped stdout the result already shipped,
        // but we still let the user see the in-surface visualisation). Skip it
        // entirely once the surface has been torn down by `render_error`: the
        // alt screen is gone and the plain error already printed, so re-entering
        // the loop would block on input against the restored terminal.
        let should_dismiss = {
            let surface = self.surface.borrow();
            dismiss_loop_should_run(surface.stdout_is_tty, surface.is_active())
        };
        if should_dismiss {
            self.run_dismiss_loop()?;
        }
        self.surface.borrow_mut().teardown();

        // Emit any captured failure summary plain on stderr, unless render_error
        // already emitted it (ahead of the error block) on the failure path.
        if !self.failure_summary_emitted {
            self.emit_failure_summary();
        }

        for trace in self.pending_traces.drain(..) {
            crate::frontend::write_stderr_line(&trace);
        }

        if let Some(rendered) = self.pending_scrollback.take() {
            let options = self.surface.borrow().options.clone();
            crate::frontend::emit_with_options(rendered, &options)?;
        }
        Ok(())
    }
}

#[cfg(test)]
impl FullscreenFrontend {
    /// Borrow the shared surface for state assertions.
    fn surf(&self) -> std::cell::Ref<'_, FullscreenSurface> {
        self.surface.borrow()
    }

    /// Mutably borrow the shared surface for test setup.
    fn surf_mut(&self) -> std::cell::RefMut<'_, FullscreenSurface> {
        self.surface.borrow_mut()
    }

    /// Captured first-failure context, if any (cloned out of the surface).
    pub fn last_failure(&self) -> Option<FailureSummary> {
        self.surface.borrow().last_failure.clone()
    }

    /// Visible offset into the Result panel controlled by ↑/↓ in the dismiss loop.
    pub fn result_scroll(&self) -> usize {
        self.surface.borrow().result_scroll
    }

    /// Visible offset into `summary_entries` controlled by PgUp/PgDn in the dismiss loop.
    pub fn summary_scroll(&self) -> usize {
        self.surface.borrow().summary_scroll
    }

    /// Transition the main area to the [`Error`](Phase::Error) panel.
    pub fn transition_to_error(
        &mut self,
        message: impl Into<String>,
        context: Option<String>,
        suggestion: Option<String>,
    ) {
        self.surface.borrow_mut().current_phase = Phase::Error(phases::error::ErrorPanel {
            message: message.into(),
            context,
            suggestion,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::RenderOptions;

    /// Build a three-entry step-strip fixture.
    fn three_steps() -> Vec<Step> {
        use super::super::step_strip::StepRowKind;
        vec![
            Step {
                kind: StepRowKind::Workflow { runtime_index: 0 },
                title: "Define".into(),
                state: StepState::Pending,
            },
            Step {
                kind: StepRowKind::Workflow { runtime_index: 1 },
                title: "Build".into(),
                state: StepState::Pending,
            },
            Step {
                kind: StepRowKind::Workflow { runtime_index: 2 },
                title: "Run".into(),
                state: StepState::Pending,
            },
        ]
    }

    /// Build a fullscreen frontend over a fake terminal for tests.
    fn frontend(steps: Vec<Step>, stdout_is_tty: bool) -> FullscreenFrontend {
        let mut surface = FullscreenSurface::without_terminal();
        surface.options = RenderOptions::default();
        surface.title = "ags".into();
        surface.steps = steps;
        surface.stdout_is_tty = stdout_is_tty;
        FullscreenFrontend::from_surface(Rc::new(RefCell::new(surface)))
    }

    /// Build a plain key-press event for `code`.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_dismiss_loop_skipped_after_error_teardown() {
        // Live surface on a TTY: the dismiss loop runs.
        assert!(dismiss_loop_should_run(true, true));
        // `render_error` tore the surface down → inactive: skip the loop so
        // `finish` doesn't block on input after the terminal was restored.
        assert!(!dismiss_loop_should_run(true, false));
        // Piped stdout: the result already shipped, no dismiss loop either way.
        assert!(!dismiss_loop_should_run(false, true));
        assert!(!dismiss_loop_should_run(false, false));
    }

    #[test]
    fn test_render_error_leaves_surface_inactive() {
        // A headless surface starts inactive; render_error must keep it that
        // way (idempotent teardown) so `finish` never re-enters the loop.
        let mut s = frontend(three_steps(), true);
        s.render_error(&CliError::Usage {
            message: "boom".into(),
            metadata: None,
        });
        assert!(!s.surf().is_active());
    }

    #[test]
    fn test_render_error_after_step_failure_sets_dedup_flag() {
        // The failure sequence: StepFinished{Failed} captures a last_failure,
        // then render_error emits the terse pointer ahead of the error block and
        // sets `failure_summary_emitted`. `finish` gates its fallback emit on
        // `!failure_summary_emitted`, so this flag is what stops the pointer
        // printing twice. Guards the exact double-emission the flag exists for.
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::StepFinished {
            index: 1,
            summary: "compiler exit 1".into(),
            captures: Vec::new(),
            outcome: StepOutcome::Failed,
        });
        assert!(!s.failure_summary_emitted, "flag starts unset");
        s.render_error(&CliError::Usage {
            message: "build failed".into(),
            metadata: None,
        });
        assert!(
            s.failure_summary_emitted,
            "render_error must set the dedup flag once it emits the pointer, \
             so finish's fallback emit is skipped"
        );
    }

    #[test]
    fn test_render_error_without_step_failure_leaves_dedup_flag_unset() {
        // A catastrophic error with no captured step failure (e.g. a briefing
        // I/O error before any step ran): there is no pointer to emit, so the
        // flag stays false and `finish` has nothing to skip.
        let mut s = frontend(three_steps(), true);
        s.render_error(&CliError::Usage {
            message: "boom".into(),
            metadata: None,
        });
        assert!(!s.failure_summary_emitted);
    }

    #[test]
    fn test_run_started_sets_status_and_picks_up_workflow_banner() {
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::RunStarted {
            workflow_banner: Some("competitive-multiplayer".into()),
        });
        assert_eq!(s.surf().title, "competitive-multiplayer");
        assert_eq!(s.surf().status, "running");
    }

    #[test]
    fn test_step_started_marks_step_current() {
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::StepStarted {
            index: 1,
            id: "build".into(),
        });
        assert_eq!(s.surf().steps[1].state, StepState::Current);
    }

    #[test]
    fn test_step_started_routes_to_workflow_row_not_inputs() {
        use super::super::step_strip::{StepRowKind, StepState as SS};
        let mut s = frontend(vec![], true);
        s.surf_mut().steps = vec![
            Step {
                kind: StepRowKind::Inputs,
                title: "Inputs".into(),
                state: SS::Complete,
            },
            Step {
                kind: StepRowKind::Workflow { runtime_index: 0 },
                title: "one".into(),
                state: SS::Pending,
            },
            Step {
                kind: StepRowKind::Workflow { runtime_index: 1 },
                title: "two".into(),
                state: SS::Pending,
            },
        ];
        s.on_event(&FrontendEvent::StepStarted {
            index: 0,
            id: "one".into(),
        });
        let surf = s.surf();
        assert_eq!(surf.steps[0].state, SS::Complete, "Inputs row untouched");
        assert_eq!(
            surf.steps[1].state,
            SS::Current,
            "workflow row 0 became current"
        );
    }

    #[test]
    fn test_step_finished_success_appends_summary_and_marks_step_complete() {
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::StepFinished {
            index: 0,
            summary: "Defined skill stat".into(),
            captures: Vec::new(),
            outcome: StepOutcome::Success,
        });
        assert_eq!(s.surf().steps[0].state, StepState::Complete);
        assert_eq!(s.surf().summary_entries.len(), 1);
        assert_eq!(s.surf().summary_entries[0].title, "Define");
        assert!(!s.surf().cleanup_required);
    }

    #[test]
    fn test_step_finished_failed_sets_cleanup_required_flag() {
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::StepFinished {
            index: 0,
            summary: "boom".into(),
            captures: Vec::new(),
            outcome: StepOutcome::Failed,
        });
        assert!(s.surf().cleanup_required);
        assert_eq!(s.surf().steps[0].state, StepState::Failed);
    }

    #[test]
    fn test_first_step_failure_is_captured_in_last_failure() {
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::StepFinished {
            index: 1,
            summary: "compiler exit 1".into(),
            captures: Vec::new(),
            outcome: StepOutcome::Failed,
        });
        let f = s.last_failure().expect("failure captured");
        assert_eq!(f.step_index, 1);
        assert_eq!(f.step_title, "Build");
    }

    #[test]
    fn test_subsequent_failure_does_not_overwrite_first() {
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::StepFinished {
            index: 0,
            summary: "first failure".into(),
            captures: Vec::new(),
            outcome: StepOutcome::Failed,
        });
        s.on_event(&FrontendEvent::StepFinished {
            index: 1,
            summary: "second failure".into(),
            captures: Vec::new(),
            outcome: StepOutcome::Failed,
        });
        // The first failure (step index 0) is retained, not the later one.
        assert_eq!(s.last_failure().unwrap().step_index, 0);
    }

    #[test]
    fn test_success_run_has_no_failure_summary() {
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::StepFinished {
            index: 0,
            summary: "ok".into(),
            captures: Vec::new(),
            outcome: StepOutcome::Success,
        });
        assert!(s.last_failure().is_none());
        assert!(s.format_failure_summary(false).is_none());
        assert!(!s.emit_failure_summary());
    }

    #[test]
    fn test_format_failure_summary_is_terse_step_pointer() {
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::StepFinished {
            index: 2,
            summary: "exit 1".into(),
            captures: Vec::new(),
            outcome: StepOutcome::Failed,
        });
        let text = s.format_failure_summary(false).expect("summary formatted");
        // Terse pointer: light cross, step position, step title, "failed".
        assert_eq!(text, "\u{2715} Step 3: Run failed");
        // The reason lives in the error block, not this pointer.
        assert!(!text.contains("exit 1"));
    }

    #[test]
    fn test_format_failure_summary_colors_whole_line_red_when_enabled() {
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::StepFinished {
            index: 2,
            summary: "exit 1".into(),
            captures: Vec::new(),
            outcome: StepOutcome::Failed,
        });
        let text = s.format_failure_summary(true).expect("summary formatted");
        // The entire pointer line is wrapped in red when stderr color is enabled.
        assert_eq!(text, "\u{1b}[31m\u{2715} Step 3: Run failed\u{1b}[0m");
    }

    #[test]
    fn test_run_finished_updates_status_to_outcome_label() {
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::RunFinished {
            outcome: RunOutcome::Success,
        });
        assert_eq!(s.surf().status, "complete");
    }

    #[test]
    fn test_run_finished_failed_status() {
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::RunFinished {
            outcome: RunOutcome::Failed,
        });
        assert_eq!(s.surf().status, "failed");
    }

    #[test]
    fn test_run_finished_advances_current_step_to_outcome_state() {
        // A synthesised single command's `gather-inputs` row starts Current and
        // its StepFinished is suppressed, so RunFinished is the only signal that
        // moves it to its terminal state. Success → Complete (green ✔).
        let mut s = frontend(vec![], true);
        s.surf_mut().steps = vec![Step {
            kind: StepRowKind::Workflow { runtime_index: 0 },
            title: "gather-inputs".into(),
            state: StepState::Current,
        }];
        s.on_event(&FrontendEvent::RunFinished {
            outcome: RunOutcome::Success,
        });
        assert_eq!(s.surf().steps[0].state, StepState::Complete);

        // Failure → Failed (red ✕).
        let mut s = frontend(vec![], true);
        s.surf_mut().steps = vec![Step {
            kind: StepRowKind::Workflow { runtime_index: 0 },
            title: "gather-inputs".into(),
            state: StepState::Current,
        }];
        s.on_event(&FrontendEvent::RunFinished {
            outcome: RunOutcome::Failed,
        });
        assert_eq!(s.surf().steps[0].state, StepState::Failed);
    }

    #[test]
    fn test_run_finished_leaves_pending_steps_untouched() {
        // Only Current rows advance; a Pending step that never ran stays Pending.
        let mut s = frontend(three_steps(), true);
        s.surf_mut().steps[0].state = StepState::Current;
        s.on_event(&FrontendEvent::RunFinished {
            outcome: RunOutcome::Success,
        });
        let surf = s.surf();
        assert_eq!(
            surf.steps[0].state,
            StepState::Complete,
            "current → complete"
        );
        assert_eq!(surf.steps[1].state, StepState::Pending, "pending untouched");
        assert_eq!(surf.steps[2].state, StepState::Pending, "pending untouched");
    }

    #[test]
    fn test_transition_to_result_swaps_main_phase() {
        let mut s = frontend(three_steps(), true);
        s.transition_to_result("{\"id\":\"x\"}", None);
        assert!(matches!(s.surf().current_phase, Phase::Result(_)));
    }

    #[test]
    fn test_transition_to_error_swaps_main_phase() {
        let mut s = frontend(three_steps(), true);
        s.transition_to_error("oops", Some("cause".into()), Some("retry".into()));
        assert!(matches!(s.surf().current_phase, Phase::Error(_)));
    }

    #[test]
    fn test_dismiss_step_q_exits() {
        let mut s = frontend(three_steps(), true);
        assert_eq!(s.dismiss_step(key(KeyCode::Char('q'))), DismissStep::Exit);
    }

    #[test]
    fn test_dismiss_step_enter_exits() {
        let mut s = frontend(three_steps(), true);
        assert_eq!(s.dismiss_step(key(KeyCode::Enter)), DismissStep::Exit);
    }

    #[test]
    fn test_dismiss_step_down_advances_result_scroll() {
        let mut s = frontend(three_steps(), true);
        s.dismiss_step(key(KeyCode::Down));
        assert_eq!(s.result_scroll(), 1);
    }

    #[test]
    fn test_dismiss_step_down_accumulates_result_scroll() {
        let mut s = frontend(three_steps(), true);
        s.dismiss_step(key(KeyCode::Down));
        s.dismiss_step(key(KeyCode::Down));
        assert_eq!(s.result_scroll(), 2);
    }

    #[test]
    fn test_dismiss_step_up_does_not_underflow_below_zero() {
        let mut s = frontend(three_steps(), true);
        s.dismiss_step(key(KeyCode::Up));
        assert_eq!(s.result_scroll(), 0);
    }

    #[test]
    fn test_dismiss_step_other_keys_continue_without_action() {
        let mut s = frontend(three_steps(), true);
        assert_eq!(
            s.dismiss_step(key(KeyCode::Char('x'))),
            DismissStep::Continue
        );
        assert_eq!(s.result_scroll(), 0);
    }

    #[test]
    fn test_dismiss_step_page_down_advances_summary_scroll() {
        let mut s = frontend(three_steps(), true);
        s.dismiss_step(key(KeyCode::PageDown));
        assert_eq!(s.summary_scroll(), 1);
    }

    #[test]
    fn test_dismiss_step_page_up_does_not_underflow() {
        let mut s = frontend(three_steps(), true);
        s.dismiss_step(key(KeyCode::PageUp));
        assert_eq!(s.summary_scroll(), 0);
    }

    #[test]
    fn test_dismiss_step_page_down_accumulates() {
        let mut s = frontend(three_steps(), true);
        s.dismiss_step(key(KeyCode::PageDown));
        s.dismiss_step(key(KeyCode::PageDown));
        assert_eq!(s.summary_scroll(), 2);
    }

    #[test]
    fn test_step_finished_pins_summary_scroll_to_tail() {
        // A completed step pins the summary to the tail sentinel so recent entries
        // stay visible during the run (render clamps the sentinel to the panel max).
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::StepFinished {
            index: 0,
            summary: "Defined skill stat".into(),
            captures: Vec::new(),
            outcome: StepOutcome::Success,
        });
        assert_eq!(s.summary_scroll(), usize::MAX);
    }

    #[test]
    fn test_redraw_without_terminal_is_noop() {
        // No live terminal → render() errors, but redraw swallows it.
        let mut s = frontend(three_steps(), true);
        s.on_event(&FrontendEvent::RunStarted {
            workflow_banner: Some("test".into()),
        });
        s.redraw();
    }
}
