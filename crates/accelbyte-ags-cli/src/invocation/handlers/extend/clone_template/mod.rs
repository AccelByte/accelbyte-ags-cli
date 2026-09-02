//! Handler for `ags extend clone-template`.
//!
//! Clones a starter template repository and optionally extracts a sub-path
//! into the destination directory. Supports both interactive (prompt-driven
//! selection through scenario → template → language) and non-interactive
//! (`--template <name>`) modes so the command is usable in scripts and CI.

mod source_path;
pub(crate) mod starters;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::ArgMatches;

use crate::errors::CliError;
use crate::frontend::style;
use crate::frontend::{write_stderr, write_stderr_line};
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use ags_protocol::output::{CloneTemplateOutput, CommandOutput};
use ags_runtime::support::strings::{strip_terminal_control_sequences, truncate_display_text};

/// Execute the `clone-template` command.
///
/// Args are pre-parsed by the `extend` dispatcher. Resolves a starter
/// template, confirms the destructive operation, clones, and emits
/// structured output through the frontend.
pub(crate) fn handle_clone_template(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let template_name = matches.get_one::<String>("template").cloned();
    let destination_arg = matches.get_one::<String>("destination").cloned();
    let depth = matches.get_one::<u32>("depth").copied();

    let all_starters = starters::load_bundled_starters();
    if all_starters.is_empty() {
        return Err(CliError::Internal(anyhow::anyhow!(
            "No starter templates available"
        )));
    }

    let starter = resolve_starter(&all_starters, template_name.as_deref(), flags)?;
    let destination = resolve_destination(&destination_arg, &starter.url)?;
    let clone_depth = depth.unwrap_or(1);

    // Dry-run: preview what would happen without performing any I/O.
    if flags.is_dry_run {
        return dry_run_preview(&starter, &destination, clone_depth);
    }

    validate_destination(&destination)?;

    // Gate the destructive clone behind confirmation.
    confirm_clone(&starter.name, &destination, flags)?;

    clone_repository(&starter.url, &destination, clone_depth)?;

    if let Some(ref sp) = starter.source_path {
        source_path::extract_source_path(&destination, sp)?;
    }

    let output = CloneTemplateOutput {
        template_name: starter.name.clone(),
        destination: destination.display().to_string(),
        source_path: starter.source_path.clone(),
    };
    frontend.render(&CommandOutput::CloneTemplate(output))?;

    Ok(InvocationOutcome::Complete)
}

// ── Confirmation gate ──

/// Confirm the destructive clone operation according to the interaction mode.
///
/// - `--yes` skips confirmation.
/// - `--no-input` without `--yes` rejects (cannot prompt).
/// - Interactive mode prompts on stderr.
fn confirm_clone(
    template_name: &str,
    destination: &Path,
    flags: &GlobalFlags,
) -> Result<(), CliError> {
    confirm_clone_impl(template_name, destination, flags, &mut read_line_from_stdin)
}

/// Inner implementation with an injected line reader so tests never touch stdin.
fn confirm_clone_impl(
    template_name: &str,
    destination: &Path,
    flags: &GlobalFlags,
    read: &mut dyn FnMut() -> Result<String, CliError>,
) -> Result<(), CliError> {
    if flags.is_auto_confirmed {
        return Ok(());
    }

    if flags.is_no_input {
        return Err(CliError::Usage {
            message: "This command writes to disk and requires confirmation".to_string(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Use --yes to confirm in non-interactive mode",
            ))),
        });
    }

    let display_path = destination.display();
    write_stderr(&format!(
        "Clone '{template_name}' to {display_path}? [y/N] "
    ));

    let input = read()?;

    if !matches!(input.as_str(), "y" | "Y") {
        return Err(CliError::Usage {
            message: "Operation cancelled".to_string(),
            metadata: None,
        });
    }

    Ok(())
}

// ── Dry-run preview ──

/// Emit a preview of what `clone-template` would do without performing the
/// clone or writing any files. Output goes through the stderr UI sink to
/// avoid bare print macros.
fn dry_run_preview(
    starter: &starters::Starter,
    destination: &Path,
    depth: u32,
) -> Result<InvocationOutcome, CliError> {
    let color = style::is_stderr_enabled();

    write_stderr_line(&style::info("Dry run — no files will be written", color));
    write_stderr_line(&format!("  Template:    {}", starter.name));
    write_stderr_line(&format!("  Destination: {}", destination.display()));

    // Show the git command that would execute.
    let mut git_args = vec!["git", "clone"];
    let depth_str = depth.to_string();
    if depth > 0 {
        git_args.push("--depth");
        git_args.push(&depth_str);
    }
    git_args.push("--quiet");
    git_args.push(&starter.url);
    let dest_display = destination.display().to_string();
    git_args.push(&dest_display);
    write_stderr_line(&format!("  Command:     {}", git_args.join(" ")));

    if let Some(ref sp) = starter.source_path {
        write_stderr_line(&format!("  Source path: {sp} (would be extracted)"));
    }

    Ok(InvocationOutcome::Complete)
}

