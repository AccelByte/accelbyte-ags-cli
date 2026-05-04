//! Profile and global config structs, profile management, and name validation.

use std::fs;
use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::protocol::config::{ConfigSource, ResolvedEntry};
use crate::protocol::error::{RuntimeError, RuntimeErrorKind};
use crate::runtime::config::paths::{
    config_dir, global_config_lock_path, profile_config_lock_path, profile_config_path,
    profile_dir, profiles_dir,
};
use crate::runtime::config::{internal_error, DEFAULT_PROFILE, ENV_PROFILE};

// ── Shared Config Storage ──

/// Shared file-backed config store with a path, lock, and human-readable label.
struct ConfigStore {
    path: PathBuf,
    lock_path: PathBuf,
    label: String,
}

impl ConfigStore {
    /// Build the shared store wrapper for the global config file.
    fn global() -> Result<Self, RuntimeError> {
        Ok(Self {
            path: config_dir()?.join("config.json"),
            lock_path: global_config_lock_path()?,
            label: "global config".to_string(),
        })
    }

    /// Build the shared store wrapper for one profile config file.
    fn profile(profile: &str) -> Result<Self, RuntimeError> {
        Ok(Self {
            path: profile_config_path(profile)?,
            lock_path: profile_config_lock_path(profile)?,
            label: format!("profile config for '{profile}'"),
        })
    }

    /// Return the parent directory that must exist before writing or locking the file.
    fn directory(&self) -> Result<&std::path::Path, RuntimeError> {
        self.path.parent().ok_or_else(|| {
            internal_error(format!(
                "No parent directory for config file '{}'",
                self.path.display()
            ))
        })
    }

    /// Acquire the config file lock after ensuring the parent directory exists.
    fn lock(&self) -> Result<crate::support::FileLock, RuntimeError> {
        create_dir_restricted(self.directory()?)?;
        crate::support::FileLock::acquire(&self.lock_path, &self.label)
            .map_err(|e| internal_error(e.to_string()))
    }

    /// Write a complete typed config snapshot under the store lock.
    fn save_struct<T>(&self, value: &T) -> Result<(), RuntimeError>
    where
        T: Serialize,
    {
        let _lock = self.lock()?;
        self.save_struct_unlocked(value)
    }

    /// Atomically load, mutate, and save a typed config snapshot under one lock.
    fn update_struct<T, R>(
        &self,
        f: impl FnOnce(&mut T) -> Result<R, RuntimeError>,
    ) -> Result<R, RuntimeError>
    where
        T: DeserializeOwned + Default + Serialize,
    {
        let _lock = self.lock()?;
        let mut value = self.load_struct_unlocked()?;
        let result = f(&mut value)?;
        self.save_struct_unlocked(&value)?;
        Ok(result)
    }

    /// Read one raw JSON field from the config store.
    fn get_json_value(&self, json_name: &str) -> Result<Option<String>, RuntimeError> {
        load_json_value(&self.path, &self.label, json_name)
    }

    /// Set one raw JSON field in the config store under the store lock.
    fn set_json_value(&self, json_name: &str, value: &str) -> Result<(), RuntimeError> {
        let _lock = self.lock()?;
        update_json_file_field(&self.path, &self.label, json_name, Some(value))
    }

    /// Remove one raw JSON field from the config store under the store lock.
    fn unset_json_value(&self, json_name: &str) -> Result<(), RuntimeError> {
        let _lock = self.lock()?;
        update_json_file_field(&self.path, &self.label, json_name, None)
    }

    /// Load a typed config snapshot without taking the store lock.
    fn load_struct_unlocked<T>(&self) -> Result<T, RuntimeError>
    where
        T: DeserializeOwned + Default,
    {
        if !self.path.exists() {
            return Ok(T::default());
        }
        let data = fs::read_to_string(&self.path)
            .map_err(|e| internal_error(format!("Failed to read {}: {e}", self.label)))?;
        serde_json::from_str(&data)
            .map_err(|e| internal_error(format!("Failed to parse {}: {e}", self.label)))
    }

