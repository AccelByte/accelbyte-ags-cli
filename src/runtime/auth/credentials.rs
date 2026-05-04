//! Credential resolution from environment variables, profile config, and keystore.

use zeroize::Zeroize;

use crate::runtime::config::ProfileConfig;

use super::store;

/// Identifies where a credential value was sourced from.
#[derive(Debug, PartialEq)]
pub enum CredentialSource {
    /// Value came from an environment variable (e.g. `AGS_CLIENT_ID`).
    Environment,
    /// Value came from the stored config file.
    Configuration,
    /// Value came from the system keychain.
    Keystore,
}

/// Resolved credentials with the source of each field recorded.
/// Fields are `None` when not configured in any source.
#[derive(Debug)]
pub struct Credentials {
    pub base_url: Option<String>,
    #[allow(dead_code)] // Populated for symmetry with client_id_source; used in tests
    pub base_url_source: Option<CredentialSource>,
    pub client_id: Option<String>,
    pub client_id_source: Option<CredentialSource>,
    pub client_secret: Option<String>,
    pub client_secret_source: Option<CredentialSource>,
}

impl Drop for Credentials {
    /// Zeroize any resolved client secret when the credentials bundle is dropped.
    fn drop(&mut self) {
        if let Some(ref mut secret) = self.client_secret {
            secret.zeroize();
        }
    }
}

/// Resolve credentials from environment variables or stored config.
///
/// Each field is resolved independently; fields not found in any source are `None`.
/// Environment variables take priority over stored config for each field.
/// Callers are responsible for checking that required fields are present.
pub(crate) fn resolve_credentials(profile: &str) -> Credentials {
    let base = resolve_base_url(profile);
    let id = resolve_client_id(profile);
    let secret = resolve_client_secret(profile);
    Credentials {
        base_url: base.as_ref().map(|(value, _)| value.clone()),
        base_url_source: base.map(|(_, source)| source),
        client_id: id.as_ref().map(|(value, _)| value.clone()),
        client_id_source: id.map(|(_, source)| source),
        client_secret: secret.as_ref().map(|(value, _)| value.clone()),
        client_secret_source: secret.map(|(_, source)| source),
    }
}

/// Resolve the client ID from environment variable or stored config.
/// Returns `None` if not set in any source.
pub(crate) fn resolve_client_id(profile: &str) -> Option<(String, CredentialSource)> {
    if let Ok(id) = std::env::var(crate::runtime::config::ENV_CLIENT_ID) {
        return Some((id, CredentialSource::Environment));
    }

    ProfileConfig::load(profile)
        .ok()?
        .client_id
        .map(|id| (id, CredentialSource::Configuration))
}

/// Resolve the client secret from environment variable or keystore.
/// Returns `None` if not set in any source.
pub(crate) fn resolve_client_secret(profile: &str) -> Option<(String, CredentialSource)> {
    if let Ok(secret) = std::env::var(crate::runtime::config::ENV_CLIENT_SECRET) {
        return Some((secret, CredentialSource::Environment));
    }

    store::get_secret(profile)
        .ok()
        .flatten()
        .map(|secret| ((*secret).clone(), CredentialSource::Keystore))
}

/// Resolve the base URL from environment variable or stored config.
/// Returns `None` if not set in any source.
pub(crate) fn resolve_base_url(profile: &str) -> Option<(String, CredentialSource)> {
    if let Ok(url) = std::env::var(crate::runtime::config::ENV_BASE_URL) {
        return Some((url, CredentialSource::Environment));
    }

    ProfileConfig::load(profile)
        .ok()?
        .base_url
        .map(|url| (url, CredentialSource::Configuration))
}

/// Resolve the base URL for `profile`, dropping the source tag.
pub(crate) fn resolve_base_url_value(profile: &str) -> Option<String> {
    resolve_base_url(profile).map(|(url, _)| url)
}

/// Resolve the client ID for `profile`, dropping the source tag.
pub(crate) fn resolve_client_id_value(profile: &str) -> Option<String> {
    resolve_client_id(profile).map(|(id, _)| id)
}

