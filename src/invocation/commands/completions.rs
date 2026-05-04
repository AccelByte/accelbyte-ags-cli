//! `ags completions` command: generate a shell-completion script.

use clap_complete::Shell;

use crate::errors::{CliError, ErrorMetadata};
use crate::invocation::clap_helpers;
use crate::invocation::InvocationOutcome;

/// Resolve the target shell from the `$SHELL` environment variable, with a
/// Windows fallback to PowerShell. Returns `Err` when detection fails and
/// the user should be shown the four-shell usage error.
pub(super) fn resolve_target_shell(
    env_shell: Option<&str>,
    is_windows: bool,
) -> Result<Shell, CliError> {
    if let Some(path) = env_shell {
        let basename = path.rsplit('/').next().unwrap_or(path);
        let basename = basename.rsplit('\\').next().unwrap_or(basename);
        // Strip trailing `.exe` for Windows binaries.
        let basename = basename.strip_suffix(".exe").unwrap_or(basename);
        match basename {
            "bash" => return Ok(Shell::Bash),
            "zsh" => return Ok(Shell::Zsh),
            "fish" => return Ok(Shell::Fish),
            "pwsh" | "powershell" => return Ok(Shell::PowerShell),
            _ => {}
        }
    }

    if is_windows {
        return Ok(Shell::PowerShell);
    }

    Err(CliError::Usage {
        message: "Could not detect shell from $SHELL".to_string(),
        metadata: Some(Box::new(ErrorMetadata::with_suggestion(
            "Pass the shell explicitly: ags completions <bash|zsh|fish|powershell>",
        ))),
    })
}

/// Convert a `clap_complete::Shell` into the lowercase name the CLI uses on the command line.
fn shell_to_cli_name(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => "bash",
        Shell::Zsh => "zsh",
        Shell::Fish => "fish",
        Shell::PowerShell => "powershell",
        // clap_complete::Shell is #[non_exhaustive]; build_completions_command
        // restricts the value_parser to the four variants above, and resolve_target_shell
        // returns only those four, so any other variant here is a bug.
        _ => unreachable!("unsupported clap_complete::Shell variant"),
    }
}

/// Build the shell-specific install snippet shown alongside the generated completions script.
fn install_hint(shell: Shell, name: &str) -> String {
    match shell {
        Shell::Bash | Shell::Zsh => format!("source <(ags completions {name})"),
        Shell::Fish => format!("ags completions {name} | source"),
        Shell::PowerShell => format!("ags completions {name} | Out-String | Invoke-Expression"),
        _ => unreachable!("unsupported clap_complete::Shell variant"),
    }
}

/// Handle the `ags completions` command.
pub(crate) fn handle_completions(
    args: &[String],
    flags: &crate::invocation::flags::GlobalFlags,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    use crate::frontend::RenderOptions;
    use crate::invocation::builder;
    use crate::protocol::output::{CommandOutput, CompletionsOutput};

    let mut command = builder::build_completions_command();
    let argv = clap_helpers::build_argv("completions", args);

    let matches = match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
        Ok(m) => m,
        Err(error) => return clap_helpers::outcome_from_clap_error(error),
    };

    let shell_arg: Option<&Shell> = matches.get_one::<Shell>("shell");
    let (shell, hint) = match shell_arg {
        Some(&s) => (s, None),
        None => {
            let shell =
                resolve_target_shell(std::env::var("SHELL").ok().as_deref(), cfg!(windows))?;
            let shell_name = shell_to_cli_name(shell);
            let install_command = install_hint(shell, shell_name);
            (
                shell,
                Some(format!(
                    "Detected {shell_name}. To install, run: {install_command}"
                )),
            )
        }
    };

    let script = crate::runtime::completions::generate_completion_script(shell);

    frontend.render(
        &CommandOutput::Completions(CompletionsOutput { script, hint }),
        &RenderOptions::from(flags),
    )?;
    Ok(InvocationOutcome::Complete)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detects_zsh_from_shell_env() {
        assert!(matches!(
            resolve_target_shell(Some("/bin/zsh"), false).unwrap(),
            Shell::Zsh
        ));
    }

    #[test]
    fn test_detects_bash_from_shell_env() {
        assert!(matches!(
            resolve_target_shell(Some("/bin/bash"), false).unwrap(),
            Shell::Bash
        ));
    }

    #[test]
    fn test_detects_fish_from_shell_env() {
        assert!(matches!(
            resolve_target_shell(Some("/usr/local/bin/fish"), false).unwrap(),
            Shell::Fish
        ));
    }

    #[test]
    fn test_detects_powershell_from_shell_env() {
        assert!(matches!(
            resolve_target_shell(Some("pwsh"), false).unwrap(),
            Shell::PowerShell
        ));
    }

    #[test]
    fn test_detects_powershell_windows_exe_suffix() {
        assert!(matches!(
            resolve_target_shell(Some("C:\\Program Files\\PowerShell\\pwsh.exe"), true).unwrap(),
            Shell::PowerShell
        ));
    }

    #[test]
    fn test_windows_without_shell_env_defaults_to_powershell() {
        assert!(matches!(
            resolve_target_shell(None, true).unwrap(),
            Shell::PowerShell
        ));
    }

    #[test]
    fn test_windows_with_unrecognized_shell_defaults_to_powershell() {
        assert!(matches!(
            resolve_target_shell(Some("C:\\Windows\\System32\\cmd.exe"), true).unwrap(),
            Shell::PowerShell
        ));
    }

    #[test]
    fn test_non_windows_without_shell_env_errors() {
        assert!(resolve_target_shell(None, false).is_err());
    }

    #[test]
    fn test_unknown_shell_errors() {
        assert!(resolve_target_shell(Some("/bin/tcsh"), false).is_err());
    }
}
