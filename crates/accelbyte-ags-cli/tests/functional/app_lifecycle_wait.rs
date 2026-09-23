//! End-to-end coverage for the Extend app lifecycle `--wait` commands.
//!
//! Exercises the full seam the unit/wiremock tests cannot: the shim's primary
//! call dispatches through the service path, and — because `--wait` was
//! requested and the call completed — the CLI then polls
//! `GET /csm/v5/.../apps/{app}` until a terminal `appStatus` is reached.
//!
//! The primary call and the status `GET` are distinguished only by HTTP
//! method (and, for delete, by a 404 signalling the app is gone), which is
//! exactly how the real CSM API behaves.

use crate::common::cli_helpers::ags_with_base_url;
use crate::common::wiremock_helpers::mount_token_success;
use predicates::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const APP_PATH: &str = "/csm/v5/admin/namespaces/test-ns/apps/my-app";
/// `apps delete` resolves to CSM v5 (the highest bundled version), the same
/// version the status poll uses.
const DELETE_PATH: &str = "/csm/v5/admin/namespaces/test-ns/apps/my-app";
/// `deploy-app` (deployments create) resolves to CSM v5 (the highest bundled
/// version); the create response carries the new `deploymentId` and the poll
/// also uses the v5 app status.
const DEPLOY_CREATE_PATH: &str = "/csm/v5/admin/namespaces/test-ns/apps/my-app/deployments";
/// `start-app` / `stop-app` resolve to the CSM v5 `PUT .../start` and
/// `.../stop` endpoints; the poll still uses the v5 app status ([`APP_PATH`]).
const START_PATH: &str = "/csm/v5/admin/namespaces/test-ns/apps/my-app/start";
const STOP_PATH: &str = "/csm/v5/admin/namespaces/test-ns/apps/my-app/stop";

/// Happy path: create succeeds, then polling sees a non-terminal status
/// followed by `app-undeployed`. The command blocks, then exits 0 and prints
/// the success message.
#[tokio::test]
async fn create_app_wait_polls_until_ready() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    // Primary operation: create returns 200.
    Mock::given(method("POST"))
        .and(path(APP_PATH))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "appName": "my-app" })),
        )
        .mount(&server)
        .await;

    // First status poll: still creating (exhausted after one call).
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "appStatus": "app-creating" })),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    // Subsequent polls: ready.
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "appStatus": "app-undeployed" })),
        )
        .with_priority(2)
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "create-app",
            "--app",
            "my-app",
            "--json",
            r#"{"scenario":"service-extension"}"#,
            "--wait",
            "--wait-interval",
            "1",
            "--wait-limit",
            "10",
        ]);

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("app created and ready"),
        "expected success message on stderr, got:\n{stderr}"
    );
}

/// Timeout path: create succeeds but the app never reaches a terminal state
/// within `--wait-limit`. The CLI stops waiting and exits non-zero.
#[tokio::test]
async fn create_app_wait_times_out_non_zero() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("POST"))
        .and(path(APP_PATH))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "appName": "my-app" })),
        )
        .mount(&server)
        .await;

    // Every poll: still creating — never terminal.
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "appStatus": "app-creating" })),
        )
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "create-app",
            "--app",
            "my-app",
            "--json",
            r#"{"scenario":"service-extension"}"#,
            "--wait",
            "--wait-interval",
            "1",
            "--wait-limit",
            "1",
        ]);

    cmd.assert().failure().stderr(predicate::str::contains(
        "timeout waiting for app to be created",
    ));
}

/// delete-app --wait: the primary DELETE succeeds, then polling sees the app
/// mid-teardown and finally a 404 (gone), which counts as success. Proves the
/// not-found → success path end to end through the real CLI.
#[tokio::test]
async fn delete_app_wait_succeeds_when_app_gone() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    // Primary operation: delete accepted (v2).
    Mock::given(method("DELETE"))
        .and(path(DELETE_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    // First status poll: still removing (exhausted after one call).
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "appStatus": "app-removing" })),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    // Subsequent polls: gone (404 → NotFound → success).
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(ResponseTemplate::new(404).set_body_json(
            serde_json::json!({ "errorCode": 13102, "errorMessage": "app not found" }),
        ))
        .with_priority(2)
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "delete-app",
            "--app",
            "my-app",
            "--yes",
            "--wait",
            "--wait-interval",
            "1",
            "--wait-limit",
            "10",
        ]);

    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("app deleted"),
        "expected delete success message on stderr, got:\n{stderr}"
    );
}

