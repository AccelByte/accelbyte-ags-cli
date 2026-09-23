//! Auth facade — `Runtime` methods for login, logout, status, token, and refresh.
//!
//! Stateless credential lookups live as free functions on
//! [`crate::runtime::auth::credentials`].

use ags_protocol::error::RuntimeError;

impl crate::runtime::Runtime {
    /// Snapshot the auth state for `profile` and render it as a user-facing `AuthView`.
    pub fn auth_status(
        &self,
        profile: &str,
    ) -> Result<ags_protocol::output::AuthView, RuntimeError> {
        use crate::runtime::auth::operations;

        let snapshot = operations::auth_snapshot(profile)?;
        Ok(auth_snapshot_to_view(snapshot))
    }

    /// Resolve the access token for `profile` the way an API call would, and
    /// render it as an `AuthView::Token` for `ags auth token` to print.
    ///
    /// Resolution is [`crate::runtime::auth::session::resolve_access_token`]
    /// itself — not a parallel implementation — so a token this command prints
    /// is the same one the next request would send, refresh included.
    pub async fn auth_token(
        &self,
        profile: &str,
    ) -> Result<ags_protocol::output::AuthView, RuntimeError> {
        use crate::runtime::auth::session;

        let resolution = session::resolve_access_token(&self.reqwest_client, profile).await?;
        Ok(token_resolution_to_view(
            resolution,
            crate::support::unix_now(),
        ))
    }

    /// Clear stored credentials for `profile` and report which artefacts were removed.
    pub async fn auth_logout(
        &self,
        profile: &str,
    ) -> Result<ags_protocol::output::AuthView, RuntimeError> {
        use crate::runtime::auth::operations;
        use ags_protocol::output::{AuthView, LogoutData, Presence};

        let outcome = operations::logout_profile(profile).await?;

        let to_presence = |was_present: bool| -> Presence {
            if was_present {
                Presence::Cleared
            } else {
                Presence::Missing
            }
        };

        Ok(AuthView::LogoutSuccess(LogoutData {
            client_id: to_presence(outcome.had_client_id),
            client_secret: to_presence(outcome.had_client_secret),
            access_token: to_presence(outcome.had_access_token),
            refresh_token: to_presence(outcome.had_refresh_token),
        }))
    }

    /// Clear stored credentials for every known profile and report how many were affected.
    pub async fn auth_logout_all(&self) -> Result<ags_protocol::output::AuthView, RuntimeError> {
        use crate::runtime::auth::operations;
        use ags_protocol::output::{AuthView, LogoutAllData};

        let outcome = operations::logout_all_profiles().await?;

        Ok(AuthView::LogoutAllSuccess(LogoutAllData {
            profiles_cleared: outcome.profiles_cleared,
        }))
    }

    /// Probe whether `profile` has a usable session. Returns `Some(AuthView)` if
    /// the user is already covered (either still-valid or just-refreshed) and
    /// the caller should NOT start a fresh login flow. Returns `None` when a
    /// fresh flow is required.
    pub async fn auth_probe_existing_session(
        &self,
        profile: &str,
        base_url: String,
        client_id: String,
        login_type: &'static str,
        sink: &mut dyn ags_protocol::event::ProgressSink,
    ) -> Result<Option<ags_protocol::output::AuthView>, ags_protocol::error::RuntimeError> {
        let outcome = crate::runtime::auth::operations::probe_existing_session(
            &self.reqwest_client,
            profile,
            base_url,
            client_id,
            login_type,
            sink,
        )
        .await?;
        Ok(outcome.map(login_outcome_to_view))
    }

    /// Complete the authorization-code flow by exchanging `code` for a token and persisting it.
    pub async fn auth_login_authorization_code(
        &self,
        profile: &str,
        base_url: String,
        client_id: String,
        code: String,
        code_verifier: String,
        sink: &mut dyn ags_protocol::event::ProgressSink,
    ) -> Result<ags_protocol::output::AuthView, RuntimeError> {
        use crate::runtime::auth::operations;

        let outcome = operations::login_with_authorization_code(
            &self.reqwest_client,
            operations::AuthorizationCodeLogin {
                profile: profile.to_string(),
                base_url,
                client_id,
                code,
                code_verifier,
            },
            sink,
        )
        .await?;

        Ok(login_outcome_to_view(outcome))
    }

    /// Re-mint the profile's token from stored credentials and render the result.
    pub async fn auth_refresh(
        &self,
        profile: &str,
        sink: &mut dyn ags_protocol::event::ProgressSink,
    ) -> Result<ags_protocol::output::AuthView, RuntimeError> {
        use crate::runtime::auth::operations;
        let outcome = operations::refresh_profile(&self.reqwest_client, profile, sink).await?;
        Ok(refresh_outcome_to_view(outcome))
    }

