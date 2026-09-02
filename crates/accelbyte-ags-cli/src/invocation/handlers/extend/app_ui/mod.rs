//! `app-ui` subcommands under `ags extend`.
//!
//! Routes the `setup-env` and `upload` subcommands to their handlers.
//! The `create` migration shortcut lives in the shim layer and never
//! reaches this module — it is rewritten to a service call before
//! dispatch.

pub(crate) mod setup_env;
pub(crate) mod upload;

use clap::ArgMatches;

use crate::errors::CliError;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;

/// Route `ags extend app-ui <subcommand>` to the appropriate handler.
pub(crate) async fn handle_app_ui(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    match matches.subcommand() {
        Some(("setup-env", sub)) => setup_env::handle_app_ui_setup_env(sub, flags, frontend).await,
        Some(("upload", sub)) => upload::handle_app_ui_upload(sub, flags, frontend).await,
        _ => {
            // Unreachable for non-shim subcommands: clap's
            // `subcommand_required` rejects unknown names before dispatch.
            Err(CliError::Usage {
                message: "Unknown app-ui subcommand".to_string(),
                metadata: None,
            })
        }
    }
}
