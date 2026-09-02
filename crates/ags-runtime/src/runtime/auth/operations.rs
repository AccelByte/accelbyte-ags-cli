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

use ags_protocol::error::RuntimeError;
use ags_protocol::event::ProgressSink;

use super::credentials;
use super::errors::AuthError;
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
    /// Token lifetime in seconds for `LoggedIn` and `Refreshed`. `None` when
    /// no new token was fetched (`AlreadyAuthenticated`), since the stored
    /// token's remaining lifetime is reported through the `tip` instead.
    pub expires_in_secs: Option<u64>,
}

/// What the login function actually did.
#[derive(Debug)]
pub enum LoginOutcomeKind {
    /// A new token was fetched and stored.
    LoggedIn,
    /// A valid (or refreshable) session already existed; no work was done.
    /// `tip` is the message to surface to the user.
    AlreadyAuthenticated { tip: String },
    /// The stored access token was stale and was refreshed in place — no
    /// fresh OAuth flow was run.
    Refreshed,
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

/// Perform a client-credentials grant login. The caller is responsible for
/// calling [`probe_existing_session`] first so we don't issue a redundant
/// grant when a usable session already exists.
pub async fn login_with_client_credentials(
    client: &Client,
    request: ClientCredentialsLogin,
    sink: &mut dyn ProgressSink,
) -> Result<LoginOutcome, RuntimeError> {
    use crate::support::unix_now;
    use ags_protocol::event::ProgressEvent;

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
        ags_protocol::request::GrantType::ClientCredentials,
        unix_now(),
        &request.client_id,
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
        expires_in_secs: Some(token_result.expires_in),
    })
}

