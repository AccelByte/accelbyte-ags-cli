//! Human-readable update output rendering.

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::{RenderOptions, RenderedOutput};
use ags_protocol::output::{InstallMethod, UpdateOutput};

/// Render update output as human-readable text.
pub(crate) fn render_update_output(
    output: &UpdateOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let color = style::is_stdout_enabled();
    let text = if output.update_available {
        render_newer_available(output, color)
    } else {
        render_already_current(output, color)
    };
    Ok(RenderedOutput {
        stdout: Some(text),
        stderr: None,
        is_stdout_first: true,
    })
}

/// Layout when a newer release is available:
///
/// ```text
/// ↑ ags 0.5.2 is available (current: 0.5.1)
///
/// To upgrade, run:
///   <upgrade_command>
///
/// Release notes: <release_url>
/// ```
fn render_newer_available(output: &UpdateOutput, color: bool) -> String {
    let mut lines = Vec::new();

    let headline = format!(
        "{} ags {} is available (current: {})",
        style::text::SYMBOL_UPGRADE,
        output.latest,
        output.current,
    );
    lines.push(style::apply_tone(&headline, style::Tone::Info, color));

    lines.push(String::new());

    match output.install_method {
        InstallMethod::Manual => {
            if output.binary_path == "<unknown>" {
                lines.push(format!(
                    "To upgrade, download {} from {} and replace the installed ags binary",
                    output.download_archive, output.release_url,
                ));
            } else {
                lines.push(format!(
                    "To upgrade, download {} from {} and replace the binary at {}",
                    output.download_archive, output.release_url, output.binary_path,
                ));
            }
        }
        _ => {
            if let Some(ref cmd) = output.upgrade_command {
                lines.push("To upgrade, run:".to_string());
                lines.push(format!("  {cmd}"));
            }
        }
    }

    lines.push(String::new());
    lines.push(format!("Release notes: {}", output.release_url));

    lines.join("\n")
}

/// Layout when the current version is the latest:
///
/// ```text
/// ✔ ags 0.5.1 is the latest release.
/// ```
fn render_already_current(output: &UpdateOutput, color: bool) -> String {
    let text = format!(
        "{} ags {} is the latest release.",
        style::text::SYMBOL_SUCCESS,
        output.current,
    );
    style::apply_tone(&text, style::Tone::Success, color)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::RenderOptions;
    use ags_protocol::output::UpdateOutput;

    /// When `binary_path` is `<unknown>`, the manual instruction omits the
    /// misleading path and uses a generic "replace the installed ags binary"
    /// phrasing instead.
    #[test]
    fn manual_instruction_omits_unknown_binary_path() {
        let output = UpdateOutput {
            current: "0.5.0".to_string(),
            latest: "0.6.0".to_string(),
            update_available: true,
            install_method: InstallMethod::Manual,
            binary_path: "<unknown>".to_string(),
            upgrade_command: None,
            download_archive: "accelbyte-ags-cli-x86_64-unknown-linux-gnu.tar.xz".to_string(),
            release_url: "https://github.com/AccelByte/accelbyte-ags-cli/releases/tag/v0.6.0"
                .to_string(),
        };
        let rendered = render_update_output(&output, &RenderOptions::default()).unwrap();
        let text = rendered.stdout.unwrap();
        assert!(
            !text.contains("<unknown>"),
            "must not show '<unknown>' as a binary path; got: {text}"
        );
        assert!(
            text.contains("replace the installed ags binary"),
            "must use the generic phrasing; got: {text}"
        );
        assert!(
            text.contains("accelbyte-ags-cli-x86_64-unknown-linux-gnu.tar.xz"),
            "must still name the archive; got: {text}"
        );
    }

    /// When `binary_path` is a real path, the manual instruction includes it.
    #[test]
    fn manual_instruction_shows_known_binary_path() {
        let output = UpdateOutput {
            current: "0.5.0".to_string(),
            latest: "0.6.0".to_string(),
            update_available: true,
            install_method: InstallMethod::Manual,
            binary_path: "/usr/local/bin/ags".to_string(),
            upgrade_command: None,
            download_archive: "accelbyte-ags-cli-x86_64-unknown-linux-gnu.tar.xz".to_string(),
            release_url: "https://github.com/AccelByte/accelbyte-ags-cli/releases/tag/v0.6.0"
                .to_string(),
        };
        let rendered = render_update_output(&output, &RenderOptions::default()).unwrap();
        let text = rendered.stdout.unwrap();
        assert!(
            text.contains("/usr/local/bin/ags"),
            "must show the known binary path; got: {text}"
        );
    }
}
