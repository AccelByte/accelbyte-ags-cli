//! Human-readable frontend — stdout/stderr with ANSI styling.

use crate::errors::CliError;
use crate::frontend::human::progress::StatusLineSink;
use crate::frontend::{Frontend, RenderOptions};
use crate::protocol::catalogue::OperationSchema;
use crate::protocol::event::ProgressSink;
use crate::protocol::output::CommandOutput;
use crate::protocol::result::CommandPreview;

pub struct HumanFrontend {
    sink: StatusLineSink,
}

impl HumanFrontend {
    /// Build a human frontend with progress output suppressed when verbosity is `Quiet`.
    pub fn new(verbosity: crate::protocol::request::Verbosity) -> Self {
        Self {
            sink: StatusLineSink::new(verbosity.is_quiet()),
        }
    }
}

impl Frontend for HumanFrontend {
    fn render(&mut self, output: &CommandOutput, options: &RenderOptions) -> Result<(), CliError> {
        let rendered = crate::frontend::human::render(output, options)?;
        crate::frontend::emit_with_options(rendered, options)?;
        Ok(())
    }

    fn render_warning(&mut self, message: &str, reason: Option<&str>, tip: Option<&str>) {
        crate::frontend::write_stderr_line(
            &crate::frontend::human::templates::render_warning_text(
                message,
                reason,
                tip,
                crate::frontend::style::is_stderr_enabled(),
            ),
        );
    }

    fn render_resolution_trace(&mut self, trace: &crate::protocol::output::ResolutionTrace) {
        crate::frontend::write_stderr_line(
            &crate::frontend::human::commands::service::render_resolution_trace(trace),
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
                &crate::frontend::human::commands::service::render_execution_trace_string(trace),
            );
        }
        let rendered = crate::frontend::human::templates::render_error_text(
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

    fn confirm(&mut self, preview: &CommandPreview) -> Result<bool, CliError> {
        crate::frontend::human::prompt::confirm(preview)
    }

    fn progress_sink(&mut self) -> &mut dyn ProgressSink {
        &mut self.sink
    }

    fn gather_body(&mut self, _operation: &OperationSchema) -> Result<serde_json::Value, CliError> {
        Err(CliError::Usage {
            message: "Interactive body input is not yet supported".to_string(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Pass the request body via --body '<json>'",
            ))),
        })
    }
}
