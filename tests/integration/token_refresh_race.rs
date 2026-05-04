//! Repro for the token-refresh race observed in the 2026-04-22 QA sweep.
//!
//! When N callers concurrently find the stored access token stale and
//! trigger `resolve_access_token`, each one independently issues a
//! refresh-token request to IAM. AccelByte rotates the refresh_token on
//! each successful grant, so only the *first* concurrent call succeeds;
//! the rest send an already-rotated refresh_token and get rejected.
//!
//! These tests now pin the *fixed* behaviour: one refresh request
//! regardless of concurrent callers, and zero user-visible failures
//! even when the server rotates refresh tokens.
//!
//! **Scope:** these tests exercise the in-process case (multiple Tokio tasks
//! in one process), which was the original failure mode. Cross-process
//! correctness is covered by the `FileLock` unit tests in
//! `src/support/file_system.rs`, which verify that two OS threads competing
//! for the same lock file both complete their write without a lost update.

#![allow(clippy::await_holding_lock)]

use std::time::{SystemTime, UNIX_EPOCH};

use ags::runtime::auth::session as service;
use ags::runtime::auth::store::{self, TokenData};
use ags::runtime::config::ProfileConfig;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct TempEnvGuard {
    key: &'static str,
    original: Option<String>,
}

impl TempEnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, original }
    }

    fn remove(key: &'static str) -> Self {
        let original = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, original }
    }
}

impl Drop for TempEnvGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(val) => std::env::set_var(self.key, val),
            None => std::env::remove_var(self.key),
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Ten concurrent `resolve_access_token` calls with a stale access token
/// must make exactly ONE refresh request — losers wait on the in-flight
/// refresh and pick up the fresh token from storage on their re-read.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn concurrent_resolve_access_token_issues_exactly_one_refresh() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
    let _no_env_token = TempEnvGuard::remove("AGS_ACCESS_TOKEN");

    let server = MockServer::start().await;

    // Mock accepts any refresh-token request and returns a new token pair.
    // We intentionally do NOT cap `.expect(1)` yet — we want to observe
    // how many the race actually fires.
    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"access_token":"new-access","expires_in":3600,"token_type":"Bearer","refresh_token":"rotated","refresh_expires_in":7200}"#
        ))
        .mount(&server)
        .await;

    // Persist profile config pointing at the mock server.
    let profile = "default";
    let cfg = ProfileConfig {
        base_url: Some(server.uri()),
        client_id: Some("test-client".to_string()),
        ..Default::default()
    };
    cfg.save(profile).unwrap();

    // Write a stale access token with a still-valid refresh token.
    let now = now_secs();
    let stale = TokenData {
        access_token: "expired-access".to_string(),
        expires_at: now.saturating_sub(300), // expired 5 minutes ago
        refresh_token: Some("stale-refresh".to_string()),
        refresh_expires_at: Some(now + 86_400),
        grant_type: Some(ags::protocol::request::GrantType::AuthorizationCode),
    };
    store::store_token_data(profile, &stale).unwrap();

    // Fire 10 concurrent resolves.
    let client = reqwest::Client::new();
    let n = 10;
    let mut handles = Vec::with_capacity(n);
    for _ in 0..n {
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            service::resolve_access_token(&client, "default").await
        }));
    }

    let mut ok = 0;
    let mut err = 0;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        for h in handles {
            match h.await.unwrap() {
                Ok(_) => ok += 1,
                Err(_) => err += 1,
            }
        }
    })
    .await
    .expect("concurrent token resolution should not deadlock");

    // Ask the mock how many refresh calls it received.
    let refresh_calls = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| {
            r.method.as_str() == "POST"
                && r.url.path() == "/iam/v3/oauth/token"
                && std::str::from_utf8(&r.body)
                    .map(|b| b.contains("grant_type=refresh_token"))
                    .unwrap_or(false)
        })
        .count();

    eprintln!("repro: ok={ok} err={err} refresh_calls={refresh_calls} workers={n}");

    assert_eq!(ok, n, "every task should receive a token");
    assert_eq!(
        refresh_calls, 1,
        "expected exactly one refresh request across {n} concurrent callers, got {refresh_calls}"
    );
}

