//! Auth subcommands: login, logout, status.

mod oauth;

use clap::ArgMatches;

use crate::errors::CliError;
use crate::frontend::human::progress;
use crate::frontend::style;
use crate::invocation::builder;
use crate::invocation::clap_helpers;
use crate::invocation::flags::GlobalFlags;
use crate::invocation::InvocationOutcome;
use crate::protocol::output::{AuthOutput, CommandOutput};
use crate::runtime::config;
use crate::runtime::dispatch::http::build_http_client;
use crate::support::strings::strip_terminal_control_sequences;

use crate::protocol::error::RuntimeError;
use crate::runtime::auth::errors::AuthError;

/// Route `ags auth <subcommand>` to the appropriate handler.
pub(crate) async fn handle_auth(
    args: &[String],
    flags: &GlobalFlags,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<InvocationOutcome, CliError> {
    let mut command = builder::build_auth_command();
    let argv = clap_helpers::build_argv("auth", args);

    match command.try_get_matches_from_mut(argv.iter().map(String::as_str)) {
        Ok(matches) => {
            let runtime = crate::runtime::Runtime::from_reqwest(
                crate::runtime::execution::ExecutionContext::default(),
                build_http_client(flags.timeout)?,
            );

            match matches.subcommand() {
                Some(("login", sub_matches)) => {
                    let profile = config::resolve_profile_name(flags.profile.as_deref())?;
                    let output =
                        handle_auth_login(sub_matches, flags, &profile, &runtime, frontend).await?;
                    frontend.render(&output, &crate::frontend::RenderOptions::from(flags))?;
                    Ok(InvocationOutcome::Complete)
                }
                Some(("logout", sub_matches)) => {
                    let output = handle_auth_logout(sub_matches, flags, &runtime).await?;
                    frontend.render(&output, &crate::frontend::RenderOptions::from(flags))?;
                    Ok(InvocationOutcome::Complete)
                }
                Some(("status", _)) => {
                    let profile = config::resolve_profile_name(flags.profile.as_deref())?;
                    let output = handle_auth_status(flags, &profile, &runtime).await?;
                    frontend.render(&output, &crate::frontend::RenderOptions::from(flags))?;
                    Ok(InvocationOutcome::Complete)
                }
                _ => {
                    let _ = command.print_help();
                    Ok(InvocationOutcome::Complete)
                }
            }
        }
        Err(error) => clap_helpers::outcome_from_clap_error(error),
    }
}

// ── Login ──

/// Handle `ags auth login`.
async fn handle_auth_login(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    profile: &str,
    runtime: &crate::runtime::Runtime,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<CommandOutput, CliError> {
    use crate::protocol::request::GrantType;

    let grant_type = matches
        .get_one::<GrantType>("grant")
        .copied()
        .unwrap_or(GrantType::AuthorizationCode);

    match grant_type {
        GrantType::ClientCredentials => {
            handle_login_with_client_credentials(matches, flags, profile, runtime, frontend).await
        }
        GrantType::AuthorizationCode => {
            // Authorization code login opens a browser and prints instructions
            // to stderr. That is incompatible with any non-interactive mode:
            // --no-input, or --format json which is inherently machine-driven.
            if flags.is_no_input
                || flags.format == Some(crate::protocol::request::OutputFormat::Json)
            {
                return Err(CliError::Usage {
                    message: "Authorization code flow requires browser interaction and cannot run non-interactively.\n\
                     Use '--grant client-credentials' for headless authentication."
                        .to_string(),
                    metadata: None,
                });
            }
            handle_login_with_browser(matches, flags, profile, runtime, frontend).await
        }
    }
}

/// Handle browser-based OAuth2 login (authorization code + PKCE).
///
/// The CLI handles all terminal interaction (printing the authorize URL,
/// running the callback server, displaying the wait spinner) and delegates
/// the token exchange + persistence to `operations::login_with_authorization_code`.
async fn handle_login_with_browser(
    matches: &ArgMatches,
    _flags: &GlobalFlags,
    profile: &str,
    runtime: &crate::runtime::Runtime,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<CommandOutput, CliError> {
    let flag_base_url = matches.get_one::<String>("base-url").cloned();
    let flag_client_id = matches.get_one::<String>("client-id").cloned();
    let callback_port = matches
        .get_one::<String>("port")
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8080);

    let base_url = resolve_login_value(
        flag_base_url,
        crate::runtime::auth::credentials::resolve_base_url_value(profile),
        false,
        "Enter Base URL (e.g. https://demo.accelbyte.io): ",
        "Base URL is required.",
        "When reading from stdin, --base-url or AGS_BASE_URL must be provided.",
    )?;

    let client_id = resolve_login_value(
        flag_client_id,
        crate::runtime::auth::credentials::resolve_client_id_value(profile),
        false,
        "Enter Client ID: ",
        "Client ID is required.",
        "When reading from stdin, --client-id or AGS_CLIENT_ID must be provided.",
    )?;

    // Generate PKCE pair and state
    let (code_verifier, code_challenge) = oauth::generate_pkce_pair();
    let state = oauth::generate_state();

    // Build the authorize URL first so an invalid base URL fails fast,
    // before we bind the callback port.
    let authorize_url = oauth::build_authorize_url(&base_url, &client_id, &state, &code_challenge)
        .map_err(crate::protocol::error::RuntimeError::from)?;

    // Bind callback port before printing the URL to avoid TOCTOU races
    let listener = oauth::bind_callback_port(callback_port).await?;

    crate::frontend::write_stderr_line(""); // blank line separator
    crate::frontend::write_stderr_line(&style::info(
        "Open this URL in your browser to authenticate",
        style::is_stderr_enabled(),
    ));
    crate::frontend::write_stderr_line(&format!("    {authorize_url}"));
    crate::frontend::write_stderr_line(""); // blank line separator

    let (code, returned_state) = await_callback(listener, callback_port).await?;

    if returned_state != state {
        return Err(RuntimeError::from(AuthError::OAuthStateMismatch).into());
    }

    // Exchange code for tokens via the runtime facade.
    let view = runtime
        .auth_login_authorization_code(
            profile,
            base_url,
            client_id,
            code,
            code_verifier,
            frontend.progress_sink(),
        )
        .await?;

    Ok(CommandOutput::Auth(AuthOutput { view }))
}

/// Handle client credentials login.
async fn handle_login_with_client_credentials(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    profile: &str,
    runtime: &crate::runtime::Runtime,
    frontend: &mut dyn crate::frontend::Frontend,
) -> Result<CommandOutput, CliError> {
    let flag_base_url = matches.get_one::<String>("base-url").cloned();
    let flag_client_id = matches.get_one::<String>("client-id").cloned();
    let flag_client_secret = matches.get_one::<String>("client-secret").cloned();
    let flag_client_secret_stdin = matches.get_flag("client-secret-stdin");

    if flag_client_secret.is_some() {
        frontend.render_warning(
            "--client-secret is visible in shell history. Use --client-secret-stdin for better security.",
            None,
            None,
        );
    }

    let is_prompt_blocked = flags.is_no_input || flag_client_secret_stdin;

    let base_url = resolve_login_value(
        flag_base_url,
        crate::runtime::auth::credentials::resolve_base_url_value(profile),
        is_prompt_blocked,
        "Enter Base URL (e.g. https://demo.accelbyte.io): ",
        "Base URL is required.",
        "Provide --base-url or set AGS_BASE_URL when using --no-input.",
    )?;

    let client_id = resolve_login_value(
        flag_client_id,
        crate::runtime::auth::credentials::resolve_client_id_value(profile),
        is_prompt_blocked,
        "Enter Client ID: ",
        "Client ID is required.",
        "Provide --client-id or set AGS_CLIENT_ID when using --no-input.",
    )?;

    let client_secret = resolve_client_secret_for_login(
        profile,
        flag_client_secret,
        flag_client_secret_stdin,
        is_prompt_blocked,
    )
    .await?;

    // Visual separator between credential input and the login progress spinner.
    crate::frontend::write_stderr_line("");

    let view = runtime
        .auth_login_client_credentials(
            profile,
            base_url,
            client_id,
            client_secret,
            frontend.progress_sink(),
        )
        .await?;

    Ok(CommandOutput::Auth(AuthOutput { view }))
}

// ── Logout ──

/// Handle `ags auth logout` or `ags auth logout --all`.
async fn handle_auth_logout(
    matches: &ArgMatches,
    flags: &GlobalFlags,
    runtime: &crate::runtime::Runtime,
) -> Result<CommandOutput, CliError> {
    let is_all = matches.get_flag("all");

    if is_all && flags.profile.is_some() {
        return Err(CliError::Usage {
            message: "--all and --profile are mutually exclusive.".into(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Use --all to log out from every profile, or --profile to target one.",
            ))),
        });
    }

    if is_all {
        let view = runtime.auth_logout_all().await?;
        return Ok(CommandOutput::Auth(AuthOutput { view }));
    }

    let profile = config::resolve_profile_name(flags.profile.as_deref())?;
    let view = runtime.auth_logout(&profile).await?;
    Ok(CommandOutput::Auth(AuthOutput { view }))
}

