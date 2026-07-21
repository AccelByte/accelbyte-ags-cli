//! In-memory store of parsed `ServiceSchema` values, wrapping the on-disk
//! catalogue cache. Populated lazily on first `get_or_load` call per service.

use std::collections::HashMap;

use super::{parser, repository};
use ags_protocol::catalogue::ServiceSchema;
use ags_protocol::error::RuntimeError;

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

    /// Like [`Self::get_or_load`], but a miss parses the bundled spec directly
    /// and never consults the on-disk parsed-schema cache — so it acquires no
    /// file lock and does no disk I/O.
    pub fn get_or_load_bundled(&mut self, service: &str) -> Result<&ServiceSchema, RuntimeError> {
        if !self.services.contains_key(service) {
            let spec = repository::load_bundled_spec(service)?;
            self.services
                .insert(service.to_string(), parser::parse_spec(service, &spec));
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

impl ServiceSchemaMemoryCache {
    /// Insert a fully-parsed `ServiceSchema` directly into the in-memory cache.
    /// Exposed for test injection via `Catalogue::insert_for_tests`.
    pub(crate) fn insert(&mut self, service: String, schema: ServiceSchema) {
        self.services.insert(service, schema);
    }
}
