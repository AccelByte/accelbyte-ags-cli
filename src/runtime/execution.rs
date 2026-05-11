//! Process-scoped execution context — resolved profile, namespace, auth state,
//! and HTTP client configuration. Built once at startup and handed to `Runtime`.

/// Source of the active profile selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfileSource {
    /// `--profile` flag.
    Flag,
    /// `AGS_PROFILE` environment variable.
    Environment,
    /// Falls back to global config.
    GlobalConfig,
}

/// Source of the resolved base URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BaseUrlSource {
    /// `AGS_BASE_URL` environment variable.
    Environment,
    /// Profile configuration file.
    Configuration,
    /// OS keystore.
    Keystore,
    /// Default placeholder used when no real URL is available (dry-run only).
    Default,
}

/// Source of the active access token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccessTokenSource {
    /// Token has not been resolved (initial state).
    None,
    /// `AGS_ACCESS_TOKEN` environment variable.
    Environment,
    /// Stored token in the keychain or fallback file.
    Stored,
    /// Refreshed via the OAuth refresh token.
    Refreshed,
    /// Newly minted via the client-credentials grant.
    ClientCredentials,
    /// Synthetic placeholder used during dry-run.
    DryRun,
}

/// Source of the active namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NamespaceSource {
    /// `--namespace` flag.
    Flag,
    /// `AGS_NAMESPACE` environment variable.
    Environment,
    /// Profile configuration file.
    Configuration,
}

impl ProfileSource {
    /// User-facing label, must match the strings the renderer used previously.
    pub fn label(self) -> &'static str {
        match self {
            ProfileSource::Flag => "--profile flag",
            ProfileSource::Environment => "AGS_PROFILE env",
            ProfileSource::GlobalConfig => "global config",
        }
    }
}

impl BaseUrlSource {
    /// User-facing label.
    pub fn label(self) -> &'static str {
        match self {
            BaseUrlSource::Environment => "AGS_BASE_URL env",
            BaseUrlSource::Configuration => "profile config",
            BaseUrlSource::Keystore => "keystore",
            BaseUrlSource::Default => "default",
        }
    }
}

impl AccessTokenSource {
    /// User-facing label.
    pub fn label(self) -> &'static str {
        match self {
            AccessTokenSource::None => "none",
            AccessTokenSource::Environment => "AGS_ACCESS_TOKEN env",
            AccessTokenSource::Stored => "stored",
            AccessTokenSource::Refreshed => "refreshed",
            AccessTokenSource::ClientCredentials => "client-credentials grant",
            AccessTokenSource::DryRun => "dry-run",
        }
    }
}

impl NamespaceSource {
    /// User-facing label.
    pub fn label(self) -> &'static str {
        match self {
            NamespaceSource::Flag => "--namespace flag",
            NamespaceSource::Environment => "AGS_NAMESPACE env",
            NamespaceSource::Configuration => "profile config",
        }
    }
}

/// Ambient state that persists across the lifetime of a `Runtime` instance.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub profile: String,
    pub profile_source: ProfileSource,
    pub namespace: Option<String>,
    pub namespace_source: Option<NamespaceSource>,
    pub base_url: String,
    pub base_url_source: BaseUrlSource,
    pub access_token: String,
    pub access_token_source: AccessTokenSource,
    pub access_token_expiry: Option<String>,
    pub access_token_warnings: Vec<String>,
}

/// Input for resolution — populated by callers from flags, daemon config,
/// or AI adapter config. No dependency on invocation types.
#[derive(Debug, Clone)]
pub struct ResolutionInput {
    pub profile: Option<String>,
    pub namespace: Option<String>,
    pub is_dry_run: bool,
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self {
            profile: String::new(),
            profile_source: ProfileSource::GlobalConfig,
            namespace: None,
            namespace_source: None,
            base_url: String::new(),
            base_url_source: BaseUrlSource::Default,
            access_token: String::new(),
            access_token_source: AccessTokenSource::None,
            access_token_expiry: None,
            access_token_warnings: vec![],
        }
    }
}

