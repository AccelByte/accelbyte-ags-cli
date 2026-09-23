//! Poll-until-terminal-state logic for the Extend app lifecycle shims
//! (`create-app`/`deploy-app`/`start-app`/`stop-app`/`delete-app`)
//! Faithfully mirrors `extend-helper-cli`'s `WaitUntilAppIs`:
//! sleep first, then poll `GET .../apps/{app}` and evaluate the `appStatus`
//! against a per-command set of terminal states, emitting progress between
//! polls and giving up once `--wait-limit` seconds have elapsed.

use std::time::Duration;

use crate::errors::{ApiErrorCategory, CliError};
use crate::frontend::streams::UiSink;

use super::api::{self, AppLookup, AppState};

/// Per-command wait target: which `appStatus` values are terminal, and the
/// user-facing copy. One `static` instance per lifecycle command. Referenced
/// from the `SHIMS` table via `ExtendShim::wait`.
pub(crate) struct WaitSpec {
    /// `appStatus` values that mean the operation succeeded.
    pub(crate) success_states: &'static [&'static str],
    /// `appStatus` values that mean the operation reached a failed terminal
    /// state (stop polling, report failure).
    pub(crate) failure_states: &'static [&'static str],
    /// When true, a poll that finds the app gone (HTTP 404 / CSM
    /// `AppNotFound`) counts as success. Used by `delete-app`.
    pub(crate) not_found_is_success: bool,
    /// When true, the caller captures the deployment id from the command's own
    /// create response and the wait only accepts a terminal state once the app
    /// is reporting THAT deployment. This is the identity guard that stops
    /// `deploy-app --wait` from reporting success for a stale, still-running
    /// previous deployment. Only `deploy-app` sets this — see the note on
    /// [`START_APP_WAIT`] for why `start-app` deliberately does not.
    pub(crate) guard_by_deployment_id: bool,
    /// Message shown when the target state is reached.
    pub(crate) success_message: &'static str,
    /// Message shown when the app reaches a failed terminal state. The
    /// offending `appStatus` is appended (e.g. "app creation failed: ...").
    pub(crate) failed_message: &'static str,
    /// Message shown when `--wait-limit` is exceeded before any terminal
    /// state is reached.
    pub(crate) timeout_message: &'static str,
}

/// `create-app` waits for `app-undeployed`; `app-creation-failed` /
/// `app-creation-timeout` are failed terminal states.
pub(crate) static CREATE_APP_WAIT: WaitSpec = WaitSpec {
    success_states: &["app-undeployed"],
    failure_states: &["app-creation-failed", "app-creation-timeout"],
    not_found_is_success: false,
    guard_by_deployment_id: false,
    success_message: "app created and ready",
    failed_message: "app creation failed",
    timeout_message: "timeout waiting for app to be created",
};

/// `deploy-app` waits for `deployment-running` — but only for OUR deployment.
/// A redeploy of an app that is already `deployment-running` from a previous
/// deploy would otherwise match the success state on the very first poll, so
/// `guard_by_deployment_id` holds the wait open until the app reports the
/// deployment id captured from this command's own create response.
pub(crate) static DEPLOY_APP_WAIT: WaitSpec = WaitSpec {
    success_states: &["deployment-running"],
    // `deployment-down` = the deployment came up and then went down (crash on
    // boot / failed readiness / CrashLoopBackOff). CSM only ever sets it from a
    // running app (never as a pre-startup blip during a rollout), so observing
    // it inside the wait window is an unambiguous failed rollout — fail fast
    // instead of polling to the limit and reporting a misleading timeout.
    failure_states: &["deployment-failed", "deployment-timeout", "deployment-down"],
    not_found_is_success: false,
    guard_by_deployment_id: true,
    success_message: "app deployed and running",
    failed_message: "deployment failed",
    timeout_message: "timeout waiting for deployment",
};