/// Look up the stored client secret for `profile`, returning `None` if absent
/// or unreadable. Uses the async `get_secret_async` wrapper so callers running
/// on the Tokio runtime do not park a worker on a blocking keychain read.
pub(crate) async fn resolve_stored_client_secret(profile: &str) -> Option<String> {
    store::get_secret_async(profile)
        .await
        .ok()
        .flatten()
        .map(|secret| (*secret).clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    /// RAII helper that removes the named environment variables on drop.
    ///
    /// `#[serial]` prevents concurrent test interference, but a panicking
    /// assertion still leaks any env vars set before the panic. Holding one of
    /// these guards in scope ensures the cleanup runs even on panic.
    struct EnvGuard(&'static [&'static str]);

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for name in self.0 {
                std::env::remove_var(name);
            }
        }
    }

    /// All credential fields must be None when no environment variables or config exist.
    #[test]
    #[serial]
    fn test_resolve_credentials_returns_none_for_every_field_when_empty() {
        let _guard = EnvGuard(&[
            "AGS_BASE_URL",
            "AGS_CLIENT_ID",
            "AGS_CLIENT_SECRET",
            "AGS_NO_KEYCHAIN",
            "AGS_HOME",
        ]);
        std::env::remove_var("AGS_BASE_URL");
        std::env::remove_var("AGS_CLIENT_ID");
        std::env::remove_var("AGS_CLIENT_SECRET");
        std::env::set_var("AGS_NO_KEYCHAIN", "1");
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        std::env::set_var("AGS_HOME", temp_dir.path());

        let result = resolve_credentials("default");
        assert!(result.base_url.is_none());
        assert!(result.client_id.is_none());
        assert!(result.client_secret.is_none());
    }

    /// Two profiles with different base_urls resolve independently.
    #[test]
    #[serial]
    fn test_resolve_base_url_isolates_profiles() {
        let _guard = EnvGuard(&["AGS_BASE_URL", "AGS_HOME"]);
        std::env::remove_var("AGS_BASE_URL");
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        std::env::set_var("AGS_HOME", temp_dir.path());

        let staging = ProfileConfig {
            base_url: Some("https://staging.example.com".to_string()),
            ..Default::default()
        };
        staging.save("staging").unwrap();

        let prod = ProfileConfig {
            base_url: Some("https://prod.example.com".to_string()),
            ..Default::default()
        };
        prod.save("prod").unwrap();

        assert_eq!(
            resolve_base_url("staging").map(|(v, _)| v),
            Some("https://staging.example.com".to_string())
        );
        assert_eq!(
            resolve_base_url("prod").map(|(v, _)| v),
            Some("https://prod.example.com".to_string())
        );
    }

    /// Environment variables must take priority and be tagged with CredentialSource::Environment.
    #[test]
    #[serial]
    fn test_resolve_credentials_uses_environment_variables() {
        let _guard = EnvGuard(&["AGS_BASE_URL", "AGS_CLIENT_ID", "AGS_CLIENT_SECRET"]);
        std::env::set_var("AGS_BASE_URL", "https://example.accelbyte.io");
        std::env::set_var("AGS_CLIENT_ID", "my-id");
        std::env::set_var("AGS_CLIENT_SECRET", "my-secret");

        let result = resolve_credentials("default");
        assert_eq!(
            result.base_url.as_deref(),
            Some("https://example.accelbyte.io")
        );
        assert_eq!(result.base_url_source, Some(CredentialSource::Environment));
        assert_eq!(result.client_id.as_deref(), Some("my-id"));
        assert_eq!(result.client_id_source, Some(CredentialSource::Environment));
        assert_eq!(result.client_secret.as_deref(), Some("my-secret"));
        assert_eq!(
            result.client_secret_source,
            Some(CredentialSource::Environment)
        );
    }

    /// Stored client-secret lookup returns None cleanly when no keychain or
    /// fallback-file secret exists for the profile.
    #[tokio::test]
    #[serial]
    async fn test_resolve_stored_client_secret_returns_none_when_empty() {
        let _guard = EnvGuard(&["AGS_HOME", "AGS_NO_KEYCHAIN"]);
        let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
        std::env::set_var("AGS_HOME", temp_dir.path());
        std::env::set_var("AGS_NO_KEYCHAIN", "1");

        let secret = resolve_stored_client_secret("default").await;

        assert!(secret.is_none());
    }
}
