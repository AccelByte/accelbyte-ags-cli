//! `InlineFrontend` — inline TUI driven by `on_event`. No `unsafe`.

use std::cell::RefCell;
use std::rc::Rc;

use ags_protocol::event::ProgressEvent;
use ags_protocol::output::CommandOutput;
use ags_runtime::support::strings::strip_terminal_control_sequences;

use crate::errors::CliError;
use crate::frontend::event::{FrontendEvent, StepOutcome};
use crate::frontend::style::text;
use crate::frontend::terminal::inline::progress_state::{redraw, InlineProgressState};
use crate::frontend::terminal::inline::session::InlineSession;
use crate::frontend::{Frontend, RenderFormat, RenderOptions};

pub struct InlineFrontend {
    options: RenderOptions,
    progress: InlineProgressState,
    /// Shared terminal session — see [`InlineSession`] for the borrow-safety invariant.
    session: Rc<RefCell<InlineSession>>,
    pending_scrollback: Option<crate::frontend::RenderedOutput>,
    /// Resolution traces emitted while the inline viewport is active. The TUI
    /// holds them until `finish` so mid-flow stderr writes do not corrupt the
    /// inline frame, then flushes them in order alongside the scrollback.
    pending_traces: Vec<String>,
    /// Workflow lifecycle lines (RunStarted banner + per-step summaries)
    /// buffered while the inline viewport is active. Flushed to stderr by
    /// `finish` before `pending_traces` and `pending_scrollback`.
    workflow_scrollback: Vec<String>,
}

impl InlineFrontend {
    /// Acquire the terminal and enable raw mode. Returns `Err` if the
    /// terminal cannot be installed (raw mode failure, no TTY, etc.).
    pub fn new(options: RenderOptions) -> Result<Self, CliError> {
        let session = Rc::new(RefCell::new(InlineSession::new()?));
        Ok(Self::from_session(session, options))
    }

    /// Test seam mirroring [`new`](Self::new) with the stderr-TTY predicate
    /// injected, so the no-TTY refusal can be asserted deterministically
    /// regardless of the test process's real stderr. A `false` predicate
    /// short-circuits before any real terminal acquisition.
    #[cfg(test)]
    fn new_with_stderr_tty(
        options: RenderOptions,
        is_stderr_tty: impl FnOnce() -> bool,
    ) -> Result<Self, CliError> {
        let session = Rc::new(RefCell::new(InlineSession::acquire_if_stderr_tty(
            is_stderr_tty,
        )?));
        Ok(Self::from_session(session, options))
    }

    /// Construct from an already-acquired session. The workflow-surface
    /// factory calls this — via `inline_phase_surfaces_from_session` — to share
    /// one session between `InlineFrontend` and `InlineInteraction` without
    /// acquiring a second terminal.
    pub fn from_session(session: Rc<RefCell<InlineSession>>, options: RenderOptions) -> Self {
        Self {
            options,
            progress: InlineProgressState::default(),
            session,
            pending_scrollback: None,
            pending_traces: Vec::new(),
            workflow_scrollback: Vec::new(),
        }
    }
}

impl Drop for InlineFrontend {
    fn drop(&mut self) {
        // Panic-safety fallback. Errors are swallowed; `finish` is the
        // path that surfaces them.
        self.session.borrow_mut().teardown();
    }
}

impl Frontend for InlineFrontend {
    fn on_event(&mut self, event: &FrontendEvent) {
        match event {
            FrontendEvent::RunStarted { workflow_banner } => {
                if let Some(name) = workflow_banner {
                    self.workflow_scrollback
                        .push(format!("{} Running workflow: {name}", text::SYMBOL_INFO,));
                }
            }
            FrontendEvent::RunFinished { outcome } => {
                // Record the outcome so the next draw (or scrollback) reflects it.
                self.progress.finish_with(*outcome);
            }
            FrontendEvent::Progress {
                event: progress_event,
                ..
            } => {
                if self.options.verbosity.is_quiet() {
                    // `--quiet`: suppress all inline progress chrome. The result
                    // phase and scrollback artifact are unaffected.
                    return;
                }
                match progress_event {
                    ProgressEvent::Started { message } => {
                        self.progress.set_message(message.clone())
                    }
                    ProgressEvent::Message { text } => self.progress.set_message(text.clone()),
                    ProgressEvent::Page { current, total } => {
                        self.progress.set_page(*current, *total)
                    }
                    ProgressEvent::Finished => self.progress.finish(),
                }
                let snapshot = self.progress.snapshot();
                let mut session = self.session.borrow_mut();
                if let Ok(terminal) = session.terminal_mut() {
                    redraw(terminal, &snapshot);
                }
            }
            FrontendEvent::StepStarted { index, id } => {
                // Carry the step's frame name into the progress viewport so the
                // running box keeps "Step N: <id>" (matching the review frame)
                // instead of flipping to "Running". The verb still flows through
                // ProgressEvent::Started, which triggers the redraw.
                self.progress
                    .set_step_label(Some(format!("Step {}: {id}", index + 1)));
            }
            FrontendEvent::StepFinished {
                index,
                summary,
                outcome,
                ..
            } => {
                // Clear the step's frame name now the step is done, symmetric with
                // StepStarted. The running box only reads step_label while outcome
                // is None, so a stale label is currently harmless — but clearing it
                // makes the state self-documenting and safe if the progress state is
                // ever reused across steps/runs.
                self.progress.set_step_label(None);
                // Uniform per-outcome line. `summary` already carries
                // "<id> <status>", and the whole line is coloured by outcome to
                // match the fullscreen strip: green ok, red failed, dim skipped.
                let (glyph, paint): (&str, fn(&str, bool) -> String) = match outcome {
                    StepOutcome::Success => {
                        (text::SYMBOL_SUCCESS, crate::frontend::style::ansi::green)
                    }
                    StepOutcome::Failed => (text::SYMBOL_ERROR, crate::frontend::style::ansi::red),
                    StepOutcome::Skipped | StepOutcome::Cancelled => {
                        (text::SYMBOL_SKIPPED, crate::frontend::style::ansi::dim)
                    }
                };
                let line = format!("{glyph} Step {}: {summary}", index + 1);
                self.workflow_scrollback
                    .push(paint(&line, crate::frontend::style::is_stderr_enabled()));
            }
        }
    }

