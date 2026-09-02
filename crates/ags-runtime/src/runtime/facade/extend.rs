//! Extend facade — `Runtime` methods for Extend-platform operations.
//!
//! The EHS credential fetch is extracted here as a standalone function so
//! both `docker-login` (workflow-backed) and `image-upload` (imperative
//! handler) can call it without duplicating the HTTP call or parsing.

use crate::support::strings::{encode_url_path_segment, strip_terminal_control_sequences};
use ags_protocol::error::{RuntimeError, RuntimeErrorKind};

/// Docker registry credentials returned by the EHS `GetUploadTokenV1`
/// endpoint. All three fields are required by the API spec.
#[derive(Debug, Clone)]
pub struct DockerCredentials {
    /// Container registry base URL (e.g. `https://registry.example.com`).
    pub registry_url: String,
    /// Username for authenticating to the container registry.
    pub username: String,
    /// Short-lived access token used as the password.
    pub token: String,
}

/// Deserialization target matching the EHS `GetUploadTokenV1` response.
#[derive(serde::Deserialize)]
struct GetUploadTokenResponse {
    #[serde(rename = "repositoryBaseUrl")]
    repository_base_url: String,
    username: String,
    token: String,
}

impl crate::runtime::Runtime {
    /// Fetch Docker registry credentials from the Extend Helper Service.
    ///
    /// Calls `GET /ehs/v1/namespaces/{namespace}/apps/{app}/token` with the
    /// runtime's current access token. Returns the three credential fields
    /// needed by `DockerLoginAction` and by `--print` output.
    pub async fn fetch_docker_credentials(
        &self,
        namespace: &str,
        app: &str,
    ) -> Result<DockerCredentials, RuntimeError> {
        let encoded_namespace = encode_url_path_segment(namespace, "namespace")?;
        let encoded_app = encode_url_path_segment(app, "app")?;
        let url = format!(
            "{}/ehs/v1/namespaces/{}/apps/{}/token",
            self.context.base_url.trim_end_matches('/'),
            encoded_namespace,
            encoded_app
        );

        let response = self
            .reqwest_client
            .get(&url)
            .bearer_auth(&self.context.access_token)
            .send()
            .await
            .map_err(crate::runtime::dispatch::http::network_error)?;

        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            let error_body =
                serde_json::from_str::<serde_json::Value>(&body_text).unwrap_or_else(|_| {
                    let cleaned = strip_terminal_control_sequences(&body_text);
                    if cleaned.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::json!({ "errorMessage": cleaned })
                    }
                });
            return Err(
                crate::runtime::dispatch::classify::classify_to_runtime_error(
                    status.as_u16(),
                    &error_body,
                    "ehs",
                    "repository-credentials",
                    "get",
                ),
            );
        }

        let resp: GetUploadTokenResponse = response.json().await.map_err(|e| RuntimeError {
            kind: RuntimeErrorKind::Upstream {
                status: 0,
                code: None,
            },
            message: format!("failed to parse EHS GetUploadToken response: {e}"),
            details: None,
            hint: None,
            trace: None,
        })?;

        Ok(DockerCredentials {
            registry_url: resp.repository_base_url,
            username: resp.username,
            token: resp.token,
        })
    }

    /// Fetch the `appRepoUrl` for an Extend app from the CSM `GetAppV2`
    /// endpoint.
    ///
    /// Calls `GET /csm/v2/admin/namespaces/{namespace}/apps/{app}` with
    /// the runtime's current access token. Returns the repository URL
    /// the app was built into, or a `Validation` error when the app
    /// exists but has never been built (the field is optional in the API
    /// schema).
    pub async fn fetch_app_repo_url(
        &self,
        namespace: &str,
        app: &str,
    ) -> Result<String, RuntimeError> {
        let encoded_namespace = encode_url_path_segment(namespace, "namespace")?;
        let encoded_app = encode_url_path_segment(app, "app")?;
        let url = format!(
            "{}/csm/v2/admin/namespaces/{}/apps/{}",
            self.context.base_url.trim_end_matches('/'),
            encoded_namespace,
            encoded_app
        );

        let response = self
            .reqwest_client
            .get(&url)
            .bearer_auth(&self.context.access_token)
            .send()
            .await
            .map_err(crate::runtime::dispatch::http::network_error)?;

        let status = response.status();
        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            let error_body =
                serde_json::from_str::<serde_json::Value>(&body_text).unwrap_or_else(|_| {
                    let cleaned = strip_terminal_control_sequences(&body_text);
                    if cleaned.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::json!({ "errorMessage": cleaned })
                    }
                });
            return Err(
                crate::runtime::dispatch::classify::classify_to_runtime_error(
                    status.as_u16(),
                    &error_body,
                    "csm",
                    "apps",
                    "get",
                ),
            );
        }

        let resp: GetAppV2Response = response.json().await.map_err(|e| RuntimeError {
            kind: RuntimeErrorKind::Upstream {
                status: 0,
                code: None,
            },
            message: format!("failed to parse CSM GetAppV2 response: {e}"),
            details: None,
            hint: None,
            trace: None,
        })?;

        match resp.app_repo_url {
            Some(url) if !url.is_empty() => Ok(url),
            _ => Err(RuntimeError {
                kind: RuntimeErrorKind::Validation,
                message: format!(
                    "app '{app}' does not have a container repository URL — \
                     it may not have been built yet"
                ),
                details: None,
                hint: Some(
                    "Build the app at least once before running image-upload, \
                     or check the app name."
                        .to_string(),
                ),
                trace: None,
            }),
        }
    }
}

