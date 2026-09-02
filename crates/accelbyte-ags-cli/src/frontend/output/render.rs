//! Shared top-level rendering dispatch.

use crate::errors::CliError;
use crate::frontend::{RenderFormat, RenderOptions, RenderedOutput};
use ags_protocol::output::{BinaryWrittenDestination, BinaryWrittenOutput, CommandOutput};

/// Render a `CommandOutput` into a `RenderedOutput` using the selected text format.
pub(crate) fn render_output(
    format: RenderFormat,
    output: &CommandOutput,
    options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    match output {
        CommandOutput::Auth(auth_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::auth::render_auth_output(
                    auth_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::auth::render_auth_output(
                    auth_output,
                    options,
                )
            }
        },
        CommandOutput::Config(config_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::config::render_config_output(
                    config_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::config::render_config_output(
                    config_output,
                    options,
                )
            }
        },
        CommandOutput::Profile(profile_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::profile::render_profile_output(
                    profile_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::profile::render_profile_output(
                    profile_output,
                    options,
                )
            }
        },
        CommandOutput::Service(api_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::service::render_api_output(
                    api_output, options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::service::render_api_output(
                    api_output, options,
                )
            }
        },
        CommandOutput::AmsUpload(ams_upload_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::ams_upload::render_ams_upload_output(
                    ams_upload_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::ams_upload::render_ams_upload_output(
                    ams_upload_output,
                    options,
                )
            }
        },
        CommandOutput::DryRun(dry_run_result) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::service::render_dry_run_output(
                    dry_run_result,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::service::render_dry_run_output(
                    dry_run_result,
                )
            }
        },
        CommandOutput::Doctor(doctor_result) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::doctor::render_doctor_output(
                    doctor_result,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::doctor::render_doctor_output(
                    doctor_result,
                    options,
                )
            }
        },
        CommandOutput::Completions(completions_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::completions::render_completions_output(
                    completions_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::completions::render_completions_output(
                    completions_output,
                    options,
                )
            }
        },
        CommandOutput::Version(version_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::version::render_version_output(
                    version_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::version::render_version_output(
                    version_output,
                    options,
                )
            }
        },
        CommandOutput::CloneTemplate(clone_template_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::clone_template::render_clone_template_output(
                    clone_template_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::clone_template::render_clone_template_output(
                    clone_template_output,
                    options,
                )
            }
        },
        CommandOutput::SetupEnv(setup_env_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::setup_env::render_setup_env_output(
                    setup_env_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::setup_env::render_setup_env_output(
                    setup_env_output,
                    options,
                )
            }
        },
        CommandOutput::AppUiUpload(upload_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::app_ui_upload::render_app_ui_upload_output(
                    upload_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::app_ui_upload::render_app_ui_upload_output(
                    upload_output,
                    options,
                )
            }
        },
        CommandOutput::UpdateVar(update_var_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::update_var::render_update_var_output(
                    update_var_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::update_var::render_update_var_output(
                    update_var_output,
                    options,
                )
            }
        },
        CommandOutput::BinaryWritten(binary_written_output) => {
            render_binary_written(format, binary_written_output)
        }
        CommandOutput::UpdateSecret(update_secret_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::update_secret::render_update_secret_output(
                    update_secret_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::update_secret::render_update_secret_output(
                    update_secret_output,
                    options,
                )
            }
        },
        CommandOutput::RefreshSpecs(refresh_specs_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::refresh_specs::render_refresh_specs_output(
                    refresh_specs_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::refresh_specs::render_refresh_specs_output(
                    refresh_specs_output,
                    options,
                )
            }
        },
        CommandOutput::Skeleton(skeleton) => render_raw_json_value(&skeleton.body),
        CommandOutput::Describe(describe) => render_raw_json_value(&describe.envelope),
        CommandOutput::Workflow {
            workflow_id,
            outputs,
            step_summaries,
            completion,
            output_view,
        } => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::workflow::render_workflow(
                    workflow_id,
                    outputs,
                    step_summaries,
                    completion,
                    output_view.as_ref(),
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::workflow::render_workflow(
                    workflow_id,
                    outputs,
                    step_summaries,
                    completion,
                    options,
                )
            }
        },
        CommandOutput::WorkflowDryRun {
            workflow_id,
            step_previews,
        } => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::workflow_dry_run::render_workflow_dry_run(
                    workflow_id,
                    step_previews,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::workflow::render_workflow_dry_run(
                    workflow_id,
                    step_previews,
                )
            }
        },
        CommandOutput::WorkflowCatalogue { entries } => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::workflow::render_workflow_catalogue(
                    entries, options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::workflow::render_workflow_catalogue(
                    entries, options,
                )
            }
        },
        CommandOutput::WorkflowAdd(workflow_add_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::workflow::render_workflow_add(
                    workflow_add_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::workflow::render_workflow_add(
                    workflow_add_output,
                    options,
                )
            }
        },
        CommandOutput::WorkflowTemplate(workflow_template_output) => {
            render_workflow_template(format, workflow_template_output)
        }
        CommandOutput::WorkflowRemove(workflow_remove_output) => match format {
            RenderFormat::Human => {
                crate::frontend::output::human::commands::workflow::render_workflow_remove(
                    workflow_remove_output,
                    options,
                )
            }
            RenderFormat::Json => {
                crate::frontend::output::json::commands::workflow::render_workflow_remove(
                    workflow_remove_output,
                    options,
                )
            }
        },
    }
}

