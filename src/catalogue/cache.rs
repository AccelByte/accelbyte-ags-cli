//! Parsed service-schema cache helpers.

use std::path::{Path, PathBuf};

use crate::catalogue::manifest::ServiceId;
use crate::protocol::catalogue::ServiceSchema;
use crate::protocol::error::RuntimeError;
use crate::runtime::config;

/// Wrapper written to and read from the on-disk spec cache.
///
/// `cli_version` is the sole compatibility gate: any mismatch is treated as a
/// cache miss and the schema is rebuilt from the bundled spec. Unknown fields
/// are intentionally tolerated (no `deny_unknown_fields`) so that a cache file
/// written by a newer CLI version does not hard-fail on an older one — the
/// version mismatch will catch it regardless.
///
/// Any new optional fields must carry `#[serde(default)]` so cache files written
/// by an older CLI version deserialise successfully and reach the version check
/// rather than the corruption branch.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedEnvelope {
    cli_version: String,
    schema: ServiceSchema,
}

/// Build the filesystem path for a service's definition cache file.
pub(crate) fn cache_path(service: ServiceId) -> Result<PathBuf, RuntimeError> {
    let dir = config::cache_dir().map_err(|e| config::internal_error(e.to_string()))?;
    Ok(dir.join(format!("{}.json", service.as_str())))
}

/// Build the filesystem path for the spec cache lock file.
pub(crate) fn cache_lock_path() -> Result<PathBuf, RuntimeError> {
    Ok(config::cache_dir()
        .map_err(|e| config::internal_error(e.to_string()))?
        .join(".cache.lock"))
}

/// Load a cached schema envelope from disk, treating corruption and version
/// mismatch as cache misses so callers can rebuild from the bundled spec.
pub(crate) fn load_from_cache(path: &Path) -> Result<Option<ServiceSchema>, RuntimeError> {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(config::internal_error(format!(
                "Failed to read spec cache: {e}"
            )))
        }
    };
    let envelope: CachedEnvelope = match serde_json::from_str(&data) {
        Ok(e) => e,
        Err(_) => {
            let _ = std::fs::remove_file(path);
            return Ok(None);
        }
    };
    if envelope.cli_version != env!("CARGO_PKG_VERSION") {
        let _ = std::fs::remove_file(path);
        return Ok(None);
    }
    Ok(Some(envelope.schema))
}

/// Delete all cached parsed service definitions so they can be regenerated.
pub(crate) fn clear_definition_cache() -> Result<(), RuntimeError> {
    let cache_dir = config::cache_dir().map_err(|e| config::internal_error(e.to_string()))?;
    if !cache_dir.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(&cache_dir)
        .map_err(|e| config::internal_error(format!("Failed to read spec cache dir: {e}")))?
    {
        let entry = entry
            .map_err(|e| config::internal_error(format!("Failed to inspect cache entry: {e}")))?;
        let path = entry.path();
        let is_json = path.extension().map(|e| e == "json").unwrap_or(false);
        if path.is_file() && is_json {
            std::fs::remove_file(&path).map_err(|e| {
                config::internal_error(format!(
                    "Failed to clear cached spec '{}': {e}",
                    path.display()
                ))
            })?;
        }
    }

    Ok(())
}

/// Serialize a `ServiceSchema` to JSON and write it to the cache path.
///
/// Requires the parent directory to already exist. All callers acquire the
/// cache lock via `FileLock::acquire` before reaching this function, which
/// creates the directory as a side-effect.
pub(crate) fn write_to_cache(path: &Path, schema: &ServiceSchema) -> Result<(), RuntimeError> {
    debug_assert!(
        path.parent().map(|d| d.exists()).unwrap_or(false),
        "write_to_cache: parent directory must exist (guaranteed by FileLock::acquire)"
    );
    let envelope = CachedEnvelope {
        cli_version: env!("CARGO_PKG_VERSION").to_string(),
        schema: schema.clone(),
    };
    let data = serde_json::to_string(&envelope)
        .map_err(|e| config::internal_error(format!("Failed to serialize spec cache: {e}")))?;
    crate::support::file_system::write_file_restricted(path, &data).map_err(|e| {
        config::internal_error(format!(
            "Failed to write spec cache '{}': {e}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Catalogue;

    /// Known manifest service ids produce valid cache paths.
    #[test]
    fn test_cache_path_accepts_valid_ids() {
        assert!(cache_path(Catalogue::find_id("iam").unwrap()).is_ok());
        assert!(cache_path(Catalogue::find_id("match2").unwrap()).is_ok());
        assert!(cache_path(Catalogue::find_id("cloudsave").unwrap()).is_ok());
    }

    /// RAII guard that resets env vars on drop so a panicking assertion does
    /// not leak state into other serial tests in the same process.
    struct EnvGuard(&'static [&'static str]);

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for name in self.0 {
                std::env::remove_var(name);
            }
        }
    }

    /// Clearing the cache removes *.json files but preserves non-JSON files such as .cache.lock.
    #[test]
    #[serial_test::serial]
    fn test_clear_definition_cache_removes_cached_files() {
        let _guard = EnvGuard(&[crate::runtime::config::ENV_HOME]);
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, tmp.path());

        let cache = tmp.path().join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("iam.json"), "{}").unwrap();
        std::fs::write(cache.join("platform.json"), "{}").unwrap();
        std::fs::write(cache.join(".cache.lock"), "").unwrap();

        clear_definition_cache().unwrap();

        assert!(
            !cache.join("iam.json").exists(),
            "iam.json should be removed"
        );
        assert!(
            !cache.join("platform.json").exists(),
            "platform.json should be removed"
        );
        assert!(
            cache.join(".cache.lock").exists(),
            ".cache.lock must be preserved"
        );
    }

    /// Corrupted cached definitions are treated as cache misses so the caller can rebuild.
    #[test]
    fn test_load_from_cache_returns_none_for_corrupted_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("corrupted.json");
        std::fs::write(&path, "{not-json").unwrap();

        let loaded = load_from_cache(&path).unwrap();

        assert!(loaded.is_none());
    }
}