// ── Template resolution ──

/// Resolve a starter template from the catalogue.
///
/// With `--template <name>`, finds the exact match (case-insensitive).
/// Without `--template`, prompts the user through scenario → template →
/// language narrowing.
fn resolve_starter(
    starters: &[starters::Starter],
    template_name: Option<&str>,
    flags: &GlobalFlags,
) -> Result<starters::Starter, CliError> {
    resolve_starter_impl(starters, template_name, flags, &mut read_line_from_stdin)
}

/// Inner implementation with an injected line reader so tests never touch stdin.
fn resolve_starter_impl(
    starters: &[starters::Starter],
    template_name: Option<&str>,
    flags: &GlobalFlags,
    read: &mut dyn FnMut() -> Result<String, CliError>,
) -> Result<starters::Starter, CliError> {
    if let Some(name) = template_name {
        return starters::find_by_name(starters, name).ok_or_else(|| {
            let available = starters
                .iter()
                .map(|s| format!("  {}", s.name))
                .collect::<Vec<_>>()
                .join("\n");
            CliError::Usage {
                message: format!("Unknown template: '{name}'"),
                metadata: Some(Box::new(crate::errors::ErrorMetadata {
                    suggestion: Some(format!("Available templates:\n{available}")),
                    ..Default::default()
                })),
            }
        });
    }

    // Interactive selection: scenario → template → language.
    let scenarios = starters::unique_scenarios(starters);
    let scenario = pick_one_impl("scenario", &scenarios, flags, read)?;
    let filtered = starters::filter_by_scenario(starters, &scenario);

    let templates = starters::unique_templates(&filtered);
    let template = pick_one_impl("template", &templates, flags, read)?;
    let filtered = starters::filter_by_template(&filtered, &template);

    let languages = starters::unique_languages(&filtered);
    let language = pick_one_impl("language", &languages, flags, read)?;
    let filtered = starters::filter_by_language(&filtered, &language);

    match filtered.len() {
        0 => Err(CliError::Usage {
            message: format!(
                "No starter matches scenario '{scenario}', template '{template}', language '{language}'"
            ),
            metadata: None,
        }),
        1 => {
            let starter = filtered.into_iter().next().ok_or_else(|| {
                CliError::Internal(anyhow::anyhow!(
                    "filtered list had len 1 but yielded no element"
                ))
            })?;
            Ok(starter)
        }
        _ => {
            let names: Vec<String> = filtered.iter().map(|s| s.name.clone()).collect();
            let name = pick_one_impl("starter", &names, flags, read)?;
            starters::find_by_name(&filtered, &name).ok_or_else(|| {
                CliError::Internal(anyhow::anyhow!("Selected starter not found: {name}"))
            })
        }
    }
}

/// Prompt the user to select one item from a list. If the list has exactly
/// one item, returns it without prompting. The injected `read` closure
/// supplies the user's input line; production callers pass
/// `read_line_from_stdin`, tests pass a scripted closure.
fn pick_one_impl(
    label: &str,
    choices: &[String],
    flags: &GlobalFlags,
    read: &mut dyn FnMut() -> Result<String, CliError>,
) -> Result<String, CliError> {
    if choices.is_empty() {
        return Err(CliError::Usage {
            message: format!("No {label} options available"),
            metadata: None,
        });
    }
    if choices.len() == 1 {
        return Ok(choices[0].clone());
    }

    // Check whether interactive input is allowed.
    if flags.is_no_input {
        return Err(CliError::Usage {
            message: format!(
                "Multiple {label} options available but interactive input is disabled"
            ),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Use --template <name> to select a template non-interactively",
            ))),
        });
    }

    write_stderr_line(&format!("Select {label}:"));
    for (i, choice) in choices.iter().enumerate() {
        write_stderr_line(&format!("  [{}] {choice}", i + 1));
    }
    write_stderr(&format!("Enter number (1-{}): ", choices.len()));

    let input = read()?;

    let index: usize = input.parse::<usize>().map_err(|_| CliError::Usage {
        message: format!("Invalid selection: '{input}'"),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            format!("Enter a number between 1 and {}", choices.len()),
        ))),
    })?;

    if index < 1 || index > choices.len() {
        return Err(CliError::Usage {
            message: format!("Selection out of range: {index}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                format!("Enter a number between 1 and {}", choices.len()),
            ))),
        });
    }

    Ok(choices[index - 1].clone())
}

