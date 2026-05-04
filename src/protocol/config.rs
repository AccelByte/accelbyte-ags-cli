//! Configuration resolution protocol types — describe a resolved config entry
//! and the source that produced its value.

/// Where a config value was sourced from
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub enum ConfigSource {
    Environment,
    Profile(String),
    Global,
    NotSet,
}

/// A config key with its resolved value and source
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResolvedEntry {
    pub key: String,
    pub value: Option<String>,
    pub source: ConfigSource,
}
