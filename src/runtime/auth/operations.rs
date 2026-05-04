//! Auth operations: login, logout, and status.
//!
//! These functions sequence the lower-level primitives in
//! [`super::session`] and [`super::store`] into complete operations callable
//! from any invoker (CLI, daemon, AI adapter). Inputs are fully resolved —
//! callers handle stdin prompts and config lookups themselves. Outputs are
//! runtime-layer outcome types; the invocation layer maps them to render
//! types.
//!
//! Network-bound flows (`login_with_client_credentials`,
//! `login_with_authorization_code`) accept a `&mut dyn ProgressSink` so user-
//! visible warnings (e.g. "Server did not return an expiry") and progress
//! events flow through the same `ProgressSink` interface used by service
//! commands. Storage-only flows (`logout_profile`, `logout_all_profiles`,
//! `auth_snapshot`) take no sink because they emit no progress events.

use reqwest::Client;

use crate::protocol::error::RuntimeError;
use crate::protocol::event::ProgressSink;

use super::credentials;
use super::session;
use super::store;
use super::tokens;

// ── Input types ──

/// Inputs needed to perform a client-credentials login.
#[derive(Debug)]
pub struct ClientCredentialsLogin {
    pub profile: String,
    pub base_url: String,
    pub client_id: String,
    pub client_secret: String,
}

/// Inputs needed to complete an authorization-code login. The caller has
/// already obtained the authorization `code` from the OAuth callback server.
#[derive(Debug)]
pub struct AuthorizationCodeLogin {
    pub profile: String,
    pub base_url: String,
    pub client_id: String,
    pub code: String,
    pub code_verifier: String,
}

// ── Outcome types ──

/// Result of a successful login attempt.
#[derive(Debug)]
pub struct LoginOutcome {
    pub kind: LoginOutcomeKind,
    pub base_url: String,
    pub client_id: String,
    /// Friendly login-type label, e.g. `"client credentials"` or
    /// `"authorization code"`. Used by the renderer to display the login type.
    pub login_type: &'static str,
    /// Token lifetime in seconds. `0` if the user was already authenticated
    /// (no new token was fetched).
    pub expires_in_secs: u64,
}

/// What the login function actually did.
#[derive(Debug)]
pub enum LoginOutcomeKind {
    /// A new token was fetched and stored.
    LoggedIn,
    /// A valid (or refreshable) session already existed; no work was done.
    /// `tip` is the message to surface to the user.
    AlreadyAuthenticated { tip: String },
}

/// Result of clearing credentials for a single profile.
#[derive(Debug, PartialEq)]
pub struct LogoutOutcome {
    pub had_client_id: bool,
    pub had_client_secret: bool,
    pub had_access_token: bool,
    pub had_refresh_token: bool,
}

/// Result of clearing credentials from every known profile.
#[derive(Debug, PartialEq)]
pub struct LogoutAllOutcome {
    pub profiles_cleared: Vec<String>,
}

/// A snapshot of current auth state for status display.
#[derive(Debug)]
pub enum AuthSnapshot {
    /// `AGS_ACCESS_TOKEN` is set in the environment.
    EnvironmentToken,
    /// `AGS_CLIENT_ID` and `AGS_CLIENT_SECRET` are set in the environment.
    EnvironmentCredentials { base_url: String, client_id: String },
    /// Credentials are stored locally for this profile.
    Stored {
        base_url: String,
        client_id: String,
        has_client_secret: bool,
        token_state: StoredTokenState,
        namespace: Option<String>,
    },
    /// No credentials anywhere.
    NoCredentials,
}

/// The state of a stored access token at the time of inspection.
#[derive(Debug)]
pub enum StoredTokenState {
    /// Token is present and not yet expired.
    Valid {
        expires_in_secs: u64,
        login_type: Option<String>,
        refresh_token: RefreshTokenState,
    },
    /// Token is present but expired. May or may not have a refresh token.
    Expired {
        login_type: Option<String>,
        refresh_token: RefreshTokenState,
    },
    /// No token in storage.
    Missing,
}

/// The state of a stored refresh token.
#[derive(Debug)]
pub enum RefreshTokenState {
    /// Refresh token is present and not yet expired.
    Valid { expires_in_secs: u64 },
    /// Refresh token is present but the server did not return an expiry.
    Present,
    /// Refresh token is present but expired.
    Expired,
    /// No refresh token.
    Missing,
}

