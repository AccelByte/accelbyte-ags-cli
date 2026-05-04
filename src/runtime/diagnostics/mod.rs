//! Runtime-layer diagnostics orchestration.
//!
//! Hosts the check functions, tier runner, and per-profile orchestration
//! that backs the `ags doctor` command. Callers (invocation layer,
//! daemon, AI adapter) invoke [`run_profile`] for a single profile or
//! [`run_all`] to iterate every known profile.
//!
//! Check functions and `TierRunner` are split into submodules
//! (`checks`, `runner`). Check result types are canonical in
//! [`crate::protocol::diagnostics`]. The orchestration entry points
//! are re-exported at the module root for convenience.

pub mod checks;
pub mod runner;

pub use runner::{run_all, run_profile};
