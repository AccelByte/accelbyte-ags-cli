//! `extend` subcommands: Extend-platform tooling and migration shortcuts.
//!
//! Contains the `clone-template` command (non-API, filesystem-only),
//! `app-ui` subcommands (API-backed), and migration shortcuts that
//! forward `extend-helper-cli` invocation names to their canonical
//! `ags csm` service operations.

pub(crate) mod app_ui;
pub(crate) mod clone_template;
pub(crate) mod csm_error;
pub(crate) mod image_upload;
pub(crate) mod remote_debug;
pub(crate) mod session_log;
pub(crate) mod tunnel;
pub(crate) mod update_secret;
pub(crate) mod update_var;
// Public for integration-test access; not a supported API surface.
#[doc(hidden)]
pub mod service_shims;

use crate::errors::CliError;
use crate::invocation::builder;
use crate::invocation::clap_helpers;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;

/// Route `ags extend <subcommand>` to the appropriate handler.
pub(crate) async fn handle_extend(
    args: &[String],
    flags: &GlobalFlags,
    frontend: &mut dyn crate::frontend::Frontend,
    frontend_context: &crate::invocation::context::FrontendContext,
) -> Result<InvocationOutcome, CliError> {
    let mut command = builder::build_extend_command();
    let argv = clap_helpers::build_argv("extend", args);

    match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
        Ok(matches) => match matches.subcommand() {
            Some(("clone-template", sub)) => {
                clone_template::handle_clone_template(sub, flags, frontend)
            }
            Some(("app-ui", sub)) => app_ui::handle_app_ui(sub, flags, frontend).await,
            Some(("tunnel", sub)) => tunnel::handle_tunnel(sub, flags, frontend).await,
            Some(("update-secret", sub)) => {
                update_secret::handle_update_secret(sub, flags, frontend).await
            }
            Some(("update-var", sub)) => update_var::handle_update_var(sub, flags, frontend).await,
            Some(("remote-debug", sub)) => match sub.subcommand() {
                Some(("connect", connect_sub)) => {
                    remote_debug::handle_remote_debug_connect(connect_sub, flags, frontend).await
                }
                Some(("enable", enable_sub)) => {
                    remote_debug::enable::handle_remote_debug_enable(
                        enable_sub,
                        flags,
                        frontend,
                        frontend_context,
                    )
                    .await
                }
                Some(("disable", disable_sub)) => {
                    remote_debug::disable::handle_remote_debug_disable(
                        disable_sub,
                        flags,
                        frontend,
                        frontend_context,
                    )
                    .await
                }
                _ => {
                    // Unrecognised subcommand — print remote-debug help.
                    if let Some(remote_debug) = command.find_subcommand_mut("remote-debug") {
                        let _ = remote_debug.print_help();
                    }
                    Ok(InvocationOutcome::Exit(1))
                }
            },
            _ => {
                let _ = command.print_help();
                Ok(InvocationOutcome::Exit(1))
            }
        },
        Err(error) => clap_helpers::outcome_from_clap_error(error),
    }
}
