//! Repository orchestration for loading and caching bundled service schemas.

use super::bundled;
use super::cache;
use super::manifest::find_id;
use super::parser;
use crate::protocol::catalogue::ServiceSchema;
use crate::protocol::error::RuntimeError;
use crate::runtime::config;
use crate::support::FileLock;

/// How the spec was loaded.
#[derive(Debug, Clone, Copy)]
pub enum SpecSource {
    /// Loaded from parsed cache on disk
    Cache,
    /// Decompressed from bundled spec
    Bundled,
}

/// Aggregate outcome of a multi-service refresh: one entry per bundled
/// service, split by success or failure, plus the total wall-clock time.
///
/// `cache_clear_error` is set when clearing the cache directory itself
/// failed before any per-service work began. It is surfaced separately
/// from `failed` so the handler can render an actionable cache-dir
/// message instead of leaking a synthetic service name.
#[derive(Debug)]
pub struct RefreshReport {
    pub succeeded: Vec<String>,
    pub failed: Vec<(String, RuntimeError)>,
    pub cache_clear_error: Option<RuntimeError>,
    pub duration: std::time::Duration,
}

/// Rebuild a single service's cache file from its bundled spec.
///
/// Decompresses the bundled spec, re-parses it, and writes a fresh cache
/// file via the atomic temp-and-rename in `write_file_restricted`, replacing
/// any existing cache. Returns an error if the bundled spec is missing,
/// decompression fails, parsing fails, or the write fails.
///
/// Unlike `load_service`, there is no post-lock re-check: the intent is always
/// to force a rebuild, so a concurrent write by another process is harmless —
/// both writes produce identical content from the same bundled spec.
pub fn refresh_one(service: &str) -> Result<(), RuntimeError> {
    // Load and validate the bundled spec before touching the cache. If the
    // service is unknown the existing cache is left intact.
    let spec = load_bundled_spec(service)?;
    let service_id = find_id(service).ok_or_else(|| {
        config::internal_error(format!("Unknown service in manifest: '{service}'"))
    })?;
    let cache_path = cache::cache_path(service_id)?;
    let _lock = FileLock::acquire(&cache::cache_lock_path()?, "spec cache")
        .map_err(|e| config::internal_error(e.to_string()))?;
    let schema = parser::parse_spec(service, &spec);
    cache::write_to_cache(&cache_path, &schema)
}

/// Rebuild every bundled service's cache file in a single pass.
///
/// Clears the cache directory once, then iterates `BUNDLED_SPECS`. A
/// per-service failure does not stop the loop; the returned report lists
/// successes and failures separately so the caller can surface all
/// problems at once. Services that succeed have their cache files
/// rewritten even if siblings failed.
///
/// The cache lock is held for the entire operation — across the directory
/// clear and all 24 per-service writes. This serialises any concurrent
/// `ags` invocation that calls `load_service` against the lock, which will
/// print "Waiting for file lock on spec cache…" until the rebuild finishes.
/// The lock must span the full cycle to prevent a concurrent reader from
/// seeing the post-clear empty directory as a permanent cache miss.
pub fn refresh_all() -> RefreshReport {
    let start = std::time::Instant::now();
    let mut succeeded = Vec::with_capacity(bundled::BUNDLED_SPECS.len());
    let mut failed = Vec::new();

    let lock_path = match cache::cache_lock_path() {
        Ok(p) => p,
        Err(e) => {
            return RefreshReport {
                succeeded,
                failed,
                cache_clear_error: Some(e),
                duration: start.elapsed(),
            };
        }
    };
    let _lock = match FileLock::acquire(&lock_path, "spec cache") {
        Ok(l) => l,
        Err(e) => {
            return RefreshReport {
                succeeded,
                failed,
                cache_clear_error: Some(config::internal_error(e.to_string())),
                duration: start.elapsed(),
            };
        }
    };

    if let Err(e) = cache::clear_definition_cache() {
        // If we can't clear the cache dir, every service would fail in the
        // same way. Surface the error in its own field so the handler can
        // render a cache-dir-specific message rather than reporting a
        // synthetic service name.
        return RefreshReport {
            succeeded,
            failed,
            cache_clear_error: Some(e),
            duration: start.elapsed(),
        };
    }

    for (service, _) in bundled::BUNDLED_SPECS {
        match rebuild_one_inner(service) {
            Ok(()) => succeeded.push((*service).to_string()),
            Err(e) => failed.push(((*service).to_string(), e)),
        }
    }

    RefreshReport {
        succeeded,
        failed,
        cache_clear_error: None,
        duration: start.elapsed(),
    }
}

/// Shared build-and-write step used by both `refresh_one` and `refresh_all`.
/// Does not clear the cache dir; callers do that as appropriate.
///
/// **Callers must hold the spec cache lock** (acquired via [`FileLock::acquire`]
/// on `cache_lock_path()`) for the duration of this call. The write path
/// inside `write_to_cache` debug-asserts the parent directory exists but does
/// not itself enforce mutual exclusion against other processes.
fn rebuild_one_inner(service: &str) -> Result<(), RuntimeError> {
    let service_id = find_id(service).ok_or_else(|| {
        config::internal_error(format!("Unknown service in manifest: '{service}'"))
    })?;
    let cache_path = cache::cache_path(service_id)?;
    let spec = load_bundled_spec(service)?;
    let schema = parser::parse_spec(service, &spec);
    cache::write_to_cache(&cache_path, &schema)?;
    Ok(())
}

