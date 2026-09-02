//! Session lifecycle: resolve a usable access token from environment, storage,
//! refresh, or client credentials.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use reqwest::Client;

use crate::support::unix_now;
use ags_protocol::error::RuntimeError;

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
#[derive(Debug)]
pub struct TokenResolution {
    pub token: String,
    pub source: TokenSource,
    pub expires_in_secs: Option<u64>,
    /// User-visible warnings surfaced during token resolution, such as token
    /// expiry defaults or file-storage fallback when the keychain rejects a write.
    pub warnings: Vec<String>,
}

/// Why a refresh attempt could not even be made.
///
/// The caller uses this to choose the correct error to surface for
/// authorization-code profiles, without having to re-read storage and risk a
/// TOCTOU race with concurrent CLI processes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UnavailableReason {
    /// No stored token at all — the user has never logged in for this
    /// profile (or has logged out).
    NoStoredToken,
    /// A token is stored but it has no refresh token (e.g. minted via
    /// client-credentials grant).
    NoRefreshToken,
    /// A refresh token exists but its local expiry has already passed.
    RefreshTokenExpired,
    /// The stored token was minted by a different client than the caller
    /// expects. `stored_client_id` is always present (the both-`Some` rule),
    /// so it is a `String`, carried here so the caller can build an error
    /// naming the stored client without re-reading the store (which would
    /// race a concurrent overwrite).
    ClientMismatch { stored_client_id: String },
}

/// Outcome of an attempt to refresh a stored session in place.
///
/// Distinguishes server-side rejection (recoverable by a fresh OAuth flow)
/// from transport errors (caller should propagate as failure).
#[derive(Debug)]
pub(crate) enum RefreshOutcome {
    /// A usable token is in place — either freshly fetched from the
    /// refresh endpoint or the stored access token was still valid after
    /// re-checking under the refresh lock.
    Refreshed {
        token: String,
        source: TokenSource,
        expires_in_secs: u64,
        warnings: Vec<String>,
    },
    /// No refresh attempt is possible. The reason is decided from the
    /// stored token state observed during this call, so callers do not
    /// need to re-read storage.
    Unavailable { reason: UnavailableReason },
    /// The server rejected the refresh. Carries the human-readable
    /// message for diagnostics; the caller decides how to react.
    Rejected { message: String },
}

/// Whether a refresh may reuse a still-fresh stored access token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RefreshMode {
    /// Reuse a still-fresh stored access token (the hot-path optimisation used
    /// by token resolution and the login probe).
    Probe,
    /// Always call the refresh endpoint, even if the stored access token is
    /// still valid. Used by `ags auth refresh`.
    Force,
}

/// Whether a stored token may be used for `expected` client. The binding is
/// enforced only when BOTH ids are present and they differ; a `None` on either
/// side (legacy token, or no client configured) is treated as a match. On a
/// real mismatch, returns the stored client id (raw, for the error message).
///
/// Comparison is normalised (hyphens stripped, lowercased) the same way client
/// IDs are canonicalised on `ags config set client-id`, so a casing/hyphenation
/// difference between sources (e.g. a raw `AGS_CLIENT_ID` vs a normalised stored
/// config value) for the *same* IAM client is not reported as a spurious mismatch.
fn client_matches(expected: Option<&str>, token: &store::TokenData) -> Result<(), String> {
    use crate::runtime::config::client_ids_match;
    match (expected, token.client_id.as_deref()) {
        (Some(exp), Some(stored)) if !client_ids_match(exp, stored) => Err(stored.to_string()),
        _ => Ok(()),
    }
}