// ── Destination resolution ──

/// Resolve the destination directory. Uses `--destination` if provided,
/// otherwise derives a name from the repository URL.
fn resolve_destination(explicit: &Option<String>, repo_url: &str) -> Result<PathBuf, CliError> {
    if let Some(dest) = explicit {
        return Ok(PathBuf::from(dest));
    }
    Ok(PathBuf::from(default_dest_from_url(repo_url)))
}

/// Derive a default destination directory name from a repository URL by
/// extracting the last path segment and stripping `.git`.
fn default_dest_from_url(url: &str) -> String {
    let last = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("repo");
    let name = last.strip_suffix(".git").unwrap_or(last);
    if name.is_empty() {
        "repo".to_string()
    } else {
        name.to_string()
    }
}

/// Validate that the destination does not already exist or is empty.
fn validate_destination(destination: &Path) -> Result<(), CliError> {
    if destination.exists() {
        let is_empty = destination
            .read_dir()
            .map(|mut d| d.next().is_none())
            .unwrap_or(false);
        if !is_empty {
            return Err(CliError::Usage {
                message: format!(
                    "Destination directory already exists and is not empty: {}",
                    destination.display()
                ),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Choose a different destination with --destination <path>, or remove the existing directory",
                ))),
            });
        }
    }
    Ok(())
}

// ── Git operations ──

/// Cap on git stderr bytes propagated into user-facing error strings so a
/// misbehaving remote returning a large error page cannot blow up the UI.
const GIT_STDERR_DISPLAY_LIMIT: usize = 512;

/// Maximum wall-clock time to wait for `git clone` to complete.
///
/// Template repositories are moderate-sized (Extend starter projects), and
/// `--depth 1` is the default, so most clones finish in under 30 seconds.
/// 5 minutes accommodates cloning over slow or high-latency links (e.g.
/// behind a corporate proxy) while still catching a truly hung connection
/// (e.g. the remote host is unreachable but TCP never times out).
const GIT_CLONE_TIMEOUT: Duration = Duration::from_secs(300);

/// Build the argument list for a `git clone` invocation.
///
/// When `depth` is 0, no `--depth` flag is emitted (full clone). Any positive
/// `depth` produces `--depth <n>`.
fn build_git_clone_args(url: &str, destination: &Path, depth: u32) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["clone".into()];

    if depth > 0 {
        args.push("--depth".into());
        args.push(depth.to_string().into());
    }

    // Suppress git's own progress output so it does not interleave with
    // the CLI's stderr formatting.
    args.push("--quiet".into());

    args.push(url.into());
    args.push(destination.as_os_str().to_owned());

    args
}

/// Configure a [`Command`] for subprocess execution where the child must
/// not read from the parent's stdin.
///
/// `spawn()` inherits the parent's stdin by default — unlike `output()`
/// which nulled it implicitly. A child that prompts for credentials (e.g.
/// `git clone` over HTTPS to a private repo with no embedded token) would
/// hang reading a stdin that will never answer in CI or any scripted run.
/// Explicitly nulling stdin ensures a credential prompt fails fast with
/// EOF rather than blocking for the full timeout.
fn configure_subprocess_stdio(cmd: &mut std::process::Command) {
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
}

/// Clone a git repository using the system `git` binary.
///
/// `NotFound` from the spawn is mapped to a "git is not installed" usage error,
/// avoiding a separate probe spawn (TOCTOU-free). The child is waited with a
/// bounded timeout so a hung connection cannot block the command indefinitely.
fn clone_repository(url: &str, destination: &Path, depth: u32) -> Result<(), CliError> {
    let args = build_git_clone_args(url, destination, depth);

    let mut cmd = std::process::Command::new("git");
    cmd.args(&args);
    configure_subprocess_stdio(&mut cmd);

    let color = style::is_stderr_enabled();
    write_stderr_line(&style::status("Cloning...", color));

    let child = cmd.spawn().map_err(map_git_spawn_error)?;
    let output = ags_runtime::support::process::wait_with_timeout(child, GIT_CLONE_TIMEOUT)
        .map_err(|e| map_git_clone_wait_error(e, url))?;

    if !output.status.success() {
        let stderr = sanitize_git_stderr(&output.stderr);
        return Err(CliError::Network {
            message: format!("git clone failed: {stderr}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check the repository URL and your network connection",
            ))),
        });
    }

    Ok(())
}

