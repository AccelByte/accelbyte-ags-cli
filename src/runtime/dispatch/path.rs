//! Path-template substitution for dispatch requests.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::protocol::error::{RuntimeError, RuntimeErrorKind};
use crate::support::strings::encode_url_path_segment;

/// Regex used to locate `{name}` placeholders in operation path templates.
static PATH_PARAM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{(\w+)\}").unwrap());

/// Substitute `{name}` placeholders in a path template with sanitized values
/// from `params`. Returns a `Validation` error when a required placeholder is
/// missing. Shared by `execute_operation` and `Runtime::dry_run_command` so
/// both apply identical validation and encoding.
pub(crate) fn substitute_path_params(
    path_template: &str,
    params: &BTreeMap<String, String>,
) -> Result<String, RuntimeError> {
    let placeholders: Vec<String> = PATH_PARAM_RE
        .captures_iter(path_template)
        .map(|capture| capture[1].to_string())
        .collect();

    let mut path = path_template.to_string();
    for param_name in &placeholders {
        let value = params.get(param_name).ok_or_else(|| RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message: format!("Missing required parameter: --{param_name}"),
            details: None,
            hint: None,
            trace: None,
        })?;
        let encoded = encode_url_path_segment(value, param_name)?;
        path = path.replace(&format!("{{{param_name}}}"), &encoded);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All placeholders in a path template are replaced with encoded values.
    #[test]
    fn test_substitute_path_params_replaces_all_placeholders() {
        let mut params = BTreeMap::new();
        params.insert("namespace".to_string(), "demo".to_string());
        params.insert("userId".to_string(), "user/123".to_string());

        let path =
            substitute_path_params("/iam/namespaces/{namespace}/users/{userId}", &params).unwrap();

        assert_eq!(path, "/iam/namespaces/demo/users/user%2F123");
    }

    /// Missing required placeholders return a validation error that names the flag.
    #[test]
    fn test_substitute_path_params_missing_param_returns_error() {
        let params = BTreeMap::new();

        let error = substitute_path_params("/iam/users/{userId}", &params).unwrap_err();

        assert_eq!(error.kind, RuntimeErrorKind::Validation);
        assert_eq!(error.message, "Missing required parameter: --userId");
    }

    /// Templates with no placeholders pass through unchanged.
    #[test]
    fn test_substitute_path_params_without_placeholders_passthrough() {
        let params = BTreeMap::new();

        let path = substitute_path_params("/iam/users/me", &params).unwrap();

        assert_eq!(path, "/iam/users/me");
    }
}
