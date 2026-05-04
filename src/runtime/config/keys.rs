//! Config key registry and value validation.

use crate::runtime::config::{ENV_BASE_URL, ENV_CLIENT_ID, ENV_NAMESPACE, ENV_PROFILE};

// ── Config key registry ──

/// Whether a config key belongs to global or profile scope
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigScope {
    Global,
    Profile,
}

/// Definition of a known config key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigKeyDef {
    /// CLI name (kebab-case, what users type)
    pub cli_name: &'static str,
    /// JSON field name (snake_case, stored in files)
    pub json_name: &'static str,
    /// Whether this key is global or profile-scoped
    pub scope: ConfigScope,
}

/// All recognised config keys.
pub static KNOWN_KEYS: &[ConfigKeyDef] = &[
    // Global keys
    ConfigKeyDef {
        cli_name: "active-profile",
        json_name: "active_profile",
        scope: ConfigScope::Global,
    },
    ConfigKeyDef {
        cli_name: "format",
        json_name: "format",
        scope: ConfigScope::Global,
    },
    ConfigKeyDef {
        cli_name: "no-color",
        json_name: "no_color",
        scope: ConfigScope::Global,
    },
    ConfigKeyDef {
        cli_name: "timeout",
        json_name: "timeout",
        scope: ConfigScope::Global,
    },
    ConfigKeyDef {
        cli_name: "page-limit",
        json_name: "page_limit",
        scope: ConfigScope::Global,
    },
    // Profile-scoped keys
    ConfigKeyDef {
        cli_name: "base-url",
        json_name: "base_url",
        scope: ConfigScope::Profile,
    },
    ConfigKeyDef {
        cli_name: "client-id",
        json_name: "client_id",
        scope: ConfigScope::Profile,
    },
    ConfigKeyDef {
        cli_name: "namespace",
        json_name: "namespace",
        scope: ConfigScope::Profile,
    },
    ConfigKeyDef {
        cli_name: "grant-type",
        json_name: "grant_type",
        scope: ConfigScope::Profile,
    },
];

/// Find a config key definition by CLI name.
pub fn find_key(cli_name: &str) -> Option<&'static ConfigKeyDef> {
    KNOWN_KEYS.iter().find(|k| k.cli_name == cli_name)
}

/// Return the environment variable that overrides a config key, when one exists.
pub fn env_var_name_for_key(cli_name: &str) -> Option<&'static str> {
    match cli_name {
        "base-url" => Some(ENV_BASE_URL),
        "client-id" => Some(ENV_CLIENT_ID),
        "namespace" => Some(ENV_NAMESPACE),
        "active-profile" => Some(ENV_PROFILE),
        _ => None,
    }
}

// ── Value validation predicates ──
// Used by both `ags config set` and `ags doctor` to enforce consistent rules.

/// Returns true if the value is a valid base URL (parseable, http/https scheme).
pub fn is_valid_base_url(value: &str) -> bool {
    url::Url::parse(value)
        .map(|url| url.scheme() == "https" || url.scheme() == "http")
        .unwrap_or(false)
}

/// Returns true if the value is a valid client ID (32-char hex, hyphens allowed).
pub fn is_valid_client_id(value: &str) -> bool {
    let stripped: String = value.chars().filter(|c| *c != '-').collect();
    stripped.len() == 32 && stripped.chars().all(|c| c.is_ascii_hexdigit())
}

/// Normalise a client ID: strip hyphens, lowercase.
pub fn normalise_client_id(value: &str) -> String {
    value
        .chars()
        .filter(|c| *c != '-')
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Returns true if the value is a valid namespace (lowercase alphanumeric and hyphens, max 48 chars).
pub fn is_valid_namespace(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 48
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Key lookup returns the registered definition.
    #[test]
    fn test_find_key_returns_registered_definition() {
        let key = find_key("base-url").unwrap();
        assert_eq!(key.json_name, "base_url");
        assert_eq!(key.scope, ConfigScope::Profile);
    }

    /// Environment-variable lookup only exists for keys with env overrides.
    #[test]
    fn test_env_var_name_for_key_returns_known_overrides() {
        assert_eq!(env_var_name_for_key("base-url"), Some(ENV_BASE_URL));
        assert_eq!(env_var_name_for_key("active-profile"), Some(ENV_PROFILE));
        assert_eq!(env_var_name_for_key("format"), None);
    }
}
