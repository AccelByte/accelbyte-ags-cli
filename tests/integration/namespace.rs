use ags::runtime::config::ProfileConfig;
use ags::runtime::execution::ResolutionInput;

/// RAII guard that sets an env var and restores the original value on drop.
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

/// --namespace flag overrides env vars and config to give explicit invocations deterministic behavior
#[tokio::test]
#[serial_test::serial]
async fn test_namespace_from_flag() {
    let _guard = TempEnvGuard::set("AGS_NAMESPACE", "env-ns");

    let tmp = tempfile::tempdir().unwrap();
    let _config_guard = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let config = ProfileConfig::default();
    config.save("default").unwrap();
    let mut global = ags::runtime::config::GlobalConfig::load().unwrap();
    global.active_profile = Some("default".to_string());
    global.save().unwrap();

    let input = ResolutionInput {
        profile: None,
        namespace: Some("flag-ns".to_string()),
        is_dry_run: true,
    };
    let http = reqwest::Client::new();
    let ctx = ags::runtime::execution::ExecutionContext::resolve(&input, &http)
        .await
        .unwrap();
    assert_eq!(ctx.namespace.as_deref(), Some("flag-ns"));
    assert_eq!(
        ctx.namespace_source.map(|s| s.label()),
        Some("--namespace flag")
    );
}

/// AGS_NAMESPACE env var provides a session-wide default so users don't repeat --namespace on every call
#[tokio::test]
#[serial_test::serial]
async fn test_namespace_from_env() {
    let _guard = TempEnvGuard::set("AGS_NAMESPACE", "env-ns");

    let tmp = tempfile::tempdir().unwrap();
    let _config_guard = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let config = ProfileConfig::default();
    config.save("default").unwrap();
    let mut global = ags::runtime::config::GlobalConfig::load().unwrap();
    global.active_profile = Some("default".to_string());
    global.save().unwrap();

    let input = ResolutionInput {
        profile: None,
        namespace: None,
        is_dry_run: true,
    };
    let http = reqwest::Client::new();
    let ctx = ags::runtime::execution::ExecutionContext::resolve(&input, &http)
        .await
        .unwrap();
    assert_eq!(ctx.namespace.as_deref(), Some("env-ns"));
    assert_eq!(
        ctx.namespace_source.map(|s| s.label()),
        Some("AGS_NAMESPACE env")
    );
}

/// Profile config acts as the lowest-priority persistent default, useful for project-scoped setups
#[tokio::test]
#[serial_test::serial]
async fn test_namespace_from_config() {
    let _ns_guard = TempEnvGuard::remove("AGS_NAMESPACE");

    let tmp = tempfile::tempdir().unwrap();
    let _config_guard = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());

    let config = ProfileConfig {
        namespace: Some("config-ns".to_string()),
        ..Default::default()
    };
    config.save("default").unwrap();
    let mut global = ags::runtime::config::GlobalConfig::load().unwrap();
    global.active_profile = Some("default".to_string());
    global.save().unwrap();

    let input = ResolutionInput {
        profile: None,
        namespace: None,
        is_dry_run: true,
    };
    let http = reqwest::Client::new();
    let ctx = ags::runtime::execution::ExecutionContext::resolve(&input, &http)
        .await
        .unwrap();
    assert_eq!(ctx.namespace.as_deref(), Some("config-ns"));
    assert_eq!(
        ctx.namespace_source.map(|s| s.label()),
        Some("profile config")
    );
}

/// Missing namespace from all sources resolves to None — namespace is optional at the context layer
#[tokio::test]
#[serial_test::serial]
async fn test_namespace_missing_resolves_to_none() {
    let _ns_guard = TempEnvGuard::remove("AGS_NAMESPACE");

    let tmp = tempfile::tempdir().unwrap();
    let _config_guard = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let config = ProfileConfig::default();
    config.save("default").unwrap();
    let mut global = ags::runtime::config::GlobalConfig::load().unwrap();
    global.active_profile = Some("default".to_string());
    global.save().unwrap();

    let input = ResolutionInput {
        profile: None,
        namespace: None,
        is_dry_run: true,
    };
    let http = reqwest::Client::new();
    let ctx = ags::runtime::execution::ExecutionContext::resolve(&input, &http)
        .await
        .unwrap();
    assert!(ctx.namespace.is_none());
    assert!(ctx.namespace_source.is_none());
}
