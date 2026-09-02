//! Config facade — `Runtime` methods for reading, writing, and unsetting
//! configuration entries at profile and global scope.

use ags_protocol::error::{RuntimeError, RuntimeErrorKind};

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
    ) -> Result<ags_protocol::output::ConfigView, RuntimeError> {
        use crate::runtime::config;
        use ags_protocol::config::ConfigSource;
        use ags_protocol::output::ConfigView;

        match key {
            None => {
                let mut entries = config::resolved_config_entries(profile);
                // Append the keychain-managed client secret as a read-only
                // entry so its presence is visible alongside the writable keys,
                // without ever exposing the value.
                entries.push(client_secret_entry(profile));
                Ok(ConfigView::GetAll {
                    profile: profile.to_string(),
                    entries,
                })
            }
            // The client secret lives in the keychain; reading it shows only
            // presence (read-only), never the value.
            Some(cli_name) if config::is_client_secret_key(cli_name) => {
                let entry = client_secret_entry(profile);
                Ok(ConfigView::GetOne {
                    key: entry.key,
                    value: entry.value,
                    source: entry.source,
                    read_only: true,
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
                    read_only: false,
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
    ) -> Result<ags_protocol::output::ConfigView, RuntimeError> {
        use crate::runtime::config;
        use ags_protocol::output::ConfigView;

        if config::is_client_secret_key(key) {
            return Err(secret_managed_error());
        }
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
    ) -> Result<ags_protocol::output::ConfigView, RuntimeError> {
        use crate::runtime::config;
        use ags_protocol::output::ConfigView;

        if config::is_client_secret_key(key) {
            return Err(secret_managed_error());
        }
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
            "no-color" => validate_boolean(value, "no-color"),
            "first-run-hint-seen" => validate_first_run_hint_seen(value),
            "timeout" => validate_timeout(value),
            "page-limit" => validate_page_limit(value),
            "grant-type" => validate_grant_type(value),
            "active-profile" => validate_active_profile(value),
            "update-check" => validate_boolean(value, "update-check"),
            _ => Ok(None),
        }
    }
}

/// Build the targeted error returned when a user tries to `set`/`unset` the
/// client secret through `ags config`. It is stored in the OS keychain, not a
/// plaintext config file, so it cannot be managed here — point at the commands
/// that do manage it.
fn secret_managed_error() -> RuntimeError {
    RuntimeError {
        kind: RuntimeErrorKind::Validation,
        message: "client-secret is stored in your OS keychain, not the config file, so it \
                  cannot be managed with 'ags config'"
            .to_string(),
        details: None,
        hint: Some(
            "Set it with 'ags auth login --client-secret <value>' or the AGS_CLIENT_SECRET \
             environment variable; 'ags config get' shows whether it is set"
                .to_string(),
        ),
        trace: None,
    }
}

/// Build the read-only config entry describing the client secret's presence and
/// source (keychain or environment), never its value.
fn client_secret_entry(profile: &str) -> ags_protocol::config::ResolvedEntry {
    use crate::runtime::auth::credentials::{resolve_client_secret, CredentialSource};
    use ags_protocol::config::ConfigSource;

    // Only the source is kept; the resolved secret string is dropped immediately.
    let source = match resolve_client_secret(profile) {
        Some((_, CredentialSource::Environment)) => ConfigSource::Environment,
        Some((_, _)) => ConfigSource::Keychain,
        None => ConfigSource::NotSet,
    };
    ags_protocol::config::ResolvedEntry {
        key: "client-secret".to_string(),
        value: None,
        source,
        read_only: true,
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

/// Validate that `value` is a boolean literal accepted by the first-run-hint-seen flag.
fn validate_first_run_hint_seen(value: &str) -> Result<Option<String>, RuntimeError> {
    if value != "true" && value != "false" {
        return Err(validation_error(
            format!("Invalid value '{value}' for first-run-hint-seen"),
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

/// Validate that `value` is a boolean literal (`true`/`false`), naming `key`
/// in the error. A generalisation of `validate_no_color` for any bool key.
fn validate_boolean(value: &str, key: &str) -> Result<Option<String>, RuntimeError> {
    if value != "true" && value != "false" {
        return Err(validation_error(
            format!("Invalid value '{value}' for {key}"),
            Some("Use 'true' or 'false'"),
        ));
    }

    Ok(None)
}
