//! JSON frontend — machine-readable output.

use crate::errors::CliError;
use crate::frontend::event::FrontendEvent;
use crate::frontend::{Frontend, RenderOptions};
use ags_protocol::output::CommandOutput;

pub struct JsonFrontend {
    options: RenderOptions,
}

impl JsonFrontend {
    /// Build a JSON frontend; progress is suppressed unconditionally for machine consumers.
    pub fn new(options: RenderOptions) -> Self {
        Self { options }
    }
}

impl Frontend for JsonFrontend {
    fn on_event(&mut self, _event: &FrontendEvent) {
        // JSON mode emits no progress indicators.
    }

    fn render(&mut self, output: &CommandOutput) -> Result<(), CliError> {
        let rendered = crate::frontend::output::json::render(output, &self.options)?;
        crate::frontend::emit_with_options(rendered, &self.options)?;
        Ok(())
    }

    fn render_warning(&mut self, _message: &str, _reason: Option<&str>, _tip: Option<&str>) {
        // JSON mode: drop decorative warnings to keep stderr machine-parseable.
    }

    fn render_resolution_trace(&mut self, _trace: &ags_protocol::output::ResolutionTrace) {
        // JSON mode: drop verbose human-formatted trace.
    }

    fn render_error(&mut self, err: &CliError) {
        // Error envelopes go to stderr so stdout stays clean JSON on success.
        // Automation consumers can capture both with `2>&1` or redirect stderr separately.
        let view = err.view();
        let mut envelope = serde_json::json!({
            "error": view.message,
            "exit_code": view.exit_code,
        });
        if let Some(reason) = view.reason {
            envelope["reason"] = serde_json::Value::String(reason);
        }
        if let Some(detail) = view.detail {
            envelope["detail"] = serde_json::Value::String(detail);
        }
        if let Some(suggestion) = view.suggestion {
            envelope["suggestion"] = serde_json::Value::String(suggestion);
        }
        if let Some(tip) = view.tip {
            envelope["tip"] = serde_json::Value::String(tip);
        }
        match crate::frontend::output::json::format_json(&envelope) {
            Ok(text) => crate::frontend::write_stderr_line(&text),
            Err(_) => crate::frontend::write_stderr_line(&format!(
                "{{\"error\":\"internal: failed to serialize error envelope\",\"exit_code\":{}}}",
                view.exit_code,
            )),
        }
    }

    fn finish(self: Box<Self>) -> Result<(), CliError> {
        Ok(())
    }
}