/// Complete an authorization-code grant login. The caller is responsible for
/// (a) obtaining the `code` and `code_verifier` from the OAuth callback
/// server and (b) calling [`probe_existing_session`] first so we don't run
/// the browser flow when a refreshable session already exists.
pub async fn login_with_authorization_code(
    client: &Client,
    request: AuthorizationCodeLogin,
    sink: &mut dyn ProgressSink,
) -> Result<LoginOutcome, RuntimeError> {
    use crate::support::unix_now;
    use ags_protocol::event::ProgressEvent;

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
        ags_protocol::request::GrantType::AuthorizationCode,
        unix_now(),
        &request.client_id,
    );
    // Captured before `token_data` moves into `persist_login` below.
    let access_token = token_data.access_token.clone();

    persist_login(
        &request.profile,
        &request.base_url,
        &request.client_id,
        token_data,
        None, // authorization-code flow does not store a client secret
        sink,
    )
    .await?;

    // Merge this install's pre-login anonymous person into the real user,
    // exactly once per install. Authorization-code only: its `sub` is a real
    // IAM user id, unlike a client-credentials `sub` (an IAM Client id — see
    // `login_with_client_credentials`, which deliberately never calls this).
    // Fire-and-forget and bounded (`EMIT_FLUSH_TIMEOUT`); never changes this
    // function's return value.
    if let Some(sub) = crate::runtime::telemetry::decode_sub(&access_token) {
        let email = crate::runtime::telemetry::read_cached_email(&request.profile, &access_token);
        crate::runtime::telemetry::emit_identity_merge(&request.profile, &sub, email.as_deref())
            .await;
    }

    Ok(LoginOutcome {
        kind: LoginOutcomeKind::LoggedIn,
        base_url: request.base_url,
        client_id: request.client_id,
        login_type: "authorization code",
        expires_in_secs: Some(token_result.expires_in),
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
    // The cached email was fetched for the token just cleared above; drop it
    // so a future login re-fetches rather than silently reusing a stale value.
    crate::runtime::telemetry::clear_cached_email(profile);

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
        crate::runtime::telemetry::clear_cached_email(name);
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

/// True if the profile's stored base URL and client ID match the requested
/// values, or if either side is absent (no stored value to compare against).
/// A mismatch means the stored token belongs to a different identity than
/// the caller is asking to log into, so the probe must not short-circuit.
///
/// If the profile config cannot be loaded (corruption, permissions), we
/// cannot verify identity, so refuse to short-circuit — the stored token
/// may have been minted for a different base URL or client ID, and using
/// it against the caller's requested target could leak a dev token to
/// prod (or vice versa).
fn stored_identity_matches(
    profile: &str,
    requested_base_url: &str,
    requested_client_id: &str,
) -> bool {
    use crate::runtime::config::ProfileConfig;

    let Ok(config) = ProfileConfig::load(profile) else {
        return false;
    };
    if let Some(stored) = config.base_url.as_deref() {
        if stored != requested_base_url {
            return false;
        }
    }
    if let Some(stored) = config.client_id.as_deref() {
        // Normalised compare (shared helper) so the same client in a different
        // casing/hyphenation is recognised as a match, consistent with the
        // token-binding checks. base_url stays an exact compare.
        if !crate::runtime::config::client_ids_match(stored, requested_client_id) {
            return false;
        }
    }
    true
}

/// Inspect stored token data and return a "you are already authenticated" tip
/// if the access token is comfortably within its expiry window. Returns `None`
/// when the caller should probe for a refreshable session or proceed with a
/// fresh login — the refresh-token branch that previously lived in
/// `check_already_authenticated` moved to `session::try_refresh_stored_session`,
/// invoked via `probe_existing_session`.
fn existing_access_token_still_valid(profile: &str, expected_client_id: &str) -> Option<String> {
    use crate::support::unix_now;

    let token_data = store::get_token_data(profile).ok()??;

    // A token minted by a different client is not a usable session for this
    // login, regardless of freshness — force a fresh flow. Compared via the
    // shared normalised helper so a casing/hyphenation difference for the same
    // client is not treated as a mismatch (matches session::client_matches).
    if let Some(stored) = token_data.client_id.as_deref() {
        if !crate::runtime::config::client_ids_match(stored, expected_client_id) {
            return None;
        }
    }

    let now = unix_now();
    if now + session::TOKEN_EXPIRY_BUFFER_SECS < token_data.expires_at {
        let remaining = token_data.expires_at - now;
        return Some(format!(
            "Already authenticated (token expires in {}). Run 'ags auth logout' first to re-authenticate.",
            crate::support::format_duration(remaining)
        ));
    }

    None
}

/// Probe whether the profile already has a usable session, refreshing in
/// place if the access token is stale but the refresh token still works.
///
/// Returns:
/// - `Ok(Some(AlreadyAuthenticated))` — access token still comfortably valid;
///   caller should not start a fresh flow.
/// - `Ok(Some(Refreshed))` — access token was stale but the refresh token
///   was successfully used to mint a fresh one; new token already persisted;
///   caller should not start a fresh flow.
/// - `Ok(None)` — caller must proceed with a fresh OAuth or grant flow. Any
///   stale stored token has already been cleared via best-effort delete; an
///   informational `ProgressEvent::Message` was emitted to the sink if
///   `None` is due to a server-side refresh rejection.
/// - `Err(_)` — transport-level failure during the probe (e.g. network down);
///   caller should propagate the error and abort.
pub async fn probe_existing_session(
    client: &Client,
    profile: &str,
    base_url: String,
    client_id: String,
    login_type: &'static str,
    sink: &mut dyn ProgressSink,
) -> Result<Option<LoginOutcome>, RuntimeError> {
    use ags_protocol::event::ProgressEvent;

    // The stored token was minted for the profile's stored identity. If the
    // caller is asking to log into a different base URL or client ID, the
    // stored session isn't a match — fall through to a fresh flow.
    if !stored_identity_matches(profile, &base_url, &client_id) {
        return Ok(None);
    }

    if let Some(tip) = existing_access_token_still_valid(profile, &client_id) {
        return Ok(Some(LoginOutcome {
            kind: LoginOutcomeKind::AlreadyAuthenticated { tip },
            base_url,
            client_id,
            login_type,
            expires_in_secs: None,
        }));
    }

    match session::try_refresh_stored_session(
        client,
        profile,
        session::RefreshMode::Probe,
        Some(&client_id),
    )
    .await?
    {
        session::RefreshOutcome::Refreshed {
            expires_in_secs, ..
        } => Ok(Some(LoginOutcome {
            kind: LoginOutcomeKind::Refreshed,
            base_url,
            client_id,
            login_type,
            expires_in_secs: Some(expires_in_secs),
        })),
        session::RefreshOutcome::Rejected { .. } => {
            sink.on_event(ProgressEvent::Message {
                text: "Existing session was invalid — starting fresh login...".to_string(),
            });
            // Best-effort clear. If this fails (keychain locked, read-only
            // config dir) the rejected token will keep coming back on every
            // invocation, so surface the failure rather than swallowing it —
            // the next login flow will still proceed and overwrite the
            // stored token on success.
            if let Err(error) = store::delete_token_data_async(profile).await {
                sink.on_event(ProgressEvent::Message {
                    text: format!("Warning: could not clear stale session data: {error}"),
                });
            }
            Ok(None)
        }
        session::RefreshOutcome::Unavailable { .. } => Ok(None),
    }
}

/// Persist a successful login: write token data, save base URL + client ID +
/// grant type, optionally store the client secret. Surfaces any keychain-
/// fallback warning from `store::store_token_data` via the sink as a `Message`
/// event.
async fn persist_login(
    profile: &str,
    base_url: &str,
    client_id: &str,
    token_data: store::TokenData,
    client_secret: Option<&str>,
    sink: &mut dyn ProgressSink,
) -> Result<(), RuntimeError> {
    use crate::runtime::config::ProfileConfig;
    use ags_protocol::event::ProgressEvent;

    // Capture the grant type before `token_data` is moved into the store, so the
    // profile config records how this profile most recently authenticated
    // (`profile show` and `config get grant-type` read it back). `GrantType` is
    // `Copy`, so this does not disturb the subsequent move.
    let grant_type = token_data.grant_type;

    let store_outcome = store::store_token_data_async(profile, token_data).await?;
    if let Some(warning) = store_outcome.warning {
        sink.on_event(ProgressEvent::Message { text: warning });
    }

    ProfileConfig::update(profile, |profile_config| {
        profile_config.base_url = Some(base_url.to_string());
        profile_config.client_id = Some(client_id.to_string());
        if let Some(grant_type) = grant_type {
            profile_config.grant_type = Some(grant_type);
        }
        Ok(())
    })?;

    if let Some(secret) = client_secret {
        store::store_secret_async(profile, secret.to_string()).await?;
    }

    Ok(())
}

// ── Refresh outcome + operation ──

/// The result of `refresh_profile`: a freshly minted token and how it was minted.
#[derive(Debug)]
pub struct TokenRefreshOutcome {
    /// Which grant re-minted the token (ClientCredentials or AuthorizationCode).
    pub grant_type: ags_protocol::request::GrantType,
    pub base_url: String,
    pub client_id: String,
    /// Lifetime of the new access token in seconds.
    pub expires_in_secs: u64,
    /// For the authorization-code path, the permission caveat; else `None`.
    pub note: Option<String>,
}

/// Re-mint the profile's access token from its stored credentials, without
/// clearing anything. Client-credentials profiles re-run the grant (new
/// permissions guaranteed); authorization-code profiles force a refresh-token
/// exchange (newer token, but new permissions may still need a full login).
pub async fn refresh_profile(
    client: &Client,
    profile: &str,
    sink: &mut dyn ProgressSink,
) -> Result<TokenRefreshOutcome, RuntimeError> {
    use crate::runtime::config::{self, ProfileConfig};
    use crate::support::unix_now;
    use ags_protocol::error::{RuntimeError as RErr, RuntimeErrorKind};
    use ags_protocol::event::ProgressEvent;
    use ags_protocol::request::GrantType;

    // 1. External token: nothing to re-mint.
    if std::env::var(config::ENV_ACCESS_TOKEN).is_ok() {
        return Err(RErr {
            kind: RuntimeErrorKind::Validation,
            message: "Access token is set via AGS_ACCESS_TOKEN, so there is nothing to refresh"
                .to_string(),
            details: None,
            hint: Some("Unset AGS_ACCESS_TOKEN to use stored credentials".to_string()),
            trace: None,
        });
    }

    // 2. Resolve credentials + recorded grant type + stored refresh token.
    let creds = credentials::resolve_credentials(profile);
    // Keychain read goes through the async variant (matches
    // `probe_existing_session`); `ProfileConfig::load` stays sync, as it does
    // there too — it is a config-file read, not a keychain round trip.
    let grant_type = ProfileConfig::load(profile).ok().and_then(|c| c.grant_type);
    let has_refresh_token = store::get_token_data_async(profile)
        .await
        .ok()
        .flatten()
        .and_then(|t| t.refresh_token.clone())
        .is_some();

    // 3. Route. Prefer the recorded grant type; fall back to available creds.
    let use_client_credentials = matches!(grant_type, Some(GrantType::ClientCredentials))
        || (grant_type.is_none() && creds.client_secret.is_some());
    let use_authorization_code = matches!(grant_type, Some(GrantType::AuthorizationCode))
        || (grant_type.is_none() && creds.client_secret.is_none() && has_refresh_token);

    if use_client_credentials {
        let base_url = creds
            .base_url
            .clone()
            .ok_or_else(|| RuntimeError::from(AuthError::BaseUrlMissing))?;
        let client_id = creds
            .client_id
            .clone()
            .ok_or_else(|| RuntimeError::from(AuthError::ClientIdMissing))?;
        let client_secret = creds
            .client_secret
            .clone()
            .ok_or_else(|| RuntimeError::from(AuthError::ClientSecretMissing))?;

        sink.on_event(ProgressEvent::Started {
            message: "Refreshing token (client credentials)...".to_string(),
        });
        let result =
            tokens::fetch_client_credentials_token(client, &base_url, &client_id, &client_secret)
                .await;
        sink.on_event(ProgressEvent::Finished);
        let mut result = result?;
        if let Some(warning) = result.expires_in_warning.take() {
            sink.on_event(ProgressEvent::Message { text: warning });
        }
        let expires_in = result.expires_in;
        let token_data = tokens::token_result_to_token_data(
            &result,
            GrantType::ClientCredentials,
            unix_now(),
            &client_id,
        );
        persist_login(
            profile,
            &base_url,
            &client_id,
            token_data,
            Some(&client_secret),
            sink,
        )
        .await?;

        return Ok(TokenRefreshOutcome {
            grant_type: GrantType::ClientCredentials,
            base_url,
            client_id,
            expires_in_secs: expires_in,
            note: None,
        });
    }

    if use_authorization_code {
        let base_url = creds
            .base_url
            .clone()
            .ok_or_else(|| RuntimeError::from(AuthError::BaseUrlMissing))?;
        let client_id = creds
            .client_id
            .clone()
            .ok_or_else(|| RuntimeError::from(AuthError::ClientIdMissing))?;

        sink.on_event(ProgressEvent::Started {
            message: "Refreshing token (authorization code)...".to_string(),
        });
        let outcome = session::try_refresh_stored_session(
            client,
            profile,
            session::RefreshMode::Force,
            Some(&client_id),
        )
        .await?;
        sink.on_event(ProgressEvent::Finished);

        return match outcome {
            session::RefreshOutcome::Refreshed {
                expires_in_secs,
                warnings,
                ..
            } => {
                for warning in warnings {
                    sink.on_event(ProgressEvent::Message { text: warning });
                }
                Ok(TokenRefreshOutcome {
                    grant_type: GrantType::AuthorizationCode,
                    base_url,
                    client_id,
                    expires_in_secs,
                    note: Some(
                        "If you added permissions, they may need a full `ags auth login` to take effect".to_string(),
                    ),
                })
            }
            // Forced refresh: the access token may still be valid, so these
            // read as refresh-token/IdP failures, not "session expired".
            session::RefreshOutcome::Rejected { message } => {
                Err(RuntimeError::from(AuthError::RefreshRejected(message)))
            }
            session::RefreshOutcome::Unavailable { reason } => Err(match reason {
                session::UnavailableReason::NoRefreshToken => {
                    RuntimeError::from(AuthError::RefreshNoRefreshToken)
                }
                session::UnavailableReason::RefreshTokenExpired => {
                    RuntimeError::from(AuthError::RefreshTokenExpired)
                }
                session::UnavailableReason::NoStoredToken => nothing_to_refresh_error(),
                session::UnavailableReason::ClientMismatch { stored_client_id } => {
                    RuntimeError::from(AuthError::StoredSessionClientMismatch {
                        profile: profile.to_string(),
                        stored_client_id,
                        current_client_id: client_id.clone(),
                    })
                }
            }),
        };
    }

    // 4. Neither path is possible.
    Err(nothing_to_refresh_error())
}

/// Actionable "nothing to refresh" error pointing at login.
fn nothing_to_refresh_error() -> RuntimeError {
    use ags_protocol::error::RuntimeErrorKind;
    RuntimeError {
        kind: RuntimeErrorKind::NotAuthenticated,
        message: "Nothing to refresh for this profile".to_string(),
        details: None,
        hint: Some("Run `ags auth login` to authenticate".to_string()),
        trace: None,
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    use crate::runtime::auth::store::{self, TokenData};
    use crate::runtime::config::ProfileConfig;
    use crate::support::test_helpers::{now_secs, TempEnvGuard};
    use ags_protocol::event::ProgressSink;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct NullSink;
    impl ProgressSink for NullSink {
        fn on_event(&mut self, _event: ags_protocol::event::ProgressEvent) {}
    }

    /// Snapshot + clear the auth env vars for the test's duration, restoring
    /// them on drop. Bind the result (`let _env = clear_cred_env();`) so the
    /// guards live to the end of the test. The runtime resolves auth from env
    /// FIRST (`credentials::resolve_credentials`, `resolve_base_url`), so a bare
    /// `remove_var` that isn't restored on panic would leak into later serial
    /// tests, and a developer's live `AGS_BASE_URL` would point the test at the
    /// wrong host instead of the wiremock server.
    fn clear_cred_env() -> (TempEnvGuard, TempEnvGuard, TempEnvGuard, TempEnvGuard) {
        (
            TempEnvGuard::remove("AGS_ACCESS_TOKEN"),
            TempEnvGuard::remove("AGS_CLIENT_ID"),
            TempEnvGuard::remove("AGS_CLIENT_SECRET"),
            TempEnvGuard::remove("AGS_BASE_URL"),
        )
    }

    /// Client-credentials profile: refresh re-mints via the grant and preserves
    /// the stored secret; grant_type is ClientCredentials with no note.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_refresh_client_credentials_remints_and_keeps_secret() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
        let _env = clear_cred_env();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .and(body_string_contains("grant_type=client_credentials"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"cc-new","expires_in":3600,"token_type":"Bearer"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        ProfileConfig {
            base_url: Some(server.uri()),
            client_id: Some("cid".to_string()),
            grant_type: Some(ags_protocol::request::GrantType::ClientCredentials),
            ..Default::default()
        }
        .save("default")
        .unwrap();
        store::store_secret("default", "sekret").unwrap();
        let now = now_secs();
        store::store_token_data(
            "default",
            &TokenData {
                access_token: "old".to_string(),
                expires_at: now + 3600,
                refresh_token: None,
                refresh_expires_at: None,
                grant_type: Some(ags_protocol::request::GrantType::ClientCredentials),
                client_id: None,
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let outcome = refresh_profile(&client, "default", &mut NullSink)
            .await
            .unwrap();

        assert_eq!(
            outcome.grant_type,
            ags_protocol::request::GrantType::ClientCredentials
        );
        assert!(outcome.note.is_none());
        assert_eq!(
            store::get_token_data("default")
                .unwrap()
                .unwrap()
                .access_token,
            "cc-new"
        );
        // get_secret returns Option<Zeroizing<String>>; deref to &str to compare.
        let secret = store::get_secret("default")
            .unwrap()
            .expect("secret still stored");
        assert_eq!(secret.as_str(), "sekret");
    }

    /// Authorization-code profile with a STILL-VALID access token: refresh must
    /// force the refresh-token exchange (mock is only reachable via Force),
    /// persist, and return the permission note.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_refresh_authorization_code_forces_exchange_and_sets_note() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
        let _env = clear_cred_env();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"ac-new","expires_in":3600,"token_type":"Bearer","refresh_token":"rot","refresh_expires_in":7200}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        ProfileConfig {
            base_url: Some(server.uri()),
            client_id: Some("cid".to_string()),
            grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
            ..Default::default()
        }
        .save("default")
        .unwrap();
        let now = now_secs();
        store::store_token_data(
            "default",
            &TokenData {
                access_token: "still-valid".to_string(),
                expires_at: now + 3600, // valid — only Force reaches the endpoint
                refresh_token: Some("rt".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let outcome = refresh_profile(&client, "default", &mut NullSink)
            .await
            .unwrap();

        assert_eq!(
            outcome.grant_type,
            ags_protocol::request::GrantType::AuthorizationCode
        );
        assert!(
            outcome.note.is_some(),
            "auth-code refresh must carry the permission note"
        );
        assert_eq!(
            store::get_token_data("default")
                .unwrap()
                .unwrap()
                .access_token,
            "ac-new"
        );
    }

    /// Forced refresh whose refresh-token exchange the IdP rejects, while the
    /// access token is still valid: the error must NOT claim the session
    /// expired — it reads as a server-side refresh rejection.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_refresh_authorization_code_rejection_is_not_session_expired() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
        let _env = clear_cred_env();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                r#"{"error":"invalid_grant","error_description":"refresh token revoked"}"#,
            ))
            .mount(&server)
            .await;

        ProfileConfig {
            base_url: Some(server.uri()),
            client_id: Some("cid".to_string()),
            grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
            ..Default::default()
        }
        .save("default")
        .unwrap();
        let now = now_secs();
        store::store_token_data(
            "default",
            &TokenData {
                access_token: "still-valid".to_string(),
                expires_at: now + 3600, // access token comfortably valid
                refresh_token: Some("rt".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let err = refresh_profile(&client, "default", &mut NullSink)
            .await
            .unwrap_err();
        let msg = err.message.to_lowercase();
        assert!(
            !msg.contains("session expired"),
            "forced-refresh rejection must not claim the session expired: {}",
            err.message
        );
        assert!(
            msg.contains("rejected"),
            "expected a refresh-rejected message: {}",
            err.message
        );
    }

    /// `AGS_ACCESS_TOKEN` set → error, no network.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_refresh_errors_when_env_access_token_set() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
        let _tok = TempEnvGuard::set("AGS_ACCESS_TOKEN", "external");

        let client = reqwest::Client::new();
        let err = refresh_profile(&client, "default", &mut NullSink)
            .await
            .unwrap_err();
        assert!(err.message.to_lowercase().contains("access token"));
    }

    /// No secret and no refresh token → actionable error naming login.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_refresh_errors_when_nothing_to_refresh() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
        let _env = clear_cred_env();

        let client = reqwest::Client::new();
        let err = refresh_profile(&client, "default", &mut NullSink)
            .await
            .unwrap_err();
        assert!(
            err.message.to_lowercase().contains("refresh")
                || err.message.to_lowercase().contains("authenticate")
        );
    }

    /// `ags auth refresh` on an authorization-code profile whose stored token was
    /// minted by a different client refuses with a clear mismatch error rather than
    /// refreshing another client's token.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_refresh_errors_on_client_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
        // Clears AGS_ACCESS_TOKEN (else refresh_profile early-returns) + client
        // id/secret + base URL so ProfileConfig drives resolution.
        let _env = clear_cred_env();

        // Local mock: the token endpoint must NOT be hit. Before the fix (the
        // caller passes None), the mismatch is undetected and Force mode would
        // attempt a refresh grant against base_url — pointing that at a MockServer
        // with .expect(0) makes the red step fail deterministically and locally,
        // with no external DNS/network dependency. After the fix the guard returns
        // ClientMismatch first, so the endpoint stays untouched.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        // Current client B, authorization-code grant, no secret.
        ProfileConfig {
            base_url: Some(server.uri()),
            client_id: Some("client-B".to_string()),
            grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
            ..Default::default()
        }
        .save("default")
        .unwrap();

        let now = now_secs();
        store::store_token_data(
            "default",
            &TokenData {
                access_token: "stale".to_string(),
                expires_at: now.saturating_sub(60), // stale → Force will try to refresh
                refresh_token: Some("rt".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: Some("client-A".to_string()),
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let err = refresh_profile(&client, "default", &mut NullSink)
            .await
            .unwrap_err();
        assert!(
            err.message.contains("different client"),
            "unexpected message: {}",
            err.message
        );
    }

    /// `login_with_client_credentials` stamps the minting client onto the stored
    /// token end-to-end (through `persist_login`), not only inside the pure
    /// converter. Guards the call site at `login_with_client_credentials` against
    /// a future refactor silently dropping the `client_id` argument.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_login_with_client_credentials_stamps_client_id() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
        let _env = clear_cred_env();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .and(body_string_contains("grant_type=client_credentials"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"cc-access","expires_in":3600,"token_type":"Bearer"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let mut sink = NullSink;
        login_with_client_credentials(
            &reqwest::Client::new(),
            ClientCredentialsLogin {
                profile: "default".to_string(),
                base_url: server.uri(),
                client_id: "login-client-cc".to_string(),
                client_secret: "sekret".to_string(),
            },
            &mut sink,
        )
        .await
        .unwrap();

        let stored = store::get_token_data("default")
            .unwrap()
            .expect("token stored");
        assert_eq!(stored.client_id.as_deref(), Some("login-client-cc"));
        assert_eq!(stored.access_token, "cc-access");
    }

    /// `login_with_authorization_code` likewise stamps the minting client — a
    /// distinct call site from the client-credentials path, so a copy-paste drop
    /// at one is not masked by the other.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_login_with_authorization_code_stamps_client_id() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
        let _env = clear_cred_env();

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"ac-access","expires_in":3600,"token_type":"Bearer","refresh_token":"rt","refresh_expires_in":7200}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let mut sink = NullSink;
        login_with_authorization_code(
            &reqwest::Client::new(),
            AuthorizationCodeLogin {
                profile: "default".to_string(),
                base_url: server.uri(),
                client_id: "login-client-ac".to_string(),
                code: "auth-code".to_string(),
                code_verifier: "verifier".to_string(),
            },
            &mut sink,
        )
        .await
        .unwrap();

        let stored = store::get_token_data("default")
            .unwrap()
            .expect("token stored");
        assert_eq!(stored.client_id.as_deref(), Some("login-client-ac"));
        assert_eq!(stored.access_token, "ac-access");
    }
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

#[cfg(test)]
mod probe_tests {
    use super::*;
    use crate::runtime::auth::store::TokenData;
    use crate::runtime::config::ProfileConfig;
    use crate::support::test_helpers::{now_secs, TempEnvGuard};
    use ags_protocol::event::{ProgressEvent, ProgressSink};
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Test sink that captures every emitted progress message for assertions.
    #[derive(Default)]
    struct CapturingSink {
        messages: Vec<String>,
    }

    impl ProgressSink for CapturingSink {
        /// Capture message events; drop other events.
        fn on_event(&mut self, event: ProgressEvent) {
            if let ProgressEvent::Message { text } = event {
                self.messages.push(text);
            }
        }
    }

    /// A fresh stored access token shortcuts straight to AlreadyAuthenticated
    /// with no network call.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_returns_already_authenticated_when_access_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

        let profile = "default";
        ProfileConfig {
            base_url: Some("https://unused.invalid".to_string()),
            client_id: Some("cid".to_string()),
            ..Default::default()
        }
        .save(profile)
        .unwrap();

        let now = now_secs();
        store::store_token_data(
            profile,
            &TokenData {
                access_token: "fresh".to_string(),
                expires_at: now + 3600,
                refresh_token: Some("rt".to_string()),
                refresh_expires_at: Some(now + 7200),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let mut sink = CapturingSink::default();
        let outcome = probe_existing_session(
            &client,
            profile,
            "https://unused.invalid".to_string(),
            "cid".to_string(),
            "authorization code",
            &mut sink,
        )
        .await
        .unwrap();

        match outcome {
            Some(LoginOutcome {
                kind: LoginOutcomeKind::AlreadyAuthenticated { tip },
                ..
            }) => {
                assert!(tip.contains("Already authenticated"));
            }
            other => panic!("expected AlreadyAuthenticated, got {other:?}"),
        }
        assert!(
            sink.messages.is_empty(),
            "no progress messages expected, got {:?}",
            sink.messages
        );
    }

    /// Successful refresh: probe returns Refreshed and the stored token is
    /// updated.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_returns_refreshed_when_refresh_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"new-access","expires_in":3600,"token_type":"Bearer","refresh_token":"rotated","refresh_expires_in":7200}"#,
            ))
            .mount(&server)
            .await;

        let profile = "default";
        ProfileConfig {
            base_url: Some(server.uri()),
            client_id: Some("cid".to_string()),
            ..Default::default()
        }
        .save(profile)
        .unwrap();

        let now = now_secs();
        store::store_token_data(
            profile,
            &TokenData {
                access_token: "expired".to_string(),
                expires_at: now.saturating_sub(60),
                refresh_token: Some("valid-rt".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let mut sink = CapturingSink::default();
        let outcome = probe_existing_session(
            &client,
            profile,
            server.uri(),
            "cid".to_string(),
            "authorization code",
            &mut sink,
        )
        .await
        .unwrap();

        assert!(
            matches!(
                outcome,
                Some(LoginOutcome {
                    kind: LoginOutcomeKind::Refreshed,
                    ..
                })
            ),
            "expected Refreshed, got {outcome:?}"
        );
        let stored = store::get_token_data(profile).unwrap().unwrap();
        assert_eq!(stored.access_token, "new-access");
        assert!(
            sink.messages.is_empty(),
            "no probe-failure message expected on success, got {:?}",
            sink.messages
        );
    }

    /// Rejection path: probe returns None, emits the user-facing message,
    /// and deletes the stale stored token data.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_returns_none_and_clears_state_on_rejection() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ResponseTemplate::new(401).set_body_string(
                r#"{"error":"invalid_grant","error_description":"refresh expired"}"#,
            ))
            .mount(&server)
            .await;

        let profile = "default";
        ProfileConfig {
            base_url: Some(server.uri()),
            client_id: Some("cid".to_string()),
            ..Default::default()
        }
        .save(profile)
        .unwrap();

        let now = now_secs();
        store::store_token_data(
            profile,
            &TokenData {
                access_token: "expired".to_string(),
                expires_at: now.saturating_sub(60),
                refresh_token: Some("dead-rt".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let mut sink = CapturingSink::default();
        let outcome = probe_existing_session(
            &client,
            profile,
            server.uri(),
            "cid".to_string(),
            "authorization code",
            &mut sink,
        )
        .await
        .unwrap();

        assert!(outcome.is_none(), "expected None, got {outcome:?}");
        assert!(
            store::get_token_data(profile).unwrap().is_none(),
            "stale token data must be cleared on probe rejection"
        );
        assert!(
            sink.messages
                .iter()
                .any(|m| m.contains("Existing session was invalid")),
            "expected probe-failure message, got {:?}",
            sink.messages
        );
    }

    /// Unavailable path (no refresh token): probe returns None, no message
    /// is emitted, and any stored token is left alone (it's already useless
    /// but we don't proactively clear).
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_returns_none_silently_when_unavailable() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

        let profile = "default";
        ProfileConfig {
            base_url: Some("https://unused.invalid".to_string()),
            client_id: Some("cid".to_string()),
            ..Default::default()
        }
        .save(profile)
        .unwrap();

        let now = now_secs();
        store::store_token_data(
            profile,
            &TokenData {
                access_token: "expired".to_string(),
                expires_at: now.saturating_sub(60),
                refresh_token: None,
                refresh_expires_at: None,
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let mut sink = CapturingSink::default();
        let outcome = probe_existing_session(
            &client,
            profile,
            "https://unused.invalid".to_string(),
            "cid".to_string(),
            "authorization code",
            &mut sink,
        )
        .await
        .unwrap();

        assert!(outcome.is_none(), "expected None, got {outcome:?}");
        assert!(
            sink.messages.is_empty(),
            "no message expected for Unavailable, got {:?}",
            sink.messages
        );
    }

    /// Probe rejection must not delete the profile's saved base_url /
    /// client_id — only the token data. Otherwise users would have to
    /// reconfigure after every server-side session expiry.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_rejection_preserves_profile_config() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string(r#"{"error":"invalid_grant"}"#),
            )
            .mount(&server)
            .await;

        let profile = "default";
        ProfileConfig {
            base_url: Some(server.uri()),
            client_id: Some("cid".to_string()),
            namespace: Some("preserved-ns".to_string()),
            ..Default::default()
        }
        .save(profile)
        .unwrap();

        let now = now_secs();
        store::store_token_data(
            profile,
            &TokenData {
                access_token: "expired".to_string(),
                expires_at: now.saturating_sub(60),
                refresh_token: Some("dead-rt".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let mut sink = CapturingSink::default();
        let _ = probe_existing_session(
            &client,
            profile,
            server.uri(),
            "cid".to_string(),
            "authorization code",
            &mut sink,
        )
        .await
        .unwrap();

        let cfg = ProfileConfig::load(profile).unwrap();
        assert_eq!(cfg.base_url.as_deref(), Some(server.uri().as_str()));
        assert_eq!(cfg.client_id.as_deref(), Some("cid"));
        assert_eq!(cfg.namespace.as_deref(), Some("preserved-ns"));
    }

    /// When the caller asks to log into a base URL that differs from the
    /// stored one, the probe must NOT short-circuit — the stored token
    /// belongs to a different identity.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_returns_none_when_base_url_differs_from_stored() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

        let profile = "default";
        ProfileConfig {
            base_url: Some("https://dev.example.com".to_string()),
            client_id: Some("cid".to_string()),
            ..Default::default()
        }
        .save(profile)
        .unwrap();

        let now = now_secs();
        store::store_token_data(
            profile,
            &TokenData {
                access_token: "fresh".to_string(),
                expires_at: now + 3600,
                refresh_token: Some("rt".to_string()),
                refresh_expires_at: Some(now + 7200),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let mut sink = CapturingSink::default();
        let outcome = probe_existing_session(
            &client,
            profile,
            "https://demo.example.com".to_string(),
            "cid".to_string(),
            "authorization code",
            &mut sink,
        )
        .await
        .unwrap();

        assert!(
            outcome.is_none(),
            "expected None when base_url differs, got {outcome:?}"
        );
    }

    /// When the caller asks to log in with a client ID that differs from
    /// the stored one, the probe must NOT short-circuit.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_returns_none_when_client_id_differs_from_stored() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

        let profile = "default";
        ProfileConfig {
            base_url: Some("https://dev.example.com".to_string()),
            client_id: Some("original-cid".to_string()),
            ..Default::default()
        }
        .save(profile)
        .unwrap();

        let now = now_secs();
        store::store_token_data(
            profile,
            &TokenData {
                access_token: "fresh".to_string(),
                expires_at: now + 3600,
                refresh_token: Some("rt".to_string()),
                refresh_expires_at: Some(now + 7200),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let mut sink = CapturingSink::default();
        let outcome = probe_existing_session(
            &client,
            profile,
            "https://dev.example.com".to_string(),
            "different-cid".to_string(),
            "authorization code",
            &mut sink,
        )
        .await
        .unwrap();

        assert!(
            outcome.is_none(),
            "expected None when client_id differs, got {outcome:?}"
        );
    }

    /// Reproduces the reported CI regression: credentials come from env vars, so
    /// ProfileConfig is empty (the config-identity gate is a no-op), and a fresh
    /// token minted by a DIFFERENT client sits in the shared store. The probe must
    /// NOT report AlreadyAuthenticated — it returns Ok(None) so a fresh login runs.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_ignores_fresh_token_from_other_client() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

        // Empty ProfileConfig (env-cred style) → stored_identity_matches is a no-op.
        let now = crate::support::test_helpers::now_secs();
        store::store_token_data(
            "default",
            &store::TokenData {
                access_token: "fresh-A".to_string(),
                expires_at: now + 3600,
                refresh_token: None,
                refresh_expires_at: None,
                grant_type: Some(ags_protocol::request::GrantType::ClientCredentials),
                client_id: Some("client-A".to_string()),
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let mut sink = CapturingSink::default();
        let outcome = probe_existing_session(
            &client,
            "default",
            "https://unused.invalid".to_string(),
            "client-B".to_string(),
            "client credentials",
            &mut sink,
        )
        .await
        .unwrap();

        assert!(
            outcome.is_none(),
            "expected fresh flow (None), got {outcome:?}"
        );
    }

    /// The stored token's client_id differs from the requested one only by
    /// casing/hyphenation — the SAME IAM client. The probe must recognise the
    /// existing valid session (AlreadyAuthenticated), not force a fresh login.
    /// Regression guard that the login probe uses the normalised comparison too.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_reuses_fresh_token_when_client_matches_after_normalisation() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

        // Empty ProfileConfig (env-cred style) → stored_identity_matches is a no-op,
        // so the token-minting-identity check is what's under test here.
        let now = crate::support::test_helpers::now_secs();
        store::store_token_data(
            "default",
            &store::TokenData {
                access_token: "fresh".to_string(),
                expires_at: now + 3600,
                refresh_token: None,
                refresh_expires_at: None,
                grant_type: Some(ags_protocol::request::GrantType::ClientCredentials),
                client_id: Some("D39A8BB1-04E5-45A7-A4B1-EF6EC3D55A3C".to_string()),
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let mut sink = CapturingSink::default();
        let outcome = probe_existing_session(
            &client,
            "default",
            "https://unused.invalid".to_string(),
            "d39a8bb104e545a7a4b1ef6ec3d55a3c".to_string(),
            "client credentials",
            &mut sink,
        )
        .await
        .unwrap();

        assert!(
            matches!(
                outcome,
                Some(LoginOutcome {
                    kind: LoginOutcomeKind::AlreadyAuthenticated { .. },
                    ..
                })
            ),
            "expected AlreadyAuthenticated (same client, different format), got {outcome:?}"
        );
    }
}
