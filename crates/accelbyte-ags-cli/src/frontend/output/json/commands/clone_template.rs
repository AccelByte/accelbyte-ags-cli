//! JSON renderer for `ags extend clone-template` output.

use crate::errors::CliError;
use crate::frontend::output::json::format_json;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::CloneTemplateOutput;

pub(crate) fn render_clone_template_output(
    output: &CloneTemplateOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let value = serde_json::json!({
        "template_name": output.template_name,
        "destination": output.destination,
        "source_path": output.source_path,
    });
    Ok(RenderedOutput {
        stdout: Some(format_json(&value)?),
        stderr: None,
        is_stdout_first: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_envelope_contains_expected_keys() {
        let output = CloneTemplateOutput {
            template_name: "Extend Override :: Lootbox Roll :: Go".to_string(),
            destination: "/tmp/test".to_string(),
            source_path: Some("src/grpc-server".to_string()),
        };
        let rendered = render_clone_template_output(&output, &RenderOptions::default()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(
            json["template_name"],
            "Extend Override :: Lootbox Roll :: Go"
        );
        assert_eq!(json["destination"], "/tmp/test");
        assert_eq!(json["source_path"], "src/grpc-server");
    }

    #[test]
    fn test_json_envelope_null_source_path() {
        let output = CloneTemplateOutput {
            template_name: "test".to_string(),
            destination: "/tmp".to_string(),
            source_path: None,
        };
        let rendered = render_clone_template_output(&output, &RenderOptions::default()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert!(json["source_path"].is_null());
    }
}
