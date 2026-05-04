//! Config facade — `Runtime` methods for reading, writing, and unsetting
//! configuration entries at profile and global scope.

use crate::protocol::error::{RuntimeError, RuntimeErrorKind};

/// Minimum accepted value for the `timeout` config key, in seconds.
const TIMEOUT_MIN_SECS: u64 = 1;
/// Maximum accepted value for the `timeout` config key, in seconds.
const TIMEOUT_MAX_SECS: u64 = 3600;
/// Maximum accepted value for the `page-limit` config key.
const PAGE_LIMIT_MAX: u64 = 100;

impl crate::runtime::Runtime {
    /// Read one or all config keys for a profile.
    pub fn config_get(
        &self,
        profile: &str,
        key: Option<&str>,
    ) -> Result<crate::protocol::output::ConfigView, RuntimeError> {
        use crate::protocol::config::ConfigSource;
        use crate::protocol::output::ConfigView;
        use crate::runtime::config;

        match key {
            None => {
                let entries = config::resolved_config_entries(profile);
                Ok(ConfigView::GetAll {
                    profile: profile.to_string(),
                    entries,
                })
            }
            Some(cli_name) => {
                let key_def =
                    config::find_key(cli_name).ok_or_else(|| self.unknown_key_error(cli_name))?;
                let (value, source) = match key_def.scope {
                    config::ConfigScope::Global => {
                        let value = config::GlobalConfig::get_value(key_def.json_name)?;
                        let source = if value.is_some() {
                            ConfigSource::Global
                        } else {
                            ConfigSource::NotSet
                        };
                        (value, source)
                    }
                    config::ConfigScope::Profile => {
                        let value = config::ProfileConfig::get_value(profile, key_def.json_name)?;
                        let source = if value.is_some() {
                            ConfigSource::Profile(profile.to_string())
                        } else {
                            ConfigSource::NotSet
                        };
                        (value, source)
                    }
                };
                Ok(ConfigView::GetOne {
                    key: cli_name.to_string(),
                    value,
                    source,
                })
            }
        }
    }

    /// Set a config key to a value, with validation and normalisation.
    pub fn config_set(
        &self,
        profile: &str,
        key: &str,
        value: &str,
    ) -> Result<crate::protocol::output::ConfigView, RuntimeError> {
        use crate::protocol::output::ConfigView;
        use crate::runtime::config;

        let key_def = config::find_key(key).ok_or_else(|| self.unknown_key_error(key))?;
        let normalised = self.validate_config_value(key_def, value)?;
        let save_value = normalised.as_deref().unwrap_or(value);

        match key_def.scope {
            config::ConfigScope::Global => {
                config::GlobalConfig::set_value(key_def.json_name, save_value)?;
            }
            config::ConfigScope::Profile => {
                config::ensure_profile_exists(profile)?;
                config::ProfileConfig::set_value(profile, key_def.json_name, save_value)?;
            }
        }

        Ok(ConfigView::Set {
            key: key.to_string(),
            value: save_value.to_string(),
        })
    }

    /// Remove a config key's value.
    pub fn config_unset(
        &self,
        profile: &str,
        key: &str,
    ) -> Result<crate::protocol::output::ConfigView, RuntimeError> {
        use crate::protocol::output::ConfigView;
        use crate::runtime::config;

        let key_def = config::find_key(key).ok_or_else(|| self.unknown_key_error(key))?;

        match key_def.scope {
            config::ConfigScope::Global => {
                config::GlobalConfig::unset_value(key_def.json_name)?;
            }
            config::ConfigScope::Profile => {
                config::ProfileConfig::unset_value(profile, key_def.json_name)?;
            }
        }

        Ok(ConfigView::Unset {
            key: key.to_string(),
        })
    }

    /// Build a `Validation` error listing every known config key for an unrecognised input.
    fn unknown_key_error(&self, cli_name: &str) -> RuntimeError {
        use crate::runtime::config;
        let valid_keys: Vec<&str> = config::KNOWN_KEYS.iter().map(|k| k.cli_name).collect();
        RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message: format!("Unknown config key '{cli_name}'."),
            details: None,
            hint: Some(format!("Valid keys: {}.", valid_keys.join(", "))),
            trace: None,
        }
    }

    /// Validate `value` against the rules for `key_def`, returning a normalised replacement
    /// when the value should be rewritten (e.g. trimmed trailing slash on a URL).
    ///
    /// Returns `Ok(None)` when the input is already canonical, `Ok(Some(_))` when a
    /// normalised replacement should be persisted, and `Err(_)` when the value
    /// violates the key's validation rules. Keys that have no validation rules
    /// fall through and are stored verbatim.
    fn validate_config_value(
        &self,
        key_def: &crate::runtime::config::ConfigKeyDef,
        value: &str,
    ) -> Result<Option<String>, RuntimeError> {
        match key_def.cli_name {
            "base-url" => validate_base_url(value),
            "client-id" => validate_client_id(value),
            "namespace" => validate_namespace(value),
            "format" => validate_format(value),
            "no-color" => validate_no_color(value),
            "timeout" => validate_timeout(value),
            "page-limit" => validate_page_limit(value),
            "grant-type" => validate_grant_type(value),
            "active-profile" => validate_active_profile(value),
            _ => Ok(None),
        }
    }
}