    fn render(&mut self, output: &CommandOutput) -> Result<(), CliError> {
        use crate::frontend::terminal::inline::phases::{self, result::ResultPhase};

        // Delegate rendering to the shared `render_output` with the Human format
        // so the displayed text matches the human-frontend scrollback artifact
        // byte-for-byte.
        let rendered = crate::frontend::render_output(RenderFormat::Human, output, &self.options)?;
        // The human renderer emits ANSI escapes for colour. ratatui treats
        // them as literal characters which throws off width and alignment,
        // so strip them for the inline display. The scrollback artifact
        // (pending_scrollback) keeps the escapes so the post-exit emission
        // still shows colour in the user's terminal history.
        let display =
            strip_terminal_control_sequences(&rendered.stdout.clone().unwrap_or_default());

        // Render the structured completion below the result text, mirroring the
        // fullscreen panel, rather than relying on the stdout string.
        let completion = match output {
            CommandOutput::Workflow { completion, .. } => completion.clone(),
            _ => None,
        };

        let mut session = self.session.borrow_mut();
        let terminal = session.terminal_mut()?;
        let phase = ResultPhase::new(&display).with_completion(completion);
        let _ = phases::run(terminal, phase)?;

        drop(session);
        self.pending_scrollback = Some(rendered);
        Ok(())
    }

    fn render_error(&mut self, err: &CliError) {
        // Tear down the inline region first so the error lands on a clean stderr line.
        self.session.borrow_mut().teardown();
        let mut human =
            crate::frontend::terminal::plain::frontend::PlainFrontend::new(self.options.clone());
        human.render_error(err);
    }

    fn render_warning(&mut self, message: &str, reason: Option<&str>, tip: Option<&str>) {
        let mut human =
            crate::frontend::terminal::plain::frontend::PlainFrontend::new(self.options.clone());
        human.render_warning(message, reason, tip);
    }

    fn render_resolution_trace(&mut self, trace: &ags_protocol::output::ResolutionTrace) {
        // Buffer the trace instead of writing to stderr mid-flow: while the
        // inline viewport is active, stray stderr output corrupts the frame.
        // `finish` flushes the buffer once the terminal is back in cooked mode.
        let text =
            crate::frontend::output::human::commands::service::render_resolution_trace(trace);
        self.pending_traces.push(text);
    }

    fn finish(mut self: Box<Self>) -> Result<(), CliError> {
        self.session.borrow_mut().teardown();
        // Workflow lifecycle lines come first so the user sees the step
        // summary before the verbose resolution traces.
        for line in self.workflow_scrollback.drain(..) {
            crate::frontend::write_stderr_line(&line);
        }
        // Flush buffered resolution traces next (they precede the result text,
        // matching the human frontend's stderr-then-stdout default ordering).
        for trace in self.pending_traces.drain(..) {
            crate::frontend::write_stderr_line(&trace);
        }
        if let Some(rendered) = self.pending_scrollback.take() {
            crate::frontend::emit_with_options(rendered, &self.options)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod drop_tests {
    use super::*;

    /// The inline backend renders on stderr, so construction must refuse when
    /// stderr is not a TTY. Forcing the real stderr to be a non-TTY under
    /// `cargo test` is unreliable (it inherits the terminal), so the predicate
    /// is injected via the `new_with_stderr_tty` seam — a `false` predicate
    /// short-circuits before any terminal acquisition, making the refusal
    /// deterministic in any environment.
    #[test]
    fn test_inline_frontend_new_errors_when_stderr_is_not_a_tty() {
        let result = InlineFrontend::new_with_stderr_tty(RenderOptions::default(), || false);
        assert!(
            matches!(result, Err(CliError::Usage { .. })),
            "expected a Usage error when stderr is not a TTY"
        );
    }

    /// `from_session` constructs a `InlineFrontend` from a shared session
    /// without acquiring a new terminal. The resulting frontend holds the
    /// same session as the caller.
    #[test]
    fn test_from_session_shares_session() {
        use crate::frontend::terminal::inline::session::InlineSession;
        use std::cell::RefCell;
        use std::rc::Rc;

        let session = Rc::new(RefCell::new(InlineSession::without_terminal()));
        let _frontend = InlineFrontend::from_session(Rc::clone(&session), RenderOptions::default());
        // Both the original Rc and the one held by _frontend point to the
        // same allocation: strong_count == 2.
        assert_eq!(Rc::strong_count(&session), 2);
    }
}
