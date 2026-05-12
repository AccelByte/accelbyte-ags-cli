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
) -> Result<RefreshOutcome, RuntimeError> {
    // Fast path: read stored token without locks.
    let stored = store::get_token_data_async(profile).await.ok().flatten();
    let now = unix_now();
    let Some(token_data) = stored else {
        return Ok(RefreshOutcome::Unavailable {
            reason: UnavailableReason::NoStoredToken,
        });
    };

    if let Some(outcome) = refreshed_from_fresh_stored(&token_data, now) {
        return Ok(outcome);
    }

    // Determine refreshability locally before paying for any locks.
    let Some(refresh_token_value) = token_data.refresh_token.clone() else {
        return Ok(RefreshOutcome::Unavailable {
            reason: UnavailableReason::NoRefreshToken,
        });
    };
    let is_refresh_locally_valid = token_data
        .refresh_expires_at
        .map(|exp| now < exp)
        .unwrap_or(true);
    if !is_refresh_locally_valid {
        return Ok(RefreshOutcome::Unavailable {
            reason: UnavailableReason::RefreshTokenExpired,
        });
    }

    // Slow path: acquire locks before the network call.
    let refresh_mutex = profile_refresh_lock(profile);
    let _refresh_guard = refresh_mutex.lock().await;
    let _lock = locking::acquire_async_token_lock(profile).await?;

    // Re-read after both locks are held. If the token has vanished between
    // the unlocked read and now (another process logged out), treat it the
    // same as never having had one.
    let stored = store::get_token_data_async(profile).await.ok().flatten();
    let Some(token_data) = stored else {
        return Ok(RefreshOutcome::Unavailable {
            reason: UnavailableReason::NoStoredToken,
        });
    };
    let now = unix_now();
    if let Some(outcome) = refreshed_from_fresh_stored(&token_data, now) {
        return Ok(outcome);
    }

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
        Err(error) => {
            // Network/transport failures should abort. Anything else (HTTP
            // 4xx/5xx mapped through AuthError::TokenRefreshFailed) is a
            // server-side rejection and the caller decides how to react.
            return classify_refresh_error(error);
        }
    };

    let expires_in_warning = result.expires_in_warning.take();
    let new_token_data = tokens::token_result_to_token_data(
        &result,
        token_data
            .grant_type
            .unwrap_or(crate::protocol::request::GrantType::AuthorizationCode),
        now,
    );
    // `_lock` (acquired above) must still be in scope across this await —
    // `store_token_data_unlocked_async` writes without re-acquiring the
    // cross-process file lock, relying on the caller's lock to guard the
    // write. Do not refactor the lock guards out of this function without
    // moving the write with them.
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
    use crate::protocol::error::RuntimeErrorKind;
    if matches!(error.kind, RuntimeErrorKind::Network) {
        Err(error)
    } else {
        Ok(RefreshOutcome::Rejected {
            message: error.message,
        })
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
        None | Some(crate::protocol::request::GrantType::AuthorizationCode)
    );

    match try_refresh_stored_session(client, profile).await? {
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
        RefreshOutcome::Unavailable { reason } => {
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
            if is_authorization_code {
                match reason {
                    UnavailableReason::NoStoredToken => {}
                    UnavailableReason::NoRefreshToken => {
                        return Err(RuntimeError::from(AuthError::SessionExpiredNoRefreshToken));
                    }
                    UnavailableReason::RefreshTokenExpired => {
                        return Err(RuntimeError::from(
                            AuthError::SessionExpiredRefreshTokenExpired,
                        ));
                    }
                }
            }
        }
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
                grant_type: Some(crate::protocol::request::GrantType::AuthorizationCode),
            },
        );

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(&client, "default")
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
        let outcome = try_refresh_stored_session(&client, "default")
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
                grant_type: Some(crate::protocol::request::GrantType::AuthorizationCode),
            },
        );

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(&client, "default")
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
                grant_type: Some(crate::protocol::request::GrantType::AuthorizationCode),
            },
        );

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(&client, "default")
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
                grant_type: Some(crate::protocol::request::GrantType::AuthorizationCode),
            },
        );

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(&client, "default")
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
                grant_type: Some(crate::protocol::request::GrantType::AuthorizationCode),
            },
        );

        let client = reqwest::Client::new();
        let outcome = try_refresh_stored_session(&client, "default")
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
                grant_type: Some(crate::protocol::request::GrantType::AuthorizationCode),
            },
        );

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(500))
            .build()
            .unwrap();
        let result = try_refresh_stored_session(&client, "default").await;
        assert!(
            matches!(
                &result,
                Err(e) if matches!(e.kind, crate::protocol::error::RuntimeErrorKind::Network)
            ),
            "expected Err(Network), got {result:?}"
        );
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
                grant_type: Some(crate::protocol::request::GrantType::AuthorizationCode),
            },
        );

        let client = reqwest::Client::new();
        let a = {
            let client = client.clone();
            let p = "default".to_string();
            tokio::spawn(async move { try_refresh_stored_session(&client, &p).await })
        };
        let b = {
            let client = client.clone();
            let p = "default".to_string();
            tokio::spawn(async move { try_refresh_stored_session(&client, &p).await })
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
}
