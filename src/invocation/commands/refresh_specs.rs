//! `refresh-specs` command: rebuild parsed-schema cache from bundled specs.

use crate::catalogue::Catalogue;
use crate::errors::CliError;
use crate::frontend;
use crate::invocation::builder;
use crate::invocation::clap_helpers;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use crate::protocol::output::{CommandOutput, RefreshMode, RefreshSpecsOutput};

/// Handle the `ags refresh-specs [<service>]` command.
pub(crate) fn handle_refresh_specs(
    args: &[String],
    flags: &GlobalFlags,
    frontend: &mut dyn frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let mut command = builder::build_refresh_specs_command();
    let argv = clap_helpers::build_argv("refresh-specs", args);

    let matches = match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
        Ok(m) => m,
        Err(error) => return clap_helpers::outcome_from_clap_error(error),
    };

    let service_arg = matches.get_one::<String>("service").cloned();

    match service_arg {
        Some(name) => handle_refresh_single(&name, flags, frontend),
        None => handle_refresh_all(flags, frontend),
    }
}

/// Refresh the bundled spec for a single named service and render the timing report.
fn handle_refresh_single(
    service_arg: &str,
    flags: &GlobalFlags,
    frontend: &mut dyn frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let internal = Catalogue::internal_name(service_arg).ok_or_else(|| {
        let services = Catalogue::service_ids()
            .map(Catalogue::display_name_or_panic)
            .map(|s| format!("  {s}"))
            .collect::<Vec<_>>()
            .join("\n");
        CliError::Usage {
            message: format!("Unknown service: '{service_arg}'\n\nValid services:\n{services}"),
            metadata: None,
        }
    })?;

    let start = std::time::Instant::now();
    Catalogue::refresh(internal)?;
    let duration = start.elapsed();

    let output = RefreshSpecsOutput {
        mode: RefreshMode::Single,
        succeeded: vec![internal.to_string()],
        failed: vec![],
        duration,
    };
    frontend.render(
        &CommandOutput::RefreshSpecs(output),
        &frontend::RenderOptions::from(flags),
    )?;
    Ok(InvocationOutcome::Complete)
}

/// Refresh every bundled spec, clearing cached state first, and render the aggregate report.
fn handle_refresh_all(
    flags: &GlobalFlags,
    frontend: &mut dyn frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let report = Catalogue::refresh_all();

    if let Some(err) = report.cache_clear_error {
        return Err(CliError::Internal(anyhow::anyhow!(
            "Failed to clear cache directory: {err}"
        )));
    }

    let failed: Vec<(String, String)> = report
        .failed
        .iter()
        .map(|(service, err)| (service.clone(), err.message.clone()))
        .collect();

    let has_failures = !failed.is_empty();

    let output = RefreshSpecsOutput {
        mode: RefreshMode::All,
        succeeded: report.succeeded,
        failed,
        duration: report.duration,
    };

    frontend.render(
        &CommandOutput::RefreshSpecs(output),
        &frontend::RenderOptions::from(flags),
    )?;

    if has_failures {
        return Ok(InvocationOutcome::Exit(1));
    }
    Ok(InvocationOutcome::Complete)
}
