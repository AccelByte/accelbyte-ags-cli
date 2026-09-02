//! JSON renderer for `ags extend update-secret` output.

use crate::errors::CliError;
use crate::frontend::output::json::format_json;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::UpdateSecretOutput;

pub(crate) fn render_update_secret_output(
    output: &UpdateSecretOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let value = serde_json::json!({
        "configId": output.config_id,
        "configName": output.config_name,
        "applyMask": output.apply_mask,
        "description": output.description,
        "created": output.created,
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
    fn test_json_envelope_contains_all_fields() {
        let output = UpdateSecretOutput {
            config_id: "id-1".to_string(),
            config_name: "MY_KEY".to_string(),
            apply_mask: true,
            description: Some("desc".to_string()),
            created: false,
        };
        let rendered = render_update_secret_output(&output, &RenderOptions::default()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["configId"], "id-1");
        assert_eq!(json["configName"], "MY_KEY");
        assert_eq!(json["applyMask"], true);
        assert_eq!(json["description"], "desc");
        assert_eq!(json["created"], false);
    }

    #[test]
    fn test_json_envelope_null_description() {
        let output = UpdateSecretOutput {
            config_id: "id-1".to_string(),
            config_name: "MY_KEY".to_string(),
            apply_mask: false,
            description: None,
            created: false,
        };
        let rendered = render_update_secret_output(&output, &RenderOptions::default()).unwrap();
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert!(json["description"].is_null());
    }

    #[test]
    fn test_json_envelope_never_contains_value_key() {
        let output = UpdateSecretOutput {
            config_id: "id-1".to_string(),
            config_name: "MY_KEY".to_string(),
            apply_mask: true,
            description: Some("desc".to_string()),
            created: true,
        };
        let rendered = render_update_secret_output(&output, &RenderOptions::default()).unwrap();
        let raw = rendered.stdout.unwrap();
        assert!(
            !raw.contains("\"value\""),
            "secret value must never appear in --format json output: {raw}"
        );
    }
}
