//! Shared top-level rendering dispatch.

use crate::errors::CliError;
use crate::frontend::{FrontendKind, RenderOptions, RenderedOutput};
use crate::protocol::output::{BinaryWrittenDestination, BinaryWrittenOutput, CommandOutput};

/// Render a `CommandOutput` into a `RenderedOutput` using the selected frontend kind.
pub(crate) fn render_output(
    kind: FrontendKind,
    output: &CommandOutput,
    options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    match output {
        CommandOutput::Auth(auth_output) => match kind {
            FrontendKind::Human => {
                crate::frontend::human::commands::auth::render_auth_output(auth_output, options)
            }
            FrontendKind::Json => {
                crate::frontend::json::commands::auth::render_auth_output(auth_output, options)
            }
        },
        CommandOutput::Config(config_output) => match kind {
            FrontendKind::Human => crate::frontend::human::commands::config::render_config_output(
                config_output,
                options,
            ),
            FrontendKind::Json => crate::frontend::json::commands::config::render_config_output(
                config_output,
                options,
            ),
        },
        CommandOutput::Profile(profile_output) => match kind {
            FrontendKind::Human => {
                crate::frontend::human::commands::profile::render_profile_output(
                    profile_output,
                    options,
                )
            }
            FrontendKind::Json => crate::frontend::json::commands::profile::render_profile_output(
                profile_output,
                options,
            ),
        },
        CommandOutput::Service(api_output) => match kind {
            FrontendKind::Human => {
                crate::frontend::human::commands::service::render_api_output(api_output, options)
            }
            FrontendKind::Json => {
                crate::frontend::json::commands::service::render_api_output(api_output, options)
            }
        },
        CommandOutput::DryRun(dry_run_result) => match kind {
            FrontendKind::Human => {
                crate::frontend::human::commands::service::render_dry_run_output(dry_run_result)
            }
            FrontendKind::Json => {
                crate::frontend::json::commands::service::render_dry_run_output(dry_run_result)
            }
        },
        CommandOutput::Doctor(doctor_result) => match kind {
            FrontendKind::Human => crate::frontend::human::commands::doctor::render_doctor_output(
                doctor_result,
                options,
            ),
            FrontendKind::Json => crate::frontend::json::commands::doctor::render_doctor_output(
                doctor_result,
                options,
            ),
        },
        CommandOutput::Completions(completions_output) => match kind {
            FrontendKind::Human => {
                crate::frontend::human::commands::completions::render_completions_output(
                    completions_output,
                    options,
                )
            }
            FrontendKind::Json => {
                crate::frontend::json::commands::completions::render_completions_output(
                    completions_output,
                    options,
                )
            }
        },
        CommandOutput::Version(version_output) => match kind {
            FrontendKind::Human => {
                crate::frontend::human::commands::version::render_version_output(
                    version_output,
                    options,
                )
            }
            FrontendKind::Json => crate::frontend::json::commands::version::render_version_output(
                version_output,
                options,
            ),
        },
        CommandOutput::BinaryWritten(binary_written_output) => {
            render_binary_written(kind, binary_written_output)
        }
        CommandOutput::RefreshSpecs(refresh_specs_output) => match kind {
            FrontendKind::Human => {
                crate::frontend::human::commands::refresh_specs::render_refresh_specs_output(
                    refresh_specs_output,
                    options,
                )
            }
            FrontendKind::Json => {
                crate::frontend::json::commands::refresh_specs::render_refresh_specs_output(
                    refresh_specs_output,
                    options,
                )
            }
        },
        CommandOutput::Skeleton(skeleton) => render_raw_json_value(&skeleton.body),
        CommandOutput::Describe(describe) => render_raw_json_value(&describe.envelope),
    }
}

/// Render a pre-built JSON value as the full stdout payload.
/// Shared by `Skeleton` and `Describe` — both are single-format commands.
fn render_raw_json_value(value: &serde_json::Value) -> Result<RenderedOutput, CliError> {
    Ok(RenderedOutput {
        stdout: Some(crate::frontend::json::format_json(value)?),
        stderr: None,
        is_stdout_first: true,
    })
}

/// Render a `BinaryWritten` output for the selected frontend kind.
fn render_binary_written(
    kind: FrontendKind,
    output: &BinaryWrittenOutput,
) -> Result<RenderedOutput, CliError> {
    match kind {
        FrontendKind::Human => match &output.destination {
            BinaryWrittenDestination::Stdout => Ok(RenderedOutput::default()),
            BinaryWrittenDestination::File(path) => Ok(RenderedOutput {
                stdout: None,
                stderr: Some(format!(
                    "✔ Wrote {} bytes ({}) to {}",
                    output.bytes_written,
                    output.content_type,
                    path.display()
                )),
                is_stdout_first: false,
            }),
        },
        FrontendKind::Json => match &output.destination {
            BinaryWrittenDestination::Stdout => Ok(RenderedOutput::default()),
            BinaryWrittenDestination::File(path) => {
                let value = serde_json::json!({
                    "status": "written",
                    "destination": path.display().to_string(),
                    "bytes_written": output.bytes_written,
                    "content_type": output.content_type,
                });
                Ok(RenderedOutput {
                    stdout: Some(crate::frontend::json::format_json(&value)?),
                    stderr: None,
                    is_stdout_first: true,
                })
            }
        },
    }
}
