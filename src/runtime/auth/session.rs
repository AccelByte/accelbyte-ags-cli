//! Session lifecycle: resolve a usable access token from environment, storage,
//! refresh, or client credentials.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use reqwest::Client;

use crate::protocol::error::RuntimeError;
use crate::support::unix_now;

use super::credentials;
use super::errors::AuthError;
use super::locking;
use super::store;
use super::tokens;

/// Seconds before expiry at which a stored token is considered stale and refreshed proactively.
pub const TOKEN_EXPIRY_BUFFER_SECS: u64 = 60;

/// Per-profile registry of in-process refresh mutexes. Distinct profiles can
/// refresh concurrently; concurrent refreshes of the *same* profile within
/// one process serialise on the matching `tokio::sync::Mutex`. Cross-process
/// serialisation is layered on underneath via [`crate::support::FileLock`]
/// (which is per-fd and therefore does not provide intra-process exclusion).
///
/// Entries are never evicted. Profile names come from user configuration and
/// the active set is small and stable, so unbounded growth is not a concern
/// in production. In tests, ephemeral profiles accumulate for the lifetime
/// of the test process — also benign.
static REFRESH_LOCKS: std::sync::LazyLock<StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    std::sync::LazyLock::new(|| StdMutex::new(HashMap::new()));

