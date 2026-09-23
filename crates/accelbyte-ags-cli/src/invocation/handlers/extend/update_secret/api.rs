//! CSM secret API calls for `ags extend update-secret`.
//!
//! Plain `reqwest`-based functions that accept `client`/`base_url`/
//! `access_token` as parameters, matching the pattern in
//! `update_var/api.rs` (built on a sibling branch) and `app_ui/setup_env.rs`
//! — this keeps the functions cheap to test against a `wiremock::MockServer`.

use crate::errors::CliError;
use crate::invocation::handlers::extend::csm_error::extract_csm_error_detail;

/// A CSM app secret, as returned by `GetListOfSecretsV5`, `SaveSecretV5`,
/// and `UpdateSecretV5`.
///
/// Deliberately has NO `value` field. The real `UpdateAppConfigV5Response`
/// (the update response) echoes the plaintext secret value back — this
/// struct must never capture it, so `serde` silently drops it on every
/// deserialize instead of it flowing into rendered output.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub(crate) struct SecretRecord {
    #[serde(rename = "configId")]
    pub(crate) config_id: String,
    #[serde(rename = "configName")]
    pub(crate) config_name: String,
    #[serde(rename = "applyMask", default)]
    pub(crate) apply_mask: bool,
    #[serde(default)]
    pub(crate) description: Option<String>,
}

#[derive(serde::Deserialize)]
struct ListSecretsResponse {
    #[serde(default)]
    data: Vec<SecretRecord>,
}

/// Page size used when walking `GetListOfSecretsV5`.
const PAGE_LIMIT: u64 = 100;

/// Maximum number of pages to walk before giving up. Prevents infinite
/// loops when a misbehaving server always returns a full page.
const MAX_PAGES: u64 = 50;

/// `GetListOfSecretsV5` — page through secrets looking for `target_key`.
///
/// Stops early as soon as the page containing `target_key` is fetched.
/// If the key is genuinely absent (a short page was seen), returns all
/// accumulated records so the caller can treat `.find() → None` as "not
/// found." If the page cap is exhausted without a short page or a match,
/// returns an error — the list may be truncated and the result unreliable.
pub(crate) async fn list_secrets(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    namespace: &str,
    app: &str,
    target_key: &str,
) -> Result<Vec<SecretRecord>, CliError> {
    list_secrets_paged(
        client,
        base_url,
        access_token,
        namespace,
        app,
        target_key,
        PAGE_LIMIT,
        MAX_PAGES,
    )
    .await
}

/// Inner implementation of [`list_secrets`] that accepts pagination
/// parameters. Production callers use the constants via the wrapper; tests
/// can pass smaller values to keep mock data manageable.
#[allow(clippy::too_many_arguments)]
async fn list_secrets_paged(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    namespace: &str,
    app: &str,
    target_key: &str,
    page_limit: u64,
    max_pages: u64,
) -> Result<Vec<SecretRecord>, CliError> {
    // Encode path segments per the CONTRIBUTING.md convention: never
    // interpolate user input into URL paths without encoding.
    let encoded_ns = ags_runtime::support::strings::encode_url_path_segment(namespace, "namespace")
        .map_err(CliError::from)?;
    let encoded_app = ags_runtime::support::strings::encode_url_path_segment(app, "app")
        .map_err(CliError::from)?;

    let url = format!(
        "{}/csm/v5/admin/namespaces/{}/apps/{}/secrets",
        base_url.trim_end_matches('/'),
        encoded_ns,
        encoded_app
    );

    let mut records = Vec::new();
    let mut offset: u64 = 0;
    let mut saw_short_page = false;

    for _ in 0..max_pages {
        let response = client
            .get(&url)
            .query(&[
                ("limit", &page_limit.to_string()),
                ("offset", &offset.to_string()),
            ])
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| CliError::Network {
                message: format!("failed to list CSM secrets: {e}"),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Check your network connection and base URL",
                ))),
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail = extract_csm_error_detail(&body);
            return Err(CliError::Api {
                message: format!(
                    "CSM GetListOfSecretsV5 returned HTTP {status} for namespace '{namespace}' app '{app}'{detail}"
                ),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    "Check the namespace, app name, and your permissions",
                ))),
                category: crate::errors::ApiErrorCategory::Upstream,
            });
        }

        let body: ListSecretsResponse = response.json().await.map_err(|e| CliError::Api {
            message: format!("failed to parse CSM secrets list response: {e}"),
            metadata: None,
            category: crate::errors::ApiErrorCategory::Upstream,
        })?;

        let page_len = body.data.len() as u64;
        let key_found = body.data.iter().any(|r| r.config_name == target_key);
        records.extend(body.data);

        if key_found {
            return Ok(records);
        }

        if page_len < page_limit {
            saw_short_page = true;
            break;
        }
        offset += page_len;
    }

    if saw_short_page {
        Ok(records)
    } else {
        Err(CliError::Api {
            message: format!(
                "secret '{target_key}' was not found within the first {offset} records \
                 in namespace '{namespace}' app '{app}'"
            ),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "The app may contain more secrets than were searched. \
                 Check the secret name or contact your administrator.",
            ))),
            category: crate::errors::ApiErrorCategory::Upstream,
        })
    }
}