    /// Write a complete typed config snapshot without taking the store lock.
    fn save_struct_unlocked<T>(&self, value: &T) -> Result<(), RuntimeError>
    where
        T: Serialize,
    {
        let data = serde_json::to_string_pretty(value)
            .map_err(|e| internal_error(format!("Failed to serialize {}: {e}", self.label)))?;
        write_file_restricted(&self.path, &data)
    }
}

// ── Global Config ──

/// Global CLI settings stored at the config root
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct GlobalConfig {
    /// Name of the currently active profile
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_profile: Option<String>,
    /// Default output format (e.g. "json")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<crate::protocol::request::OutputFormat>,
    /// Whether to disable colour output
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_color: Option<bool>,
    /// CLI-wide request timeout in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// Default max pages for --page-all
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_limit: Option<u64>,
}

impl GlobalConfig {
    /// Load global config, returning defaults if the file does not exist
    pub fn load() -> Result<Self, RuntimeError> {
        ConfigStore::global()?.load_struct_unlocked()
    }

    /// Write a complete global-config snapshot to disk with restricted permissions.
    #[allow(dead_code)]
    pub fn save(&self) -> Result<(), RuntimeError> {
        ConfigStore::global()?.save_struct(self)
    }

    /// Atomically load, mutate, and save the shared global config under one lock.
    pub fn update<T>(
        f: impl FnOnce(&mut GlobalConfig) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        ConfigStore::global()?.update_struct(f)
    }

    /// Read one raw JSON field from the global config file.
    pub fn get_value(json_name: &str) -> Result<Option<String>, RuntimeError> {
        ConfigStore::global()?.get_json_value(json_name)
    }

    /// Set one raw JSON field in the global config file.
    pub fn set_value(json_name: &str, value: &str) -> Result<(), RuntimeError> {
        ConfigStore::global()?.set_json_value(json_name, value)
    }

    /// Remove one raw JSON field from the global config file.
    pub fn unset_value(json_name: &str) -> Result<(), RuntimeError> {
        ConfigStore::global()?.unset_json_value(json_name)
    }
}

// ── Profile Config ──

/// Per-profile configuration for a specific AGS environment
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileConfig {
    /// AccelByte base URL for API requests
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// OAuth2 client ID used during authentication
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Default namespace sent with API requests
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// OAuth2 grant type used for authentication
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_type: Option<crate::protocol::request::GrantType>,
}

impl ProfileConfig {
    /// Load profile config, returning defaults if the file does not exist
    pub fn load(profile: &str) -> Result<Self, RuntimeError> {
        ConfigStore::profile(profile)?.load_struct_unlocked()
    }

    /// Write a complete profile-config snapshot to disk with restricted permissions.
    pub fn save(&self, profile: &str) -> Result<(), RuntimeError> {
        ConfigStore::profile(profile)?.save_struct(self)
    }

    /// Atomically load, mutate, and save one profile config under one lock.
    pub fn update<T>(
        profile: &str,
        f: impl FnOnce(&mut ProfileConfig) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        ConfigStore::profile(profile)?.update_struct(f)
    }

    /// Read one raw JSON field from a profile config file.
    pub fn get_value(profile: &str, json_name: &str) -> Result<Option<String>, RuntimeError> {
        ConfigStore::profile(profile)?.get_json_value(json_name)
    }

    /// Set one raw JSON field in a profile config file.
    pub fn set_value(profile: &str, json_name: &str, value: &str) -> Result<(), RuntimeError> {
        ConfigStore::profile(profile)?.set_json_value(json_name, value)
    }

    /// Remove one raw JSON field from a profile config file.
    pub fn unset_value(profile: &str, json_name: &str) -> Result<(), RuntimeError> {
        ConfigStore::profile(profile)?.unset_json_value(json_name)
    }

