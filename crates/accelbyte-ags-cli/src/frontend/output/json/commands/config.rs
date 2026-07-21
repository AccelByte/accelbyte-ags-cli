//! JSON rendering for config command output.

use crate::errors::CliError;
use crate::frontend::output::templates::config_source_label;
use crate::frontend::RenderOptions;
use crate::frontend::RenderedOutput;
use ags_protocol::output::{ConfigOutput, ConfigView};

/// Render config command output as JSON
pub(crate) fn render_config_output(
    output: &ConfigOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    render_config_view_json(&output.view)
}

/// Render config output as JSON
fn render_config_view_json(view: &ConfigView) -> Result<RenderedOutput, CliError> {
    let value = match view {
        ConfigView::GetAll { profile, entries } => {
            let items: Vec<serde_json::Value> = entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "key": e.key,
                        "value": e.value,
                        "source": config_source_label(&e.source),
                        "read_only": e.read_only,
                    })
                })
                .collect();
            serde_json::json!({
                "profile": profile,
                "config": items,
            })
        }
        ConfigView::GetOne {
            key,
            value,
            source,
            read_only,
        } => {
            serde_json::json!({
                "key": key,
                "value": value,
                "source": config_source_label(source),
                "read_only": read_only,
            })
        }
        ConfigView::Set { key, value } => {
            serde_json::json!({ "status": "set", "key": key, "value": value })
        }
        ConfigView::Unset { key } => {
            serde_json::json!({ "status": "unset", "key": key })
        }
    };

    Ok(RenderedOutput {
        stdout: Some(crate::frontend::output::json::format_json(&value)?),
        stderr: None,
        is_stdout_first: false,
    })
}
