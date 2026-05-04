//! Service discovery: load, parse, and cache OpenAPI specs into CLI-ready definitions.

mod bundled;
mod cache;
mod manifest;
mod memory_cache;
pub(crate) mod openapi;
mod parser;
mod repository;
mod skeleton;

use serde_json::Value;

use crate::protocol::catalogue::{BodySchema, OperationSchema, ServiceSchema};
use crate::protocol::error::RuntimeError;

pub use manifest::ServiceId;
pub use repository::{RefreshReport, SpecSource};

/// Facade for service-schema metadata, loading, caching, and skeleton generation.
///
/// Production callers should use this type rather than reaching into the
/// catalogue internals directly.
pub struct Catalogue {
    memory_cache: memory_cache::ServiceSchemaMemoryCache,
}

impl Catalogue {
    /// Create an empty in-memory catalogue cache.
    pub fn new() -> Self {
        Self {
            memory_cache: memory_cache::ServiceSchemaMemoryCache::new(),
        }
    }

    /// Load a service schema into the in-memory cache on first miss and return
    /// a shared borrow of the cached value.
    pub fn get_or_load(&mut self, service: &str) -> Result<&ServiceSchema, RuntimeError> {
        self.memory_cache.get_or_load(service)
    }

    /// Load a service schema bypassing the in-memory cache, using the on-disk
    /// cache when available.
    ///
    /// Prefer [`Catalogue::get_or_load`] when you hold a `Catalogue` instance —
    /// it consults the in-memory cache first and avoids repeated disk reads.
    pub fn load_uncached(service: &str) -> Result<(ServiceSchema, SpecSource), RuntimeError> {
        repository::load_service(service)
    }

    /// Parse a service schema directly from the bundled OpenAPI spec, bypassing
    /// both the in-memory and parsed-schema disk caches.
    ///
    /// Avoid calling in a loop or hot path — every call re-decompresses and
    /// re-parses the spec. For repeated access prefer [`Catalogue::get_or_load`];
    /// for one-shot loads through the disk cache use [`Catalogue::load_uncached`].
    pub fn load_bundled(service: &str) -> Result<ServiceSchema, RuntimeError> {
        let spec = repository::load_bundled_spec(service)?;
        Ok(parser::parse_spec(service, &spec))
    }

    /// Return whether a parsed on-disk cache entry currently exists.
    pub fn has_cached(service: &str) -> bool {
        repository::has_cached_service(service)
    }

    /// Force a single service schema to be rebuilt from its bundled spec.
    pub fn refresh(service: &str) -> Result<(), RuntimeError> {
        repository::refresh_one(service)
    }

    /// Force every bundled service schema to be rebuilt from bundled specs.
    pub fn refresh_all() -> RefreshReport {
        repository::refresh_all()
    }

    /// Iterate the known internal service ids in CLI display order.
    pub fn service_ids() -> impl Iterator<Item = &'static str> {
        manifest::service_ids()
    }

    /// Resolve an internal service name to the CLI display name.
    pub fn display_name(internal: &str) -> Option<&'static str> {
        manifest::display_name(internal)
    }

    /// Return the display name for a service that came from
    /// [`Catalogue::service_ids`]. Panics if `internal` is not a known service —
    /// the only way to reach that branch is for callers to manufacture a service
    /// id outside the manifest, which is a bug.
    pub fn display_name_or_panic(internal: &str) -> &'static str {
        Self::display_name(internal).expect("manifest service id should resolve to a display name")
    }

    /// Resolve a CLI display name or internal name to the internal service id.
    pub fn internal_name(name: &str) -> Option<&'static str> {
        manifest::internal_name(name)
    }

    /// Look up a service by display or internal name; return its `ServiceId`.
    pub fn find_id(name: &str) -> Option<ServiceId> {
        manifest::find_id(name)
    }

    /// Return the one-line service description, or an empty string if unknown.
    pub fn service_description(service: &str) -> &'static str {
        manifest::service_description(service)
    }

    /// Return the hand-authored resource description when available.
    pub fn resource_description(service: &str, resource: &str) -> Option<&'static str> {
        manifest::get_resource_description(service, resource)
    }

    /// Build a JSON skeleton for an operation's request body.
    pub fn build_body_skeleton(operation: &OperationSchema) -> Value {
        skeleton::build_body_skeleton(operation)
    }

    /// Build a JSON skeleton directly from a request body schema, for callers
    /// that already hold a `BodySchema` and don't want to look up the full
    /// `OperationSchema`. Prefer [`Catalogue::build_body_skeleton`] when you
    /// have an operation in hand.
    pub fn build_body_skeleton_from_schema(schema: &BodySchema) -> Value {
        skeleton::build_body_schema_skeleton(schema)
    }
}

impl Default for Catalogue {
    fn default() -> Self {
        Self::new()
    }
}
