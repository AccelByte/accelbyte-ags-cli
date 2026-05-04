//! Diagnostics types — structured output from `ags doctor` and similar.

/// Status of a diagnostic check
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CheckStatus {
    Pass,
    Warning,
    Fail,
    Skipped,
}

/// Which tier a check belongs to
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CheckTier {
    Config,
    Auth,
    Network,
}

/// Result of a single diagnostic check
#[derive(Debug, Clone, serde::Serialize)]
pub struct CheckResult {
    pub tier: CheckTier,
    pub name: &'static str,
    pub title: &'static str,
    pub status: CheckStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
}

/// Diagnostic results for a single profile
#[derive(Debug, Clone, serde::Serialize)]
pub struct DoctorReport {
    pub profile: String,
    pub is_active: bool,
    pub checks: Vec<CheckResult>,
}

impl DoctorReport {
    /// Returns true if any check in this report has failed.
    pub fn has_failures(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == CheckStatus::Fail)
    }

    /// Returns true if any check in this report has a warning (but no failures).
    pub fn has_warnings(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == CheckStatus::Warning)
    }
}

/// Wrapper for doctor command results
#[derive(Debug, Clone, serde::Serialize)]
pub struct DoctorResult {
    pub reports: Vec<DoctorReport>,
}

impl DoctorResult {
    /// Returns true if any report has a failed check.
    pub fn has_failures(&self) -> bool {
        self.reports.iter().any(|report| report.has_failures())
    }
}