/// Perform a client-credentials grant login. Returns an `AlreadyAuthenticated`
/// outcome (no work done) if a valid or refreshable session already exists.
pub async fn login_with_client_credentials(
    client: &Client,
    request: ClientCredentialsLogin,
    sink: &mut dyn ProgressSink,
) -> Result<LoginOutcome, RuntimeError> {
    use crate::protocol::event::ProgressEvent;
    use crate::support::unix_now;

    if let Some(tip) = check_already_authenticated(&request.profile) {
        return Ok(LoginOutcome {
            kind: LoginOutcomeKind::AlreadyAuthenticated { tip },
            base_url: request.base_url,
            client_id: request.client_id,
            login_type: "client credentials",
            expires_in_secs: 0,
        });
    }

    sink.on_event(ProgressEvent::Started {
        message: "Authenticating (client credentials)...".to_string(),
    });

    let token_result = tokens::fetch_client_credentials_token(
        client,
        &request.base_url,
        &request.client_id,
        &request.client_secret,
    )
    .await;

    sink.on_event(ProgressEvent::Finished);

    let mut token_result = token_result?;

    if let Some(warning) = token_result.expires_in_warning.take() {
        sink.on_event(ProgressEvent::Message { text: warning });
    }

    let token_data = tokens::token_result_to_token_data(
        &token_result,
        crate::protocol::request::GrantType::ClientCredentials,
        unix_now(),
    );

    persist_login(
        &request.profile,
        &request.base_url,
        &request.client_id,
        token_data,
        Some(&request.client_secret),
        sink,
    )
    .await?;

    Ok(LoginOutcome {
        kind: LoginOutcomeKind::LoggedIn,
        base_url: request.base_url,
        client_id: request.client_id,
        login_type: "client credentials",
        expires_in_secs: token_result.expires_in,
    })
}

/// Complete an authorization-code grant login. The caller is responsible for
/// obtaining the `code` and `code_verifier` from the OAuth callback server.
pub async fn login_with_authorization_code(
    client: &Client,
    request: AuthorizationCodeLogin,
    sink: &mut dyn ProgressSink,
) -> Result<LoginOutcome, RuntimeError> {
    use crate::protocol::event::ProgressEvent;
    use crate::support::unix_now;

    if let Some(tip) = check_already_authenticated(&request.profile) {
        return Ok(LoginOutcome {
            kind: LoginOutcomeKind::AlreadyAuthenticated { tip },
            base_url: request.base_url,
            client_id: request.client_id,
            login_type: "authorization code",
            expires_in_secs: 0,
        });
    }

    sink.on_event(ProgressEvent::Started {
        message: "Authenticating (authorization code)...".to_string(),
    });

    let token_result = tokens::exchange_authorization_code(
        client,
        &request.base_url,
        &request.client_id,
        None,
        &request.code,
        &request.code_verifier,
    )
    .await;

    sink.on_event(ProgressEvent::Finished);

    let mut token_result = token_result?;

    if let Some(warning) = token_result.expires_in_warning.take() {
        sink.on_event(ProgressEvent::Message { text: warning });
    }

    let token_data = tokens::token_result_to_token_data(
        &token_result,
        crate::protocol::request::GrantType::AuthorizationCode,
        unix_now(),
    );

    persist_login(
        &request.profile,
        &request.base_url,
        &request.client_id,
        token_data,
        None, // authorization-code flow does not store a client secret
        sink,
    )
    .await?;

    Ok(LoginOutcome {
        kind: LoginOutcomeKind::LoggedIn,
        base_url: request.base_url,
        client_id: request.client_id,
        login_type: "authorization code",
        expires_in_secs: token_result.expires_in,
    })
}

/// Clear stored credentials for a single profile.
pub async fn logout_profile(profile: &str) -> Result<LogoutOutcome, RuntimeError> {
    use crate::runtime::config::ProfileConfig;

    let had_client_id = ProfileConfig::load(profile)
        .map(|cfg| cfg.client_id.is_some())
        .unwrap_or(false);
    let had_client_secret = store::get_secret_async(profile)
        .await
        .ok()
        .flatten()
        .is_some();
    let token_data = store::get_token_data_async(profile).await.ok().flatten();
    let had_access_token = token_data.is_some();
    let had_refresh_token = token_data
        .as_ref()
        .map(|t| t.refresh_token.is_some())
        .unwrap_or(false);

    let _ = store::delete_secret_async(profile).await;
    let _ = store::delete_token_data_async(profile).await;
    crate::runtime::config::ProfileConfig::update(profile, |profile_config| {
        profile_config.client_id = None;
        Ok(())
    })?;

    Ok(LogoutOutcome {
        had_client_id,
        had_client_secret,
        had_access_token,
        had_refresh_token,
    })
}