    /// Complete the client-credentials flow by exchanging the secret for a token and persisting it.
    pub async fn auth_login_client_credentials(
        &self,
        profile: &str,
        base_url: String,
        client_id: String,
        client_secret: String,
        sink: &mut dyn ags_protocol::event::ProgressSink,
    ) -> Result<ags_protocol::output::AuthView, RuntimeError> {
        use crate::runtime::auth::operations;

        let outcome = operations::login_with_client_credentials(
            &self.reqwest_client,
            operations::ClientCredentialsLogin {
                profile: profile.to_string(),
                base_url,
                client_id,
                client_secret,
            },
            sink,
        )
        .await?;

        Ok(login_outcome_to_view(outcome))
    }
}

/// Render a resolved access token as the `AuthView::Token` payload printed by
/// `ags auth token`.
///
/// `now` is passed in rather than read here so the relative `expires_in_secs`
/// the session layer reports converts against a single instant the caller can
/// pin in a test.
fn token_resolution_to_view(
    resolution: crate::runtime::auth::session::TokenResolution,
    now: u64,
) -> ags_protocol::output::AuthView {
    use crate::runtime::auth::session::TokenSource;
    use ags_protocol::output::{AuthTokenData, AuthTokenSource, AuthView};

    let source = match resolution.source {
        TokenSource::Environment => AuthTokenSource::Environment,
        TokenSource::Stored => AuthTokenSource::Stored,
        TokenSource::Refreshed => AuthTokenSource::Refreshed,
        TokenSource::ClientCredentials => AuthTokenSource::ClientCredentials,
    };

    AuthView::Token(AuthTokenData {
        access_token: resolution.token,
        expires_at: resolution
            .expires_in_secs
            .map(|expires_in_secs| now.saturating_add(expires_in_secs)),
        source,
        warnings: resolution.warnings,
    })
}

/// Render a successful refresh as an `AuthView::RefreshSuccess` payload.
fn refresh_outcome_to_view(
    outcome: crate::runtime::auth::operations::TokenRefreshOutcome,
) -> ags_protocol::output::AuthView {
    use ags_protocol::output::{AuthActionData, AuthActionStatus, AuthView};
    use ags_protocol::request::GrantType;

    let login_type = match outcome.grant_type {
        GrantType::ClientCredentials => "client credentials",
        GrantType::AuthorizationCode => "authorization code",
    };
    AuthView::RefreshSuccess(AuthActionData {
        status: AuthActionStatus::Refreshed,
        base_url: Some(outcome.base_url),
        login_type: Some(login_type.to_string()),
        client_id: Some(outcome.client_id),
        token_expires_in_secs: Some(outcome.expires_in_secs),
        tip: outcome.note,
    })
}

/// Render an `AuthSnapshot` as the user-facing `AuthView` payload returned by `auth status`.
fn auth_snapshot_to_view(
    snapshot: crate::runtime::auth::operations::AuthSnapshot,
) -> ags_protocol::output::AuthView {
    use crate::runtime::auth::operations::AuthSnapshot;
    use ags_protocol::output::{AuthSource, AuthStatusData, AuthView, Presence, TokenState};

    match snapshot {
        AuthSnapshot::EnvironmentToken => AuthView::Authenticated(AuthStatusData {
            source: AuthSource::EnvironmentAccessToken,
            base_url: None,
            login_type: None,
            client_id: None,
            client_secret: Presence::Unknown,
            access_token: TokenState::Present,
            refresh_token: TokenState::Unknown,
            namespace: None,
            next_step: None,
        }),
        AuthSnapshot::EnvironmentCredentials {
            base_url,
            client_id,
        } => AuthView::Authenticated(AuthStatusData {
            source: AuthSource::EnvironmentClientCredentials,
            base_url: Some(base_url),
            login_type: None,
            client_id: Some(client_id),
            client_secret: Presence::Unknown,
            access_token: TokenState::Unknown,
            refresh_token: TokenState::Unknown,
            namespace: None,
            next_step: None,
        }),
        AuthSnapshot::Stored {
            base_url,
            client_id,
            has_client_secret,
            token_state,
            namespace,
        } => stored_snapshot_to_view(
            base_url,
            client_id,
            has_client_secret,
            token_state,
            namespace,
        ),
        AuthSnapshot::NoCredentials => AuthView::NotAuthenticated {
            next_step: Some("Run 'ags auth login'.".to_string()),
            tip: Some(
                "You can also set AGS_BASE_URL, AGS_CLIENT_ID, AGS_CLIENT_SECRET for non-interactive workflows.".to_string(),
            ),
        },
    }
}