#[derive(serde::Serialize)]
struct SaveSecretRequest<'a> {
    #[serde(rename = "configName")]
    config_name: &'a str,
    value: &'a str,
    #[serde(rename = "applyMask")]
    apply_mask: bool,
    description: Option<&'a str>,
    /// Required by the shared `apimodel.SaveSecretV5Request` shape.
    /// The legacy Go CLI hardcodes this to `"plaintext"` on create and
    /// never sends it on update (see
    /// `extend-helper-cli/internal/cmd/update_secret.go:106-118`).
    source: &'a str,
}

#[derive(serde::Serialize)]
struct UpdateSecretRequest<'a> {
    value: &'a str,
    #[serde(rename = "applyMask")]
    apply_mask: bool,
    description: Option<&'a str>,
}

/// `SaveSecretV5` — create a new secret.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_secret(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    namespace: &str,
    app: &str,
    key: &str,
    value: &str,
    apply_mask: bool,
    description: Option<&str>,
) -> Result<SecretRecord, CliError> {
    let encoded_ns = ags_runtime::support::strings::encode_url_path_segment(namespace, "namespace")
        .map_err(CliError::from)?;
    let encoded_app = ags_runtime::support::strings::encode_url_path_segment(app, "app")
        .map_err(CliError::from)?;

    let url = format!(
        "{}/csm/v5/admin/namespaces/{}/apps/{}/secrets",
        base_url.trim_end_matches('/'),
        encoded_ns,
        encoded_app
    );
    let response = client
        .post(&url)
        .bearer_auth(access_token)
        .json(&SaveSecretRequest {
            config_name: key,
            value,
            apply_mask,
            description,
            source: "plaintext",
        })
        .send()
        .await
        .map_err(|e| CliError::Network {
            message: format!("failed to create CSM secret '{key}': {e}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check your network connection and base URL",
            ))),
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let detail = extract_csm_error_detail(&body);
        return Err(CliError::Api {
            message: format!(
                "CSM SaveSecretV5 returned HTTP {status} for secret '{key}' in namespace '{namespace}'{detail}"
            ),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check the namespace, app name, and your permissions",
            ))),
            category: crate::errors::ApiErrorCategory::Upstream,
        });
    }

    response.json().await.map_err(|e| CliError::Api {
        message: format!("failed to parse CSM SaveSecretV5 response: {e}"),
        metadata: None,
        category: crate::errors::ApiErrorCategory::Upstream,
    })
}

