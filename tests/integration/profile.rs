use ags::runtime::config::{self, ProfileConfig};
use ags::runtime::execution::ResolutionInput;

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

/// Two profiles with different namespaces resolve independently
#[tokio::test]
#[serial_test::serial]
async fn test_profile_namespace_isolation() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());
    let _ns = TempEnvGuard::remove("AGS_NAMESPACE");

    let staging = ProfileConfig {
        namespace: Some("staging-ns".to_string()),
        ..Default::default()
    };
    staging.save("staging").unwrap();

    let prod = ProfileConfig {
        namespace: Some("prod-ns".to_string()),
        ..Default::default()
    };
    prod.save("prod").unwrap();

    let http = reqwest::Client::new();

    let staging_input = ResolutionInput {
        profile: Some("staging".to_string()),
        namespace: None,
        is_dry_run: true,
    };
    let staging_ctx = ags::runtime::execution::ExecutionContext::resolve(&staging_input, &http)
        .await
        .unwrap();
    assert_eq!(staging_ctx.namespace.as_deref(), Some("staging-ns"));
    assert_eq!(
        staging_ctx.namespace_source.map(|s| s.label()),
        Some("profile config")
    );

    let prod_input = ResolutionInput {
        profile: Some("prod".to_string()),
        namespace: None,
        is_dry_run: true,
    };
    let prod_ctx = ags::runtime::execution::ExecutionContext::resolve(&prod_input, &http)
        .await
        .unwrap();
    assert_eq!(prod_ctx.namespace.as_deref(), Some("prod-ns"));
    assert_eq!(
        prod_ctx.namespace_source.map(|s| s.label()),
        Some("profile config")
    );
}

/// list_profiles returns sorted names
#[test]
#[serial_test::serial]
fn test_list_profiles_sorted() {
    let tmp = tempfile::tempdir().unwrap();
    let _home = TempEnvGuard::set("AGS_HOME", tmp.path().to_str().unwrap());

    config::ensure_profile_exists("zebra").unwrap();
    config::ensure_profile_exists("alpha").unwrap();
    config::ensure_profile_exists("mid").unwrap();

    let profiles = config::list_profiles().unwrap();
    assert_eq!(profiles, vec!["alpha", "mid", "zebra"]);
}
