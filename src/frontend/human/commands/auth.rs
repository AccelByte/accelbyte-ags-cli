//! Human-readable rendering for auth command output.

use crate::errors::CliError;
use crate::frontend::human::templates;
use crate::frontend::presenters::auth as auth_presenter;
use crate::frontend::style;
use crate::frontend::RenderOptions;
use crate::frontend::RenderedOutput;
use crate::protocol::output::{
    AuthActionData, AuthOutput, AuthStatusData, AuthView, LogoutData, Presence, TokenState,
};

/// Render auth command output as human-readable text
pub(crate) fn render_auth_output(
    output: &AuthOutput,
    _options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let (stdout, stderr) = render_auth_view_text(&output.view);
    Ok(RenderedOutput {
        stdout: Some(stdout),
        stderr,
        is_stdout_first: true,
    })
}

/// Render an auth view as styled stdout headline and optional stderr details
fn render_auth_view_text(view: &AuthView) -> (String, Option<String>) {
    match view {
        AuthView::Authenticated(data) => (
            style::success("Authenticated", style::is_stdout_enabled()),
            Some(render_authenticated_details(data)),
        ),
        AuthView::RequiresAttention(data) => (
            style::warning(
                "Authentication requires attention",
                style::is_stdout_enabled(),
            ),
            Some(render_attention_details(data)),
        ),
        AuthView::LoginSuccess(data) => (
            style::success("Authenticated", style::is_stdout_enabled()),
            Some(render_login_success_details(data)),
        ),
        AuthView::LogoutSuccess(data) => (
            style::success("Credentials cleared", style::is_stdout_enabled()),
            Some(render_logout_details(data)),
        ),
        AuthView::LogoutAllSuccess(data) => {
            (render_logout_all_summary(&data.profiles_cleared), None)
        }
        AuthView::NotAuthenticated { next_step, tip } => {
            let mut stderr_lines = Vec::new();
            if let Some(next_step) = next_step {
                stderr_lines.push(format!("{} Next: {next_step}", style::fix_prefix()));
            }
            if let Some(tip) = tip {
                stderr_lines.push(templates::render_tip_text(tip, style::is_stderr_enabled()));
            }
            (
                style::error("Not authenticated", style::is_stdout_enabled()),
                Some(stderr_lines.join("\n")).filter(|s| !s.is_empty()),
            )
        }
    }
}

/// Format the credential detail block for an authenticated status
fn render_authenticated_details(data: &AuthStatusData) -> String {
    let rows = build_auth_status_rows(data);
    let mut lines = vec![templates::render_label_value_block_text(
        &rows,
        crate::frontend::style::Tone::Dim,
        style::is_stderr_enabled(),
    )];
    if let Some(next_step) = &data.next_step {
        lines.push(format!("{} Next: {next_step}", style::fix_prefix()));
    }
    lines.join("\n")
}

/// Format the credential detail block when attention is needed
fn render_attention_details(data: &AuthStatusData) -> String {
    let mut lines = vec![templates::render_label_value_block_text(
        &build_auth_status_rows(data),
        crate::frontend::style::Tone::Dim,
        style::is_stderr_enabled(),
    )];
    if let Some(next_step) = &data.next_step {
        lines.push(format!("{} Next: {next_step}", style::fix_prefix()));
    }
    lines.join("\n")
}

/// Format the detail block shown after a successful login
fn render_login_success_details(data: &AuthActionData) -> String {
    let mut rows = Vec::new();
    if let Some(base_url) = &data.base_url {
        rows.push(("Base URL:", base_url.clone()));
    }
    if let Some(login_type) = &data.login_type {
        rows.push(("Login Type:", login_type.clone()));
    }
    if let Some(client_id) = &data.client_id {
        rows.push(("Client ID:", client_id.clone()));
    }
    if let Some(token_expires_in_secs) = data.token_expires_in_secs {
        rows.push((
            "Token Expires:",
            crate::support::format_duration(token_expires_in_secs),
        ));
    }

    let mut lines = Vec::new();
    if !rows.is_empty() {
        lines.push(templates::render_label_value_block_text(
            &rows,
            crate::frontend::style::Tone::Dim,
            style::is_stderr_enabled(),
        ));
    }
    if let Some(tip) = &data.tip {
        lines.push(templates::render_tip_text(tip, style::is_stderr_enabled()));
    }
    lines.join("\n")
}