    /// Resolve one config key for a profile to its effective value and source.
    fn resolve_key(
        profile: &str,
        key_def: &crate::runtime::config::ConfigKeyDef,
    ) -> (Option<String>, ConfigSource) {
        if let Some(env_name) = crate::runtime::config::env_var_name_for_key(key_def.cli_name) {
            if let Ok(value) = std::env::var(env_name) {
                if !value.is_empty() {
                    return (Some(value), ConfigSource::Environment);
                }
            }
        }

        match key_def.scope {
            crate::runtime::config::ConfigScope::Profile => {
                if let Ok(Some(value)) = Self::get_value(profile, key_def.json_name) {
                    return (Some(value), ConfigSource::Profile(profile.to_string()));
                }
            }
            crate::runtime::config::ConfigScope::Global => {
                if let Ok(Some(value)) = GlobalConfig::get_value(key_def.json_name) {
                    return (Some(value), ConfigSource::Global);
                }
            }
        }

        (None, ConfigSource::NotSet)
    }
}

// ── Resolved Config View ──

/// Resolve one config key into the effective entry shown by `ags config get`.
fn resolved_config_entry(
    profile: &str,
    key_def: &crate::runtime::config::ConfigKeyDef,
) -> ResolvedEntry {
    let (value, source) = ProfileConfig::resolve_key(profile, key_def);
    ResolvedEntry {
        key: key_def.cli_name.to_string(),
        value,
        source,
    }
}

/// Resolve all known config keys for a profile, including their effective sources.
pub fn resolved_config_entries(profile: &str) -> Vec<ResolvedEntry> {
    crate::runtime::config::KNOWN_KEYS
        .iter()
        .map(|key_def| resolved_config_entry(profile, key_def))
        .collect()
}

// ── JSON Field Helpers ──

/// Read one stringified JSON field from a config file, returning `None` when absent.
fn load_json_value(
    path: &std::path::Path,
    label: &str,
    json_name: &str,
) -> Result<Option<String>, RuntimeError> {
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read_to_string(path)
        .map_err(|e| internal_error(format!("Failed to read {label}: {e}")))?;
    let obj: serde_json::Value = serde_json::from_str(&data)
        .map_err(|e| internal_error(format!("Failed to parse {label}: {e}")))?;
    Ok(obj.get(json_name).and_then(value_to_string))
}

/// Update one raw JSON field in a config file while preserving unrelated fields.
fn update_json_file_field(
    path: &std::path::Path,
    label: &str,
    json_name: &str,
    value: Option<&str>,
) -> Result<(), RuntimeError> {
    let mut obj = if path.exists() {
        let data = fs::read_to_string(path)
            .map_err(|e| internal_error(format!("Failed to read {label}: {e}")))?;
        serde_json::from_str(&data).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if let Some(value) = value {
        obj[json_name] = parse_json_value(value);
    } else if let Some(map) = obj.as_object_mut() {
        map.remove(json_name);
    }

    let data = serde_json::to_string_pretty(&obj)
        .map_err(|e| internal_error(format!("Failed to serialize {label}: {e}")))?;
    write_file_restricted(path, &data)
}

/// Convert a JSON field value into its display-string form.
fn value_to_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Null => None,
        _ => Some(v.to_string()),
    }
}

/// Parse a CLI string into the JSON value shape stored in config files.
fn parse_json_value(value: &str) -> serde_json::Value {
    match value {
        "true" => serde_json::Value::Bool(true),
        "false" => serde_json::Value::Bool(false),
        _ => {
            if let Ok(n) = value.parse::<u64>() {
                serde_json::Value::Number(n.into())
            } else {
                serde_json::Value::String(value.to_string())
            }
        }
    }
}

// ── Profile Directory Lifecycle ──

/// Whether a named profile directory exists on disk
pub fn profile_exists(name: &str) -> Result<bool, RuntimeError> {
    Ok(profile_dir(name)?.is_dir())
}

/// List all profile names (sorted alphabetically)
pub fn list_profiles() -> Result<Vec<String>, RuntimeError> {
    let dir = profiles_dir()?;
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| internal_error(format!("Failed to read profiles directory: {e}")))?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();
    Ok(names)
}