/// Render the `Stored` arm of `AuthSnapshot`, dispatched on the inner `StoredTokenState`.
fn stored_snapshot_to_view(
    base_url: String,
    client_id: String,
    has_client_secret: bool,
    token_state: crate::runtime::auth::operations::StoredTokenState,
    namespace: Option<String>,
) -> ags_protocol::output::AuthView {
    use crate::runtime::auth::operations::StoredTokenState;
    use ags_protocol::output::{AuthSource, AuthStatusData, AuthView, Presence, TokenState};

    let client_secret = if has_client_secret {
        Presence::Stored
    } else {
        Presence::Missing
    };

    match token_state {
        StoredTokenState::Valid {
            expires_in_secs,
            login_type,
            refresh_token,
        } => AuthView::Authenticated(AuthStatusData {
            source: AuthSource::Stored,
            base_url: Some(base_url),
            login_type: Some(friendly_grant_type(login_type.as_deref()).to_string()),
            client_id: Some(client_id),
            client_secret,
            access_token: TokenState::Valid {
                expires_in_secs: Some(expires_in_secs),
            },
            refresh_token: refresh_token_to_render(refresh_token),
            namespace,
            next_step: None,
        }),
        StoredTokenState::Expired {
            login_type,
            refresh_token,
        } => {
            let render_refresh = refresh_token_to_render(refresh_token);
            let can_refresh = matches!(
                render_refresh,
                TokenState::Valid { .. } | TokenState::Present
            );
            AuthView::RequiresAttention(AuthStatusData {
                source: AuthSource::Stored,
                base_url: Some(base_url),
                login_type: Some(friendly_grant_type(login_type.as_deref()).to_string()),
                client_id: Some(client_id),
                client_secret,
                access_token: TokenState::Expired,
                refresh_token: render_refresh,
                namespace,
                next_step: Some(if can_refresh {
                    "Token will auto-refresh on next API call.".to_string()
                } else {
                    "Run 'ags auth login'.".to_string()
                }),
            })
        }
        StoredTokenState::Missing => AuthView::RequiresAttention(AuthStatusData {
            source: AuthSource::Stored,
            base_url: Some(base_url),
            login_type: None,
            client_id: Some(client_id),
            client_secret,
            access_token: TokenState::Missing,
            refresh_token: TokenState::Unknown,
            namespace,
            next_step: Some("Run 'ags auth login'.".to_string()),
        }),
    }
}

/// Map a stored grant-type slug to a human-readable label, falling back to "unknown".
fn friendly_grant_type(grant_type: Option<&str>) -> &str {
    match grant_type {
        Some("authorization-code") => "authorization code",
        Some("client-credentials") => "client credentials",
        Some(other) => other,
        None => "unknown",
    }
}

/// Convert a runtime `RefreshTokenState` to the protocol `TokenState` used in views.
fn refresh_token_to_render(
    state: crate::runtime::auth::operations::RefreshTokenState,
) -> ags_protocol::output::TokenState {
    use crate::runtime::auth::operations::RefreshTokenState;
    use ags_protocol::output::TokenState;
    match state {
        RefreshTokenState::Valid { expires_in_secs } => TokenState::Valid {
            expires_in_secs: Some(expires_in_secs),
        },
        RefreshTokenState::Present => TokenState::Present,
        RefreshTokenState::Expired => TokenState::Expired,
        RefreshTokenState::Missing => TokenState::Missing,
    }
}

/// Render a successful login outcome as the user-facing `AuthView` payload.
fn login_outcome_to_view(
    outcome: crate::runtime::auth::operations::LoginOutcome,
) -> ags_protocol::output::AuthView {
    use crate::runtime::auth::operations::LoginOutcomeKind;
    use ags_protocol::output::{AuthActionData, AuthActionStatus, AuthView};

    match outcome.kind {
        LoginOutcomeKind::AlreadyAuthenticated { tip } => AuthView::LoginSuccess(AuthActionData {
            status: AuthActionStatus::AlreadyAuthenticated,
            base_url: None,
            login_type: None,
            client_id: None,
            token_expires_in_secs: None,
            tip: Some(tip),
        }),
        LoginOutcomeKind::LoggedIn => AuthView::LoginSuccess(AuthActionData {
            status: AuthActionStatus::LoggedIn,
            base_url: Some(outcome.base_url),
            login_type: Some(outcome.login_type.to_string()),
            client_id: Some(outcome.client_id),
            token_expires_in_secs: outcome.expires_in_secs,
            tip: None,
        }),
        LoginOutcomeKind::Refreshed => AuthView::LoginSuccess(AuthActionData {
            status: AuthActionStatus::Refreshed,
            base_url: Some(outcome.base_url),
            login_type: Some(outcome.login_type.to_string()),
            client_id: Some(outcome.client_id),
            token_expires_in_secs: outcome.expires_in_secs,
            tip: None,
        }),
    }
}
