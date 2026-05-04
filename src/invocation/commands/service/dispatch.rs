use crate::catalogue::{Catalogue, SpecSource};
use crate::errors::CliError;
use crate::invocation::commands::service::parser::ParsedServiceCommand;
use crate::invocation::errors::ExecutorError;
use crate::invocation::flags;
use crate::protocol::output::ResolutionTrace;

/// Validate output format, handle dry-run and confirmation, then execute the command.
/// Returns `Ok(None)` when the user declines confirmation.
pub(super) async fn dispatch_command(
    runtime: &mut crate::runtime::Runtime,
    parsed: &ParsedServiceCommand,
    flags: &flags::GlobalFlags,
    context: &crate::runtime::execution::ExecutionContext,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<Option<crate::protocol::output::CommandOutput>, CliError> {
    if flags.is_dry_run && flags.verbosity.is_verbose() {
        let service_display = Catalogue::display_name(parsed.service_id.as_str())
            .unwrap_or(parsed.service_id.as_str());
        let spec_source_label = match parsed.spec_source {
            SpecSource::Cache => format!("{} loaded from cache", service_display.to_uppercase()),
            SpecSource::Bundled => {
                format!(
                    "{} decompressed from bundle",
                    service_display.to_uppercase()
                )
            }
        };
        let token_expiry_label = context
            .access_token_expiry
            .as_ref()
            .map(|duration| format!("expires in {duration}"));
        let trace = ResolutionTrace {
            spec_source: spec_source_label,
            profile: (
                context.profile.clone(),
                context.profile_source.label().to_string(),
            ),
            base_url: (
                context.base_url.clone(),
                context.base_url_source.label().to_string(),
            ),
            namespace: context
                .namespace
                .as_ref()
                .zip(context.namespace_source.as_ref())
                .map(|(namespace, source)| (namespace.clone(), source.label().to_string())),
            token_source: context.access_token_source.label().to_string(),
            token_expiry: token_expiry_label,
        };
        frontend.render_resolution_trace(&trace);
    }

    if flags.is_dry_run {
        let report = runtime.dry_run_command(&parsed.command_request)?;
        return Ok(Some(crate::protocol::output::CommandOutput::DryRun(report)));
    }

    let preview = runtime.preview_command(&parsed.command_request)?;
    if preview.confirmation_required && !flags.is_auto_confirmed {
        if flags.is_no_input {
            return Err(CliError::from(ExecutorError::ConfirmationRequired));
        }
        let confirmed = frontend.confirm(&preview)?;
        if !confirmed {
            return Ok(None);
        }
    }

    let output = runtime
        .run_command(parsed.command_request.clone(), frontend.progress_sink())
        .await?;
    Ok(Some(output))
}