// ── Profile name validation ──

/// Validate and normalise a profile name (alphanumeric + hyphens, lowercase)
pub fn validate_profile_name(name: &str) -> Result<String, RuntimeError> {
    let normalised = name.to_lowercase();
    if normalised.is_empty() {
        return Err(RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message: "Profile name cannot be empty.".to_string(),
            details: None,
            hint: Some("Use a name like 'default', 'staging', or 'prod-us'.".to_string()),
                    trace: None,
        });
    }
    let valid = normalised
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    let starts_valid = normalised
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric());
    if !valid || !starts_valid {
        return Err(RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message: format!("Invalid profile name '{name}'."),
            details: None,
            hint: Some("Profile names must start with a letter or digit and contain only lowercase letters, digits, and hyphens.".to_string()),
            trace: None,
        });
    }
    Ok(normalised)
}

// ── Profile resolution ──

/// Resolve the active profile name: --profile flag -> AGS_PROFILE env -> global config -> first-run default -> error
pub fn resolve_profile_name(flag: Option<&str>) -> Result<String, RuntimeError> {
    if let Some(name) = flag {
        return validate_profile_name(name);
    }
    if let Ok(name) = std::env::var(ENV_PROFILE) {
        if !name.is_empty() {
            return validate_profile_name(&name);
        }
    }
    if let Some(name) = GlobalConfig::load()?.active_profile {
        if !name.is_empty() {
            return validate_profile_name(&name);
        }
    }

    // First run: profiles directory doesn't exist yet — create default profile
    let profiles = profiles_dir()?;
    if !profiles.exists() {
        ensure_profile_exists(DEFAULT_PROFILE)?;
        GlobalConfig::update(|global| {
            global.active_profile = Some(DEFAULT_PROFILE.to_string());
            Ok(())
        })?;
        return Ok(DEFAULT_PROFILE.to_string());
    }

    // Profiles directory exists but no active profile — user must choose
    Err(RuntimeError {
        kind: RuntimeErrorKind::Validation,
        message: "No active profile.".to_string(),
        details: None,
        hint: Some(
            "Run 'ags profile create <name>' and 'ags profile use <name>' to get started."
                .to_string(),
        ),
            trace: None,
    })
}

/// Create the profile directory and an empty config if the profile does not exist
pub fn ensure_profile_exists(name: &str) -> Result<(), RuntimeError> {
    let dir = profile_dir(name)?;
    if !dir.is_dir() {
        create_dir_restricted(&dir)?;
        ProfileConfig::default().save(name)?;
    }
    Ok(())
}

// ── Filesystem Wrappers ──

/// Create a restricted directory tree and wrap filesystem errors as config runtime errors.
fn create_dir_restricted(dir: &std::path::Path) -> Result<(), RuntimeError> {
    crate::support::file_system::create_dir_restricted(dir).map_err(|e| {
        internal_error(format!(
            "Failed to create directory '{}': {e}",
            dir.display()
        ))
    })
}

