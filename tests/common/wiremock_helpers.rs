use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Mount a successful client-credentials token response.
pub async fn mount_token_success(server: &MockServer) {
    let body = r#"{"access_token":"test-access-token","expires_in":3600,"token_type":"Bearer"}"#;
    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
}

/// Mount a 401 token error response.
pub async fn mount_token_error_401(server: &MockServer) {
    let body =
        r#"{"error":"unauthorized_client","error_description":"invalid client credentials"}"#;
    Mock::given(method("POST"))
        .and(path("/iam/v3/oauth/token"))
        .respond_with(ResponseTemplate::new(401).set_body_string(body))
        .mount(server)
        .await;
}

/// Mount a generic API error response at the given path.
pub async fn mount_api_error(server: &MockServer, api_path: &str, status: u16, body: &str) {
    Mock::given(method("GET"))
        .and(path(api_path))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(server)
        .await;
}