/// Look up (or create) the refresh mutex for `profile`. Returns an `Arc` so
/// callers can drop the registry guard before awaiting on the per-profile
/// mutex.
pub(crate) fn profile_refresh_lock(profile: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut registry = REFRESH_LOCKS
        .lock()
        .expect("refresh-lock registry poisoned");
    registry
        .entry(profile.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// How the access token was obtained.
#[derive(Debug, Clone)]
pub enum TokenSource {
    /// From `AGS_ACCESS_TOKEN` environment variable.
    Environment,
    /// From a valid stored token.
    Stored,
    /// Via refresh token grant.
    Refreshed,
    /// Via client credentials grant.
    ClientCredentials,
}

/// Result of token resolution including source metadata for verbose output.
pub struct TokenResolution {
    pub token: String,
    pub source: TokenSource,
    pub expires_in_secs: Option<u64>,
    /// User-visible warnings surfaced during token resolution, such as token
    /// expiry defaults or file-storage fallback when the keychain rejects a write.
    pub warnings: Vec<String>,
}

/// Resolve an access token by trying each source in priority order:
/// environment variable → stored token → refresh token → client credentials grant.
pub async fn resolve_access_token(
    client: &Client,
    profile: &str,
) -> Result<TokenResolution, RuntimeError> {
    if let Ok(token) = std::env::var(crate::runtime::config::ENV_ACCESS_TOKEN) {
        return Ok(TokenResolution {
            token,
            source: TokenSource::Environment,
            expires_in_secs: None,
            warnings: vec![],
        });
    }

    let stored = store::get_token_data_async(profile).await.ok().flatten();
    if let Some(ref token_data) = stored {
        let now = unix_now();
        if now + TOKEN_EXPIRY_BUFFER_SECS < token_data.expires_at {
            return Ok(TokenResolution {
                token: token_data.access_token.clone(),
                source: TokenSource::Stored,
                expires_in_secs: Some(token_data.expires_at.saturating_sub(now)),
                warnings: vec![],
            });
        }
    }

    // Slow path: serialize refresh work across concurrent tasks in this
    // process before taking the cross-process file lock below.
    let refresh_mutex = profile_refresh_lock(profile);
    let _refresh_guard = refresh_mutex.lock().await;
    // Cross-process lock: prevents a second CLI process from refreshing or
    // replacing the stored token at the same time.
    let _lock = locking::acquire_async_token_lock(profile).await?;

    // Re-read after both locks are held. Another task or process may have
    // refreshed the token while we were waiting, in which case we can skip
    // the network call entirely.
    let stored = store::get_token_data_async(profile).await.ok().flatten();
    if let Some(ref token_data) = stored {
        let now = unix_now();
        if now + TOKEN_EXPIRY_BUFFER_SECS < token_data.expires_at {
            return Ok(TokenResolution {
                token: token_data.access_token.clone(),
                source: TokenSource::Stored,
                expires_in_secs: Some(token_data.expires_at.saturating_sub(now)),
                warnings: vec![],
            });
        }
    }

    let credentials = credentials::resolve_credentials(profile);

    if let Some(token_data) = stored {
        let now = unix_now();
        // Treat tokens stored without a recorded `grant_type` as authorization-
        // code tokens. Older CLI versions persisted tokens before this field
        // existed, and a legacy refresh-failure must surface as "Session
        // expired" rather than silently falling through to client credentials
        // (which then errors with the misleading "Not authenticated").
        let is_authorization_code = matches!(
            token_data.grant_type,
            None | Some(crate::protocol::request::GrantType::AuthorizationCode)
        );

        if let Some(ref refresh_token) = token_data.refresh_token {
            let is_refresh_valid = token_data
                .refresh_expires_at
                .map(|exp| now < exp)
                .unwrap_or(true);

            if is_refresh_valid {
                let base_url = credentials
                    .base_url
                    .as_ref()
                    .ok_or_else(|| RuntimeError::from(AuthError::BaseUrlMissing))?;
                let client_id = credentials
                    .client_id
                    .as_ref()
                    .ok_or_else(|| RuntimeError::from(AuthError::ClientIdMissing))?;

                match tokens::fetch_refresh_token(
                    client,
                    base_url,
                    client_id,
                    credentials.client_secret.as_deref(),
                    refresh_token,
                )
                .await
                {
                    Ok(mut result) => {
                        let expires_in_warning = result.expires_in_warning.take();
                        let new_token_data = tokens::token_result_to_token_data(
                            &result,
                            token_data
                                .grant_type
                                .unwrap_or(crate::protocol::request::GrantType::AuthorizationCode),
                            now,
                        );
                        // `_lock` must remain in scope until the write future resolves —
                        // it guards the cross-process critical section across this await.
                        let outcome =
                            store::store_token_data_unlocked_async(profile, new_token_data).await?;
                        return Ok(TokenResolution {
                            token: result.access_token.clone(),
                            source: TokenSource::Refreshed,
                            expires_in_secs: Some(result.expires_in),
                            warnings: merge_warnings(expires_in_warning, outcome.warning),
                        });
                    }
                    Err(error) => {
                        if is_authorization_code {
                            return Err(RuntimeError::from(
                                AuthError::SessionExpiredRefreshFailed(error.to_string()),
                            ));
                        }
                    }
                }
            } else if is_authorization_code {
                return Err(RuntimeError::from(
                    AuthError::SessionExpiredRefreshTokenExpired,
                ));
            }
        } else if is_authorization_code {
            return Err(RuntimeError::from(AuthError::SessionExpiredNoRefreshToken));
        }
    }

    // Last resort: if no usable stored token remains, fall back to client
    // credentials using the resolved profile credentials.
    let base_url = credentials
        .base_url
        .clone()
        .ok_or_else(|| RuntimeError::from(AuthError::BaseUrlMissing))?;
    let client_id = credentials
        .client_id
        .clone()
        .ok_or_else(|| RuntimeError::from(AuthError::ClientIdMissing))?;
    let client_secret = credentials
        .client_secret
        .clone()
        .ok_or_else(|| RuntimeError::from(AuthError::ClientSecretMissing))?;

    let mut result =
        tokens::fetch_client_credentials_token(client, &base_url, &client_id, &client_secret)
            .await?;
    let expires_in_warning = result.expires_in_warning.take();

    let token_data = tokens::token_result_to_token_data(
        &result,
        crate::protocol::request::GrantType::ClientCredentials,
        unix_now(),
    );
    let outcome = store::store_token_data_unlocked_async(profile, token_data).await?;

    Ok(TokenResolution {
        token: result.access_token.clone(),
        source: TokenSource::ClientCredentials,
        expires_in_secs: Some(result.expires_in),
        warnings: merge_warnings(expires_in_warning, outcome.warning),
    })
}

/// Merge two optional warnings into a stable ordered list.
fn merge_warnings(a: Option<String>, b: Option<String>) -> Vec<String> {
    match (a, b) {
        (Some(first), Some(second)) => vec![first, second],
        (Some(first), None) => vec![first],
        (None, Some(second)) => vec![second],
        (None, None) => vec![],
    }
}

#[cfg(test)]
mod refresh_lock_registry_tests {
    use super::profile_refresh_lock;
    use std::sync::Arc;

    /// Same profile must hand back the same Arc so concurrent refreshes
    /// serialise within one process.
    #[test]
    fn test_same_profile_returns_same_mutex() {
        let a = profile_refresh_lock("default");
        let b = profile_refresh_lock("default");
        assert!(Arc::ptr_eq(&a, &b), "same profile should share one mutex");
    }

    /// Distinct profiles must hand back distinct Arcs so unrelated tenants
    /// can refresh in parallel.
    #[test]
    fn test_different_profiles_return_different_mutexes() {
        let a = profile_refresh_lock("default");
        let b = profile_refresh_lock("staging");
        assert!(
            !Arc::ptr_eq(&a, &b),
            "distinct profiles must hold distinct mutexes"
        );
    }
}
