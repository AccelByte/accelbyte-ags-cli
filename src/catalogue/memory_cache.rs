//! In-memory store of parsed `ServiceSchema` values, wrapping the on-disk
//! catalogue cache. Populated lazily on first `get_or_load` call per service.

use std::collections::HashMap;

use super::repository;
use crate::protocol::catalogue::ServiceSchema;
use crate::protocol::error::RuntimeError;

/// Lazy in-memory cache of parsed `ServiceSchema` values.
///
/// A cache miss falls through to `repository::load_service`, which handles the
/// on-disk cache and bundled-spec fallback. Subsequent hits return a borrow
/// of the stored value.
pub struct ServiceSchemaMemoryCache {
    services: HashMap<String, ServiceSchema>,
}

impl ServiceSchemaMemoryCache {
    /// Create an empty in-memory schema cache.
    pub fn new() -> Self {
        Self {
            services: HashMap::new(),
        }
    }

    /// Look up a service by name, loading it from disk/bundle on first miss.
    pub fn get_or_load(&mut self, service: &str) -> Result<&ServiceSchema, RuntimeError> {
        if !self.services.contains_key(service) {
            let (schema, _source) = repository::load_service(service)?;
            self.services.insert(service.to_string(), schema);
        }
        Ok(self
            .services
            .get(service)
            .expect("service just inserted or already present"))
    }
}

impl Default for ServiceSchemaMemoryCache {
    /// Create an empty store via `ServiceSchemaMemoryCache::new`.
    fn default() -> Self {
        Self::new()
    }
}