/// Format the credential clearing summary after logout
fn render_logout_details(data: &LogoutData) -> String {
    templates::render_label_value_block_text(
        &[
            (
                "Client ID:",
                auth_presenter::presence_label(data.client_id).to_string(),
            ),
            (
                "Client Secret:",
                auth_presenter::presence_label(data.client_secret).to_string(),
            ),
            (
                "Access Token:",
                auth_presenter::presence_label(data.access_token).to_string(),
            ),
            (
                "Refresh Token:",
                auth_presenter::presence_label(data.refresh_token).to_string(),
            ),
        ],
        crate::frontend::style::Tone::Dim,
        style::is_stderr_enabled(),
    )
}

/// Render the human summary line listing every profile cleared by `auth logout --all`.
fn render_logout_all_summary(profiles: &[String]) -> String {
    let color = style::is_stdout_enabled();
    if profiles.is_empty() {
        return style::info("No profiles to clear", color);
    }
    let count = profiles.len();
    let noun = if count == 1 { "profile" } else { "profiles" };
    let mut lines = vec![style::success(
        &format!("Credentials cleared from {count} {noun}"),
        color,
    )];
    for name in profiles {
        lines.push(format!("    {name}"));
    }
    lines.join("\n")
}

/// Build label-value rows from auth status data for the detail block
fn build_auth_status_rows(data: &AuthStatusData) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();

    if let Some(source_label) = auth_presenter::human_source_label(data.source) {
        rows.push(("Source:", source_label.to_string()));
    }
    if auth_presenter::is_source_only_status(data.source) {
        return rows;
    }

    if let Some(base_url) = &data.base_url {
        rows.push(("Base URL:", base_url.clone()));
    }
    if let Some(login_type) = &data.login_type {
        rows.push(("Login Type:", login_type.clone()));
    }
    if let Some(client_id) = &data.client_id {
        rows.push(("Client ID:", client_id.clone()));
    }
    if !matches!(data.client_secret, Presence::Unknown) {
        rows.push((
            "Client Secret:",
            auth_presenter::presence_label(data.client_secret).to_string(),
        ));
    }

    if !matches!(data.access_token, TokenState::Unknown) {
        rows.push((
            "Access Token:",
            format_token_state(&data.access_token, style::is_stderr_enabled()),
        ));
    }
    if !matches!(data.refresh_token, TokenState::Unknown) {
        rows.push((
            "Refresh Token:",
            format_token_state(&data.refresh_token, style::is_stderr_enabled()),
        ));
    }
    if let Some(namespace) = &data.namespace {
        rows.push(("Namespace:", namespace.clone()));
    }

    rows
}

/// Render a token state with color coding (green for valid, red for expired)
fn format_token_state(state: &TokenState, color_enabled: bool) -> String {
    if let Some(expires_in_secs) = auth_presenter::token_expiry_secs(state) {
        return style::apply_tone(
            &format!(
                "valid (expires in {})",
                crate::support::format_duration(expires_in_secs)
            ),
            style::Tone::Success,
            color_enabled,
        );
    }

    match state {
        TokenState::Valid { .. } => style::apply_tone(
            auth_presenter::token_state_label(state),
            style::Tone::Success,
            color_enabled,
        ),
        TokenState::Expired => style::apply_tone(
            auth_presenter::token_state_label(state),
            style::Tone::Error,
            color_enabled,
        ),
        TokenState::Missing | TokenState::Present | TokenState::Unknown => {
            auth_presenter::token_state_label(state).to_string()
        }
    }
}