/// Redeploy race: the app is ALREADY `deployment-running` from a PREVIOUS
/// deploy, and every poll reports that previous deployment's id. The command
/// must NOT report success for the old deployment — it must keep polling until
/// the app flips to OUR deployment id (which never happens here), then time
/// out non-zero. Without the identity guard the very first poll matches the
/// success state and the command exits 0 while the old image is still serving.
#[tokio::test]
async fn deploy_app_wait_does_not_succeed_on_previous_deployment() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    // Primary operation: create deployment returns our NEW deployment id.
    Mock::given(method("POST"))
        .and(path(DEPLOY_CREATE_PATH))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(serde_json::json!({ "deploymentId": "deploy-new" })),
        )
        .mount(&server)
        .await;

    // Every poll: already running, but still reporting the PREVIOUS deployment.
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "appStatus": "deployment-running",
            "deploymentId": "deploy-old"
        })))
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "deploy-app",
            "--app",
            "my-app",
            "--yes",
            "--json",
            r#"{"imageTag":"v2"}"#,
            "--wait",
            "--wait-interval",
            "1",
            "--wait-limit",
            "2",
        ]);

    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "deploy-app --wait must NOT succeed while the app still reports the \
         previous deployment id; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("timeout waiting for deployment"),
        "expected a timeout (kept polling past the stale deployment), got:\n{stderr}"
    );
}

/// Redeploy happy path: the app first reports the PREVIOUS deployment
/// (`deployment-running`, stale id), then flips to OUR new deployment id and
/// `deployment-running`. The command must wait for the flip, then exit 0.
#[tokio::test]
async fn deploy_app_wait_succeeds_once_new_deployment_is_running() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("POST"))
        .and(path(DEPLOY_CREATE_PATH))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(serde_json::json!({ "deploymentId": "deploy-new" })),
        )
        .mount(&server)
        .await;

    // First poll: previous deployment still serving (exhausted after one call).
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "appStatus": "deployment-running",
            "deploymentId": "deploy-old"
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    // Subsequent polls: our deployment is now running.
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "appStatus": "deployment-running",
            "deploymentId": "deploy-new"
        })))
        .with_priority(2)
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "deploy-app",
            "--app",
            "my-app",
            "--yes",
            "--json",
            r#"{"imageTag":"v2"}"#,
            "--wait",
            "--wait-interval",
            "1",
            "--wait-limit",
            "10",
        ]);

    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected success once the new deployment is running, stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("app deployed and running"),
        "expected deploy success message on stderr, got:\n{stderr}"
    );
}

/// Crash-on-boot: the deployment saga completes, the container comes up and
/// then crashes (bad image / failed readiness / CrashLoopBackOff), and CSM's
/// reconciler flips the app to `deployment-down`. The CLI's polls miss the
/// transient `deployment-running` and observe `deployment-down` for OUR
/// deployment. That is a definitively failed rollout — the wait must fail fast
/// with `deployment failed: deployment-down`, NOT poll for the full
/// `--wait-limit` and exit as a timeout.
#[tokio::test]
async fn deploy_app_wait_fails_fast_on_deployment_down() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("POST"))
        .and(path(DEPLOY_CREATE_PATH))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(serde_json::json!({ "deploymentId": "deploy-new" })),
        )
        .mount(&server)
        .await;

    // First poll: our deployment still rolling out (exhausted after one call).
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "appStatus": "deployment-in-progress",
            "deploymentId": "deploy-new"
        })))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    // Subsequent polls: our deployment came up and crashed — down.
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "appStatus": "deployment-down",
            "deploymentId": "deploy-new"
        })))
        .with_priority(2)
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "deploy-app",
            "--app",
            "my-app",
            "--yes",
            "--json",
            r#"{"imageTag":"v2"}"#,
            "--wait",
            "--wait-interval",
            "1",
            "--wait-limit",
            "20",
        ]);

    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "deploy-app --wait must fail on deployment-down, not succeed; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("deployment failed: deployment-down"),
        "expected a fast failed-rollout error naming deployment-down, got:\n{stderr}"
    );
    assert!(
        !stderr.contains("timeout waiting for deployment"),
        "must NOT time out on a definitively-down deployment, got:\n{stderr}"
    );
}

/// start-app --wait: the primary PUT `.../start` succeeds, then polling sees
/// the app mid-start and finally `deployment-running`, which is the success
/// state. The command blocks, then exits 0 with the success message.
#[tokio::test]
async fn start_app_wait_polls_until_running() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    // Primary operation: start accepted.
    Mock::given(method("PUT"))
        .and(path(START_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    // First status poll: still coming up (exhausted after one call).
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "appStatus": "deployment-starting" })),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    // Subsequent polls: running.
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "appStatus": "deployment-running" })),
        )
        .with_priority(2)
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "start-app",
            "--app",
            "my-app",
            "--yes",
            "--wait",
            "--wait-interval",
            "1",
            "--wait-limit",
            "10",
        ]);

    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected success once running, stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("app started"),
        "expected start success message on stderr, got:\n{stderr}"
    );
}