/// Write a file atomically with restricted permissions and config-layer errors.
fn write_file_restricted(path: &std::path::Path, data: &str) -> Result<(), RuntimeError> {
    crate::support::file_system::write_file_restricted(path, data)
        .map_err(|e| internal_error(format!("Failed to write '{}': {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RAII guard that restores an environment variable after a test mutates it.
    struct TempEnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl TempEnvGuard {
        /// Set an environment variable for the lifetime of the guard.
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, original }
        }

        /// Clear an environment variable for the lifetime of the guard.
        fn clear(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, original }
        }
    }

    impl Drop for TempEnvGuard {
        /// Restore the original environment variable value when the guard is dropped.
        fn drop(&mut self) {
            match &self.original {
                Some(val) => std::env::set_var(self.key, val),
                None => std::env::remove_var(self.key),
            }
        }
    }

    /// AGS_HOME overrides the platform default so tests can isolate config state.
    #[test]
    #[serial_test::serial]
    fn test_config_dir_uses_env_override() {
        let _guard = TempEnvGuard::set(crate::runtime::config::ENV_HOME, "/tmp/ags-test-config");
        let dir = crate::runtime::config::config_dir().unwrap();
        assert_eq!(dir, std::path::PathBuf::from("/tmp/ags-test-config"));
    }

    /// Valid profile names are accepted unchanged.
    #[test]
    fn test_validate_profile_name_accepts_valid_names() {
        assert_eq!(validate_profile_name("default").unwrap(), "default");
        assert_eq!(validate_profile_name("staging").unwrap(), "staging");
        assert_eq!(validate_profile_name("prod-us").unwrap(), "prod-us");
        assert_eq!(validate_profile_name("dev-1").unwrap(), "dev-1");
    }

    /// Profile-name validation normalises case to lowercase.
    #[test]
    fn test_validate_profile_name_normalises_case() {
        assert_eq!(validate_profile_name("Staging").unwrap(), "staging");
        assert_eq!(validate_profile_name("PROD-US").unwrap(), "prod-us");
    }

    /// Empty profile names are rejected.
    #[test]
    fn test_validate_profile_name_rejects_empty() {
        assert!(validate_profile_name("").is_err());
    }

    /// Profile names with unsupported characters are rejected.
    #[test]
    fn test_validate_profile_name_rejects_invalid_chars() {
        assert!(validate_profile_name("my profile").is_err());
        assert!(validate_profile_name("my_profile").is_err());
        assert!(validate_profile_name("my.profile").is_err());
        assert!(validate_profile_name("../escape").is_err());
    }

    /// Profile names cannot start with a hyphen.
    #[test]
    fn test_validate_profile_name_rejects_leading_hyphen() {
        assert!(validate_profile_name("-staging").is_err());
    }

    /// An explicit profile flag wins over every other source.
    #[test]
    #[serial_test::serial]
    fn test_resolve_profile_flag_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _env = TempEnvGuard::set(ENV_PROFILE, "from-env");
        assert_eq!(
            resolve_profile_name(Some("from-flag")).unwrap(),
            "from-flag"
        );
    }

    /// AGS_PROFILE overrides the stored active profile.
    #[test]
    #[serial_test::serial]
    fn test_resolve_profile_env_wins_over_config() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _env = TempEnvGuard::set(ENV_PROFILE, "from-env");

        let global = GlobalConfig {
            active_profile: Some("from-config".to_string()),
            ..Default::default()
        };
        global.save().unwrap();

        assert_eq!(resolve_profile_name(None).unwrap(), "from-env");
    }

    /// Stored global config wins when there is no flag or environment override.
    #[test]
    #[serial_test::serial]
    fn test_resolve_profile_config_wins_over_default() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _env = TempEnvGuard::clear(ENV_PROFILE);

        let global = GlobalConfig {
            active_profile: Some("from-config".to_string()),
            ..Default::default()
        };
        global.save().unwrap();

        assert_eq!(resolve_profile_name(None).unwrap(), "from-config");
    }

    /// First-run profile resolution falls back to the default profile.
    #[test]
    #[serial_test::serial]
    fn test_resolve_profile_falls_back_to_default() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );
        let _env = TempEnvGuard::clear(ENV_PROFILE);
        assert_eq!(resolve_profile_name(None).unwrap(), "default");
    }

    /// Global config snapshots round-trip through disk.
    #[test]
    #[serial_test::serial]
    fn test_global_config_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        let config = GlobalConfig {
            active_profile: Some("staging".to_string()),
            ..Default::default()
        };
        config.save().unwrap();

        let loaded = GlobalConfig::load().unwrap();
        assert_eq!(loaded.active_profile.as_deref(), Some("staging"));
    }

    /// Missing global config files load as defaults.
    #[test]
    #[serial_test::serial]
    fn test_global_config_missing_file_returns_default() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        let loaded = GlobalConfig::load().unwrap();
        assert!(loaded.active_profile.is_none());
    }

    /// Profile config snapshots round-trip through disk.
    #[test]
    #[serial_test::serial]
    fn test_profile_config_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        let config = ProfileConfig {
            base_url: Some("https://staging.accelbyte.io".to_string()),
            client_id: Some("my-client".to_string()),
            namespace: Some("my-ns".to_string()),
            grant_type: Some(crate::protocol::request::GrantType::AuthorizationCode),
        };
        config.save("staging").unwrap();

        let loaded = ProfileConfig::load("staging").unwrap();
        assert_eq!(
            loaded.base_url.as_deref(),
            Some("https://staging.accelbyte.io")
        );
        assert_eq!(loaded.client_id.as_deref(), Some("my-client"));
        assert_eq!(loaded.namespace.as_deref(), Some("my-ns"));
        assert_eq!(
            loaded.grant_type,
            Some(crate::protocol::request::GrantType::AuthorizationCode),
        );
    }

    /// Missing profile config files load as defaults.
    #[test]
    #[serial_test::serial]
    fn test_profile_config_missing_file_returns_default() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        let loaded = ProfileConfig::load("nonexistent").unwrap();
        assert!(loaded.base_url.is_none());
    }

    /// Different profiles keep separate config state.
    #[test]
    #[serial_test::serial]
    fn test_two_profiles_are_isolated() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

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

        let loaded_staging = ProfileConfig::load("staging").unwrap();
        let loaded_prod = ProfileConfig::load("prod").unwrap();
        assert_eq!(
            loaded_staging.base_url.as_deref(),
            Some("https://staging.example.com")
        );
        assert_eq!(
            loaded_prod.base_url.as_deref(),
            Some("https://prod.example.com")
        );
    }

    /// Creating a missing profile creates its directory and config file.
    #[test]
    #[serial_test::serial]
    fn test_ensure_profile_exists_creates_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        assert!(!profile_exists("new-profile").unwrap());
        ensure_profile_exists("new-profile").unwrap();
        assert!(profile_exists("new-profile").unwrap());
    }

    /// Creating an existing profile is a no-op.
    #[test]
    #[serial_test::serial]
    fn test_ensure_profile_exists_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        ensure_profile_exists("default").unwrap();
        ensure_profile_exists("default").unwrap();
        assert!(profile_exists("default").unwrap());
    }

    /// `ProfileConfig::update` performs a read-modify-write that leaves untouched fields intact
    #[test]
    #[serial_test::serial]
    fn test_update_profile_config_preserves_unrelated_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        let initial = ProfileConfig {
            namespace: Some("demo".to_string()),
            ..Default::default()
        };
        initial.save("default").unwrap();

        ProfileConfig::update("default", |config| {
            config.client_id = Some("client".to_string());
            Ok(())
        })
        .unwrap();

        let loaded = ProfileConfig::load("default").unwrap();
        assert_eq!(loaded.namespace.as_deref(), Some("demo"));
        assert_eq!(loaded.client_id.as_deref(), Some("client"));
    }

    /// `GlobalConfig::update` performs a read-modify-write that leaves untouched fields intact
    #[test]
    #[serial_test::serial]
    fn test_update_global_config_preserves_unrelated_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let _home = TempEnvGuard::set(
            crate::runtime::config::ENV_HOME,
            tmp.path().to_str().unwrap(),
        );

        let initial = GlobalConfig {
            format: Some(crate::protocol::request::OutputFormat::Json),
            ..Default::default()
        };
        initial.save().unwrap();

        GlobalConfig::update(|config| {
            config.active_profile = Some("default".to_string());
            Ok(())
        })
        .unwrap();

        let loaded = GlobalConfig::load().unwrap();
        assert_eq!(
            loaded.format,
            Some(crate::protocol::request::OutputFormat::Json)
        );
        assert_eq!(loaded.active_profile.as_deref(), Some("default"));
    }
}