/// Probe whether the stored session for `profile` can be refreshed in place.
///
/// Sequence:
/// 1. Fast-path read of stored token — if access is still fresh, return
///    `Refreshed` reusing that token.
/// 2. If no refresh token (or it's locally expired), return `Unavailable`.
/// 3. Acquire in-process refresh lock + cross-process file lock.
/// 4. Re-read stored token under locks (double-check: another caller may
///    have just refreshed). If now fresh, return `Refreshed`.
/// 5. Call the refresh-token endpoint.
///    - Success: persist new token, return `Refreshed`.
///    - HTTP rejection (non-2xx): return `Rejected`.
///    - Transport error: return `Err`.
///
/// The locking and double-check pattern mirror `resolve_access_token` — this
/// helper is the shared core; the resolver and login flows both call it.
pub(crate) async fn try_refresh_stored_session(
    client: &Client,
    profile: &str,
    mode: RefreshMode,
    expected_client_id: Option<&str>,
) -> Result<RefreshOutcome, RuntimeError> {
    // Fast path: read stored token without locks.
    let stored = store::get_token_data_async(profile).await.ok().flatten();
    let now = unix_now();
    let Some(token_data) = stored else {
        return Ok(RefreshOutcome::Unavailable {
            reason: UnavailableReason::NoStoredToken,
        });
    };

    if let Err(stored_client_id) = client_matches(expected_client_id, &token_data) {
        return Ok(RefreshOutcome::Unavailable {
            reason: UnavailableReason::ClientMismatch { stored_client_id },
        });
    }

    // Probe reuses a still-fresh token; Force always refreshes.
    if mode == RefreshMode::Probe {
        if let Some(outcome) = refreshed_from_fresh_stored(&token_data, now) {
            return Ok(outcome);
        }
    }

    // Cheap refreshability check before paying for locks. The value is
    // re-derived under the lock below, so only presence/validity matter here.
    if let Err(reason) = refresh_token_availability(&token_data, now) {
        return Ok(RefreshOutcome::Unavailable { reason });
    }

    // Slow path: acquire locks before the network call.
    let refresh_mutex = profile_refresh_lock(profile);
    let _refresh_guard = refresh_mutex.lock().await;
    let _lock = locking::acquire_async_token_lock(profile).await?;

    // Re-read after both locks are held. A concurrent (forced) refresh may have
    // rotated the token between the unlocked read and now.
    let stored = store::get_token_data_async(profile).await.ok().flatten();
    let Some(token_data) = stored else {
        return Ok(RefreshOutcome::Unavailable {
            reason: UnavailableReason::NoStoredToken,
        });
    };

    if let Err(stored_client_id) = client_matches(expected_client_id, &token_data) {
        return Ok(RefreshOutcome::Unavailable {
            reason: UnavailableReason::ClientMismatch { stored_client_id },
        });
    }

    let now = unix_now();
    if mode == RefreshMode::Probe {
        if let Some(outcome) = refreshed_from_fresh_stored(&token_data, now) {
            return Ok(outcome);
        }
    }

    // Re-derive the refresh token from the locked re-read — NOT the pre-lock
    // read. Otherwise a concurrent forced refresh that rotated the token would
    // leave this caller sending an invalidated token and getting rejected.
    let refresh_token_value = match refresh_token_availability(&token_data, now) {
        Ok(token) => token,
        Err(reason) => return Ok(RefreshOutcome::Unavailable { reason }),
    };

    let credentials = credentials::resolve_credentials(profile);
    let base_url = credentials
        .base_url
        .as_ref()
        .ok_or_else(|| RuntimeError::from(AuthError::BaseUrlMissing))?;
    let client_id = credentials
        .client_id
        .as_ref()
        .ok_or_else(|| RuntimeError::from(AuthError::ClientIdMissing))?;

    let fetch_result = tokens::fetch_refresh_token(
        client,
        base_url,
        client_id,
        credentials.client_secret.as_deref(),
        &refresh_token_value,
    )
    .await;

    let mut result = match fetch_result {
        Ok(result) => result,
        Err(error) => return classify_refresh_error(error),
    };

    let expires_in_warning = result.expires_in_warning.take();
    let new_token_data = tokens::token_result_to_token_data(
        &result,
        token_data
            .grant_type
            .unwrap_or(ags_protocol::request::GrantType::AuthorizationCode),
        now,
        client_id,
    );
    // `_lock` must still be in scope across this await (unlocked write relies on
    // the caller's lock). Do not move the write out of this function.
    let outcome = store::store_token_data_unlocked_async(profile, new_token_data).await?;

    Ok(RefreshOutcome::Refreshed {
        token: result.access_token.clone(),
        source: TokenSource::Refreshed,
        expires_in_secs: result.expires_in,
        warnings: merge_warnings(expires_in_warning, outcome.warning),
    })
}

/// Map a RuntimeError from `fetch_refresh_token` into either a transport
/// failure (caller propagates) or a server-side rejection (Ok(Rejected)).
///
/// Transport errors arrive as `RuntimeErrorKind::Network`. Everything else
/// from the auth-token endpoint is `NotAuthenticated` (the
/// `AuthError::TokenRefreshFailed` conversion), which we treat as rejection.
fn classify_refresh_error(error: RuntimeError) -> Result<RefreshOutcome, RuntimeError> {
    use ags_protocol::error::RuntimeErrorKind;
    if matches!(error.kind, RuntimeErrorKind::Network) {
        Err(error)
    } else {
        Ok(RefreshOutcome::Rejected {
            message: error.message,
        })
    }
}

/// The refresh token, when it is present and not locally expired. Returns the
/// token value on success, or the [`UnavailableReason`] to surface otherwise.
///
/// Called from both the pre-lock and post-lock refreshability checks in
/// `try_refresh_stored_session` so the two cannot drift.
fn refresh_token_availability(
    token_data: &crate::runtime::auth::store::TokenData,
    now: u64,
) -> Result<String, UnavailableReason> {
    let Some(token) = token_data.refresh_token.clone() else {
        return Err(UnavailableReason::NoRefreshToken);
    };
    let is_locally_valid = token_data
        .refresh_expires_at
        .map(|exp| now < exp)
        .unwrap_or(true);
    if is_locally_valid {
        Ok(token)
    } else {
        Err(UnavailableReason::RefreshTokenExpired)
    }
}

