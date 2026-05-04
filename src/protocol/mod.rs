//! Shared boundary types and boundary-local semantics.
//!
//! This module defines the serializable request, result, output, event, error,
//! config, diagnostics, and catalogue shapes shared across invocation,
//! runtime, and frontend layers.
//!
//! Small helpers that interpret already-constructed protocol values are
//! allowed here. Ingestion logic that builds protocol values from external
//! formats such as OpenAPI specs does not belong here.

pub mod catalogue;
pub mod config;
pub mod diagnostics;
pub mod error;
pub mod event;
pub mod output;
pub(crate) mod output_views;
pub mod request;
pub mod result;
