//! GitHub Releases lookup and version comparison.
//!
//! Fetches the latest release from GitHub, normalises the tag (strips a leading
//! `v`), and compares it against the running version with `semver`.

use std::time::Duration;

use super::{UpdateCheckError, UpdateCheckResult};

/// Minimal projection of GitHub's "latest release" response — we need only the
/// tag; serde ignores the rest of the payload.
#[derive(serde::Deserialize)]
struct GithubRelease {
    tag_name: String,
}

/// The real GitHub "latest release" endpoint.
pub(super) const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/AccelByte/accelbyte-ags-cli/releases/latest";

/// Hard timeout for the update-check client — independent of the user's `--timeout`.
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);

/// Compare the running version against a GitHub tag. Returns `None` if either
/// string is not valid semver (a silent miss).
pub(crate) fn compare_versions(current: &str, latest: &str) -> Option<UpdateCheckResult> {
    let current_version = semver::Version::parse(current).ok()?;
    let latest_version = semver::Version::parse(latest).ok()?;

    Some(UpdateCheckResult {
        current: current.to_string(),
        latest: latest.to_string(),
        is_newer: latest_version > current_version,
    })
}

/// Compare the running version against a GitHub tag, returning the value that
/// failed to parse as semver. Unlike [`compare_versions`], which returns
/// `None` on either-side failure, this reports the offending string so the
/// caller can surface it in an error message.
pub(crate) fn try_compare_versions(
    current: &str,
    latest: &str,
) -> Result<UpdateCheckResult, String> {
    let current_version = semver::Version::parse(current).map_err(|_| current.to_string())?;
    let latest_version = semver::Version::parse(latest).map_err(|_| latest.to_string())?;

    Ok(UpdateCheckResult {
        current: current.to_string(),
        latest: latest.to_string(),
        is_newer: latest_version > current_version,
    })
}

/// Fetch and normalise the latest release version from a GitHub-releases-style
/// endpoint. Returns the version string on success, or a typed error on
/// transport failure, non-success HTTP status, or malformed JSON.
///
/// `url` is a parameter so tests can point it at a mock server.
pub(super) async fn fetch_latest_version(
    client: &reqwest::Client,
    url: &str,
) -> Result<String, UpdateCheckError> {
    let response = client
        .get(url)
        .header("User-Agent", "accelbyte-ags-cli")
        .send()
        .await
        .map_err(|e| UpdateCheckError::Transport(e.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        return Err(UpdateCheckError::HttpStatus(status.as_u16()));
    }

    let release: GithubRelease = response
        .json()
        .await
        .map_err(|e| UpdateCheckError::Transport(e.to_string()))?;

    let normalized = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    Ok(normalized.to_string())
}

/// Build the update-check client with a caller-chosen timeout. Split out so
/// tests can use a tiny timeout without hitting the real 3-second wait.
pub(super) fn build_client_with_timeout(timeout: Duration) -> Option<reqwest::Client> {
    reqwest::Client::builder().timeout(timeout).build().ok()
}

/// The dedicated update-check HTTP client (hard 3-second timeout).
pub(super) fn build_client() -> Option<reqwest::Client> {
    build_client_with_timeout(HTTP_TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn newer_latest_is_flagged() {
        let result = compare_versions("0.4.0", "0.5.0").expect("both are valid semver versions");
        assert!(result.is_newer);
        assert_eq!(result.current, "0.4.0");
        assert_eq!(result.latest, "0.5.0");
    }

    #[test]
    fn older_latest_is_not_flagged() {
        let result = compare_versions("0.5.0", "0.4.0").expect("valid semver");
        assert!(!result.is_newer);
    }

    #[test]
    fn equal_versions_are_not_flagged() {
        let result = compare_versions("0.5.0", "0.5.0").expect("valid semver");
        assert!(!result.is_newer);
    }

    #[test]
    fn prerelease_above_current_core_is_flagged() {
        // 0.5.0-beta.1 > 0.4.0 because the core 0.5.0 > 0.4.0.
        // The pre-release tag is never consulted here - cores differ.
        let result = compare_versions("0.4.0", "0.5.0-beta.1").expect("valid semver");
        assert!(result.is_newer);
    }

    #[test]
    fn final_release_beats_its_own_prerelease() {
        let result = compare_versions("0.5.0", "0.5.0-beta.1").expect("valid semver");
        assert!(!result.is_newer);
    }

    #[test]
    fn unparseable_version_is_silent_miss() {
        assert!(compare_versions("not-a-version", "0.5.0").is_none());
        assert!(compare_versions("0.4.0", "garbage").is_none());
    }

    /// `try_compare_versions` reports the value that failed to parse, not a
    /// generic "invalid" message. Tests both sides: an invalid latest returns
    /// the latest string, an invalid current returns the current string.
    #[test]
    fn compare_versions_reports_which_side_is_invalid() {
        // Invalid latest → Err contains the latest tag.
        let err = try_compare_versions("1.0.0", "garbage").unwrap_err();
        assert_eq!(err, "garbage", "must report the invalid latest tag");

        // Invalid current → Err contains the current tag.
        let err = try_compare_versions("not-a-version", "1.0.0").unwrap_err();
        assert_eq!(err, "not-a-version", "must report the invalid current tag");
    }

    #[tokio::test]
    async fn fetch_strip_leading_v() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"tag_name": "v0.5.0"})),
            )
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/releases/latest", server.uri());

        assert_eq!(fetch_latest_version(&client, &url).await.unwrap(), "0.5.0");
    }

    #[tokio::test]
    async fn fetch_accepts_tag_without_v() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "tag_name": "0.5.0" })),
            )
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/releases/latest", server.uri());
        assert_eq!(fetch_latest_version(&client, &url).await.unwrap(), "0.5.0");
    }

    #[tokio::test]
    async fn fetch_returns_http_status_on_404() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let url = format!("{}/releases/latest", server.uri());
        assert!(matches!(
            fetch_latest_version(&client, &url).await,
            Err(UpdateCheckError::HttpStatus(404))
        ));
    }

    #[tokio::test]
    async fn fetch_times_out_when_server_is_slow() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/releases/latest"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(500))
                    .set_body_json(serde_json::json!({ "tag_name": "v0.5.0" })),
            )
            .mount(&server)
            .await;

        // A 50ms timeout against a 500ms server -> the client aborts -> Transport.
        let client = build_client_with_timeout(Duration::from_millis(50)).unwrap();
        let url = format!("{}/releases/latest", server.uri());
        assert!(matches!(
            fetch_latest_version(&client, &url).await,
            Err(UpdateCheckError::Transport(_))
        ));
    }
}