/// If `token_data` is still comfortably within its expiry window, return
/// the `RefreshOutcome::Refreshed` that reuses it; otherwise `None`.
///
/// Called twice in `try_refresh_stored_session` — once unlocked and once
/// after both locks are held — so the double-check cannot drift.
fn refreshed_from_fresh_stored(
    token_data: &crate::runtime::auth::store::TokenData,
    now: u64,
) -> Option<RefreshOutcome> {
    if now + TOKEN_EXPIRY_BUFFER_SECS < token_data.expires_at {
        Some(RefreshOutcome::Refreshed {
            token: token_data.access_token.clone(),
            source: TokenSource::Stored,
            expires_in_secs: token_data.expires_at.saturating_sub(now),
            warnings: vec![],
        })
    } else {
        None
    }
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

    // Probe stored session: fast path for fresh tokens, refresh attempt
    // for stale ones with a refresh token.
    let stored_grant_type = store::get_token_data_async(profile)
        .await
        .ok()
        .flatten()
        .and_then(|t| t.grant_type);
    let is_authorization_code = matches!(
        stored_grant_type,
        None | Some(ags_protocol::request::GrantType::AuthorizationCode)
    );

    let current_client_id = credentials::resolve_client_id_value(profile);

    match try_refresh_stored_session(
        client,
        profile,
        RefreshMode::Probe,
        current_client_id.as_deref(),
    )
    .await?
    {
        RefreshOutcome::Refreshed {
            token,
            source,
            expires_in_secs,
            warnings,
        } => {
            return Ok(TokenResolution {
                token,
                source,
                expires_in_secs: Some(expires_in_secs),
                warnings,
            });
        }
        RefreshOutcome::Rejected { message } => {
            if is_authorization_code {
                return Err(RuntimeError::from(AuthError::SessionExpiredRefreshFailed(
                    message,
                )));
            }
            // Confidential clients fall through to a fresh client-credentials grant.
        }
        // For authorization-code profiles with an existing stored token
        // that can't be refreshed, surface a precise session-expiry
        // error using the reason the refresh helper already observed —
        // avoids a second storage read and the TOCTOU window it would
        // introduce. `NoStoredToken` is left to fall through so callers
        // configured via `AGS_CLIENT_ID`/`AGS_CLIENT_SECRET` env vars
        // can still obtain a token via the client-credentials grant
        // below; if those credentials are also missing the grant's own
        // missing-credentials error surfaces with the right suggestion.
        // Confidential clients always fall through.
        RefreshOutcome::Unavailable { reason } => match reason {
            UnavailableReason::ClientMismatch { stored_client_id } => {
                // The stored session belongs to a different client. If the
                // currently-configured client can re-mint (a secret is
                // resolvable now), discard the stale token and fall through to
                // a fresh client-credentials grant below. Otherwise we cannot
                // silently re-authenticate (no secret → needs a browser), so
                // refuse rather than call the API with the wrong client.
                //
                // Check the env var first (non-blocking); only fall back to the
                // async keychain wrapper so this collision path — the exact one
                // hit under CI parallelism — never parks a Tokio worker on a
                // blocking keychain read.
                //
                // The secret is resolved per-profile, not per-client (as it is
                // everywhere else via `resolve_credentials`); by that convention
                // its presence is taken as "the current client's secret".
                let can_remint = std::env::var(crate::runtime::config::ENV_CLIENT_SECRET).is_ok()
                    || credentials::resolve_stored_client_secret(profile)
                        .await
                        .is_some();
                if !can_remint {
                    return Err(RuntimeError::from(AuthError::StoredSessionClientMismatch {
                        profile: profile.to_string(),
                        stored_client_id,
                        current_client_id: current_client_id.clone().unwrap_or_default(),
                    }));
                }
                // else: fall through to the client-credentials grant.
            }
            UnavailableReason::NoStoredToken => {
                // Fall through to the client-credentials grant below.
            }
            UnavailableReason::NoRefreshToken => {
                if is_authorization_code {
                    return Err(RuntimeError::from(AuthError::SessionExpiredNoRefreshToken));
                }
            }
            UnavailableReason::RefreshTokenExpired => {
                if is_authorization_code {
                    return Err(RuntimeError::from(
                        AuthError::SessionExpiredRefreshTokenExpired,
                    ));
                }
            }
        },
    }

    // Last resort: client-credentials grant. Reached only for confidential
    // clients where stored state was insufficient or refresh was rejected.
    //
    // `try_refresh_stored_session` already acquired and released the
    // per-profile mutex and file lock above. We re-acquire them here to
    // guard the `store_token_data_unlocked_async` write below — the two
    // critical sections are independent (no usable stored token survives
    // the gap), so the brief lock release between them is intentional.
    let credentials = credentials::resolve_credentials(profile);
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

    let refresh_mutex = profile_refresh_lock(profile);
    let _refresh_guard = refresh_mutex.lock().await;
    let _lock = locking::acquire_async_token_lock(profile).await?;

    let mut result =
        tokens::fetch_client_credentials_token(client, &base_url, &client_id, &client_secret)
            .await?;
    let expires_in_warning = result.expires_in_warning.take();

    let token_data = tokens::token_result_to_token_data(
        &result,
        ags_protocol::request::GrantType::ClientCredentials,
        unix_now(),
        &client_id,
    );
    let outcome = store::store_token_data_unlocked_async(profile, token_data).await?;

    Ok(TokenResolution {
        token: result.access_token.clone(),
        source: TokenSource::ClientCredentials,
        expires_in_secs: Some(result.expires_in),
        warnings: merge_warnings(expires_in_warning, outcome.warning),
    })
}