/// Map a [`WaitError`][ags_runtime::support::process::WaitError] from the
/// `git clone` child process into a [`CliError`].
///
/// The clone URL is available at the production call site and could carry
/// embedded credentials (`https://user:token@host/repo`). This function
/// receives it so tests can prove it never appears in the emitted error.
fn map_git_clone_wait_error(err: ags_runtime::support::process::WaitError, _url: &str) -> CliError {
    match err {
        ags_runtime::support::process::WaitError::TimedOut(d) => CliError::Network {
            message: format!("git clone timed out after {}s", d.as_secs()),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "The remote host did not respond in time. \
                 Check your network connection and verify the repository URL is reachable.",
            ))),
        },
        ags_runtime::support::process::WaitError::Wait(e) => CliError::Network {
            message: format!("Failed to wait for git clone: {e}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Ensure 'git' is installed and available on your PATH",
            ))),
        },
    }
}

/// Map a `git` spawn error to the appropriate `CliError`.
///
/// `NotFound` means the `git` binary is missing — a usage-level problem.
/// Other I/O errors (permission denied, broken pipe, etc.) are transport
/// failures.
fn map_git_spawn_error(e: std::io::Error) -> CliError {
    if e.kind() == std::io::ErrorKind::NotFound {
        CliError::Usage {
            message: "git is not installed or not found on PATH".to_string(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Install git from https://git-scm.com/ and ensure it is on your PATH",
            ))),
        }
    } else {
        CliError::Network {
            message: format!("Failed to run git clone: {e}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Ensure 'git' is installed and available on your PATH",
            ))),
        }
    }
}

/// Sanitize raw git stderr for safe terminal display.
///
/// Strips ANSI escape sequences and control characters to prevent injection,
/// then truncates to a bounded display length.
fn sanitize_git_stderr(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let stripped = strip_terminal_control_sequences(text.trim());
    truncate_display_text(&stripped, GIT_STDERR_DISPLAY_LIMIT)
}

