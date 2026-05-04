//! Config subcommands: get, set, unset.

use crate::errors::CliError;
use crate::invocation::builder;
use crate::invocation::clap_helpers;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use crate::protocol::output::{CommandOutput, ConfigOutput};
use crate::runtime::config::{self, ConfigScope};

/// Route `ags config <subcommand>` to the appropriate handler
pub(crate) async fn handle_config(
    args: &[String],
    flags: &GlobalFlags,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let mut command = builder::build_config_command();
    let argv = clap_helpers::build_argv("config", args);

    match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
        Ok(matches) => {
            let runtime = crate::runtime::Runtime::from_reqwest(
                crate::runtime::execution::ExecutionContext::default(),
                crate::runtime::dispatch::http::build_http_client(flags.timeout)?,
            );

            let view = match matches.subcommand() {
                Some(("get", sub)) => {
                    let key = sub.get_one::<String>("key").map(|s| s.as_str());
                    let profile = resolve_target_profile(flags)?;
                    runtime.config_get(&profile, key)?
                }
                Some(("set", sub)) => {
                    let key = sub.get_one::<String>("key").unwrap();
                    let value = sub.get_one::<String>("value").unwrap();
                    let is_global = sub.get_flag("global");
                    validate_scope_flags(key, is_global, flags.profile.is_some())?;
                    let profile = resolve_profile_if_needed(key, flags)?;
                    runtime.config_set(&profile, key, value)?
                }
                Some(("unset", sub)) => {
                    let key = sub.get_one::<String>("key").unwrap();
                    let is_global = sub.get_flag("global");
                    validate_scope_flags(key, is_global, flags.profile.is_some())?;
                    let profile = resolve_profile_if_needed(key, flags)?;
                    runtime.config_unset(&profile, key)?
                }
                _ => {
                    let _ = command.print_help();
                    return Ok(InvocationOutcome::Complete);
                }
            };

            let output = CommandOutput::Config(ConfigOutput { view });
            frontend.render(&output, &crate::frontend::RenderOptions::from(flags))?;
            Ok(InvocationOutcome::Complete)
        }
        Err(error) => clap_helpers::outcome_from_clap_error(error),
    }
}

/// Resolve which profile to target for profile-scoped operations
fn resolve_target_profile(flags: &GlobalFlags) -> Result<String, CliError> {
    Ok(config::resolve_profile_name(flags.profile.as_deref())?)
}

/// Resolve the profile for set/unset — only needed for profile-scoped keys.
/// Global-scoped keys pass an empty profile string since the facade ignores it.
fn resolve_profile_if_needed(key: &str, flags: &GlobalFlags) -> Result<String, CliError> {
    match config::find_key(key) {
        Some(key_def) if key_def.scope == ConfigScope::Global => Ok(String::new()),
        _ => resolve_target_profile(flags),
    }
}

/// CLI-specific guard: reject contradictory --global / --profile flags.
fn validate_scope_flags(key: &str, is_global: bool, has_profile: bool) -> Result<(), CliError> {
    use crate::errors::ErrorMetadata;

    if let Some(key_def) = config::find_key(key) {
        if is_global && key_def.scope == ConfigScope::Profile {
            return Err(CliError::Usage {
                message: format!(
                    "'{name}' is a profile-scoped key and cannot be used with --global.",
                    name = key_def.cli_name
                ),
                metadata: Some(Box::new(ErrorMetadata::with_suggestion(
                    "Remove --global, or use --profile <name> to target a specific profile.",
                ))),
            });
        }
        if has_profile && key_def.scope == ConfigScope::Global {
            return Err(CliError::Usage {
                message: format!(
                    "'{name}' is a global key and cannot be used with --profile.",
                    name = key_def.cli_name
                ),
                metadata: Some(Box::new(ErrorMetadata::with_suggestion(
                    "Remove --profile. Global keys apply to all profiles.",
                ))),
            });
        }
    }
    Ok(())
}
