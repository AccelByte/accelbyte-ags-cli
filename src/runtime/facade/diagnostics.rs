//! Diagnostics facade — `Runtime` entry point for `ags doctor` checks.

use crate::protocol::error::RuntimeError;

impl crate::runtime::Runtime {
    /// Run diagnostics across all known profiles.
    pub async fn run_diagnostics(
        &mut self,
        is_offline: bool,
    ) -> Result<crate::protocol::diagnostics::DoctorResult, RuntimeError> {
        crate::runtime::diagnostics::run_all(is_offline).await
    }

    /// Run diagnostics for a single profile, resolved from the profile flag.
    ///
    /// If `profile_flag` is `None`, resolves the active profile. If no active
    /// profile exists, returns a pre-canned failure report instead of an error.
    pub async fn run_diagnostics_for_profile(
        &mut self,
        profile_flag: Option<&str>,
        is_offline: bool,
    ) -> Result<crate::protocol::diagnostics::DoctorResult, RuntimeError> {
        use crate::protocol::diagnostics::DoctorResult;
        use crate::runtime::config;
        use crate::runtime::diagnostics::runner;

        let profile = match config::resolve_profile_name(profile_flag) {
            Ok(name) => name,
            Err(_) => {
                return Ok(DoctorResult {
                    reports: vec![runner::no_profile_report(
                        "No active profile",
                        "Run 'ags profile create <name>' and 'ags profile use <name>'",
                        is_offline,
                    )],
                });
            }
        };

        let global = config::GlobalConfig::load()?;
        let is_active = global
            .active_profile
            .as_deref()
            .is_some_and(|active| active == profile);
        let report =
            crate::runtime::diagnostics::run_profile(&profile, is_active, is_offline).await;

        Ok(DoctorResult {
            reports: vec![report],
        })
    }
}