/// Clear stored credentials from every known profile.
pub async fn logout_all_profiles() -> Result<LogoutAllOutcome, RuntimeError> {
    use crate::runtime::config;

    let profiles = config::list_profiles()?;
    let mut cleared = Vec::new();

    for name in &profiles {
        let _ = store::delete_secret_async(name).await;
        let _ = store::delete_token_data_async(name).await;
        let _ = crate::runtime::config::ProfileConfig::update(name, |profile_config| {
            profile_config.client_id = None;
            Ok(())
        });
        cleared.push(name.clone());
    }

    Ok(LogoutAllOutcome {
        profiles_cleared: cleared,
    })
}

/// Compute a snapshot of the current auth state for status display.
pub fn auth_snapshot(profile: &str) -> Result<AuthSnapshot, RuntimeError> {
    use crate::runtime::config::{self, ProfileConfig};
    use crate::support::unix_now;

    if std::env::var(config::ENV_ACCESS_TOKEN).is_ok() {
        return Ok(AuthSnapshot::EnvironmentToken);
    }

    let credentials = credentials::resolve_credentials(profile);

    let is_env_credentials = matches!(
        credentials.client_id_source,
        Some(credentials::CredentialSource::Environment)
    ) && matches!(
        credentials.client_secret_source,
        Some(credentials::CredentialSource::Environment)
    );

    if is_env_credentials {
        let base_url = credentials
            .base_url
            .clone()
            .unwrap_or_else(|| "unset".to_string());
        let client_id = credentials.client_id.clone().unwrap_or_default();
        return Ok(AuthSnapshot::EnvironmentCredentials {
            base_url,
            client_id,
        });
    }

    let namespace = ProfileConfig::load(profile)
        .ok()
        .and_then(|cfg| cfg.namespace);

    let (base_url, client_id) = match (
        credentials.base_url.as_deref(),
        credentials.client_id.as_deref(),
    ) {
        (Some(b), Some(c)) => (b.to_string(), c.to_string()),
        _ => return Ok(AuthSnapshot::NoCredentials),
    };

    let has_client_secret = credentials.client_secret.is_some();
    let now = unix_now();
    let token_state = match store::get_token_data(profile) {
        Ok(Some(token_data)) if now < token_data.expires_at => StoredTokenState::Valid {
            expires_in_secs: token_data.expires_at - now,
            login_type: token_data.grant_type.map(|g| g.to_string()),
            refresh_token: refresh_token_state(
                token_data.refresh_token.is_some(),
                token_data.refresh_expires_at,
                now,
            ),
        },
        Ok(Some(token_data)) => StoredTokenState::Expired {
            login_type: token_data.grant_type.map(|g| g.to_string()),
            refresh_token: refresh_token_state(
                token_data.refresh_token.is_some(),
                token_data.refresh_expires_at,
                now,
            ),
        },
        _ => StoredTokenState::Missing,
    };

    Ok(AuthSnapshot::Stored {
        base_url,
        client_id,
        has_client_secret,
        token_state,
        namespace,
    })
}

/// Convert stored refresh-token fields into the status shape shown by auth status.
fn refresh_token_state(
    has_refresh: bool,
    refresh_expires_at: Option<u64>,
    now: u64,
) -> RefreshTokenState {
    match (has_refresh, refresh_expires_at) {
        (false, _) => RefreshTokenState::Missing,
        (true, Some(exp)) if now < exp => RefreshTokenState::Valid {
            expires_in_secs: exp - now,
        },
        (true, Some(_)) => RefreshTokenState::Expired,
        (true, None) => RefreshTokenState::Present,
    }
}

// ── Internal helpers ──

/// Inspect stored token data and return a "you are already authenticated" tip
/// if a usable session exists. Returns `None` if the user should proceed with
/// a fresh login.
fn check_already_authenticated(profile: &str) -> Option<String> {
    use crate::support::unix_now;

    let token_data = store::get_token_data(profile).ok()??;
    let now = unix_now();

    if now + session::TOKEN_EXPIRY_BUFFER_SECS < token_data.expires_at {
        let remaining = token_data.expires_at - now;
        return Some(format!(
            "Already authenticated (token expires in {}). Run 'ags auth logout' first to re-authenticate.",
            crate::support::format_duration(remaining)
        ));
    }

    let has_refresh = token_data.refresh_token.is_some()
        && token_data
            .refresh_expires_at
            .map(|exp| now < exp)
            .unwrap_or(true);

    if has_refresh {
        return Some(
            "Session is active (token will auto-refresh on next API call). Run 'ags auth logout' first to re-authenticate.".to_string()
        );
    }

    None
}