/// Read and trim one line from stdin.
fn read_line_from_stdin() -> Result<String, CliError> {
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| CliError::Usage {
            message: format!("Failed to read input: {e}"),
            metadata: None,
        })?;
    Ok(input.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test helpers ──

    /// Build `GlobalFlags` with the specified confirmation and input modes.
    fn test_flags(is_auto_confirmed: bool, is_no_input: bool) -> GlobalFlags {
        GlobalFlags {
            is_auto_confirmed,
            is_no_input,
            ..Default::default()
        }
    }

    /// Build a synthetic `Starter` for unit tests (no git/network dependency).
    fn synthetic_starter(
        name: &str,
        scenario: &str,
        template: &str,
        language: &str,
    ) -> starters::Starter {
        starters::Starter {
            name: name.to_string(),
            url: format!("https://example.com/{name}.git"),
            scenario: scenario.to_string(),
            template: template.to_string(),
            language: language.to_string(),
            source_path: None,
        }
    }

    /// Create a scripted reader that yields answers in order. Panics if
    /// called more times than answers were provided.
    fn scripted<'a>(answers: &'a [&'a str]) -> impl FnMut() -> Result<String, CliError> + 'a {
        let mut iter = answers.iter();
        move || {
            let answer = iter.next().expect("scripted reader exhausted");
            Ok(answer.to_string())
        }
    }

    // ── default_dest_from_url ──

    #[test]
    fn test_default_dest_from_url_strips_git_suffix() {
        assert_eq!(
            default_dest_from_url("https://github.com/AccelByte/extend-event-handler-go.git"),
            "extend-event-handler-go"
        );
    }

    #[test]
    fn test_default_dest_from_url_handles_no_suffix() {
        assert_eq!(
            default_dest_from_url("https://github.com/AccelByte/some-repo"),
            "some-repo"
        );
    }

    #[test]
    fn test_default_dest_from_url_handles_trailing_slash() {
        assert_eq!(
            default_dest_from_url("https://github.com/AccelByte/some-repo/"),
            "some-repo"
        );
    }

    #[test]
    fn test_default_dest_from_url_fallback() {
        assert_eq!(default_dest_from_url(""), "repo");
    }

    // ── validate_destination ──

    #[test]
    fn test_validate_destination_nonexistent_is_ok() {
        let dir = tempfile::TempDir::new().unwrap();
        let dest = dir.path().join("nonexistent");
        assert!(validate_destination(&dest).is_ok());
    }

    #[test]
    fn test_validate_destination_empty_dir_is_ok() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(validate_destination(dir.path()).is_ok());
    }

    #[test]
    fn test_validate_destination_non_empty_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("file.txt"), "content").unwrap();
        let result = validate_destination(dir.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not empty"));
    }

    // ── Git spawn error mapping ──

    /// `NotFound` from a git spawn maps to a Usage error telling the user
    /// to install git — the binary is missing, not a transport failure.
    #[test]
    fn test_git_spawn_not_found_maps_to_usage_error() {
        let err = map_git_spawn_error(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No such file or directory",
        ));
        assert!(
            matches!(err, CliError::Usage { .. }),
            "NotFound should produce Usage, got: {err:?}"
        );
        assert!(
            err.to_string().contains("not installed"),
            "message should mention installation: {}",
            err
        );
    }

    /// A non-NotFound I/O error (e.g. permission denied) maps to a Network
    /// error — the binary may exist but could not be spawned.
    #[test]
    fn test_git_spawn_other_error_maps_to_network_error() {
        let err = map_git_spawn_error(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "access denied",
        ));
        assert!(
            matches!(err, CliError::Network { .. }),
            "PermissionDenied should produce Network, got: {err:?}"
        );
        assert!(
            err.to_string().contains("Failed to run git clone"),
            "message should describe spawn failure: {}",
            err
        );
    }

    // ── Stderr sanitisation ──

    /// ANSI escape sequences in git stderr are stripped before embedding in
    /// the error message so untrusted external text cannot inject terminal
    /// control sequences.
    #[test]
    fn test_sanitize_git_stderr_strips_ansi_sequences() {
        let raw = b"\x1b[31mfatal: repository not found\x1b[0m";
        let sanitized = sanitize_git_stderr(raw);
        assert_eq!(sanitized, "fatal: repository not found");
        assert!(!sanitized.contains('\x1b'), "ANSI escapes must be stripped");
    }

    /// Very long git stderr is truncated to prevent unbounded error messages.
    #[test]
    fn test_sanitize_git_stderr_truncates_long_output() {
        let long_message = "x".repeat(2000);
        let sanitized = sanitize_git_stderr(long_message.as_bytes());
        assert!(
            sanitized.len() < 2000,
            "long stderr should be truncated, got {} bytes",
            sanitized.len()
        );
        assert!(
            sanitized.contains("(truncated)"),
            "truncated output should include marker"
        );
    }

    // ── Dry-run preview ──

    /// `dry_run_preview` returns success without creating the destination
    /// directory and without attempting any git operation.
    #[test]
    fn test_dry_run_preview_does_not_create_destination() {
        let dir = tempfile::TempDir::new().unwrap();
        let dest = dir.path().join("should-not-exist");

        let starter = starters::Starter {
            name: "Test :: Template :: Go".to_string(),
            url: "https://github.com/example/repo.git".to_string(),
            scenario: "Test".to_string(),
            template: "Template".to_string(),
            language: "Go".to_string(),
            source_path: None,
        };

        let result = dry_run_preview(&starter, &dest, 1);
        assert!(result.is_ok(), "dry-run preview should succeed");
        assert!(!dest.exists(), "destination must not be created in dry-run");
    }

    // ── confirm_clone ──

    #[test]
    fn test_confirm_clone_auto_confirmed_skips_prompt() {
        let flags = test_flags(true, false);
        let result = confirm_clone_impl("template", Path::new("/tmp/dest"), &flags, &mut || {
            panic!("should not read when auto-confirmed")
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_confirm_clone_no_input_without_yes_rejects() {
        let flags = test_flags(false, true);
        let result = confirm_clone_impl("template", Path::new("/tmp/dest"), &flags, &mut || {
            panic!("should not read in no-input mode")
        });
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("requires confirmation"),
            "expected 'requires confirmation' in: {msg}"
        );
    }

    #[test]
    fn test_confirm_clone_interactive_y_proceeds() {
        let flags = test_flags(false, false);
        let result = confirm_clone_impl(
            "template",
            Path::new("/tmp/dest"),
            &flags,
            &mut scripted(&["y"]),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_confirm_clone_interactive_uppercase_y_proceeds() {
        let flags = test_flags(false, false);
        let result = confirm_clone_impl(
            "template",
            Path::new("/tmp/dest"),
            &flags,
            &mut scripted(&["Y"]),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_confirm_clone_interactive_n_cancels() {
        let flags = test_flags(false, false);
        let result = confirm_clone_impl(
            "template",
            Path::new("/tmp/dest"),
            &flags,
            &mut scripted(&["n"]),
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("cancelled"), "expected 'cancelled' in: {msg}");
    }

    #[test]
    fn test_confirm_clone_interactive_empty_cancels() {
        let flags = test_flags(false, false);
        let result = confirm_clone_impl(
            "template",
            Path::new("/tmp/dest"),
            &flags,
            &mut scripted(&[""]),
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("cancelled"), "expected 'cancelled' in: {msg}");
    }

    // ── pick_one ──

    #[test]
    fn test_pick_one_empty_choices_returns_error() {
        let flags = test_flags(false, false);
        let result = pick_one_impl("widget", &[], &flags, &mut || {
            panic!("should not read for empty choices")
        });
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("No widget options"),
            "expected 'No widget options' in: {msg}"
        );
    }

    #[test]
    fn test_pick_one_single_choice_returns_without_prompt() {
        let flags = test_flags(false, false);
        let choices = vec!["alpha".to_string()];
        let result = pick_one_impl("item", &choices, &flags, &mut || {
            panic!("should not read for single choice")
        });
        assert_eq!(result.unwrap(), "alpha");
    }

    #[test]
    fn test_pick_one_valid_selection_returns_correct_choice() {
        let flags = test_flags(false, false);
        let choices = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        let result = pick_one_impl("item", &choices, &flags, &mut scripted(&["2"]));
        assert_eq!(result.unwrap(), "beta");
    }

    #[test]
    fn test_pick_one_first_item_selection() {
        let flags = test_flags(false, false);
        let choices = vec!["alpha".to_string(), "beta".to_string()];
        let result = pick_one_impl("item", &choices, &flags, &mut scripted(&["1"]));
        assert_eq!(result.unwrap(), "alpha");
    }

    #[test]
    fn test_pick_one_last_item_selection() {
        let flags = test_flags(false, false);
        let choices = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        let result = pick_one_impl("item", &choices, &flags, &mut scripted(&["3"]));
        assert_eq!(result.unwrap(), "gamma");
    }

    #[test]
    fn test_pick_one_zero_index_rejected() {
        let flags = test_flags(false, false);
        let choices = vec!["alpha".to_string(), "beta".to_string()];
        let result = pick_one_impl("item", &choices, &flags, &mut scripted(&["0"]));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("out of range"),
            "expected 'out of range' in: {msg}"
        );
    }

    #[test]
    fn test_pick_one_index_exceeding_len_rejected() {
        let flags = test_flags(false, false);
        let choices = vec!["alpha".to_string(), "beta".to_string()];
        let result = pick_one_impl("item", &choices, &flags, &mut scripted(&["3"]));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("out of range"),
            "expected 'out of range' in: {msg}"
        );
    }

    #[test]
    fn test_pick_one_non_numeric_input_rejected() {
        let flags = test_flags(false, false);
        let choices = vec!["alpha".to_string(), "beta".to_string()];
        let result = pick_one_impl("item", &choices, &flags, &mut scripted(&["abc"]));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Invalid selection"),
            "expected 'Invalid selection' in: {msg}"
        );
    }

    #[test]
    fn test_pick_one_no_input_mode_rejects_multi_choice() {
        let flags = test_flags(false, true);
        let choices = vec!["alpha".to_string(), "beta".to_string()];
        let result = pick_one_impl("item", &choices, &flags, &mut || {
            panic!("should not read in no-input mode")
        });
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("interactive input is disabled"),
            "expected 'interactive input is disabled' in: {msg}"
        );
    }

    // ── resolve_starter ──

    #[test]
    fn test_resolve_starter_by_name_exact_match() {
        let starters = vec![
            synthetic_starter("Go Template", "Override", "Lootbox", "Go"),
            synthetic_starter("Java Template", "Override", "Lootbox", "Java"),
        ];
        let result = resolve_starter_impl(
            &starters,
            Some("Go Template"),
            &test_flags(false, false),
            &mut || panic!("should not prompt for named lookup"),
        );
        assert_eq!(result.unwrap().name, "Go Template");
    }

    #[test]
    fn test_resolve_starter_by_name_unknown_returns_error() {
        let starters = vec![synthetic_starter(
            "Go Template",
            "Override",
            "Lootbox",
            "Go",
        )];
        let result = resolve_starter_impl(
            &starters,
            Some("nonexistent"),
            &test_flags(false, false),
            &mut || panic!("should not prompt"),
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Unknown template"),
            "expected 'Unknown template' in: {msg}"
        );
    }

    #[test]
    fn test_resolve_starter_single_path_no_prompt_needed() {
        // Only one scenario, one template, one language: auto-selected at every level.
        let starters = vec![synthetic_starter(
            "Only One",
            "ScenarioA",
            "TemplateA",
            "Go",
        )];
        let result = resolve_starter_impl(&starters, None, &test_flags(false, false), &mut || {
            panic!("should not prompt when every level has exactly one choice")
        });
        assert_eq!(result.unwrap().name, "Only One");
    }

    #[test]
    fn test_resolve_starter_multi_scenario_prompts_and_narrows() {
        let starters = vec![
            synthetic_starter("A :: T :: Go", "A", "T", "Go"),
            synthetic_starter("B :: T :: Go", "B", "T", "Go"),
        ];
        // Two scenarios: prompt needed. Select "2" (scenario B).
        // After narrowing: one template, one language: auto-selected.
        let result = resolve_starter_impl(
            &starters,
            None,
            &test_flags(false, false),
            &mut scripted(&["2"]),
        );
        assert_eq!(result.unwrap().name, "B :: T :: Go");
    }

    #[test]
    fn test_resolve_starter_multi_level_prompts() {
        // Two scenarios, two templates under scenario A, two languages under T1.
        let starters = vec![
            synthetic_starter("A :: T1 :: Go", "A", "T1", "Go"),
            synthetic_starter("A :: T1 :: Java", "A", "T1", "Java"),
            synthetic_starter("A :: T2 :: Go", "A", "T2", "Go"),
            synthetic_starter("B :: T3 :: Go", "B", "T3", "Go"),
        ];
        // Pick scenario A ("1"), template T1 ("1"), language Java ("2").
        let result = resolve_starter_impl(
            &starters,
            None,
            &test_flags(false, false),
            &mut scripted(&["1", "1", "2"]),
        );
        assert_eq!(result.unwrap().name, "A :: T1 :: Java");
    }

    // ── build_git_clone_args ──

    #[test]
    fn test_git_args_default_depth_appends_depth_flag() {
        let args = build_git_clone_args("https://example.com/repo.git", Path::new("/tmp/dest"), 1);
        let strings: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(strings.contains(&"--depth".to_string()));
        assert!(strings.contains(&"1".to_string()));
    }

    #[test]
    fn test_git_args_depth_zero_omits_depth_flag() {
        let args = build_git_clone_args("https://example.com/repo.git", Path::new("/tmp/dest"), 0);
        let strings: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            !strings.contains(&"--depth".to_string()),
            "depth 0 should produce a full clone (no --depth flag)"
        );
    }

    #[test]
    fn test_git_args_depth_five_appends_depth_five() {
        let args = build_git_clone_args("https://example.com/repo.git", Path::new("/tmp/dest"), 5);
        let strings: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let depth_pos = strings.iter().position(|s| s == "--depth").unwrap();
        assert_eq!(strings[depth_pos + 1], "5");
    }

    /// The builder never produces branch flags — the branch parameter was
    /// removed as dead functionality (no caller or CLI flag supplies it).
    #[test]
    fn test_git_args_never_includes_branch_flags() {
        let args = build_git_clone_args("https://example.com/repo.git", Path::new("/tmp/dest"), 1);
        let strings: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(!strings.contains(&"--branch".to_string()));
        assert!(!strings.contains(&"--single-branch".to_string()));
    }

    #[test]
    fn test_git_args_always_includes_quiet() {
        let args = build_git_clone_args("https://example.com/repo.git", Path::new("/tmp/dest"), 0);
        let strings: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(strings.contains(&"--quiet".to_string()));
    }

    #[test]
    fn test_git_args_url_and_destination_are_last_two() {
        let args = build_git_clone_args("https://example.com/repo.git", Path::new("/tmp/dest"), 1);
        let len = args.len();
        assert_eq!(
            args[len - 2].to_string_lossy(),
            "https://example.com/repo.git"
        );
        assert_eq!(args[len - 1].to_string_lossy(), "/tmp/dest");
    }

    // ── Git clone timeout ──

    /// The timeout constant must be long enough for large template clones
    /// over slow links but short enough to catch a truly hung connection.
    #[test]
    fn test_git_clone_timeout_is_reasonable() {
        assert!(
            GIT_CLONE_TIMEOUT >= std::time::Duration::from_secs(60),
            "timeout too short for slow-network clones"
        );
        assert!(
            GIT_CLONE_TIMEOUT <= std::time::Duration::from_secs(600),
            "timeout too long — a hung clone should not block for 10+ minutes"
        );
    }

    /// The timeout error maps to `CliError::Network` (exit code 4) and
    /// includes the program name and elapsed limit in the message.
    #[test]
    fn test_git_clone_timeout_produces_network_error() {
        // Call the production error-mapping function.
        let cli_err = map_git_clone_wait_error(
            ags_runtime::support::process::WaitError::TimedOut(std::time::Duration::from_secs(300)),
            "https://example.com/repo.git",
        );
        assert!(
            matches!(cli_err, CliError::Network { .. }),
            "timeout should map to Network, got: {cli_err:?}"
        );
        assert_eq!(cli_err.exit_code(), 4);
        let msg = cli_err.to_string();
        assert!(
            msg.contains("timed out"),
            "message should mention timeout: {msg}"
        );
        assert!(
            msg.contains("300s"),
            "message should include the duration: {msg}"
        );
    }

    /// The `WaitError::Wait` arm maps to `CliError::Network` with a message
    /// describing the OS-level failure.
    #[test]
    fn test_git_clone_wait_error_maps_to_network_error() {
        let io_err = std::io::Error::other("waitpid failed");
        let cli_err = map_git_clone_wait_error(
            ags_runtime::support::process::WaitError::Wait(io_err),
            "https://example.com/repo.git",
        );
        assert!(
            matches!(cli_err, CliError::Network { .. }),
            "Wait arm should map to Network, got: {cli_err:?}"
        );
        assert_eq!(cli_err.exit_code(), 4);
        let msg = cli_err.to_string();
        assert!(
            msg.contains("Failed to wait"),
            "message should describe the wait failure: {msg}"
        );
    }

    // ── Subprocess stdio configuration ──

    /// The production [`configure_subprocess_stdio`] function sets stdin to
    /// null so a child process that tries to read stdin gets immediate EOF
    /// instead of blocking on the parent's stdin.
    ///
    /// The test pre-sets stdin to `Stdio::piped()` (a pipe whose write end
    /// the parent holds open, so the child would block reading from it
    /// indefinitely). `configure_subprocess_stdio` must override that with
    /// `Stdio::null()`. Removing the null-stdin line causes the child to
    /// keep the blocking pipe and time out — proving the test is
    /// non-tautological.
    #[test]
    fn test_configure_subprocess_stdio_nulls_stdin() {
        // A command that reads from stdin until EOF, then exits.
        #[cfg(windows)]
        let mut cmd = {
            // `findstr` reads stdin line by line; at EOF it exits.
            let mut c = std::process::Command::new("findstr");
            c.arg(".");
            c
        };
        #[cfg(not(windows))]
        let mut cmd = std::process::Command::new("cat");

        // Pre-set stdin to a pipe — the child would block reading from it
        // because the parent holds the write end open and never writes.
        // configure_subprocess_stdio must override this with Stdio::null().
        cmd.stdin(std::process::Stdio::piped());
        configure_subprocess_stdio(&mut cmd);

        let child = cmd.spawn().expect("test child must spawn");

        // 5 seconds is generous — with null stdin the child exits in
        // milliseconds. A regression (piped stdin, never written to)
        // blocks for the full timeout.
        let result = ags_runtime::support::process::wait_with_timeout(
            child,
            std::time::Duration::from_secs(5),
        );

        assert!(
            result.is_ok(),
            "child reading stdin must exit before timeout; \
             a TimedOut error means stdin was not overridden to null"
        );
    }

    /// The timeout error message never leaks repository credentials.
    /// The test calls the production `map_git_clone_wait_error` function
    /// with credential-bearing URLs flowing through the same parameter
    /// path as a live invocation. If the mapping ever interpolates the
    /// URL into the error, this test catches it.
    #[test]
    fn test_git_clone_timeout_error_does_not_leak_url_credentials() {
        // URLs with embedded credentials — the pathological case.
        let urls = [
            "https://admin:super-secret-token@git.example.com/repo.git",
            "https://deploy:p%40%24%24w0rd@git.example.com/repo.git",
            // Safe: synthetic all-X placeholder in a test fixture; the assertion below proves this value never leaves the process.
            // nosemgrep: detected-github-token
            "https://ci-bot:ghp_XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX@github.com/org/repo.git",
        ];
        for url in &urls {
            let err = map_git_clone_wait_error(
                ags_runtime::support::process::WaitError::TimedOut(std::time::Duration::from_secs(
                    300,
                )),
                url,
            );
            let full_text = format!("{err:?}");
            assert!(
                !full_text.contains("super-secret-token"),
                "timeout error must not contain credentials from URL, found in: {full_text}"
            );
            assert!(
                !full_text.contains("p%40%24%24w0rd"),
                "timeout error must not contain URL-encoded credentials, found in: {full_text}"
            );
            assert!(
                !full_text.contains("ghp_"),
                "timeout error must not contain token prefixes, found in: {full_text}"
            );
        }
    }
}