/// Build a `Validation` `RuntimeError` with an optional remediation hint.
fn validation_error(message: impl Into<String>, hint: Option<&str>) -> RuntimeError {
    RuntimeError {
        kind: RuntimeErrorKind::Validation,
        message: message.into(),
        details: None,
        hint: hint.map(String::from),
        trace: None,
    }
}

/// Validate a base URL and strip a trailing slash if present.
fn validate_base_url(value: &str) -> Result<Option<String>, RuntimeError> {
    use crate::runtime::config;
    if !config::is_valid_base_url(value) {
        return Err(validation_error(
            format!("Invalid URL: {value}"),
            Some("base-url must be a valid URL (e.g. https://demo.accelbyte.io)"),
        ));
    }
    let normalised = value.trim_end_matches('/');
    if normalised != value {
        Ok(Some(normalised.to_string()))
    } else {
        Ok(None)
    }
}

/// Validate a client ID and apply case normalisation.
fn validate_client_id(value: &str) -> Result<Option<String>, RuntimeError> {
    use crate::runtime::config;
    if !config::is_valid_client_id(value) {
        return Err(validation_error(
            format!("Invalid client-id: {value}"),
            Some("client-id must be a 32-character hex string (e.g. d39a8bb104e545a7a4b1ef6ec3d55a3c)"),
        ));
    }
    let normalised = config::normalise_client_id(value);
    if normalised != value {
        Ok(Some(normalised))
    } else {
        Ok(None)
    }
}

/// Validate that `value` matches the AccelByte namespace format.
fn validate_namespace(value: &str) -> Result<Option<String>, RuntimeError> {
    use crate::runtime::config;
    if !config::is_valid_namespace(value) {
        return Err(validation_error(
            format!("Invalid namespace: {value}"),
            Some("namespace must be lowercase alphanumeric or hyphens, max 48 chars (e.g. my-game-name)"),
        ));
    }
    Ok(None)
}

/// Validate that `value` is one of the supported output formats.
fn validate_format(value: &str) -> Result<Option<String>, RuntimeError> {
    if value != "json" {
        return Err(validation_error(
            format!("Invalid format '{value}'"),
            Some("Valid formats: json"),
        ));
    }
    Ok(None)
}

/// Validate that `value` is a boolean literal accepted by the no-color flag.
fn validate_no_color(value: &str) -> Result<Option<String>, RuntimeError> {
    if value != "true" && value != "false" {
        return Err(validation_error(
            format!("Invalid value '{value}' for no-color"),
            Some("Use 'true' or 'false'"),
        ));
    }
    Ok(None)
}

/// Validate that `value` parses as a u64 within the supported timeout range.
fn validate_timeout(value: &str) -> Result<Option<String>, RuntimeError> {
    match value.parse::<u64>() {
        Ok(n) if n < TIMEOUT_MIN_SECS => Err(validation_error(
            format!("Timeout must be at least {TIMEOUT_MIN_SECS} second (minimum)"),
            None,
        )),
        Ok(n) if n > TIMEOUT_MAX_SECS => Err(validation_error(
            format!("Timeout must not exceed {TIMEOUT_MAX_SECS} seconds (maximum)"),
            None,
        )),
        Ok(_) => Ok(None),
        Err(_) => Err(validation_error(
            format!("Invalid timeout '{value}'"),
            Some(&format!(
                "Provide a number of seconds ({TIMEOUT_MIN_SECS}-{TIMEOUT_MAX_SECS})"
            )),
        )),
    }
}

/// Validate that `value` parses as a u64 within the supported page-limit range.
fn validate_page_limit(value: &str) -> Result<Option<String>, RuntimeError> {
    let hint = format!("Use a value between 1 and {PAGE_LIMIT_MAX}");
    match value.parse::<u64>() {
        Ok(n) if (1..=PAGE_LIMIT_MAX).contains(&n) => Ok(None),
        _ => Err(validation_error(
            format!("Invalid page limit '{value}'"),
            Some(&hint),
        )),
    }
}

/// Validate that `value` is one of the supported OAuth grant types.
fn validate_grant_type(value: &str) -> Result<Option<String>, RuntimeError> {
    if value != "authorization-code" && value != "client-credentials" {
        return Err(validation_error(
            format!("Invalid grant type '{value}'"),
            Some("Valid grant types: authorization-code, client-credentials"),
        ));
    }
    Ok(None)
}

/// Validate that `value` names an existing profile on disk.
fn validate_active_profile(value: &str) -> Result<Option<String>, RuntimeError> {
    let profiles = crate::runtime::config::list_profiles().unwrap_or_default();
    if !profiles.iter().any(|p| p == value) {
        return Err(validation_error(
            format!("Profile '{value}' does not exist"),
            Some("Run 'ags profile list' to see available profiles, or 'ags profile create' to create one"),
        ));
    }
    Ok(None)
}