/// Load a service schema, using the disk cache when available.
///
/// On first run (or after `refresh-specs` cleared the cache), decompresses the
/// bundled spec, parses operations using their `x-operationId` values, and
/// writes the resulting `ServiceSchema` to the platform cache directory.
/// Subsequent runs load directly from cache. Callers that want to force a
/// fresh parse should invoke `refresh_one` / `refresh_all` via the
/// `refresh-specs` subcommand instead.
pub fn load_service(service: &str) -> Result<(ServiceSchema, SpecSource), RuntimeError> {
    let service_id = find_id(service).ok_or_else(|| {
        config::internal_error(format!("Unknown service in manifest: '{service}'"))
    })?;
    let cache_path = cache::cache_path(service_id)?;

    // Hot path: check without the lock first. Individual writes are atomic
    // (temp-file rename), so readers always see a complete file. The write
    // path re-checks under the lock below to prevent a double-write.
    if let Some(cached) = cache::load_from_cache(&cache_path)? {
        return Ok((cached, SpecSource::Cache));
    }

    // Cache miss: parse the bundled spec speculatively. If another process
    // wins the lock race below and writes first, this parse work is discarded.
    // Acceptable trade-off: parse work is bounded and only occurs once per
    // service per CLI upgrade.
    let spec = load_bundled_spec(service)?;
    let schema = parser::parse_spec(service, &spec);

    // Acquire the lock before writing so concurrent first-run invocations
    // don't both write simultaneously. Re-check after acquiring in case another
    // process already wrote the cache while we were parsing.
    let _lock = FileLock::acquire(&cache::cache_lock_path()?, "spec cache")
        .map_err(|e| config::internal_error(e.to_string()))?;
    if let Some(cached) = cache::load_from_cache(&cache_path)? {
        return Ok((cached, SpecSource::Cache));
    }

    cache::write_to_cache(&cache_path, &schema)?;
    Ok((schema, SpecSource::Bundled))
}

/// Load a bundled spec by service name (raw SwaggerSpec, no caching).
pub fn load_bundled_spec(service: &str) -> Result<super::openapi::SwaggerSpec, RuntimeError> {
    bundled::load_bundled_spec(service)
}

/// Check whether a cached service definition exists on disk.
pub(crate) fn has_cached_service(service: &str) -> bool {
    find_id(service)
        .and_then(|id| cache::cache_path(id).ok())
        .map(|path| path.exists())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // Behaviour of `refresh_one` and `refresh_all` is exercised end-to-end by
    // the functional tests in `tests/functional/iam/cache.rs`, which run each
    // case in a child process with its own `AGS_HOME` and therefore don't
    // share process-global env state with sibling tests (unlike unit tests
    // that all run in the same process and race on `std::env::set_var`).

    /// A cache file written with the current CLI version is loaded successfully
    #[test]
    fn test_load_from_cache_returns_schema_on_version_match() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.json");

        let spec = bundled::load_bundled_spec("iam").unwrap();
        let schema = crate::catalogue::parser::parse_spec("iam", &spec);
        cache::write_to_cache(&path, &schema).unwrap();

        let result = cache::load_from_cache(&path).unwrap();
        assert!(result.is_some());
    }

    /// A cache file written with a different CLI version is treated as a cache miss
    #[test]
    fn test_load_from_cache_treats_version_mismatch_as_miss() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.json");

        // Use a real ServiceSchema so the envelope deserialises successfully and
        // the version-check branch is exercised (not the corruption branch).
        let spec = bundled::load_bundled_spec("iam").unwrap();
        let schema = crate::catalogue::parser::parse_spec("iam", &spec);
        let stale = serde_json::json!({
            "cli_version": "0.0.0-stale",
            "schema": schema,
        });
        std::fs::write(&path, serde_json::to_string(&stale).unwrap()).unwrap();

        let result = cache::load_from_cache(&path).unwrap();
        assert!(result.is_none(), "version mismatch should be a cache miss");
        assert!(
            !path.exists(),
            "stale version-mismatch file should be deleted by load_from_cache"
        );
    }

    /// A cache file with legacy format (no cli_version field) is treated as a cache miss
    #[test]
    fn test_load_from_cache_treats_legacy_format_as_miss() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.json");

        let spec = bundled::load_bundled_spec("iam").unwrap();
        let schema = crate::catalogue::parser::parse_spec("iam", &spec);
        let raw = serde_json::to_string(&schema).unwrap();
        std::fs::write(&path, raw).unwrap();

        let result = cache::load_from_cache(&path).unwrap();
        assert!(result.is_none(), "legacy format should be a cache miss");
    }
}
