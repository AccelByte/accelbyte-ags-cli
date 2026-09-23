//! Parse and validate "RESOURCE [ACTION]" permission override strings, using
//! the same format the Admin Portal's request form accepts.

use std::sync::OnceLock;

use regex::Regex;

/// A validated `resource`/`action` pair to submit as a permission override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedPermission {
    pub(crate) resource: String,
    pub(crate) action: String,
}

fn permission_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^([A-Za-z0-9:*_{}-]+) \[((?:CREATE|READ|UPDATE|DELETE)(?:\|(?:CREATE|READ|UPDATE|DELETE))*)\]$",
        )
        .expect("permission regex is a fixed, hand-verified pattern")
    })
}

pub(crate) fn parse_permission(input: &str, namespace: &str) -> Result<ParsedPermission, String> {
    let captures = permission_regex()
        .captures(input.trim())
        .ok_or_else(|| format_validation_error(namespace))?;
    Ok(ParsedPermission {
        resource: captures[1].to_string(),
        action: captures[2].to_string(),
    })
}

pub(crate) fn format_validation_error(namespace: &str) -> String {
    format!(
        "Must match the format RESOURCE [ACTION] — e.g. ADMIN:NAMESPACE:{namespace}:SEASON \
         [UPDATE]. Supported actions: CREATE, READ, UPDATE, DELETE."
    )
}

pub(crate) fn manual_permission_hint(namespace: &str) -> String {
    format!("e.g. ADMIN:NAMESPACE:{namespace}:SEASON [UPDATE]")
}

pub(crate) fn display_permission(resource: &str, action: &str) -> String {
    format!("{resource} [{action}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_single_action() {
        let parsed = parse_permission("NAMESPACE:ns1:USER [READ]", "ns1").unwrap();
        assert_eq!(parsed.resource, "NAMESPACE:ns1:USER");
        assert_eq!(parsed.action, "READ");
    }

    #[test]
    fn valid_multi_action_joined_by_bar() {
        let parsed = parse_permission("NAMESPACE:ns1:USER [CREATE|READ]", "ns1").unwrap();
        assert_eq!(parsed.resource, "NAMESPACE:ns1:USER");
        assert_eq!(parsed.action, "CREATE|READ");
    }

    #[test]
    fn valid_wildcard_resource_segment() {
        let parsed = parse_permission("ADMIN:NAMESPACE:*:SEASON [UPDATE]", "ns1").unwrap();
        assert_eq!(parsed.resource, "ADMIN:NAMESPACE:*:SEASON");
        assert_eq!(parsed.action, "UPDATE");
    }

    #[test]
    fn valid_templated_resource_segment() {
        let parsed =
            parse_permission("ADMIN:NAMESPACE:{namespace}:SEASON [UPDATE]", "ns1").unwrap();
        assert_eq!(parsed.resource, "ADMIN:NAMESPACE:{namespace}:SEASON");
    }

    #[test]
    fn invalid_missing_brackets_is_rejected() {
        let err = parse_permission("NAMESPACE:ns1:USER READ", "ns1").unwrap_err();
        assert_eq!(err, format_validation_error("ns1"));
    }

    #[test]
    fn invalid_lowercase_action_is_rejected() {
        assert!(parse_permission("NAMESPACE:ns1:USER [read]", "ns1").is_err());
    }

    #[test]
    fn invalid_unknown_action_is_rejected() {
        assert!(parse_permission("NAMESPACE:ns1:USER [EXECUTE]", "ns1").is_err());
    }

    #[test]
    fn invalid_missing_space_before_bracket_is_rejected() {
        assert!(parse_permission("NAMESPACE:ns1:USER[READ]", "ns1").is_err());
    }

    #[test]
    fn invalid_empty_string_is_rejected() {
        assert!(parse_permission("", "ns1").is_err());
    }

    #[test]
    fn error_message_substitutes_namespace() {
        let err = parse_permission("bad", "my-namespace").unwrap_err();
        assert!(err.contains("ADMIN:NAMESPACE:my-namespace:SEASON [UPDATE]"));
    }

    #[test]
    fn display_permission_matches_input_format() {
        assert_eq!(
            display_permission("NAMESPACE:ns1:USER", "READ"),
            "NAMESPACE:ns1:USER [READ]"
        );
    }
}