/// Force a refresh of the stored session for `profile`, bypassing the
/// still-fresh-token fast path.
///
/// Returns `Ok(true)` when a new access token was obtained and persisted, and
/// `Ok(false)` when no refresh was possible (no stored token, no refresh token,
/// client mismatch, or the refresh endpoint rejected the request). Callers use
/// this to decide whether retrying an authenticated request is worthwhile; a
/// `false` return means the caller should surface the original auth failure.
pub async fn force_refresh_session(client: &Client, profile: &str) -> Result<bool, RuntimeError> {
    match try_refresh_stored_session(client, profile, RefreshMode::Force, None).await? {
        RefreshOutcome::Refreshed { .. } => Ok(true),
        RefreshOutcome::Unavailable { .. } => Ok(false),
        RefreshOutcome::Rejected { .. } => Ok(false),
    }
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

#[cfg(test)]
mod try_refresh_tests {
    use super::*;
    use crate::runtime::auth::store::{self, TokenData};
    use crate::runtime::config::ProfileConfig;
    use crate::support::test_helpers::{now_secs, TempEnvGuard};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Standard test setup: creates an isolated `AGS_HOME`, disables the OS
    /// keychain, saves a `ProfileConfig` pointing at `base_url`, and persists
    /// `token` to the "default" profile. Returns the two env guards (must be
    /// kept alive for the duration of the test) and the tempdir.
    ///
    /// The token is stored with `grant_type` defaulted to `AuthorizationCode`
    /// when callers pass `None`, matching the rest of the test suite's fixtures.
    fn setup_profile(
        base_url: &str,
        token: TokenData,
    ) -> (tempfile::TempDir, TempEnvGuard, TempEnvGuard) {
        let tmp = tempfile::tempdir().unwrap();
        let home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

        ProfileConfig {
            base_url: Some(base_url.to_string()),
            client_id: Some("cid".to_string()),
            ..Default::default()
        }
        .save("default")
        .unwrap();
        store::store_token_data("default", &token).unwrap();

        (tmp, home, no_kc)
    }

    /// When the stored access token is comfortably within its expiry window,
    /// the helper returns Refreshed reusing the existing token without any
    /// network activity.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_returns_refreshed_when_access_token_still_fresh() {
        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            "https://unused.invalid",
            TokenData {
                access_token: "fresh-token".to_string(),
                expires_at: now + 3600,
                refresh_token: Some("refresh".to_string()),
                refresh_expires_at: Some(now + 7200),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        );

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(&client, "default", RefreshMode::Probe, None)
            .await
            .unwrap();

        match outcome {
            RefreshOutcome::Refreshed { token, source, .. } => {
                assert_eq!(token, "fresh-token");
                assert!(matches!(source, TokenSource::Stored));
            }
            other => panic!("expected Refreshed, got {other:?}"),
        }
    }

    /// When no token has ever been stored for the profile, the helper
    /// returns `Unavailable { NoStoredToken }` so callers can distinguish
    /// "never authenticated" from "session expired".
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_returns_unavailable_no_stored_token_when_storage_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

        ProfileConfig {
            base_url: Some("https://unused.invalid".to_string()),
            client_id: Some("cid".to_string()),
            ..Default::default()
        }
        .save("default")
        .unwrap();

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(&client, "default", RefreshMode::Probe, None)
            .await
            .unwrap();

        assert!(matches!(
            outcome,
            RefreshOutcome::Unavailable {
                reason: UnavailableReason::NoStoredToken
            }
        ));
    }

    /// When the access token is expired and no refresh token is stored,
    /// the helper returns Unavailable without attempting any network call.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_returns_unavailable_when_no_refresh_token() {
        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            "https://unused.invalid",
            TokenData {
                access_token: "expired".to_string(),
                expires_at: now.saturating_sub(60),
                refresh_token: None,
                refresh_expires_at: None,
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        );

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(&client, "default", RefreshMode::Probe, None)
            .await
            .unwrap();

        assert!(matches!(
            outcome,
            RefreshOutcome::Unavailable {
                reason: UnavailableReason::NoRefreshToken
            }
        ));
    }

    /// When the access token is expired and the refresh token's recorded
    /// expiry has also passed, the helper returns Unavailable without
    /// touching the network.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_returns_unavailable_when_refresh_locally_expired() {
        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            "https://unused.invalid",
            TokenData {
                access_token: "expired".to_string(),
                expires_at: now.saturating_sub(60),
                refresh_token: Some("refresh".to_string()),
                refresh_expires_at: Some(now.saturating_sub(60)),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        );

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(&client, "default", RefreshMode::Probe, None)
            .await
            .unwrap();

        assert!(matches!(
            outcome,
            RefreshOutcome::Unavailable {
                reason: UnavailableReason::RefreshTokenExpired
            }
        ));
    }

    /// When the access token is expired but the refresh token is valid,
    /// the helper calls the token endpoint, persists the new token, and
    /// returns Refreshed with TokenSource::Refreshed.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_returns_refreshed_on_successful_refresh() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"new-access","expires_in":3600,"token_type":"Bearer","refresh_token":"rotated","refresh_expires_in":7200}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            &server.uri(),
            TokenData {
                access_token: "expired".to_string(),
                expires_at: now.saturating_sub(60),
                refresh_token: Some("valid-refresh".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        );

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(&client, "default", RefreshMode::Probe, None)
            .await
            .unwrap();

        match outcome {
            RefreshOutcome::Refreshed {
                token,
                source,
                expires_in_secs,
                ..
            } => {
                assert_eq!(token, "new-access");
                assert!(matches!(source, TokenSource::Refreshed));
                assert_eq!(expires_in_secs, 3600);
            }
            other => panic!("expected Refreshed, got {other:?}"),
        }

        let stored = store::get_token_data("default").unwrap().unwrap();
        assert_eq!(stored.access_token, "new-access");
        assert_eq!(stored.refresh_token.as_deref(), Some("rotated"));
    }

    /// When the token endpoint rejects the refresh (HTTP 401), the helper
    /// returns Ok(Rejected) — NOT Err. The caller decides how to react.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_returns_rejected_on_server_401() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .respond_with(ResponseTemplate::new(401).set_body_string(
                r#"{"error":"invalid_grant","error_description":"refresh token expired"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            &server.uri(),
            TokenData {
                access_token: "expired".to_string(),
                expires_at: now.saturating_sub(60),
                refresh_token: Some("dead-refresh".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        );

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(&client, "default", RefreshMode::Probe, None)
            .await
            .unwrap();

        assert!(
            matches!(outcome, RefreshOutcome::Rejected { .. }),
            "expected Rejected, got {outcome:?}"
        );
    }

    /// Transport-level failures (server unreachable) surface as Err so the
    /// caller can abort instead of chasing a doomed fresh flow.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_propagates_network_error() {
        let now = now_secs();
        // Point at an unroutable address — any TCP connect attempt fails.
        let (_tmp, _home, _no_kc) = setup_profile(
            "http://127.0.0.1:1",
            TokenData {
                access_token: "expired".to_string(),
                expires_at: now.saturating_sub(60),
                refresh_token: Some("valid-refresh".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        );

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(500))
            .build()
            .unwrap();
        let result = try_refresh_stored_session(&client, "default", RefreshMode::Probe, None).await;
        assert!(
            matches!(
                &result,
                Err(e) if matches!(e.kind, ags_protocol::error::RuntimeErrorKind::Network)
            ),
            "expected Err(Network), got {result:?}"
        );
    }

    /// Forced mode must call the refresh endpoint even when the stored access
    /// token is still fresh (the crux of `ags auth refresh`).
    #[tokio::test]
    #[serial_test::serial]
    async fn test_forced_refresh_ignores_fresh_access_token() {
        // resolve_base_url reads AGS_BASE_URL before ProfileConfig; clear a
        // developer's live value so the test hits the wiremock server.
        let _base = TempEnvGuard::remove("AGS_BASE_URL");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"forced-new","expires_in":3600,"token_type":"Bearer","refresh_token":"rotated","refresh_expires_in":7200}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            &server.uri(),
            TokenData {
                access_token: "still-fresh".to_string(),
                expires_at: now + 3600, // comfortably valid
                refresh_token: Some("valid-refresh".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        );

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(&client, "default", RefreshMode::Force, None)
            .await
            .unwrap();

        match outcome {
            RefreshOutcome::Refreshed { token, source, .. } => {
                assert_eq!(token, "forced-new");
                assert!(matches!(source, TokenSource::Refreshed));
            }
            other => panic!("expected Refreshed via endpoint, got {other:?}"),
        }
    }

    /// Probe mode with the same fresh fixture must NOT call the endpoint — it
    /// short-circuits to the stored token. Proves existing callers are unchanged.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_mode_still_shortcircuits_on_fresh_token() {
        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            "https://unused.invalid",
            TokenData {
                access_token: "still-fresh".to_string(),
                expires_at: now + 3600,
                refresh_token: Some("valid-refresh".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        );

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(&client, "default", RefreshMode::Probe, None)
            .await
            .unwrap();

        match outcome {
            RefreshOutcome::Refreshed { token, source, .. } => {
                assert_eq!(token, "still-fresh");
                assert!(matches!(source, TokenSource::Stored));
            }
            other => panic!("expected stored short-circuit, got {other:?}"),
        }
    }

    /// Two concurrent forced refreshes must each succeed: the first uses the
    /// original refresh token, rotates it, and persists; the second, on entering
    /// the slow path, must re-derive the ROTATED token from the under-lock
    /// re-read (not reuse the pre-lock capture). The two body-matched mocks —
    /// each `.expect(1)` — fail the test if the second call reuses the original.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[serial_test::serial]
    async fn test_forced_concurrent_refresh_rederives_rotated_token() {
        use wiremock::matchers::body_string_contains;

        let _base = TempEnvGuard::remove("AGS_BASE_URL");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .and(body_string_contains("refresh_token=rt-original"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"access-1","expires_in":3600,"token_type":"Bearer","refresh_token":"rt-rotated","refresh_expires_in":7200}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .and(body_string_contains("refresh_token=rt-rotated"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"access-2","expires_in":3600,"token_type":"Bearer","refresh_token":"rt-rotated-2","refresh_expires_in":7200}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            &server.uri(),
            TokenData {
                access_token: "still-fresh".to_string(),
                expires_at: now + 3600,
                refresh_token: Some("rt-original".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        );

        let client = reqwest::Client::new();
        let a = {
            let client = client.clone();
            tokio::spawn(async move {
                try_refresh_stored_session(&client, "default", RefreshMode::Force, None).await
            })
        };
        let b = {
            let client = client.clone();
            tokio::spawn(async move {
                try_refresh_stored_session(&client, "default", RefreshMode::Force, None).await
            })
        };
        let (ra, rb) = tokio::join!(a, b);
        for outcome in [ra.unwrap().unwrap(), rb.unwrap().unwrap()] {
            assert!(
                matches!(outcome, RefreshOutcome::Refreshed { .. }),
                "both forced refreshes must succeed, got {outcome:?}"
            );
        }
    }

    /// Two concurrent callers must result in exactly ONE refresh request:
    /// the second caller, on entering the slow path, sees the freshly-stored
    /// token after the locks unwind and returns without calling the endpoint.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[serial_test::serial]
    async fn test_concurrent_callers_share_one_refresh_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"shared-new","expires_in":3600,"token_type":"Bearer","refresh_token":"rotated","refresh_expires_in":7200}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            &server.uri(),
            TokenData {
                access_token: "expired".to_string(),
                expires_at: now.saturating_sub(60),
                refresh_token: Some("valid-refresh".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        );

        let client = reqwest::Client::new();
        let a = {
            let client = client.clone();
            let p = "default".to_string();
            tokio::spawn(async move {
                try_refresh_stored_session(&client, &p, RefreshMode::Probe, None).await
            })
        };
        let b = {
            let client = client.clone();
            let p = "default".to_string();
            tokio::spawn(async move {
                try_refresh_stored_session(&client, &p, RefreshMode::Probe, None).await
            })
        };

        let (ra, rb) = tokio::join!(a, b);
        let ra = ra.unwrap().unwrap();
        let rb = rb.unwrap().unwrap();

        // Both must succeed with the new token. Mock's .expect(1) (verified on
        // drop) asserts only one of them actually called the endpoint.
        for outcome in [ra, rb] {
            match outcome {
                RefreshOutcome::Refreshed { token, .. } => {
                    assert_eq!(token, "shared-new");
                }
                other => panic!("expected Refreshed, got {other:?}"),
            }
        }
    }

    /// A fresh stored token whose client_id differs from the expected client is
    /// NOT reused: the helper returns ClientMismatch on the unlocked fast path,
    /// without any network call.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_returns_client_mismatch_on_fresh_token_wrong_client() {
        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            "https://unused.invalid",
            TokenData {
                access_token: "fresh".to_string(),
                expires_at: now + 3600,
                refresh_token: Some("refresh".to_string()),
                refresh_expires_at: Some(now + 7200),
                grant_type: Some(ags_protocol::request::GrantType::ClientCredentials),
                client_id: Some("client-A".to_string()),
            },
        );

        let client = reqwest::Client::new();
        let outcome =
            try_refresh_stored_session(&client, "default", RefreshMode::Probe, Some("client-B"))
                .await
                .unwrap();

        match outcome {
            RefreshOutcome::Unavailable {
                reason: UnavailableReason::ClientMismatch { stored_client_id },
            } => assert_eq!(stored_client_id, "client-A"),
            other => panic!("expected ClientMismatch, got {other:?}"),
        }
    }

    /// A matching client id reuses the fresh stored token as normal.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_reuses_stored_token_when_client_matches() {
        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            "https://unused.invalid",
            TokenData {
                access_token: "fresh".to_string(),
                expires_at: now + 3600,
                refresh_token: Some("refresh".to_string()),
                refresh_expires_at: Some(now + 7200),
                grant_type: Some(ags_protocol::request::GrantType::ClientCredentials),
                client_id: Some("client-A".to_string()),
            },
        );

        let client = reqwest::Client::new();
        let outcome =
            try_refresh_stored_session(&client, "default", RefreshMode::Probe, Some("client-A"))
                .await
                .unwrap();

        assert!(matches!(
            outcome,
            RefreshOutcome::Refreshed {
                source: TokenSource::Stored,
                ..
            }
        ));
    }

    /// The stored id and the expected id differ only in casing and hyphenation
    /// (the exact form `normalise_client_id` canonicalises) — they are the SAME
    /// IAM client, so the fresh token is reused, not treated as a mismatch.
    /// Regression guard for the normalised comparison in `client_matches`.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_reuses_stored_token_when_client_matches_after_normalisation() {
        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            "https://unused.invalid",
            TokenData {
                access_token: "fresh".to_string(),
                expires_at: now + 3600,
                refresh_token: Some("refresh".to_string()),
                refresh_expires_at: Some(now + 7200),
                grant_type: Some(ags_protocol::request::GrantType::ClientCredentials),
                client_id: Some("D39A8BB1-04E5-45A7-A4B1-EF6EC3D55A3C".to_string()),
            },
        );

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(
            &client,
            "default",
            RefreshMode::Probe,
            Some("d39a8bb104e545a7a4b1ef6ec3d55a3c"),
        )
        .await
        .unwrap();

        assert!(matches!(
            outcome,
            RefreshOutcome::Refreshed {
                source: TokenSource::Stored,
                ..
            }
        ));
    }

    /// A legacy token (client_id: None) matches any expected client — never break
    /// an existing session on upgrade.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_reuses_legacy_token_with_no_client_id() {
        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            "https://unused.invalid",
            TokenData {
                access_token: "fresh".to_string(),
                expires_at: now + 3600,
                refresh_token: None,
                refresh_expires_at: None,
                grant_type: Some(ags_protocol::request::GrantType::ClientCredentials),
                client_id: None,
            },
        );

        let client = reqwest::Client::new();
        let outcome =
            try_refresh_stored_session(&client, "default", RefreshMode::Probe, Some("client-B"))
                .await
                .unwrap();

        assert!(matches!(
            outcome,
            RefreshOutcome::Refreshed {
                source: TokenSource::Stored,
                ..
            }
        ));
    }

    /// A STALE token (expired access) whose refresh token is present but whose
    /// client differs from the expected client must NOT be refreshed: the guard
    /// fires before any refresh attempt. The mock endpoint is mounted with
    /// `.expect(0)`, so wiremock's on-drop verification fails if a refresh call
    /// leaks through. This is the case a bare "is the fresh token reusable" check
    /// would miss — enforcement must precede the refresh path, not just the reuse
    /// path.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_probe_client_mismatch_on_stale_token_makes_no_network_call() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"should-not-be-called","expires_in":3600,"token_type":"Bearer"}"#,
            ))
            .expect(0)
            .mount(&server)
            .await;

        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            &server.uri(),
            TokenData {
                access_token: "expired".to_string(),
                expires_at: now.saturating_sub(60),
                refresh_token: Some("refresh-A".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: Some("client-A".to_string()),
            },
        );

        let client = reqwest::Client::new();
        let outcome =
            try_refresh_stored_session(&client, "default", RefreshMode::Probe, Some("client-B"))
                .await
                .unwrap();

        match outcome {
            RefreshOutcome::Unavailable {
                reason: UnavailableReason::ClientMismatch { stored_client_id },
            } => assert_eq!(stored_client_id, "client-A"),
            other => panic!("expected ClientMismatch, got {other:?}"),
        }
        // server dropped here → `.expect(0)` asserts the refresh endpoint was never hit.
    }
}

