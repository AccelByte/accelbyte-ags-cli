//! Human-readable frontend — stdout/stderr with ANSI styling.

use crate::errors::CliError;
use crate::frontend::event::{FrontendEvent, StepOutcome};
use crate::frontend::style::text;
use crate::frontend::terminal::plain::progress::StatusLine;
use crate::frontend::{Frontend, RenderOptions};
use ags_protocol::event::ProgressEvent;
use ags_protocol::output::CommandOutput;

pub struct PlainFrontend {
    options: RenderOptions,
    status: StatusLine,
}

impl PlainFrontend {
    /// Build a human frontend with progress output suppressed when verbosity is `Quiet`.
    pub fn new(options: RenderOptions) -> Self {
        let quiet = options.verbosity.is_quiet();
        Self {
            options,
            status: StatusLine::new(quiet),
        }
    }
}

impl Frontend for PlainFrontend {
    fn on_event(&mut self, event: &FrontendEvent) {
        match event {
            FrontendEvent::RunStarted { workflow_banner } => {
                if let Some(name) = workflow_banner {
                    self.status.clear();
                    crate::frontend::write_stderr_line(&format!(
                        "{} Running workflow: {name}",
                        text::SYMBOL_INFO,
                    ));
                }
            }
            FrontendEvent::RunFinished { .. } => {
                // Clear any outstanding status/spinner state at terminal
                // completion, for every run type.
                self.status.clear();
            }
            FrontendEvent::Progress { event: p, .. } => match p {
                ProgressEvent::Started { message } => self.status.show(message),
                ProgressEvent::Page { current, total } => {
                    let text = match total {
                        Some(t) => format!("Fetching page {current}/{t}..."),
                        None => format!("Fetching page {current}..."),
                    };
                    self.status.update(&text);
                }
                ProgressEvent::Message { text } => {
                    self.status.clear();
                    crate::frontend::write_stderr_line(text);
                }
                ProgressEvent::Finished => self.status.clear(),
            },
            FrontendEvent::StepStarted { index, id } => {
                let msg = format!("Step {} ({})", index + 1, id);
                self.status.show(&msg);
            }
            FrontendEvent::StepFinished {
                index,
                summary,
                outcome,
                ..
            } => {
                self.status.clear();
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
                crate::frontend::write_stderr_line(&paint(
                    &line,
                    crate::frontend::style::is_stderr_enabled(),
                ));
            }
        }
    }

    fn render(&mut self, output: &CommandOutput) -> Result<(), CliError> {
        let rendered = crate::frontend::output::human::render(output, &self.options)?;
        crate::frontend::emit_with_options(rendered, &self.options)?;
        Ok(())
    }

    fn render_warning(&mut self, message: &str, reason: Option<&str>, tip: Option<&str>) {
        crate::frontend::write_stderr_line(
            &crate::frontend::output::human::templates::render_warning_text(
                message,
                reason,
                tip,
                crate::frontend::style::is_stderr_enabled(),
            ),
        );
    }

    fn render_resolution_trace(&mut self, trace: &ags_protocol::output::ResolutionTrace) {
        crate::frontend::write_stderr_line(
            &crate::frontend::output::human::commands::service::render_resolution_trace(trace),
        );
    }

    fn render_error(&mut self, err: &CliError) {
        let view = err.view();
        let color_enabled = crate::frontend::style::is_stderr_enabled();
        // When `--verbose` was set, render the request/response trace before
        // the error itself so the user can see what URL was tried and what
        // the server returned. Without this, verbose is silent on failures.
        if let Some(trace) = view.trace.as_deref() {
            crate::frontend::write_stderr_line(
                &crate::frontend::output::human::commands::service::render_execution_trace_string(
                    trace,
                ),
            );
        }
        let rendered = crate::frontend::output::human::templates::render_error_text(
            &view.message,
            view.reason.as_deref(),
            view.detail.as_deref(),
            view.suggestion.as_deref(),
            view.suggestion_kind,
            view.tip.as_deref(),
            color_enabled,
        );
        crate::frontend::write_stderr_line(&rendered);
    }

    fn finish(self: Box<Self>) -> Result<(), CliError> {
        // StatusLine's Drop clears the spinner; nothing else to tear down.
        Ok(())
    }
}