/// stop-app --wait: the primary PUT `.../stop` succeeds, then polling sees the
/// app mid-stop and finally `app-stopped`, the success state. The command
/// blocks, then exits 0 with the success message.
#[tokio::test]
async fn stop_app_wait_polls_until_stopped() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    // Primary operation: stop accepted.
    Mock::given(method("PUT"))
        .and(path(STOP_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    // First status poll: still stopping (exhausted after one call).
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "appStatus": "app-stopping" })),
        )
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    // Subsequent polls: stopped.
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "appStatus": "app-stopped" })),
        )
        .with_priority(2)
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "stop-app",
            "--app",
            "my-app",
            "--yes",
            "--wait",
            "--wait-interval",
            "1",
            "--wait-limit",
            "10",
        ]);

    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "expected success once stopped, stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("app stopped"),
        "expected stop success message on stderr, got:\n{stderr}"
    );
}

/// delete-app --wait must not report success on a 404 that is NOT the CSM
/// AppNotFound error. The primary DELETE succeeds, but the status poll returns
/// an unrelated 404 (e.g. a wrong route): that is not proof the app is gone, so
/// the CLI must keep polling and time out rather than falsely report "deleted".
#[tokio::test]
async fn delete_app_wait_does_not_succeed_on_unrelated_404() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("DELETE"))
        .and(path(DELETE_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    // Every poll: a 404 that is NOT AppNotFound (errorCode 13102).
    Mock::given(method("GET"))
        .and(path(APP_PATH))
        .respond_with(ResponseTemplate::new(404).set_body_json(
            serde_json::json!({ "errorCode": 20013, "errorMessage": "namespace not found" }),
        ))
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "delete-app",
            "--app",
            "my-app",
            "--yes",
            "--wait",
            "--wait-interval",
            "1",
            "--wait-limit",
            "2",
        ]);

    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "delete-app --wait must NOT treat an unrelated 404 as 'app gone'; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("timeout waiting for app to be deleted"),
        "expected a timeout (kept polling past the unrelated 404), got:\n{stderr}"
    );
}

/// `--wait-limit 0` is rejected up front as a usage error instead of silently
/// entering the poll loop zero times and reporting an instant "timeout". The
/// primary call is mounted so that, without the guard, the old behaviour would
/// succeed then immediately time out.
#[tokio::test]
async fn wait_limit_zero_is_rejected() {
    let server = MockServer::start().await;
    mount_token_success(&server).await;

    Mock::given(method("POST"))
        .and(path(DEPLOY_CREATE_PATH))
        .respond_with(
            ResponseTemplate::new(201)
                .set_body_json(serde_json::json!({ "deploymentId": "deploy-new" })),
        )
        .mount(&server)
        .await;

    let mut cmd = ags_with_base_url(&server.uri());
    cmd.env("AGS_CLIENT_ID", "test-client-id")
        .env("AGS_CLIENT_SECRET", "test-client-secret")
        .args([
            "--namespace",
            "test-ns",
            "extend",
            "deploy-app",
            "--app",
            "my-app",
            "--yes",
            "--json",
            r#"{"imageTag":"v2"}"#,
            "--wait",
            "--wait-limit",
            "0",
        ]);

    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "--wait-limit 0 must fail; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("--wait-limit must be greater than 0"),
        "expected a usage error rejecting --wait-limit 0, got:\n{stderr}"
    );
    assert!(
        !stderr.contains("timeout waiting for deployment"),
        "--wait-limit 0 must be rejected before polling, not report a timeout:\n{stderr}"
    );
}

/// `--dry-run --wait` prints the request preview but performs no polling. It
/// must say so on stderr — silently dropping `--wait` gives a false pass when
/// someone dry-runs to check their exit-code-6 timeout handling.
#[tokio::test]
async fn dry_run_with_wait_notes_that_wait_is_ignored() {
    // --dry-run makes no HTTP call, so no server / credentials are needed.
    let mut cmd = ags_with_base_url("https://dry-run.example");
    cmd.args([
        "--namespace",
        "test-ns",
        "extend",
        "start-app",
        "--app",
        "my-app",
        "--wait",
        "--wait-interval",
        "1",
        "--wait-limit",
        "5",
        "--dry-run",
    ]);

    let output = cmd.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "dry-run should exit 0, stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("--wait is ignored under --dry-run"),
        "dry-run with --wait must note that the wait is skipped, got:\n{stderr}"
    );
}