impl ExecutionContext {
    /// Resolve the active profile, base URL, and token state into a per-call execution context.
    pub async fn resolve(
        input: &ResolutionInput,
        http: &reqwest::Client,
    ) -> Result<ExecutionContext, crate::protocol::error::RuntimeError> {
        use crate::protocol::error::RuntimeError;
        use crate::runtime::auth;
        use crate::runtime::config;

        let profile = config::resolve_profile_name(input.profile.as_deref())?;

        let profile_source = if input.profile.is_some() {
            ProfileSource::Flag
        } else if config::is_env_var_set(config::ENV_PROFILE) {
            ProfileSource::Environment
        } else {
            ProfileSource::GlobalConfig
        };

        let (
            base_url,
            base_url_source,
            access_token,
            access_token_source,
            access_token_expiry,
            access_token_warnings,
        ) = if input.is_dry_run {
            let (url, source) = auth::credentials::resolve_base_url(&profile)
                .map(|(url, source)| {
                    let mapped = match source {
                        auth::credentials::CredentialSource::Environment => {
                            BaseUrlSource::Environment
                        }
                        auth::credentials::CredentialSource::Configuration => {
                            BaseUrlSource::Configuration
                        }
                        auth::credentials::CredentialSource::Keystore => BaseUrlSource::Keystore,
                    };
                    (url, mapped)
                })
                .unwrap_or(("https://<base-url>".to_string(), BaseUrlSource::Default));
            (
                url,
                source,
                "dry-run-token".to_string(),
                AccessTokenSource::DryRun,
                None,
                vec![],
            )
        } else {
            let (url, source) = auth::credentials::resolve_base_url(&profile)
                .map(|(url, source)| {
                    let mapped = match source {
                        auth::credentials::CredentialSource::Environment => {
                            BaseUrlSource::Environment
                        }
                        auth::credentials::CredentialSource::Configuration => {
                            BaseUrlSource::Configuration
                        }
                        auth::credentials::CredentialSource::Keystore => BaseUrlSource::Keystore,
                    };
                    (url, mapped)
                })
                .ok_or_else(|| RuntimeError::from(auth::errors::AuthError::BaseUrlMissing))?;

            let resolution = auth::session::resolve_access_token(http, &profile).await?;
            let token_source = match resolution.source {
                auth::session::TokenSource::Environment => AccessTokenSource::Environment,
                auth::session::TokenSource::Stored => AccessTokenSource::Stored,
                auth::session::TokenSource::Refreshed => AccessTokenSource::Refreshed,
                auth::session::TokenSource::ClientCredentials => {
                    AccessTokenSource::ClientCredentials
                }
            };
            let expiry = resolution
                .expires_in_secs
                .map(crate::support::format_duration);
            (
                url,
                source,
                resolution.token,
                token_source,
                expiry,
                resolution.warnings,
            )
        };

        // Namespace resolution: input override -> env -> profile config.
        // Namespace is optional — missing namespace is not an error at this layer.
        let (namespace, namespace_source) = if let Some(ref namespace) = input.namespace {
            (Some(namespace.clone()), Some(NamespaceSource::Flag))
        } else if let Ok(namespace) = std::env::var(config::ENV_NAMESPACE) {
            (Some(namespace), Some(NamespaceSource::Environment))
        } else if let Ok(profile_config) = config::ProfileConfig::load(&profile) {
            if let Some(namespace) = profile_config.namespace {
                (Some(namespace), Some(NamespaceSource::Configuration))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        Ok(ExecutionContext {
            profile,
            profile_source,
            namespace,
            namespace_source,
            base_url,
            base_url_source,
            access_token,
            access_token_source,
            access_token_expiry,
            access_token_warnings,
        })
    }
}

/// Resolve namespace from flag, environment, or the active profile's config.
///
/// Used for early Clap arg injection so that commands declaring `--namespace`
/// as required can still satisfy that requirement from profile config without
/// the user re-passing the flag.
///
/// Resolution order matches `ExecutionContext::resolve()`: `--namespace` flag,
/// then `AGS_NAMESPACE` env var, then the namespace stored in the active
/// profile. Errors loading config (corrupt or missing files) are silently
/// treated as "no value" so a broken config never blocks CLI startup; Clap
/// will then surface the missing-arg error.
pub fn resolve_namespace(
    namespace_flag: Option<&str>,
    profile_flag: Option<&str>,
) -> Option<(String, NamespaceSource)> {
    if let Some(namespace) = namespace_flag {
        return Some((namespace.to_string(), NamespaceSource::Flag));
    }
    if let Ok(namespace) = std::env::var(crate::runtime::config::ENV_NAMESPACE) {
        return Some((namespace, NamespaceSource::Environment));
    }
    let profile = crate::runtime::config::resolve_profile_name(profile_flag).ok()?;
    let profile_config = crate::runtime::config::ProfileConfig::load(&profile).ok()?;
    profile_config
        .namespace
        .map(|namespace| (namespace, NamespaceSource::Configuration))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::config::{ENV_HOME, ENV_NAMESPACE, ENV_PROFILE, GlobalConfig, ProfileConfig};

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

        fn clear(key: &'static str) -> Self {
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

    /// Profile config namespace is used when no flag or env override is present —
    /// this is the bug the resolver was missing before.
    #[test]
    #[serial_test::serial]
    fn test_resolve_namespace_falls_back_to_profile_config() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(ENV_HOME, tmp.path().to_str().unwrap());
        let _ns = TempEnvGuard::clear(ENV_NAMESPACE);
        let _profile = TempEnvGuard::clear(ENV_PROFILE);

        GlobalConfig {
            active_profile: Some("default".to_string()),
            ..Default::default()
        }
        .save()
        .unwrap();
        ProfileConfig {
            namespace: Some("from-config".to_string()),
            ..Default::default()
        }
        .save("default")
        .unwrap();

        let (namespace, source) = resolve_namespace(None, None).unwrap();
        assert_eq!(namespace, "from-config");
        assert_eq!(source, NamespaceSource::Configuration);
    }

    /// An explicit --namespace flag wins over a profile config value.
    #[test]
    #[serial_test::serial]
    fn test_resolve_namespace_flag_wins_over_profile_config() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(ENV_HOME, tmp.path().to_str().unwrap());
        let _ns = TempEnvGuard::clear(ENV_NAMESPACE);
        let _profile = TempEnvGuard::clear(ENV_PROFILE);

        ProfileConfig {
            namespace: Some("from-config".to_string()),
            ..Default::default()
        }
        .save("default")
        .unwrap();

        let (namespace, source) = resolve_namespace(Some("from-flag"), None).unwrap();
        assert_eq!(namespace, "from-flag");
        assert_eq!(source, NamespaceSource::Flag);
    }

    /// AGS_NAMESPACE wins over a profile config value when no flag is supplied.
    #[test]
    #[serial_test::serial]
    fn test_resolve_namespace_env_wins_over_profile_config() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(ENV_HOME, tmp.path().to_str().unwrap());
        let _ns = TempEnvGuard::set(ENV_NAMESPACE, "from-env");
        let _profile = TempEnvGuard::clear(ENV_PROFILE);

        ProfileConfig {
            namespace: Some("from-config".to_string()),
            ..Default::default()
        }
        .save("default")
        .unwrap();

        let (namespace, source) = resolve_namespace(None, None).unwrap();
        assert_eq!(namespace, "from-env");
        assert_eq!(source, NamespaceSource::Environment);
    }

