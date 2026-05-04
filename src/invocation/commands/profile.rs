//! Profile subcommands: list, create, use, show, delete, rename.

use crate::errors::CliError;
use crate::invocation::builder;
use crate::invocation::clap_helpers;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use crate::protocol::output::{CommandOutput, ProfileOutput};

/// Route `ags profile <subcommand>` to the appropriate handler
pub(crate) async fn handle_profile(
    args: &[String],
    flags: &GlobalFlags,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let mut command = builder::build_profile_command();
    let argv = clap_helpers::build_argv("profile", args);

    match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
        Ok(matches) => {
            let runtime = crate::runtime::Runtime::from_reqwest(
                crate::runtime::execution::ExecutionContext::default(),
                crate::runtime::dispatch::http::build_http_client(flags.timeout)?,
            );

            let view = match matches.subcommand() {
                Some(("list", _)) => runtime.profile_list()?,
                Some(("create", sub)) => {
                    let name = sub.get_one::<String>("name").unwrap();
                    runtime.profile_create(name)?
                }
                Some(("use", sub)) => {
                    let name = sub.get_one::<String>("name").unwrap();
                    runtime.profile_use(name)?
                }
                Some(("show", sub)) => {
                    let name = sub.get_one::<String>("name").map(|s| s.as_str());
                    runtime.profile_show(name)?
                }
                Some(("delete", sub)) => {
                    let name = sub.get_one::<String>("name").unwrap();
                    runtime.profile_delete(name)?
                }
                Some(("rename", sub)) => {
                    let old = sub.get_one::<String>("old").unwrap();
                    let new = sub.get_one::<String>("new").unwrap();
                    runtime.profile_rename(old, new)?
                }
                _ => {
                    let _ = command.print_help();
                    return Ok(InvocationOutcome::Complete);
                }
            };

            let output = CommandOutput::Profile(ProfileOutput { view });
            frontend.render(&output, &crate::frontend::RenderOptions::from(flags))?;
            Ok(InvocationOutcome::Complete)
        }
        Err(error) => clap_helpers::outcome_from_clap_error(error),
    }
}