/// Persist a successful login: write token data, save base URL + client ID,
/// optionally store the client secret. Surfaces any keychain-fallback warning
/// from `store::store_token_data` via the sink as a `Message` event.
async fn persist_login(
    profile: &str,
    base_url: &str,
    client_id: &str,
    token_data: store::TokenData,
    client_secret: Option<&str>,
    sink: &mut dyn ProgressSink,
) -> Result<(), RuntimeError> {
    use crate::protocol::event::ProgressEvent;
    use crate::runtime::config::ProfileConfig;

    let store_outcome = store::store_token_data_async(profile, token_data).await?;
    if let Some(warning) = store_outcome.warning {
        sink.on_event(ProgressEvent::Message { text: warning });
    }

    ProfileConfig::update(profile, |profile_config| {
        profile_config.base_url = Some(base_url.to_string());
        profile_config.client_id = Some(client_id.to_string());
        Ok(())
    })?;

    if let Some(secret) = client_secret {
        store::store_secret_async(profile, secret.to_string()).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// auth_snapshot must report NoCredentials when nothing is configured anywhere
    #[test]
    #[serial]
    fn test_auth_snapshot_no_credentials_when_empty() {
        std::env::remove_var("AGS_ACCESS_TOKEN");
        std::env::remove_var("AGS_BASE_URL");
        std::env::remove_var("AGS_CLIENT_ID");
        std::env::remove_var("AGS_CLIENT_SECRET");
        let temp_dir = tempfile::tempdir().expect("temp dir");
        std::env::set_var("AGS_HOME", temp_dir.path());

        let snapshot = auth_snapshot("default").expect("snapshot");
        assert!(
            matches!(snapshot, AuthSnapshot::NoCredentials),
            "Expected NoCredentials, got: {snapshot:?}"
        );

        std::env::remove_var("AGS_HOME");
    }

    /// auth_snapshot must report EnvironmentToken when AGS_ACCESS_TOKEN is set
    #[test]
    #[serial]
    fn test_auth_snapshot_environment_token() {
        std::env::set_var("AGS_ACCESS_TOKEN", "test-token");
        let temp_dir = tempfile::tempdir().expect("temp dir");
        std::env::set_var("AGS_HOME", temp_dir.path());

        let snapshot = auth_snapshot("default").expect("snapshot");
        assert!(matches!(snapshot, AuthSnapshot::EnvironmentToken));

        std::env::remove_var("AGS_ACCESS_TOKEN");
        std::env::remove_var("AGS_HOME");
    }

    /// logout_profile reports all-false when nothing is stored
    #[tokio::test]
    #[serial]
    async fn test_logout_profile_empty_state() {
        std::env::remove_var("AGS_BASE_URL");
        std::env::remove_var("AGS_CLIENT_ID");
        std::env::remove_var("AGS_CLIENT_SECRET");
        std::env::set_var("AGS_NO_KEYCHAIN", "1");
        let temp_dir = tempfile::tempdir().expect("temp dir");
        std::env::set_var("AGS_HOME", temp_dir.path());

        let outcome = logout_profile("ephemeral-test-profile")
            .await
            .expect("logout");
        assert_eq!(
            outcome,
            LogoutOutcome {
                had_client_id: false,
                had_client_secret: false,
                had_access_token: false,
                had_refresh_token: false,
            }
        );

        std::env::remove_var("AGS_HOME");
        std::env::remove_var("AGS_NO_KEYCHAIN");
    }

    /// logout_all_profiles reports an empty list when no profiles exist
    #[tokio::test]
    #[serial]
    async fn test_logout_all_profiles_empty() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        std::env::set_var("AGS_HOME", temp_dir.path());
        std::env::set_var("AGS_NO_KEYCHAIN", "1");

        let outcome = logout_all_profiles().await.expect("logout all");
        assert_eq!(
            outcome,
            LogoutAllOutcome {
                profiles_cleared: vec![],
            }
        );

        std::env::remove_var("AGS_HOME");
        std::env::remove_var("AGS_NO_KEYCHAIN");
    }
}