    /// --profile selects a specific profile's namespace, ignoring the active profile.
    #[test]
    #[serial_test::serial]
    fn test_resolve_namespace_uses_explicit_profile_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(ENV_HOME, tmp.path().to_str().unwrap());
        let _ns = TempEnvGuard::clear(ENV_NAMESPACE);
        let _profile = TempEnvGuard::clear(ENV_PROFILE);

        ProfileConfig {
            namespace: Some("default-ns".to_string()),
            ..Default::default()
        }
        .save("default")
        .unwrap();
        ProfileConfig {
            namespace: Some("staging-ns".to_string()),
            ..Default::default()
        }
        .save("staging")
        .unwrap();

        let (namespace, source) = resolve_namespace(None, Some("staging")).unwrap();
        assert_eq!(namespace, "staging-ns");
        assert_eq!(source, NamespaceSource::Configuration);
    }

    /// With no flag, no env, and no config value, the resolver yields None so Clap
    /// can surface the missing-arg error naturally.
    #[test]
    #[serial_test::serial]
    fn test_resolve_namespace_none_when_nothing_set() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(ENV_HOME, tmp.path().to_str().unwrap());
        let _ns = TempEnvGuard::clear(ENV_NAMESPACE);
        let _profile = TempEnvGuard::clear(ENV_PROFILE);

        assert!(resolve_namespace(None, None).is_none());
    }
}