/// `start-app` waits for `deployment-running` (same terminal states as deploy).
///
/// Deliberately does NOT guard by deployment id. `start-app` asks the platform
/// to bring the app's existing deployment back up; the caller's intent is
/// satisfied the moment the app is running, whichever deployment that is. An
/// already-running app is a legitimate immediate success, not a stale-state
/// race — so unlike `deploy-app` there is no "our vs. previous deployment" to
/// distinguish, and `start-app` also has no create response carrying a new id.
pub(crate) static START_APP_WAIT: WaitSpec = WaitSpec {
    success_states: &["deployment-running"],
    // See DEPLOY_APP_WAIT: an app that starts and then goes `deployment-down` is
    // a failed start, not something to wait out to the limit.
    failure_states: &["deployment-failed", "deployment-timeout", "deployment-down"],
    not_found_is_success: false,
    guard_by_deployment_id: false,
    success_message: "app started",
    failed_message: "start failed",
    timeout_message: "timeout waiting for app to start",
};

/// `stop-app` waits for `app-stopped`.
pub(crate) static STOP_APP_WAIT: WaitSpec = WaitSpec {
    success_states: &["app-stopped"],
    failure_states: &["app-stop-failed", "app-stop-timeout"],
    not_found_is_success: false,
    guard_by_deployment_id: false,
    success_message: "app stopped",
    failed_message: "stop failed",
    timeout_message: "timeout waiting for app to stop",
};

/// `delete-app` succeeds when the app reports `app-removed` OR the poll finds
/// it gone (HTTP 404 / CSM `AppNotFound`). `app-remove-timeout` is a failure.
pub(crate) static DELETE_APP_WAIT: WaitSpec = WaitSpec {
    success_states: &["app-removed"],
    failure_states: &["app-remove-timeout"],
    not_found_is_success: true,
    guard_by_deployment_id: false,
    success_message: "app deleted",
    failed_message: "delete failed",
    timeout_message: "timeout waiting for app to be deleted",
};

/// The decision for a single poll result.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PollDecision {
    /// Not yet terminal — keep polling.
    Continue,
    /// Reached a success state (or a not-found that counts as success).
    Succeeded,
    /// Reached a failed terminal state; carries the offending `appStatus`.
    Failed(String),
}

/// Classify a single poll lookup against the command's target spec. Pure —
/// unit-tested directly so the async loop below needs no clock or network.
///
/// `expected_deployment_id` is the identity guard: when the caller knows which
/// deployment this wait is for (only `deploy-app`, which captures it from its
/// own create response) and the app is currently reporting a DIFFERENT
/// deployment, the poll keeps going regardless of status — the success (or
/// failure) state we would otherwise match belongs to the PREVIOUS deployment,
/// not ours. The guard only engages when the poll actually carries a
/// deployment id; if the id is absent (the slim app status view may omit it)
/// we fall back to status-only evaluation rather than wait forever. Unknown
/// statuses still return `Continue` (fail-open) exactly as before — this guard
/// never turns an unrecognised state into a failure.
pub(crate) fn evaluate(
    lookup: &AppLookup,
    spec: &WaitSpec,
    expected_deployment_id: Option<&str>,
) -> PollDecision {
    match lookup {
        AppLookup::NotFound => {
            if spec.not_found_is_success {
                PollDecision::Succeeded
            } else {
                PollDecision::Continue
            }
        }
        AppLookup::Found(AppState {
            app_status,
            deployment_id,
        }) => {
            if is_reporting_other_deployment(expected_deployment_id, deployment_id.as_deref()) {
                return PollDecision::Continue;
            }
            if spec.success_states.contains(&app_status.as_str()) {
                PollDecision::Succeeded
            } else if spec.failure_states.contains(&app_status.as_str()) {
                PollDecision::Failed(app_status.clone())
            } else {
                PollDecision::Continue
            }
        }
    }
}

