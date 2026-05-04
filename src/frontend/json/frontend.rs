//! JSON frontend — machine-readable output.

use crate::errors::CliError;
use crate::frontend::json::progress::NoopProgressSink;
use crate::frontend::{Frontend, RenderOptions};
use crate::protocol::catalogue::OperationSchema;
use crate::protocol::event::ProgressSink;
use crate::protocol::output::CommandOutput;
use crate::protocol::result::CommandPreview;

#[derive(Default)]
pub struct JsonFrontend {
    sink: NoopProgressSink,
}

impl JsonFrontend {
    /// Build a JSON frontend; progress is suppressed unconditionally for machine consumers.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Frontend for JsonFrontend {
    fn render(&mut self, output: &CommandOutput, options: &RenderOptions) -> Result<(), CliError> {
        let rendered = crate::frontend::json::render(output, options)?;
        crate::frontend::emit_with_options(rendered, options)?;
        Ok(())
    }

    fn render_warning(&mut self, _message: &str, _reason: Option<&str>, _tip: Option<&str>) {
        // JSON mode: drop decorative warnings to keep stderr machine-parseable.
    }

    fn render_resolution_trace(&mut self, _trace: &crate::protocol::output::ResolutionTrace) {
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
        match crate::frontend::json::format_json(&envelope) {
            Ok(text) => crate::frontend::write_stderr_line(&text),
            Err(_) => crate::frontend::write_stderr_line(&format!(
                "{{\"error\":\"internal: failed to serialize error envelope\",\"exit_code\":{}}}",
                view.exit_code,
            )),
        }
    }

    fn confirm(&mut self, _preview: &CommandPreview) -> Result<bool, CliError> {
        Err(CliError::Usage {
            message: "This operation requires confirmation. Pass --yes to proceed without a prompt"
                .to_string(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Pass --yes to auto-confirm",
            ))),
        })
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