/// Deserialization target for the CSM `GetAppV2` response.
///
/// Only the `appRepoUrl` field is extracted; the rest of the response
/// is discarded. The field is optional in the API schema — an app that
/// has never been built will not have one.
#[derive(serde::Deserialize)]
struct GetAppV2Response {
    #[serde(rename = "appRepoUrl")]
    app_repo_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::runtime::execution::{
        AccessTokenSource, BaseUrlSource, ExecutionContext, ProfileSource,
    };
    use ags_protocol::error::RuntimeErrorKind;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Build a `Runtime` pointing at the given wiremock base URL with a
    /// fake access token. No profile/config/keychain setup is required.
    fn test_runtime(base_url: &str) -> crate::runtime::Runtime {
        let context = ExecutionContext {
            profile: "test".to_string(),
            profile_source: ProfileSource::Flag,
            namespace: Some("test-ns".to_string()),
            namespace_source: None,
            base_url: base_url.to_string(),
            base_url_source: BaseUrlSource::Environment,
            access_token: "test-token".to_string(),
            access_token_source: AccessTokenSource::Environment,
            access_token_expiry: None,
            access_token_warnings: vec![],
        };
        crate::runtime::Runtime::from_reqwest(context, reqwest::Client::new())
    }

    // ── Success path ──

    #[tokio::test]
    async fn test_fetch_docker_credentials_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ehs/v1/namespaces/ns/apps/myapp/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"repositoryBaseUrl":"https://registry.example.com","username":"user","token":"tok123"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let runtime = test_runtime(&server.uri());
        let creds = runtime
            .fetch_docker_credentials("ns", "myapp")
            .await
            .expect("success path must return credentials");

