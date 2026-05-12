//! Auth facade — `Runtime` methods for login, logout, and status.
//!
//! Stateless credential lookups live as free functions on
//! [`crate::runtime::auth::credentials`].

use crate::protocol::error::RuntimeError;

impl crate::runtime::Runtime {
    /// Snapshot the auth state for `profile` and render it as a user-facing `AuthView`.
    pub fn auth_status(
        &self,
        profile: &str,
    ) -> Result<crate::protocol::output::AuthView, RuntimeError> {
        use crate::runtime::auth::operations;

        let snapshot = operations::auth_snapshot(profile)?;
        Ok(auth_snapshot_to_view(snapshot))
    }

    /// Clear stored credentials for `profile` and report which artefacts were removed.
    pub async fn auth_logout(
        &self,
        profile: &str,
    ) -> Result<crate::protocol::output::AuthView, RuntimeError> {
        use crate::protocol::output::{AuthView, LogoutData, Presence};
        use crate::runtime::auth::operations;

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
    pub async fn auth_logout_all(&self) -> Result<crate::protocol::output::AuthView, RuntimeError> {
        use crate::protocol::output::{AuthView, LogoutAllData};
        use crate::runtime::auth::operations;

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
        sink: &mut dyn crate::protocol::event::ProgressSink,
    ) -> Result<Option<crate::protocol::output::AuthView>, crate::protocol::error::RuntimeError>
    {
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
        sink: &mut dyn crate::protocol::event::ProgressSink,
    ) -> Result<crate::protocol::output::AuthView, RuntimeError> {
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

    /// Complete the client-credentials flow by exchanging the secret for a token and persisting it.
    pub async fn auth_login_client_credentials(
        &self,
        profile: &str,
        base_url: String,
        client_id: String,
        client_secret: String,
        sink: &mut dyn crate::protocol::event::ProgressSink,
    ) -> Result<crate::protocol::output::AuthView, RuntimeError> {
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

/// Render an `AuthSnapshot` as the user-facing `AuthView` payload returned by `auth status`.
fn auth_snapshot_to_view(
    snapshot: crate::runtime::auth::operations::AuthSnapshot,
) -> crate::protocol::output::AuthView {
    use crate::protocol::output::{AuthSource, AuthStatusData, AuthView, Presence, TokenState};
    use crate::runtime::auth::operations::AuthSnapshot;

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
) -> crate::protocol::output::AuthView {
    use crate::protocol::output::{AuthSource, AuthStatusData, AuthView, Presence, TokenState};
    use crate::runtime::auth::operations::StoredTokenState;

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
) -> crate::protocol::output::TokenState {
    use crate::protocol::output::TokenState;
    use crate::runtime::auth::operations::RefreshTokenState;
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
) -> crate::protocol::output::AuthView {
    use crate::protocol::output::{AuthActionData, AuthActionStatus, AuthView};
    use crate::runtime::auth::operations::LoginOutcomeKind;

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
