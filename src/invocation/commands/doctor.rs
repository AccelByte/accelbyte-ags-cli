//! Doctor command: diagnose configuration, auth, and network health.

use crate::errors::CliError;
use crate::invocation::builder;
use crate::invocation::clap_helpers;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use crate::protocol::output::CommandOutput;

/// Handle the `ags doctor` command.
pub(crate) async fn handle_doctor(
    args: &[String],
    flags: &GlobalFlags,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let mut command = builder::build_doctor_command();
    let argv = clap_helpers::build_argv("doctor", args);

    let matches = match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
        Ok(m) => m,
        Err(e) => return clap_helpers::outcome_from_clap_error(e),
    };

    let is_offline = matches.get_flag("offline");
    let is_all = matches.get_flag("all");

    if is_all && flags.profile.is_some() {
        frontend.render_warning(
            "--all checks every profile; --profile is ignored",
            None,
            None,
        );
    }

    let http_client = crate::runtime::dispatch::http::build_http_client(flags.timeout)?;
    let mut runtime = crate::runtime::Runtime::from_reqwest(
        crate::runtime::execution::ExecutionContext::default(),
        http_client,
    );

    let output = if is_all {
        runtime.run_diagnostics(is_offline).await?
    } else {
        runtime
            .run_diagnostics_for_profile(flags.profile.as_deref(), is_offline)
            .await?
    };

    let has_failures = output.has_failures();

    frontend.render(
        &CommandOutput::Doctor(output),
        &crate::frontend::RenderOptions::from(flags),
    )?;

    if has_failures {
        return Ok(InvocationOutcome::Exit(1));
    }
    Ok(InvocationOutcome::Complete)
}
