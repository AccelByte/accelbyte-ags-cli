//! Shell-completion script generation. Pure in-memory — no I/O, no auth,
//! no config reads — so this sits outside the `Runtime` impl and takes
//! no context.

use clap_complete::Shell;

use crate::invocation::builder;

/// Generate a shell-completion script for the given shell over the full
/// command tree.
pub fn generate_completion_script(shell: Shell) -> String {
    let mut command = builder::build_full_command();
    let mut buf: Vec<u8> = Vec::new();
    clap_complete::generate(shell, &mut command, "ags", &mut buf);
    String::from_utf8(buf).expect("clap_complete produces UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zsh_script_has_compdef_marker() {
        let script = generate_completion_script(Shell::Zsh);
        assert!(script.contains("#compdef ags"));
        assert!(script.contains("_ags"));
    }

    #[test]
    fn test_bash_script_has_complete_marker() {
        let script = generate_completion_script(Shell::Bash);
        assert!(script.contains("complete -"));
    }

    #[test]
    fn test_fish_script_has_complete_c_ags() {
        let script = generate_completion_script(Shell::Fish);
        assert!(script.contains("complete -c ags"));
    }

    #[test]
    fn test_powershell_script_has_register_marker() {
        let script = generate_completion_script(Shell::PowerShell);
        assert!(script.contains("Register-ArgumentCompleter"));
    }
}
