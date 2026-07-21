//! Domain facades on `Runtime`.
//!
//! Each sub-module adds a domain-specific `impl Runtime { ... }` block.
//! Splitting the surface across files keeps `runtime/mod.rs` small and lets
//! each domain own its own helpers without polluting a shared module.
//! New methods should live in the facade that matches their domain.

mod auth;
mod config;
mod diagnostics;
mod profile;
mod service;