        assert_eq!(creds.registry_url, "https://registry.example.com");
        assert_eq!(creds.username, "user");
        assert_eq!(creds.token, "tok123");
    }

    // ── Non-2xx: routes through the shared error classifier ──

    #[tokio::test]
    async fn test_fetch_docker_credentials_non_2xx_produces_classified_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ehs/v1/namespaces/ns/apps/myapp/token"))
            .respond_with(ResponseTemplate::new(403).set_body_string(
                r#"{"errorCode":20013,"errorMessage":"insufficient permissions"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let runtime = test_runtime(&server.uri());
        let err = runtime
            .fetch_docker_credentials("ns", "myapp")
            .await
            .expect_err("non-2xx must produce an error");

        assert!(
            matches!(err.kind, RuntimeErrorKind::Forbidden),
            "403 must map to Forbidden, got: {:?}",
            err.kind
        );
        // No credential/token value must appear in the error message.
        assert!(
            !err.message.contains("tok"),
            "error must not leak credentials: {}",
            err.message
        );
    }

    // ── Malformed JSON body ──

    #[tokio::test]
    async fn test_fetch_docker_credentials_malformed_json_produces_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ehs/v1/namespaces/ns/apps/myapp/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .expect(1)
            .mount(&server)
            .await;

        let runtime = test_runtime(&server.uri());
        let err = runtime
            .fetch_docker_credentials("ns", "myapp")
            .await
            .expect_err("malformed body must produce an error");

        assert!(
            matches!(
                err.kind,
                RuntimeErrorKind::Upstream {
                    status: 0,
                    code: None
                }
            ),
            "parse error must map to Upstream {{status:0}}, got: {:?}",
            err.kind
        );
        assert!(
            err.message.contains("parse"),
            "error should mention parsing: {}",
            err.message
        );
    }

    // ── Path-injection coverage ──

    #[tokio::test]
    async fn test_fetch_docker_credentials_rejects_traversal_in_namespace() {
        // A `../` sequence in the namespace must be rejected by the
        // encode_url_path_segment guard before any HTTP request is made.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ehs/v1/namespaces/ns/apps/myapp/token"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let runtime = test_runtime(&server.uri());
        let err = runtime
            .fetch_docker_credentials("../admin", "myapp")
            .await
            .expect_err("traversal in namespace must be rejected");

        assert!(
            matches!(err.kind, RuntimeErrorKind::Validation),
            "traversal must produce Validation error, got: {:?}",
            err.kind
        );
        assert!(
            err.message.contains("path traversal"),
            "error message must mention path traversal: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn test_fetch_docker_credentials_encodes_slash_in_app() {
        let server = MockServer::start().await;
        // The mock expects the percent-encoded path, not a traversed one.
        Mock::given(method("GET"))
            .and(path("/ehs/v1/namespaces/ns/apps/my%2Fapp/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"repositoryBaseUrl":"https://r.io","username":"u","token":"t"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let runtime = test_runtime(&server.uri());
        let creds = runtime
            .fetch_docker_credentials("ns", "my/app")
            .await
            .expect("slash in app must be encoded, not rejected");

        assert_eq!(creds.registry_url, "https://r.io");
    }

    #[tokio::test]
    async fn test_fetch_docker_credentials_double_encodes_percent_in_namespace() {
        let server = MockServer::start().await;
        // `%2e%2e` does not contain a literal `..`, so it passes the
        // traversal guard. The `%` signs are then percent-encoded to `%25`,
        // producing `%252e%252e` in the path — safe against double-decode
        // attacks.
        Mock::given(method("GET"))
            .and(path("/ehs/v1/namespaces/%252e%252e/apps/myapp/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"repositoryBaseUrl":"https://r.io","username":"u","token":"t"}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;

        let runtime = test_runtime(&server.uri());
        let creds = runtime
            .fetch_docker_credentials("%2e%2e", "myapp")
            .await
            .expect("percent-encoded input must be double-encoded");

        assert_eq!(creds.registry_url, "https://r.io");
    }

    // ── Transport failure ──

    #[tokio::test]
    async fn test_fetch_docker_credentials_transport_failure_produces_network_error() {
        // Point at an address that will refuse the connection.
        let runtime = test_runtime("http://127.0.0.1:1");
        let err = runtime
            .fetch_docker_credentials("ns", "myapp")
            .await
            .expect_err("transport failure must produce an error");

        assert!(
            matches!(err.kind, RuntimeErrorKind::Network),
            "transport failure must map to Network, got: {:?}",
            err.kind
        );
        // No credential/token value must appear in the error.
        assert!(
            !err.message.contains("test-token"),
            "error must not leak access token: {}",
            err.message
        );
    }

    // ══════════════════════════════════════════════════════════════════
    // fetch_app_repo_url
    // ══════════════════════════════════════════════════════════════════

    #[tokio::test]
    async fn test_fetch_app_repo_url_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v2/admin/namespaces/ns/apps/myapp"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"appRepoUrl":"https://registry.example.com/repo"}"#),
            )
            .expect(1)
            .mount(&server)
            .await;

        let runtime = test_runtime(&server.uri());
        let url = runtime
            .fetch_app_repo_url("ns", "myapp")
            .await
            .expect("success path must return URL");

        assert_eq!(url, "https://registry.example.com/repo");
    }

    #[tokio::test]
    async fn test_fetch_app_repo_url_missing_field_is_validation_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v2/admin/namespaces/ns/apps/myapp"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{}"#))
            .expect(1)
            .mount(&server)
            .await;

        let runtime = test_runtime(&server.uri());
        let err = runtime
            .fetch_app_repo_url("ns", "myapp")
            .await
            .expect_err("missing appRepoUrl must produce an error");

        assert!(
            matches!(err.kind, RuntimeErrorKind::Validation),
            "missing URL must map to Validation, got: {:?}",
            err.kind
        );
        assert!(
            err.message.contains("container repository URL"),
            "error should describe the missing URL: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn test_fetch_app_repo_url_empty_string_is_validation_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v2/admin/namespaces/ns/apps/myapp"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"appRepoUrl":""}"#))
            .expect(1)
            .mount(&server)
            .await;

        let runtime = test_runtime(&server.uri());
        let err = runtime
            .fetch_app_repo_url("ns", "myapp")
            .await
            .expect_err("empty appRepoUrl must produce an error");

        assert!(
            matches!(err.kind, RuntimeErrorKind::Validation),
            "empty URL must map to Validation, got: {:?}",
            err.kind
        );
    }

    #[tokio::test]
    async fn test_fetch_app_repo_url_non_2xx_produces_classified_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v2/admin/namespaces/ns/apps/myapp"))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_string(r#"{"errorCode":73245,"errorMessage":"app not found"}"#),
            )
            .expect(1)
            .mount(&server)
            .await;

        let runtime = test_runtime(&server.uri());
        let err = runtime
            .fetch_app_repo_url("ns", "myapp")
            .await
            .expect_err("404 must produce an error");

        assert!(
            matches!(err.kind, RuntimeErrorKind::NotFound),
            "404 must map to NotFound, got: {:?}",
            err.kind
        );
    }

    #[tokio::test]
    async fn test_fetch_app_repo_url_rejects_traversal_in_namespace() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v2/admin/namespaces/ns/apps/myapp"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let runtime = test_runtime(&server.uri());
        let err = runtime
            .fetch_app_repo_url("../admin", "myapp")
            .await
            .expect_err("traversal in namespace must be rejected");

        assert!(
            matches!(err.kind, RuntimeErrorKind::Validation),
            "traversal must produce Validation error, got: {:?}",
            err.kind
        );
    }
}