// ── Status ──

/// Handle `ags auth status`.
///
/// Three-state display:
/// - Authenticated: valid access token
/// - RequiresAttention: credentials present but token expired
/// - NotAuthenticated: no credentials
async fn handle_auth_status(
    _flags: &GlobalFlags,
    profile: &str,
    runtime: &crate::runtime::Runtime,
) -> Result<CommandOutput, CliError> {
    let view = runtime.auth_status(profile)?;
    Ok(CommandOutput::Auth(AuthOutput { view }))
}

// ── Helpers ──

/// Seconds of waiting before the redirect-URI tip is printed.
const TIP_AFTER_SECS: u64 = 10;

/// Decide whether the redirect-URI tip should be emitted on this iteration.
///
/// One-shot: returns true only when the tip has not already been shown and
/// elapsed seconds have crossed `TIP_AFTER_SECS`.
fn is_tip_due(elapsed_secs: u64, is_tip_shown: bool) -> bool {
    !is_tip_shown && elapsed_secs >= TIP_AFTER_SECS
}

/// Wait for the OAuth callback server to receive the authorization code.
async fn await_callback(
    listener: tokio::net::TcpListener,
    callback_port: u16,
) -> Result<(String, String), CliError> {
    let timeout_seconds: u64 = std::env::var(crate::runtime::config::ENV_AUTH_TIMEOUT)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    let server_handle = tokio::spawn(oauth::start_callback_server(listener, timeout_seconds));
    let start = std::time::Instant::now();

    let spinner_message = |remaining: u64| -> String {
        if remaining == 0 {
            format!("Listening on http://127.0.0.1:{callback_port} for browser callback... waiting")
        } else {
            format!(
                "Listening on http://127.0.0.1:{callback_port} for browser callback... {remaining}s remaining"
            )
        }
    };

    let mut status_line = progress::StatusLine::new(false);
    status_line.show(&spinner_message(timeout_seconds));

    let mut is_tip_shown = false;

    loop {
        if server_handle.is_finished() {
            status_line.clear();
            let result = server_handle
                .await
                .map_err(|e| RuntimeError::from(AuthError::CallbackServerError(e.to_string())))?;
            return Ok(result?);
        }
        let elapsed = start.elapsed().as_secs();
        let remaining = timeout_seconds.saturating_sub(elapsed);

        if is_tip_due(elapsed, is_tip_shown) {
            // Stop the running spinner before printing a persistent line —
            // otherwise the next blink would overwrite cursor position. After
            // `clear()`, the StatusLine becomes inert (is_active=false), so we
            // construct a fresh one to resume blinking below the tip.
            status_line.clear();
            // Render via the canonical Tip template so this stays in sync with
            // `frontend::templates::render_tip` (4-space indent + "Tip:" label
            // + dim tone). We can't use `frontend::Frontend::render_tip` here
            // because we're between spinner stop/restart and writing direct.
            let tip_body = format!(
                "Stuck here after signing in? Your AccelByte IAM client must \
                 be Public and allow http://127.0.0.1:{callback_port} as a \
                 redirect URI."
            );
            let tip = crate::frontend::human::templates::render_tip_text(
                &tip_body,
                style::is_stderr_enabled(),
            );
            crate::frontend::write_stderr_line(&tip);
            crate::frontend::write_stderr_line("");
            is_tip_shown = true;
            status_line = progress::StatusLine::new(false);
            status_line.show(&spinner_message(remaining));
        } else {
            status_line.update(&spinner_message(remaining));
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// Resolve client secret from: flag → stdin → environment → keychain → interactive prompt.
async fn resolve_client_secret_for_login(
    profile: &str,
    flag_value: Option<String>,
    is_from_stdin: bool,
    is_prompt_blocked: bool,
) -> Result<String, CliError> {
    if let Some(secret) = flag_value {
        return Ok(secret);
    }
    if is_from_stdin {
        return read_stdin_line();
    }
    if let Ok(secret) = std::env::var(crate::runtime::config::ENV_CLIENT_SECRET) {
        return Ok(secret);
    }
    if let Some(secret) =
        crate::runtime::auth::credentials::resolve_stored_client_secret(profile).await
    {
        return Ok(secret);
    }
    if is_prompt_blocked {
        return Err(CliError::Usage {
            message: "Client secret not found. Provide --client-secret, set AGS_CLIENT_SECRET, \
             or run 'ags auth login' first to store it in the keychain."
                .to_string(),
            metadata: None,
        });
    }
    let secret =
        rpassword::prompt_password("Enter Client Secret: ").map_err(|e| CliError::Usage {
            message: format!("Failed to read secret: {e}"),
            metadata: None,
        })?;
    if secret.is_empty() {
        return Err(CliError::Usage {
            message: "Client Secret is required.".to_string(),
            metadata: None,
        });
    }
    Ok(secret)
}

/// Read a single line from stdin, trimming whitespace and sanitizing control characters.
fn read_stdin_line() -> Result<String, CliError> {
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| CliError::Usage {
            message: format!("Failed to read from stdin: {e}"),
            metadata: None,
        })?;
    let value = strip_terminal_control_sequences(line.trim());
    if value.is_empty() {
        return Err(CliError::Usage {
            message: "Expected a value from stdin but got empty input.".to_string(),
            metadata: None,
        });
    }
    Ok(value)
}

/// Resolve a value from: flag -> stored (environment/config) -> interactive prompt.
fn resolve_login_value(
    flag_value: Option<String>,
    stored_value: Option<String>,
    is_prompt_blocked: bool,
    prompt: &str,
    empty_error: &str,
    blocked_error: &str,
) -> Result<String, CliError> {
    if let Some(value) = flag_value {
        return Ok(value);
    }
    if let Some(value) = stored_value {
        return Ok(value);
    }
    if is_prompt_blocked {
        return Err(CliError::Usage {
            message: blocked_error.to_string(),
            metadata: None,
        });
    }
    crate::frontend::write_stderr(prompt);
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| CliError::Usage {
            message: format!("Failed to read input: {e}"),
            metadata: None,
        })?;
    let value = strip_terminal_control_sequences(input.trim());
    if value.is_empty() {
        return Err(CliError::Usage {
            message: empty_error.to_string(),
            metadata: None,
        });
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{is_tip_due, TIP_AFTER_SECS};

    #[test]
    fn test_tip_not_due_before_threshold() {
        assert!(!is_tip_due(0, false));
        assert!(!is_tip_due(TIP_AFTER_SECS - 1, false));
    }

    #[test]
    fn test_tip_due_at_and_after_threshold_when_unshown() {
        assert!(is_tip_due(TIP_AFTER_SECS, false));
        assert!(is_tip_due(TIP_AFTER_SECS + 60, false));
    }

    #[test]
    fn test_tip_never_due_once_shown() {
        assert!(!is_tip_due(TIP_AFTER_SECS, true));
        assert!(!is_tip_due(TIP_AFTER_SECS + 60, true));
        assert!(!is_tip_due(u64::MAX, true));
    }
}