/// True when we know which deployment we are waiting for and the app is
/// currently reporting a different, present deployment id. A missing polled id
/// returns false — we cannot distinguish, so evaluation falls back to status.
fn is_reporting_other_deployment(expected: Option<&str>, current: Option<&str>) -> bool {
    matches!((expected, current), (Some(expected), Some(current)) if expected != current)
}

/// Seconds to sleep before the next poll: a full `interval`, but never past the
/// remaining budget (`limit - elapsed`), so the wait honours `--wait-limit` to
/// the second instead of overshooting by up to one interval when the limit is
/// not a multiple of the interval. Only called with `elapsed < limit`.
fn next_poll_step(interval: u64, elapsed: u64, limit: u64) -> u64 {
    interval.min(limit - elapsed)
}

/// Poll `GET .../apps/{app}` every `interval` seconds until the app reaches a
/// terminal state per `spec` or `limit` seconds elapse. Progress and the
/// final line are written to stderr via `sink`.
///
/// Mirrors `WaitUntilAppIs`: sleeps *before* the first poll, ignores transient
/// GET errors (keeps polling), and returns an error on a failed terminal state
/// or on timeout so the caller exits non-zero.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn wait_until_app_reaches(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    namespace: &str,
    app: &str,
    spec: &WaitSpec,
    expected_deployment_id: Option<&str>,
    interval: u64,
    limit: u64,
    sink: &UiSink,
) -> Result<(), CliError> {
    let mut elapsed = 0u64;
    while elapsed < limit {
        // Cap the sleep to the remaining budget so the wait honours
        // `--wait-limit` to the second, rather than overshooting by up to one
        // interval when the limit is not a multiple of the interval.
        let step = next_poll_step(interval, elapsed, limit);
        tokio::time::sleep(Duration::from_secs(step)).await;
        elapsed += step;

        match api::get_app_status(client, base_url, access_token, namespace, app).await {
            Ok(lookup) => match evaluate(&lookup, spec, expected_deployment_id) {
                PollDecision::Succeeded => {
                    let _ = sink.write_line(spec.success_message);
                    return Ok(());
                }
                PollDecision::Failed(status) => {
                    return Err(CliError::Api {
                        message: format!("{}: {status}", spec.failed_message),
                        metadata: None,
                        category: ApiErrorCategory::Upstream,
                    });
                }
                PollDecision::Continue => {
                    let _ = sink.write_line("waiting...");
                }
            },
            // Transient lookup error: explicitly ignored, keep polling until
            // the limit — matches the Go predicate's error handling.
            Err(_) => {
                let _ = sink.write_line("waiting...");
            }
        }
    }

    let _ = sink.write_line("wait limit exceeded, stopping...");
    // Timeout is its OWN category (exit code 6), distinct from a failed
    // terminal state above (Upstream, exit 3): the operation may still land, so
    // a CI caller must be able to tell "timed out, safe to re-check" from
    // "server failed it, do not blindly retry" without matching message text.
    Err(CliError::Api {
        message: spec.timeout_message.to_string(),
        metadata: None,
        category: ApiErrorCategory::Timeout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_state_succeeds() {
        let lookup = found("app-undeployed");
        assert_eq!(
            evaluate(&lookup, &CREATE_APP_WAIT, None),
            PollDecision::Succeeded
        );
    }

    #[test]
    fn failure_state_fails_with_status() {
        let lookup = found("app-creation-failed");
        assert_eq!(
            evaluate(&lookup, &CREATE_APP_WAIT, None),
            PollDecision::Failed("app-creation-failed".to_string())
        );
    }

    #[test]
    fn non_terminal_state_continues() {
        let lookup = found("app-creating");
        assert_eq!(
            evaluate(&lookup, &CREATE_APP_WAIT, None),
            PollDecision::Continue
        );
    }

    #[test]
    fn not_found_continues_when_not_success() {
        // create-app: a not-found poll is not terminal — keep waiting.
        assert_eq!(
            evaluate(&AppLookup::NotFound, &CREATE_APP_WAIT, None),
            PollDecision::Continue
        );
    }

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn loop_polls_until_success_state() {
        let server = MockServer::start().await;
        let app_path = "/csm/v5/admin/namespaces/test-ns/apps/my-app";
        // First poll: still creating. Higher precedence (lower priority number),
        // exhausted after one call.
        Mock::given(method("GET"))
            .and(path(app_path))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "appStatus": "app-creating"
            })))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        // Subsequent polls: terminal success.
        Mock::given(method("GET"))
            .and(path(app_path))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "appStatus": "app-undeployed"
            })))
            .with_priority(2)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let sink = UiSink;
        let result = wait_until_app_reaches(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            &CREATE_APP_WAIT,
            None,
            1,
            10,
            &sink,
        )
        .await;
        assert!(
            result.is_ok(),
            "should succeed once app-undeployed: {result:?}"
        );
    }

    #[tokio::test]
    async fn loop_times_out_when_state_never_reached() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "appStatus": "app-creating" })),
            )
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let sink = UiSink;
        let result = wait_until_app_reaches(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            &CREATE_APP_WAIT,
            None,
            1,
            1,
            &sink,
        )
        .await;
        match result {
            Err(CliError::Api {
                message, category, ..
            }) => {
                assert!(
                    message.contains("timeout waiting for app to be created"),
                    "timeout message expected, got: {message}"
                );
                assert_eq!(
                    category,
                    ApiErrorCategory::Timeout,
                    "a wait timeout must carry the Timeout category (exit 6), \
                     distinct from a server-failed rollout"
                );
            }
            other => panic!("expected timeout Api error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn loop_fails_on_failed_terminal_state() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "appStatus": "app-creation-failed" })),
            )
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let sink = UiSink;
        let result = wait_until_app_reaches(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            &CREATE_APP_WAIT,
            None,
            1,
            10,
            &sink,
        )
        .await;
        match result {
            Err(CliError::Api {
                message, category, ..
            }) => {
                assert!(
                    message.contains("app-creation-failed"),
                    "failed-state error should name the status, got: {message}"
                );
                assert_eq!(
                    category,
                    ApiErrorCategory::Upstream,
                    "a failed terminal state is Upstream (exit 3), NOT a timeout"
                );
            }
            other => panic!("expected failed-state Api error, got: {other:?}"),
        }
    }

    #[test]
    fn not_found_succeeds_when_configured() {
        // delete-app: a not-found poll counts as success.
        assert_eq!(
            evaluate(&AppLookup::NotFound, &DELETE_APP_WAIT, None),
            PollDecision::Succeeded
        );
    }

    // ── Per-command spec state strings ──

    fn found(status: &str) -> AppLookup {
        AppLookup::Found(AppState {
            app_status: status.to_string(),
            deployment_id: None,
        })
    }

    fn found_with_deployment(status: &str, deployment_id: &str) -> AppLookup {
        AppLookup::Found(AppState {
            app_status: status.to_string(),
            deployment_id: Some(deployment_id.to_string()),
        })
    }

    #[test]
    fn poll_step_never_overshoots_the_limit() {
        // Full interval while there is room for it.
        assert_eq!(next_poll_step(10, 0, 25), 10);
        assert_eq!(next_poll_step(10, 10, 25), 10);
        // Final partial step: 5s left, not another full 10s (the old overshoot).
        assert_eq!(next_poll_step(10, 20, 25), 5);
        // Exact multiple: last step is a full interval landing on the limit.
        assert_eq!(next_poll_step(10, 20, 30), 10);
        // Sum of steps equals the limit exactly (10 + 10 + 5 = 25).
        let mut elapsed = 0;
        let mut total = 0;
        while elapsed < 25 {
            let step = next_poll_step(10, elapsed, 25);
            total += step;
            elapsed += step;
        }
        assert_eq!(total, 25, "steps must sum to exactly the limit");
    }

    #[test]
    fn deploy_spec_classifies_states() {
        assert_eq!(
            evaluate(&found("deployment-running"), &DEPLOY_APP_WAIT, None),
            PollDecision::Succeeded
        );
        assert_eq!(
            evaluate(&found("deployment-failed"), &DEPLOY_APP_WAIT, None),
            PollDecision::Failed("deployment-failed".to_string())
        );
        assert_eq!(
            evaluate(&found("deploying"), &DEPLOY_APP_WAIT, None),
            PollDecision::Continue
        );
    }

    #[test]
    fn start_spec_classifies_states() {
        assert_eq!(
            evaluate(&found("deployment-running"), &START_APP_WAIT, None),
            PollDecision::Succeeded
        );
        assert_eq!(
            evaluate(&found("deployment-failed"), &START_APP_WAIT, None),
            PollDecision::Failed("deployment-failed".to_string())
        );
    }

    /// `start-app` never guards by deployment id: an already-running app is a
    /// legitimate immediate success even when reporting some other deployment.
    /// Passing an expected id here must NOT hold the wait open.
    #[test]
    fn start_spec_ignores_deployment_id_guard() {
        assert_eq!(
            evaluate(
                &found_with_deployment("deployment-running", "some-other-deployment"),
                &START_APP_WAIT,
                Some("some-other-deployment"),
            ),
            PollDecision::Succeeded,
            "start-app must succeed as soon as the app is running, regardless of \
             which deployment id it reports"
        );
    }

    #[test]
    fn stop_spec_classifies_states() {
        assert_eq!(
            evaluate(&found("app-stopped"), &STOP_APP_WAIT, None),
            PollDecision::Succeeded
        );
        assert_eq!(
            evaluate(&found("app-stop-failed"), &STOP_APP_WAIT, None),
            PollDecision::Failed("app-stop-failed".to_string())
        );
        assert_eq!(
            evaluate(&found("app-stopping"), &STOP_APP_WAIT, None),
            PollDecision::Continue
        );
    }

    #[test]
    fn delete_spec_classifies_states() {
        // app-removed and a 404 both succeed; app-remove-timeout fails.
        assert_eq!(
            evaluate(&found("app-removed"), &DELETE_APP_WAIT, None),
            PollDecision::Succeeded
        );
        assert_eq!(
            evaluate(&AppLookup::NotFound, &DELETE_APP_WAIT, None),
            PollDecision::Succeeded
        );
        assert_eq!(
            evaluate(&found("app-remove-timeout"), &DELETE_APP_WAIT, None),
            PollDecision::Failed("app-remove-timeout".to_string())
        );
        // Mid-teardown status keeps polling.
        assert_eq!(
            evaluate(&found("app-removing"), &DELETE_APP_WAIT, None),
            PollDecision::Continue
        );
    }

    // ── deploy-app deployment-id identity guard ──

    /// The core race: the app is already `deployment-running` from a PREVIOUS
    /// deploy, reporting the previous deployment id. With our new id expected,
    /// the poll must keep going rather than match the success state.
    #[test]
    fn deploy_guard_continues_on_previous_deployment() {
        assert_eq!(
            evaluate(
                &found_with_deployment("deployment-running", "deploy-old"),
                &DEPLOY_APP_WAIT,
                Some("deploy-new"),
            ),
            PollDecision::Continue
        );
    }

    /// Once the app reports OUR deployment id and it is running, succeed.
    #[test]
    fn deploy_guard_succeeds_on_matching_deployment() {
        assert_eq!(
            evaluate(
                &found_with_deployment("deployment-running", "deploy-new"),
                &DEPLOY_APP_WAIT,
                Some("deploy-new"),
            ),
            PollDecision::Succeeded
        );
    }

    /// A failed terminal state for OUR deployment is a genuine failure.
    #[test]
    fn deploy_guard_fails_on_matching_deployment_failure() {
        assert_eq!(
            evaluate(
                &found_with_deployment("deployment-failed", "deploy-new"),
                &DEPLOY_APP_WAIT,
                Some("deploy-new"),
            ),
            PollDecision::Failed("deployment-failed".to_string())
        );
    }

    /// Fail-open regression guard: an unknown status for OUR deployment keeps
    /// polling — the guard must never turn an unrecognised state into a failure.
    #[test]
    fn deploy_guard_continues_on_unknown_status_with_matching_deployment() {
        assert_eq!(
            evaluate(
                &found_with_deployment("some-brand-new-status", "deploy-new"),
                &DEPLOY_APP_WAIT,
                Some("deploy-new"),
            ),
            PollDecision::Continue
        );
    }

    /// When the poll response carries no deployment id (the slim app view may
    /// omit it), the guard cannot engage and evaluation falls back to
    /// status-only rather than waiting forever.
    #[test]
    fn deploy_guard_falls_back_to_status_when_poll_has_no_deployment_id() {
        assert_eq!(
            evaluate(
                &found("deployment-running"),
                &DEPLOY_APP_WAIT,
                Some("deploy-new")
            ),
            PollDecision::Succeeded
        );
    }

    // ── deployment-down: came up, then crashed (failed rollout) ──

    /// `deployment-down` for OUR deployment means the app came up and then went
    /// down (bad image, failed readiness, CrashLoopBackOff). It must fail fast,
    /// not fall-open into a full-limit timeout. CSM only sets `deployment-down`
    /// from a running app, so it is never a transient pre-startup blip.
    #[test]
    fn deploy_guard_fails_on_deployment_down_with_matching_deployment() {
        assert_eq!(
            evaluate(
                &found_with_deployment("deployment-down", "deploy-new"),
                &DEPLOY_APP_WAIT,
                Some("deploy-new"),
            ),
            PollDecision::Failed("deployment-down".to_string())
        );
    }

    /// The identity guard still wins over the new failure state: a PREVIOUS
    /// deployment going down is not the deployment this wait is bound to.
    #[test]
    fn deploy_guard_continues_on_deployment_down_for_other_deployment() {
        assert_eq!(
            evaluate(
                &found_with_deployment("deployment-down", "deploy-old"),
                &DEPLOY_APP_WAIT,
                Some("deploy-new"),
            ),
            PollDecision::Continue
        );
    }

    /// `start-app` also treats `deployment-down` as a failure — an app it just
    /// brought up that immediately went down is a failed start, not a timeout.
    #[test]
    fn start_spec_fails_on_deployment_down() {
        assert_eq!(
            evaluate(&found("deployment-down"), &START_APP_WAIT, None),
            PollDecision::Failed("deployment-down".to_string())
        );
    }

    /// `deployment-down` is unreachable in the create/stop/delete windows, so it
    /// must NOT become a terminal failure there — only deploy/start added it.
    #[test]
    fn deployment_down_is_not_terminal_for_other_lifecycles() {
        for spec in [&CREATE_APP_WAIT, &STOP_APP_WAIT, &DELETE_APP_WAIT] {
            assert_eq!(
                evaluate(&found("deployment-down"), spec, None),
                PollDecision::Continue,
                "deployment-down must stay fail-open for {}",
                spec.failed_message
            );
        }
    }

    #[tokio::test]
    async fn delete_loop_succeeds_on_404() {
        let server = MockServer::start().await;
        let app_path = "/csm/v5/admin/namespaces/test-ns/apps/gone";
        // First poll: still removing.
        Mock::given(method("GET"))
            .and(path(app_path))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "appStatus": "app-removing" })),
            )
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        // Then gone.
        Mock::given(method("GET"))
            .and(path(app_path))
            .respond_with(ResponseTemplate::new(404).set_body_json(
                serde_json::json!({ "errorCode": 13102, "errorMessage": "app not found" }),
            ))
            .with_priority(2)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let sink = UiSink;
        let result = wait_until_app_reaches(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "gone",
            &DELETE_APP_WAIT,
            None,
            1,
            10,
            &sink,
        )
        .await;
        assert!(result.is_ok(), "delete should succeed on 404: {result:?}");
    }
}
