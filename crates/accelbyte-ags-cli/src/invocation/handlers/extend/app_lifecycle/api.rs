//! CSM app-status lookup for the Extend app lifecycle `--wait` polling.
//!
//! Plain `reqwest` function taking `client`/`base_url`/`access_token` as
//! parameters, matching `update_secret/api.rs` and `update_var/api.rs` so it
//! is cheap to test against a `wiremock::MockServer`. Mirrors the Go
//! `GetAppV5` call used by `extend-helper-cli`'s `WaitUntilAppIs`.

use crate::errors::{ApiErrorCategory, CliError};
use crate::invocation::handlers::extend::csm_error::{csm_error_code, extract_csm_error_detail};

/// CSM `errorCode` for AppNotFound. A `GET .../apps/{app}` for a missing app
/// returns HTTP 404 with this code (and `name: "AppNotFound"`) — confirmed
/// against the live CSM v5 endpoint. Only this 404 counts as "app gone".
const CSM_APP_NOT_FOUND_CODE: i64 = 13102;

/// Result of a single app-status poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AppLookup {
    /// The app exists; carries its status and current deployment.
    Found(AppState),
    /// The app is gone (HTTP 404 / CSM `AppNotFound`).
    NotFound,
}

/// The observable state of an app from a single poll: its `appStatus` plus the
/// deployment currently associated with it, when the response carries one.
///
/// `deployment_id` is the identity guard `deploy-app --wait` uses to avoid
/// reporting success for a stale, previously-running deployment. It is
/// `Option` because the slim app status view may omit it — callers that guard
/// on it treat a missing id as "cannot distinguish" and fall back to
/// status-only evaluation rather than hang.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppState {
    pub(crate) app_status: String,
    pub(crate) deployment_id: Option<String>,
}

#[derive(serde::Deserialize)]
struct AppItem {
    #[serde(rename = "appStatus", default)]
    app_status: String,
    #[serde(rename = "deploymentId", default)]
    deployment_id: Option<String>,
}