#[cfg(test)]
mod force_refresh_tests {
    use super::*;
    use crate::runtime::auth::store::{self, TokenData};
    use crate::runtime::config::ProfileConfig;
    use crate::support::test_helpers::{now_secs, TempEnvGuard};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Standard test setup: creates an isolated `AGS_HOME`, disables the OS
    /// keychain, saves a `ProfileConfig` pointing at `base_url`, and persists
    /// `token` to the "default" profile. Returns the guards that must be kept
    /// alive for the duration of the test.
    ///
    /// Follows the same fixture pattern as `try_refresh_tests::setup_profile`.
    fn setup_profile(
        base_url: &str,
        token: TokenData,
    ) -> (tempfile::TempDir, TempEnvGuard, TempEnvGuard) {
        let tmp = tempfile::tempdir().unwrap();
        let home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

        ProfileConfig {
            base_url: Some(base_url.to_string()),
            client_id: Some("cid".to_string()),
            ..Default::default()
        }
        .save("default")
        .unwrap();
        store::store_token_data("default", &token).unwrap();

        (tmp, home, no_kc)
    }

    /// A successful forced refresh returns `Ok(true)` and the stored token is
    /// updated to the new value from the refresh endpoint.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_force_refresh_returns_true_on_success() {
        let _base = TempEnvGuard::remove("AGS_BASE_URL");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"refreshed-token","expires_in":3600,"token_type":"Bearer","refresh_token":"rotated","refresh_expires_in":7200}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            &server.uri(),
            TokenData {
                access_token: "old-token".to_string(),
                expires_at: now.saturating_sub(60),
                refresh_token: Some("valid-refresh".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        );

        let client = reqwest::Client::new();
        let result = force_refresh_session(&client, "default").await.unwrap();
        assert!(result, "successful forced refresh must return true");

        let stored = store::get_token_data("default").unwrap().unwrap();
        assert_eq!(
            stored.access_token, "refreshed-token",
            "stored token must be updated after successful refresh"
        );
    }

    /// A rejected refresh (endpoint returns non-2xx) returns `Ok(false)`, NOT
    /// an `Err`. The caller decides whether to surface the original auth failure
    /// or attempt a different recovery path.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_force_refresh_returns_false_on_rejection() {
        let _base = TempEnvGuard::remove("AGS_BASE_URL");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .respond_with(ResponseTemplate::new(401).set_body_string(
                r#"{"error":"invalid_grant","error_description":"refresh token expired"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            &server.uri(),
            TokenData {
                access_token: "expired".to_string(),
                expires_at: now.saturating_sub(60),
                refresh_token: Some("dead-refresh".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        );

        let client = reqwest::Client::new();
        let result = force_refresh_session(&client, "default").await.unwrap();
        assert!(!result, "rejected refresh must return Ok(false), not Err");
    }

    /// No stored token returns `Ok(false)` — the wrapper must not surface the
    /// internal `Unavailable(NoStoredToken)` as an `Err`.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_force_refresh_returns_false_when_no_stored_token() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");

        ProfileConfig {
            base_url: Some("https://unused.invalid".to_string()),
            client_id: Some("cid".to_string()),
            ..Default::default()
        }
        .save("default")
        .unwrap();
        // No token stored for "default".

        let client = reqwest::Client::new();
        let result = force_refresh_session(&client, "default").await.unwrap();
        assert!(!result, "no stored token must return Ok(false), not Err");
    }

    /// Forced mode really is forced: with a still-fresh stored access token, the
    /// refresh endpoint is still called. Asserted via wiremock's `.expect(1)` —
    /// if the wrapper quietly delegated with `Probe` instead of `Force`, the
    /// endpoint would not be called (wiremock would fail on drop). This is the
    /// assertion that distinguishes `Force` from `Probe`.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_force_refresh_calls_endpoint_even_when_token_fresh() {
        let _base = TempEnvGuard::remove("AGS_BASE_URL");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"forced-new","expires_in":3600,"token_type":"Bearer","refresh_token":"rotated","refresh_expires_in":7200}"#,
            ))
            .expect(1) // MUST be called exactly once — the Force/Probe discriminator
            .mount(&server)
            .await;

        let now = now_secs();
        let (_tmp, _home, _no_kc) = setup_profile(
            &server.uri(),
            TokenData {
                access_token: "still-fresh".to_string(),
                expires_at: now + 3600, // comfortably valid
                refresh_token: Some("valid-refresh".to_string()),
                refresh_expires_at: Some(now + 86_400),
                grant_type: Some(ags_protocol::request::GrantType::AuthorizationCode),
                client_id: None,
            },
        );

        let client = reqwest::Client::new();
        let result = force_refresh_session(&client, "default").await.unwrap();
        assert!(result, "forced refresh with fresh token must return true");

        let stored = store::get_token_data("default").unwrap().unwrap();
        assert_eq!(
            stored.access_token, "forced-new",
            "stored token must be updated even though original was fresh"
        );
        // wiremock's `.expect(1)` on drop asserts the endpoint was called exactly once.
    }
}

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use crate::runtime::auth::store::{self, TokenData};
    use crate::runtime::config::ProfileConfig;
    use crate::support::test_helpers::{now_secs, TempEnvGuard};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Store a fresh token minted by `stored_client`, configure the profile's
    /// current client to `current_client`, isolate state, disable keychain.
    /// Returns guards that must stay alive for the test.
    fn setup(
        base_url: &str,
        stored_client: Option<&str>,
        current_client: &str,
    ) -> (
        tempfile::TempDir,
        TempEnvGuard,
        TempEnvGuard,
        TempEnvGuard,
        TempEnvGuard,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
        // Clear a developer's live base URL so ProfileConfig.base_url wins.
        let base = TempEnvGuard::remove("AGS_BASE_URL");
        // CRITICAL: resolve_access_token returns AGS_ACCESS_TOKEN before it ever
        // touches storage. If a developer/CI env has it set, these tests would
        // silently exercise none of the binding logic. Clear it for the whole
        // module so every case actually reaches the stored-token path.
        let atok = TempEnvGuard::remove("AGS_ACCESS_TOKEN");

        ProfileConfig {
            base_url: Some(base_url.to_string()),
            client_id: Some(current_client.to_string()),
            ..Default::default()
        }
        .save("default")
        .unwrap();

        let now = now_secs();
        store::store_token_data(
            "default",
            &TokenData {
                access_token: "stored-access".to_string(),
                expires_at: now + 3600,
                refresh_token: None,
                refresh_expires_at: None,
                grant_type: Some(ags_protocol::request::GrantType::ClientCredentials),
                client_id: stored_client.map(str::to_string),
            },
        )
        .unwrap();

        (tmp, home, no_kc, base, atok)
    }

    /// Mismatch + a secret is resolvable for the current client → the stale
    /// token is discarded and a fresh client-credentials grant is minted.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_resolve_remints_on_mismatch_when_secret_present() {
        let _secret = TempEnvGuard::remove("AGS_CLIENT_ID"); // don't override ProfileConfig
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/iam/v3/oauth/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"fresh-B","expires_in":3600,"token_type":"Bearer"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let (_tmp, _home, _no_kc, _base, _atok) =
            setup(&server.uri(), Some("client-A"), "client-B");
        let _cs = TempEnvGuard::set("AGS_CLIENT_SECRET", "secret-B");

        let client = reqwest::Client::new();
        let res = resolve_access_token(&client, "default").await.unwrap();
        assert_eq!(res.token, "fresh-B");
        assert!(matches!(res.source, TokenSource::ClientCredentials));
    }

    /// Mismatch + no secret (authorization-code client) → hard error naming
    /// both clients; no token endpoint call.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_resolve_errors_on_mismatch_when_no_secret() {
        let _cid = TempEnvGuard::remove("AGS_CLIENT_ID");
        let _cs = TempEnvGuard::remove("AGS_CLIENT_SECRET");
        let (_tmp, _home, _no_kc, _base, _atok) =
            setup("https://unused.invalid", Some("client-A"), "client-B");

        let client = reqwest::Client::new();
        let err = resolve_access_token(&client, "default").await.unwrap_err();
        assert!(
            err.message.contains("different client"),
            "unexpected message: {}",
            err.message
        );
    }

    /// Matching client id → the stored token is used as-is.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_resolve_uses_stored_when_client_matches() {
        let _cid = TempEnvGuard::remove("AGS_CLIENT_ID");
        let _cs = TempEnvGuard::remove("AGS_CLIENT_SECRET");
        let (_tmp, _home, _no_kc, _base, _atok) =
            setup("https://unused.invalid", Some("client-B"), "client-B");

        let client = reqwest::Client::new();
        let res = resolve_access_token(&client, "default").await.unwrap();
        assert_eq!(res.token, "stored-access");
        assert!(matches!(res.source, TokenSource::Stored));
    }

    /// Legacy stored token (client_id: None) → used as-is, no error.
    #[tokio::test]
    #[serial_test::serial]
    async fn test_resolve_uses_legacy_token_none_client() {
        let _cid = TempEnvGuard::remove("AGS_CLIENT_ID");
        let _cs = TempEnvGuard::remove("AGS_CLIENT_SECRET");
        let (_tmp, _home, _no_kc, _base, _atok) = setup("https://unused.invalid", None, "client-B");

        let client = reqwest::Client::new();
        let res = resolve_access_token(&client, "default").await.unwrap();
        assert_eq!(res.token, "stored-access");
        assert!(matches!(res.source, TokenSource::Stored));
    }

    /// Current client unconfigured (no `AGS_CLIENT_ID`, no `ProfileConfig.client_id`)
    /// → the expected id is `None`, so the both-`Some` rule treats a bound stored
    /// token as a match and it is used as-is (spec-enumerated current-`None` case).
    #[tokio::test]
    #[serial_test::serial]
    async fn test_resolve_uses_stored_when_current_client_unconfigured() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
        let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
        let _base = TempEnvGuard::remove("AGS_BASE_URL");
        let _atok = TempEnvGuard::remove("AGS_ACCESS_TOKEN");
        let _cid = TempEnvGuard::remove("AGS_CLIENT_ID");
        let _cs = TempEnvGuard::remove("AGS_CLIENT_SECRET");

        // ProfileConfig has no client_id → resolve_client_id_value returns None.
        ProfileConfig {
            base_url: Some("https://unused.invalid".to_string()),
            ..Default::default()
        }
        .save("default")
        .unwrap();

        let now = now_secs();
        store::store_token_data(
            "default",
            &TokenData {
                access_token: "stored-access".to_string(),
                expires_at: now + 3600,
                refresh_token: None,
                refresh_expires_at: None,
                grant_type: Some(ags_protocol::request::GrantType::ClientCredentials),
                client_id: Some("client-A".to_string()),
            },
        )
        .unwrap();

        let client = reqwest::Client::new();
        let res = resolve_access_token(&client, "default").await.unwrap();
        assert_eq!(res.token, "stored-access");
        assert!(matches!(res.source, TokenSource::Stored));
    }
}