/// Render a pre-built JSON value as the full stdout payload.
/// Shared by `Skeleton` and `Describe` — both are single-format commands.
fn render_raw_json_value(value: &serde_json::Value) -> Result<RenderedOutput, CliError> {
    Ok(RenderedOutput {
        stdout: Some(crate::frontend::output::json::format_json(value)?),
        stderr: None,
        is_stdout_first: true,
    })
}

/// Render a `BinaryWritten` output for the selected text format.
fn render_binary_written(
    format: RenderFormat,
    output: &BinaryWrittenOutput,
) -> Result<RenderedOutput, CliError> {
    match format {
        RenderFormat::Human => match &output.destination {
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
        RenderFormat::Json => match &output.destination {
            BinaryWrittenDestination::Stdout => Ok(RenderedOutput::default()),
            BinaryWrittenDestination::File(path) => {
                let value = serde_json::json!({
                    "status": "written",
                    "destination": path.display().to_string(),
                    "bytes_written": output.bytes_written,
                    "content_type": output.content_type,
                });
                Ok(RenderedOutput {
                    stdout: Some(crate::frontend::output::json::format_json(&value)?),
                    stderr: None,
                    is_stdout_first: true,
                })
            }
        },
    }
}

/// Render a `WorkflowTemplate` output for the selected text format. When the
/// destination is stdout, the YAML text itself IS the stdout payload (raw,
/// pipeable, no JSON wrapping even under `--format json` — same "single
/// format for the payload itself" precedent as `Skeleton`/`Describe`, except
/// here the payload is already the exact string to emit). When written to a
/// file, only a confirmation is rendered, mirroring `render_binary_written`.
fn render_workflow_template(
    format: RenderFormat,
    output: &ags_protocol::output_views::WorkflowTemplateOutput,
) -> Result<RenderedOutput, CliError> {
    match &output.destination {
        BinaryWrittenDestination::Stdout => Ok(RenderedOutput {
            stdout: Some(output.yaml.clone()),
            stderr: None,
            is_stdout_first: true,
        }),
        BinaryWrittenDestination::File(path) => match format {
            RenderFormat::Human => Ok(RenderedOutput {
                stdout: None,
                stderr: Some(format!("✔ Wrote workflow template to {}", path.display())),
                is_stdout_first: false,
            }),
            RenderFormat::Json => {
                let value = serde_json::json!({
                    "status": "written",
                    "destination": path.display().to_string(),
                });
                Ok(RenderedOutput {
                    stdout: Some(crate::frontend::output::json::format_json(&value)?),
                    stderr: None,
                    is_stdout_first: true,
                })
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::workflow::WorkflowId;
    use std::collections::BTreeMap;

    #[test]
    fn test_render_output_workflow_json_dispatches_to_envelope() {
        // Guards the CommandOutput::Workflow → RenderFormat::Json dispatch arm:
        // a regression here would resurrect the old "not yet supported" error.
        let mut outputs = BTreeMap::new();
        outputs.insert("poolName".to_string(), serde_json::json!("ranked-1v1"));
        let output = CommandOutput::Workflow {
            workflow_id: WorkflowId::new("competitive-multiplayer"),
            outputs,
            step_summaries: vec!["create-pool: ok".to_string()],
            completion: None,
            output_view: None,
        };
        let rendered = render_output(RenderFormat::Json, &output, &RenderOptions::default())
            .expect("workflow JSON dispatch must not error");
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["workflow"], "competitive-multiplayer");
        assert_eq!(json["status"], "success");
        assert_eq!(json["outputs"]["poolName"], "ranked-1v1");
    }

    #[test]
    fn test_render_output_workflow_dry_run_json_dispatches_to_envelope() {
        // Symmetric guard for the CommandOutput::WorkflowDryRun → RenderFormat::Json
        // dispatch arm — the same "not yet supported" error lived here pre-PR.
        use ags_protocol::catalogue::HttpMethod;
        use ags_protocol::result::DryRunResult;
        use ags_protocol::workflow::{StepDryRunAction, StepDryRunPreview};
        let preview = StepDryRunPreview {
            step_id: "create-stat".to_string(),
            step_index: 0,
            action: StepDryRunAction::Request(DryRunResult {
                http_method: HttpMethod::Post,
                url: "https://example.test/social/v1/admin/namespaces/dev/stats".to_string(),
                headers: vec![],
                query: vec![],
                body: None,
            }),
            synthesised_outputs: BTreeMap::new(),
        };
        let output = CommandOutput::WorkflowDryRun {
            workflow_id: WorkflowId::new("competitive-multiplayer"),
            step_previews: vec![preview],
        };
        let rendered = render_output(RenderFormat::Json, &output, &RenderOptions::default())
            .expect("WorkflowDryRun JSON dispatch must not error");
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["workflow"], "competitive-multiplayer");
        assert_eq!(json["dry_run"], true);
        assert!(json["steps"].is_array());
    }

    #[test]
    fn test_render_output_workflow_add_json_reports_installed_path() {
        let output = CommandOutput::WorkflowAdd(ags_protocol::output_views::WorkflowAddOutput {
            id: WorkflowId::new("my-workflow"),
            validated_only: false,
            path: Some(std::path::PathBuf::from("/tmp/workflows/my-workflow.yaml")),
        });
        let rendered = render_output(RenderFormat::Json, &output, &RenderOptions::default())
            .expect("must render");
        let json: serde_json::Value =
            serde_json::from_str(rendered.stdout.as_deref().unwrap()).unwrap();
        assert_eq!(json["id"], "my-workflow");
        assert_eq!(json["validated_only"], false);
        assert_eq!(json["path"], "/tmp/workflows/my-workflow.yaml");
    }

    #[test]
    fn test_render_output_workflow_template_stdout_is_raw_yaml() {
        let output =
            CommandOutput::WorkflowTemplate(ags_protocol::output_views::WorkflowTemplateOutput {
                yaml: "id: my-workflow\n".to_string(),
                destination: BinaryWrittenDestination::Stdout,
            });
        let rendered = render_output(RenderFormat::Human, &output, &RenderOptions::default())
            .expect("must render");
        assert_eq!(rendered.stdout.as_deref(), Some("id: my-workflow\n"));
        assert!(rendered.stderr.is_none());
    }
}