/// `GetAppV5` — fetch the app and return its status and current deployment.
///
/// `GET /csm/v5/admin/namespaces/{namespace}/apps/{app}`. A 404 maps to
/// [`AppLookup::NotFound`] (the delete-app success signal); other non-2xx
/// statuses are returned as errors, which the poll loop treats as transient
/// and retries.
///
/// NOTE: this URL is hardcoded to `/csm/v5/`, so the `--wait` status poll
/// ignores `--api-scope` / `--api-version` — unlike the primary shim call,
/// which resolves through the bundled spec and honours those flags. The two can
/// therefore disagree: an explicit `deploy-app --api-version v2 --wait` creates
/// via v2 but polls via v5, and even without a pin a client still carrying an
/// older parse cache resolves the operation to v2 while the poll stays v5. In
/// practice both read the same app record, so the effect is benign, but the two
/// halves are not guaranteed to agree. Resolving the poll through the spec path
/// (now that `csm/admin/apps/v5/get` is bundled) is planned follow-up work.
pub(crate) async fn get_app_status(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    namespace: &str,
    app: &str,
) -> Result<AppLookup, CliError> {
    let encoded_ns = ags_runtime::support::strings::encode_url_path_segment(namespace, "namespace")
        .map_err(CliError::from)?;
    let encoded_app = ags_runtime::support::strings::encode_url_path_segment(app, "app")
        .map_err(CliError::from)?;

    let url = format!(
        "{}/csm/v5/admin/namespaces/{}/apps/{}",
        base_url.trim_end_matches('/'),
        encoded_ns,
        encoded_app
    );

    let response = client
        .get(&url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| CliError::Network {
            message: format!("failed to get CSM app status for '{app}': {e}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check your network connection and base URL",
            ))),
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        // "App gone" is the delete-app success signal — but only a genuine CSM
        // AppNotFound 404 proves it. Any other 404 (a wrong route, a
        // gateway/proxy 404, or the endpoint missing in this environment) is
        // NOT proof the app is gone, so it falls through to the transient-error
        // path below: the poll keeps going and, for delete-app, times out
        // rather than falsely reporting success. AppNotFound is errorCode
        // 13102 (confirmed against the live CSM v5 GetApp endpoint).
        if status == reqwest::StatusCode::NOT_FOUND
            && csm_error_code(&body) == Some(CSM_APP_NOT_FOUND_CODE)
        {
            return Ok(AppLookup::NotFound);
        }
        let detail = extract_csm_error_detail(&body);
        return Err(CliError::Api {
            message: format!(
                "CSM GetAppV5 returned HTTP {status} for app '{app}' in namespace '{namespace}'{detail}"
            ),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check the namespace, app name, and your permissions",
            ))),
            category: ApiErrorCategory::Upstream,
        });
    }

    let item: AppItem = response.json().await.map_err(|e| CliError::Api {
        message: format!("failed to parse CSM GetAppV5 response: {e}"),
        metadata: None,
        category: ApiErrorCategory::Upstream,
    })?;
    Ok(AppLookup::Found(AppState {
        app_status: item.app_status,
        deployment_id: item.deployment_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn found_returns_app_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "appStatus": "app-undeployed" })),
            )
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let lookup = get_app_status(&client, &server.uri(), "test-token", "test-ns", "my-app")
            .await
            .expect("get_app_status should succeed");
        assert_eq!(
            lookup,
            AppLookup::Found(AppState {
                app_status: "app-undeployed".to_string(),
                deployment_id: None,
            })
        );
    }

    #[tokio::test]
    async fn found_captures_deployment_id_when_present() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "appStatus": "deployment-running",
                "deploymentId": "deploy-123"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let lookup = get_app_status(&client, &server.uri(), "test-token", "test-ns", "my-app")
            .await
            .expect("get_app_status should succeed");
        assert_eq!(
            lookup,
            AppLookup::Found(AppState {
                app_status: "deployment-running".to_string(),
                deployment_id: Some("deploy-123".to_string()),
            })
        );
    }

    #[tokio::test]
    async fn not_found_maps_to_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/gone"))
            .respond_with(ResponseTemplate::new(404).set_body_json(
                serde_json::json!({ "errorCode": 13102, "errorMessage": "app not found" }),
            ))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let lookup = get_app_status(&client, &server.uri(), "test-token", "test-ns", "gone")
            .await
            .expect("404 should map to NotFound, not an error");
        assert_eq!(lookup, AppLookup::NotFound);
    }

    #[tokio::test]
    async fn unrelated_404_is_not_treated_as_app_gone() {
        // A 404 that is NOT the CSM AppNotFound error (a different domain error,
        // or a wrong route) must surface as a transient Api error so the poll
        // keeps going — otherwise delete-app --wait would report a false
        // success on any 404.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app"))
            .respond_with(ResponseTemplate::new(404).set_body_json(
                serde_json::json!({ "errorCode": 20013, "errorMessage": "namespace not found" }),
            ))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result =
            get_app_status(&client, &server.uri(), "test-token", "test-ns", "my-app").await;
        assert!(
            matches!(result, Err(CliError::Api { .. })),
            "a non-AppNotFound 404 must be an error, not NotFound; got {result:?}"
        );
    }

    #[tokio::test]
    async fn bodyless_404_is_not_treated_as_app_gone() {
        // A 404 with no CSM error body (e.g. a gateway/proxy 404) is not proof
        // the app is gone — surface it as an error, not NotFound.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result =
            get_app_status(&client, &server.uri(), "test-token", "test-ns", "my-app").await;
        assert!(
            matches!(result, Err(CliError::Api { .. })),
            "a bodyless 404 must be an error, not NotFound; got {result:?}"
        );
    }

    #[tokio::test]
    async fn server_error_is_returned_as_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result =
            get_app_status(&client, &server.uri(), "test-token", "test-ns", "my-app").await;
        assert!(
            matches!(result, Err(CliError::Api { .. })),
            "5xx must surface as an Api error so the poll loop retries"
        );
    }
}