/// `UpdateSecretV5` — update an existing secret by `configId`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_secret(
    client: &reqwest::Client,
    base_url: &str,
    access_token: &str,
    namespace: &str,
    app: &str,
    config_id: &str,
    value: &str,
    apply_mask: bool,
    description: Option<&str>,
) -> Result<SecretRecord, CliError> {
    let encoded_ns = ags_runtime::support::strings::encode_url_path_segment(namespace, "namespace")
        .map_err(CliError::from)?;
    let encoded_app = ags_runtime::support::strings::encode_url_path_segment(app, "app")
        .map_err(CliError::from)?;
    let encoded_id = ags_runtime::support::strings::encode_url_path_segment(config_id, "config-id")
        .map_err(CliError::from)?;

    let url = format!(
        "{}/csm/v5/admin/namespaces/{}/apps/{}/secrets/{}",
        base_url.trim_end_matches('/'),
        encoded_ns,
        encoded_app,
        encoded_id
    );
    let response = client
        .put(&url)
        .bearer_auth(access_token)
        .json(&UpdateSecretRequest {
            value,
            apply_mask,
            description,
        })
        .send()
        .await
        .map_err(|e| CliError::Network {
            message: format!("failed to update CSM secret '{config_id}': {e}"),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check your network connection and base URL",
            ))),
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let detail = extract_csm_error_detail(&body);
        return Err(CliError::Api {
            message: format!(
                "CSM UpdateSecretV5 returned HTTP {status} for secret '{config_id}' in namespace '{namespace}'{detail}"
            ),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Check the namespace, app name, and your permissions",
            ))),
            category: crate::errors::ApiErrorCategory::Upstream,
        });
    }

    response.json().await.map_err(|e| CliError::Api {
        message: format!("failed to parse CSM UpdateSecretV5 response: {e}"),
        metadata: None,
        category: crate::errors::ApiErrorCategory::Upstream,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_list_secrets_returns_data() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": "old desc"}
                ]
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = list_secrets(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
        )
        .await;

        let records = result.expect("list_secrets should succeed");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].config_id, "id-1");
        assert_eq!(records[0].config_name, "MY_KEY");
        assert!(records[0].apply_mask);
        assert_eq!(records[0].description.as_deref(), Some("old desc"));
    }

    #[tokio::test]
    async fn test_list_secrets_ignores_value_field_if_present() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"configId": "id-1", "configName": "MY_KEY", "applyMask": true, "description": null, "value": "top-secret"}
                ]
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = list_secrets(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
        )
        .await;

        // SecretRecord has no `value` field, so this must deserialize
        // successfully and simply drop the extra field.
        let records =
            result.expect("list_secrets should succeed even with an unexpected value field");
        assert_eq!(records[0].config_id, "id-1");
    }

    #[tokio::test]
    async fn test_list_secrets_walks_pages_to_find_second_page_record() {
        use wiremock::matchers::query_param;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .and(query_param("limit", "2"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"configId": "id-1", "configName": "OTHER_1", "applyMask": true, "description": null},
                    {"configId": "id-2", "configName": "OTHER_2", "applyMask": true, "description": null}
                ]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .and(query_param("limit", "2"))
            .and(query_param("offset", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"configId": "id-3", "configName": "MY_KEY", "applyMask": false, "description": "found me"}
                ]
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = list_secrets_paged(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
            2,
            10,
        )
        .await;

        let records = result.expect("list_secrets_paged should succeed");
        assert_eq!(records.len(), 3);
        assert!(
            records
                .iter()
                .any(|r| r.config_name == "MY_KEY" && r.config_id == "id-3"),
            "expected the second page's record to be reached: {records:?}"
        );
    }

    #[tokio::test]
    async fn test_list_secrets_non_200_returns_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "errorCode": 20003,
                "errorMessage": "insufficient permissions to list secrets"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = list_secrets(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
        )
        .await;

        let err = result.expect_err("a 403 response must produce an error");
        assert!(matches!(err, CliError::Api { .. }));
        assert!(err.to_string().contains("403"));
        assert!(
            err.to_string()
                .contains("insufficient permissions to list secrets"),
            "expected the response body's errorMessage in the error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_create_secret_sends_source_plaintext_and_returns_record() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .and(body_json(serde_json::json!({
                "configName": "MY_KEY",
                "value": "new-value",
                "applyMask": true,
                "description": "desc",
                "source": "plaintext"
            })))
            // Real SaveAppConfigV5Response only echoes configId/configName.
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "new-id",
                "configName": "MY_KEY"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = create_secret(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
            "new-value",
            true,
            Some("desc"),
        )
        .await;

        let record = result.expect("create_secret should succeed");
        assert_eq!(record.config_id, "new-id");
    }

    #[tokio::test]
    async fn test_update_secret_sends_expected_body_without_source_and_returns_record() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets/id-1",
            ))
            .and(body_json(serde_json::json!({
                "value": "updated-value",
                "applyMask": false,
                "description": "old desc"
            })))
            // Real UpdateAppConfigV5Response echoes value back too — the
            // mock includes it to prove SecretRecord silently drops it.
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id-1",
                "configName": "MY_KEY",
                "applyMask": false,
                "description": "old desc",
                "source": "plaintext",
                "value": "updated-value"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = update_secret(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "id-1",
            "updated-value",
            false,
            Some("old desc"),
        )
        .await;

        let record = result.expect("update_secret should succeed");
        assert_eq!(record.config_id, "id-1");
        // SecretRecord has no `value` field — this line exists to document
        // that the plaintext value in the mocked response above is
        // structurally unreachable, not merely unused.
    }

    #[tokio::test]
    async fn test_create_secret_non_200_returns_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
                "errorCode": 20004,
                "errorMessage": "secret MY_KEY already exists"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = create_secret(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
            "new-value",
            true,
            None,
        )
        .await;

        let err = result.expect_err("a 409 response must produce an error");
        assert!(matches!(err, CliError::Api { .. }));
        assert!(err.to_string().contains("409"));
        assert!(
            err.to_string().contains("secret MY_KEY already exists"),
            "expected the response body's errorMessage in the error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_update_secret_non_200_returns_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets/id-1",
            ))
            .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({
                "errorCode": 20005,
                "errorMessage": "internal error updating secret id-1"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = update_secret(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "id-1",
            "updated-value",
            true,
            None,
        )
        .await;

        let err = result.expect_err("a 500 response must produce an error");
        assert!(matches!(err, CliError::Api { .. }));
        assert!(err.to_string().contains("500"));
        assert!(
            err.to_string()
                .contains("internal error updating secret id-1"),
            "expected the response body's errorMessage in the error, got: {err}"
        );
    }

    // ── Secret values must never appear in error messages ──

    /// A CSM error response whose body embeds the submitted secret value
    /// (e.g. in an `attributes` or `request` echo field) must NOT leak
    /// that value into the `CliError::Api` message. Only recognised
    /// error fields (`errorCode`, `errorMessage`) may appear.
    #[tokio::test]
    async fn test_create_secret_error_does_not_echo_submitted_secret_value() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "errorCode": 20004,
                "errorMessage": "validation error on field value",
                "attributes": {"value": "super-secret-123"}
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = create_secret(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
            "super-secret-123",
            true,
            None,
        )
        .await;

        let err = result.expect_err("a 400 response must produce an error");
        assert!(matches!(err, CliError::Api { .. }));
        let msg = err.to_string();
        assert!(
            msg.contains("400"),
            "error must include the HTTP status: {msg}"
        );
        assert!(
            msg.contains("validation error on field value"),
            "error must include the CSM errorMessage: {msg}"
        );
        assert!(
            !msg.contains("super-secret-123"),
            "error must NOT echo the submitted secret value: {msg}"
        );
    }

    /// Same as `test_create_secret_error_does_not_echo_submitted_secret_value`
    /// but for `update_secret` (PUT path).
    #[tokio::test]
    async fn test_update_secret_error_does_not_echo_submitted_secret_value() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets/id-1",
            ))
            .respond_with(ResponseTemplate::new(422).set_body_json(serde_json::json!({
                "errorCode": 20005,
                "errorMessage": "unprocessable entity",
                "request": {"value": "another-secret-456"}
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = update_secret(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "id-1",
            "another-secret-456",
            true,
            None,
        )
        .await;

        let err = result.expect_err("a 422 response must produce an error");
        assert!(matches!(err, CliError::Api { .. }));
        let msg = err.to_string();
        assert!(
            msg.contains("422"),
            "error must include the HTTP status: {msg}"
        );
        assert!(
            msg.contains("unprocessable entity"),
            "error must include the CSM errorMessage: {msg}"
        );
        assert!(
            !msg.contains("another-secret-456"),
            "error must NOT echo the submitted secret value: {msg}"
        );
    }

    // ── Pagination: early-exit when key found, cap-exhaustion error ──

    /// When the target key is on the first page, page 2 must never be
    /// requested. A mock that returns 500 on page 2 proves the request is
    /// never made.
    #[tokio::test]
    async fn test_list_secrets_stops_paging_when_key_on_first_page() {
        use wiremock::matchers::query_param;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets",
            ))
            .and(query_param("limit", "2"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"configId": "id-1", "configName": "OTHER", "applyMask": true, "description": null},
                    {"configId": "id-2", "configName": "MY_KEY", "applyMask": true, "description": null}
                ]
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets"))
            .and(query_param("offset", "2"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = list_secrets_paged(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
            2,
            10,
        )
        .await;

        let records = result.expect("should stop after finding key on page 1");
        assert!(
            records.iter().any(|r| r.config_name == "MY_KEY"),
            "result must contain the target key: {records:?}"
        );
        server.verify().await;
    }

    /// Cap exhaustion must produce an error mentioning the record count.
    #[tokio::test]
    async fn test_list_secrets_cap_exhaustion_returns_error() {
        use wiremock::matchers::query_param;

        let server = MockServer::start().await;
        let page_limit: u64 = 2;
        let max_pages: u64 = 3;

        for page in 0..max_pages {
            let offset = page * page_limit;
            Mock::given(method("GET"))
                .and(path(
                    "/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets",
                ))
                .and(query_param("limit", page_limit.to_string()))
                .and(query_param("offset", offset.to_string()))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "data": [
                        {"configId": format!("id-{}", offset), "configName": format!("OTHER_{}", offset), "applyMask": true, "description": null},
                        {"configId": format!("id-{}", offset + 1), "configName": format!("OTHER_{}", offset + 1), "applyMask": true, "description": null}
                    ]
                })))
                .mount(&server)
                .await;
        }

        let client = reqwest::Client::new();
        let result = list_secrets_paged(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "MY_KEY",
            page_limit,
            max_pages,
        )
        .await;

        let err = result.expect_err("should return an error for cap exhaustion");
        let msg = err.to_string();
        let expected_count = max_pages * page_limit;
        assert!(
            msg.contains(&format!("within the first {expected_count} records")),
            "cap error must mention record count ({expected_count}): {msg}"
        );
    }

    // ── Path segment encoding ──

    /// A namespace containing a `/` must be percent-encoded in the request
    /// path, not split into additional path segments. The mock expects the
    /// encoded path; if the code interpolates the raw value the mock will
    /// not match and the function will return a network/connection error.
    ///
    /// Contract: CONTRIBUTING.md path-parameter rule — all user-supplied
    /// path segments go through `encode_url_path_segment`.
    #[tokio::test]
    async fn test_list_secrets_encodes_namespace_in_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(
                "/csm/v5/admin/namespaces/ns%2Fevil/apps/my-app/secrets",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"configId": "id-1", "configName": "MY_KEY", "applyMask": true}
                ]
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = list_secrets(
            &client,
            &server.uri(),
            "test-token",
            "ns/evil",
            "my-app",
            "MY_KEY",
        )
        .await;

        let records = result.expect("namespace with '/' must be encoded, not split");
        assert_eq!(records.len(), 1);
    }

    /// An app name containing a `/` must be percent-encoded in the request
    /// path on the create endpoint.
    ///
    /// Contract: CONTRIBUTING.md path-parameter rule.
    #[tokio::test]
    async fn test_create_secret_encodes_app_in_url() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/app%2Fslash/secrets",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "new-id",
                "configName": "MY_KEY"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = create_secret(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "app/slash",
            "MY_KEY",
            "val",
            true,
            None,
        )
        .await;

        result.expect("app with '/' must be encoded in the create URL");
    }

    /// A config_id containing a `/` must be percent-encoded on the update
    /// path rather than creating an extra path segment.
    ///
    /// Contract: CONTRIBUTING.md path-parameter rule.
    #[tokio::test]
    async fn test_update_secret_encodes_config_id_in_url() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path(
                "/csm/v5/admin/namespaces/test-ns/apps/my-app/secrets/id%2Fslash",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "configId": "id/slash",
                "configName": "MY_KEY"
            })))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let result = update_secret(
            &client,
            &server.uri(),
            "test-token",
            "test-ns",
            "my-app",
            "id/slash",
            "val",
            true,
            None,
        )
        .await;

        result.expect("config_id with '/' must be encoded in the update URL");
    }
}