/// Same setup but the mock simulates AccelByte's refresh-token rotation:
/// the first refresh succeeds, any subsequent refresh with the same
/// (now-rotated) token is rejected with invalid_grant. Pre-fix, N-1
/// callers would fail. Post-fix, only one refresh is made (so the
/// rotation trap never triggers) and all callers succeed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial_test::serial]
async fn concurrent_refresh_with_rotation_all_callers_succeed() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
    let _no_env_token = TempEnvGuard::remove("AGS_ACCESS_TOKEN");

    let server = MockServer::start().await;

    // First refresh: succeed.
    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"access_token":"first","expires_in":3600,"token_type":"Bearer","refresh_token":"rotated","refresh_expires_in":7200}"#
        ))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    // Subsequent refresh attempts: IAM rejects the already-rotated refresh_token.
    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"error":"invalid_grant","error_description":"refresh_token rotated"}"#,
        ))
        .mount(&server)
        .await;

    let profile = "default";
    let cfg = ProfileConfig {
        base_url: Some(server.uri()),
        client_id: Some("test-client".to_string()),
        ..Default::default()
    };
    cfg.save(profile).unwrap();

    let now = now_secs();
    let stale = TokenData {
        access_token: "expired-access".to_string(),
        expires_at: now.saturating_sub(300),
        refresh_token: Some("stale-refresh".to_string()),
        refresh_expires_at: Some(now + 86_400),
        grant_type: Some(ags::protocol::request::GrantType::AuthorizationCode),
    };
    store::store_token_data(profile, &stale).unwrap();

    let client = reqwest::Client::new();
    let n = 10;
    let mut handles = Vec::with_capacity(n);
    for _ in 0..n {
        let client = client.clone();
        handles.push(tokio::spawn(async move {
            service::resolve_access_token(&client, "default").await
        }));
    }

    let mut ok = 0;
    let mut err = 0;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        for h in handles {
            match h.await.unwrap() {
                Ok(_) => ok += 1,
                Err(_) => err += 1,
            }
        }
    })
    .await
    .expect("concurrent refresh with rotation should not deadlock");

    eprintln!("repro: ok={ok} err={err} workers={n}");

    assert_eq!(
        err, 0,
        "no task should fail — the refresh mutex means only one refresh \
         is issued, so the rotation-rejection path never triggers. got ok={ok} err={err}"
    );
    assert_eq!(ok, n);
}

/// Legacy stored tokens with no recorded grant type must retain the
/// authorization-code fallback semantics on refresh failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial]
async fn legacy_token_without_grant_type_reports_session_expired_on_refresh_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _no_kc = TempEnvGuard::set("AGS_NO_KEYCHAIN", "1");
    let _no_env_token = TempEnvGuard::remove("AGS_ACCESS_TOKEN");
    let _no_client_secret = TempEnvGuard::remove("AGS_CLIENT_SECRET");

    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            r#"{"error":"invalid_grant","error_description":"refresh token expired"}"#,
        ))
        .mount(&server)
        .await;

    let profile = "default";
    let cfg = ProfileConfig {
        base_url: Some(server.uri()),
        client_id: Some("test-client".to_string()),
        ..Default::default()
    };
    cfg.save(profile).unwrap();

    let now = now_secs();
    let stale = TokenData {
        access_token: "expired-access".to_string(),
        expires_at: now.saturating_sub(300),
        refresh_token: Some("legacy-refresh".to_string()),
        refresh_expires_at: Some(now + 86_400),
        grant_type: None,
    };
    store::store_token_data(profile, &stale).unwrap();

    let client = reqwest::Client::new();
    let error = match service::resolve_access_token(&client, profile).await {
        Ok(_) => {
            panic!("legacy authorization-code token should not silently fall through")
        }
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("Session expired"),
        "expected session-expired error, got: {error}"
    );
}
