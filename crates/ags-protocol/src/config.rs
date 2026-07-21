//! Configuration resolution protocol types — describe a resolved config entry
//! and the source that produced its value.

/// Where a config value was sourced from
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum ConfigSource {
    Environment,
    Profile(String),
    Global,
    /// The OS keychain — the home of the client secret, which is never written
    /// to a plaintext config file.
    Keychain,
    NotSet,
}

/// A config key with its resolved value and source
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResolvedEntry {
    pub key: String,
    pub value: Option<String>,
    pub source: ConfigSource,
    /// Managed entries (the keychain-stored client secret) that cannot be
    /// written through `ags config set`. Such entries never carry their value
    /// in `value`; presence is conveyed by `source` (non-`NotSet` when set).
    pub read_only: bool,
}
