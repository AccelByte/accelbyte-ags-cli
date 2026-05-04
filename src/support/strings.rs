//! Shared string transforms for naming, sanitization, and display formatting.

use std::collections::HashSet;
use std::sync::LazyLock;

use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use regex::Regex;

use crate::protocol::error::{RuntimeError, RuntimeErrorKind};

/// Characters that must be percent-encoded in URL path segments.
const PATH_SEGMENT_ENCODE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

/// Regex matching ANSI CSI and OSC escape sequences.
static ANSI_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\x1b\[[0-9;]*[A-Za-z]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)").unwrap()
});

static KEBAB_CONNECTORS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([a-z]{3})(by|for|from|with)([A-Z])").unwrap());

static KEBAB_LOWER_UPPER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([a-z0-9])([A-Z])").unwrap());

static KEBAB_ACRONYM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Z]+)([A-Z][a-z])").unwrap());

/// Acronym substitutions applied before the regex passes so that mixed-case
/// runs like "userIDs" lower-case cleanly to "user-ids" instead of "user-i-ds".
const ACRONYM_REPLACEMENTS: &[(&str, &str)] = &[
    ("IDs", "Ids"),
    ("URLs", "Urls"),
    ("APIs", "Apis"),
    ("QoS", "Qos"),
    ("S2S", "S2s"),
];

// ── Identifier Transforms ──

/// Convert CamelCase or snake_case to kebab-case.
pub(crate) fn to_kebab_case(name: &str) -> String {
    let mut s = name.replace('_', "-");
    for (from, to) in ACRONYM_REPLACEMENTS {
        s = s.replace(from, to);
    }

    s = KEBAB_CONNECTORS_RE
        .replace_all(&s, "${1}-${2}-${3}")
        .to_string();
    s = KEBAB_LOWER_UPPER_RE
        .replace_all(&s, "${1}-${2}")
        .to_string();
    s = KEBAB_ACRONYM_RE.replace_all(&s, "${1}-${2}").to_string();

    s.to_lowercase()
}

/// Convert a kebab-case identifier to space-separated words.
pub(crate) fn kebab_case_to_words(name: &str) -> String {
    name.replace('-', " ")
}

/// Capitalize the first character of a string, leaving the rest unchanged.
pub(crate) fn capitalize_first(string: &str) -> String {
    let mut characters = string.chars();
    match characters.next() {
        None => String::new(),
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            upper + characters.as_str()
        }
    }
}

// ── Word Transforms ──

/// Convert a plural noun to singular form.
pub(crate) fn singularize(name: &str) -> String {
    if let Some(stem) = name.strip_suffix("ies") {
        format!("{stem}y")
    } else if let Some(stem) = name.strip_suffix("es").filter(|s| s.ends_with('s')) {
        stem.to_string()
    } else if let Some(stem) = name.strip_suffix('s').filter(|s| !s.ends_with('s')) {
        stem.to_string()
    } else {
        name.to_string()
    }
}

/// Return the plural form of a noun, or singularize if count is 1.
pub(crate) fn pluralize(noun: &str, count: usize) -> String {
    if count == 1 {
        return singularize(noun);
    }
    if noun.ends_with("sh")
        || noun.ends_with("ch")
        || noun.ends_with('x')
        || noun.ends_with('z')
        || noun.ends_with("ss")
        || noun.ends_with("us")
    {
        return format!("{noun}es");
    }
    if noun.ends_with('s') {
        return noun.to_string();
    }
    if let Some(stem) = noun.strip_suffix('y') {
        if let Some(c) = stem.chars().last() {
            if !"aeiou".contains(c) {
                return format!("{stem}ies");
            }
        }
    }
    format!("{noun}s")
}

/// Derive a display noun from a method name like "get-ban" -> "ban".
pub(crate) fn derive_noun_from_method(method_name: &str, resource_name: &str) -> String {
    if let Some(pos) = method_name.find('-') {
        let after = &method_name[pos + 1..];
        let first_word = after.split('-').next().unwrap_or("");
        if matches!(first_word, "by" | "for" | "with" | "from" | "of") {
            return kebab_case_to_words(resource_name);
        }
        if first_word == "my" {
            let rest = after.strip_prefix("my").unwrap_or(after);
            let rest = rest.strip_prefix('-').unwrap_or(rest);
            if rest.is_empty() {
                return kebab_case_to_words(resource_name);
            }
            return rest.replace('-', " ");
        }
        after.replace('-', " ")
    } else {
        kebab_case_to_words(resource_name)
    }
}

// ── Description And Display Helpers ──

/// Truncate summary text for list views.
pub(crate) fn truncate_summary(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_string();
    }
    format!("{}...", text[..max_len - 3].trim_end())
}

/// Draft a resource description from its method names.
pub(crate) fn draft_resource_description(resource: &str, method_names: &[&str]) -> String {
    let method_set: HashSet<&str> = method_names.iter().copied().collect();
    let resource_display = resource.replace('-', " ");

    let crud_verbs: HashSet<&str> = ["list", "create", "get", "update", "delete"]
        .into_iter()
        .collect();
    if method_set.intersection(&crud_verbs).count() >= 3 {
        return format!("Manage {resource_display}");
    }

    let read_only: HashSet<&str> = ["list", "get", "get-all", "list-all", "query"]
        .into_iter()
        .collect();
    if method_set.is_subset(&read_only) {
        return format!("Query {resource_display}");
    }

    if resource.ends_with("config") || resource.ends_with("configuration") {
        return format!("Configure {resource_display}");
    }

    let mut cap = resource_display.clone();
    if let Some(first) = cap.chars().next() {
        cap = format!(
            "{}{}",
            first.to_uppercase(),
            &resource_display[first.len_utf8()..]
        );
    }
    format!("{cap} operations")
}

/// Truncate display text to `max_len` bytes, appending "... (truncated)" if shortened.
pub fn truncate_display_text(body: &str, max_len: usize) -> String {
    if body.len() <= max_len {
        return body.to_string();
    }
    let mut boundary = max_len;
    while boundary > 0 && !body.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}... (truncated)", &body[..boundary])
}

// ── Sanitization And Encoding ──

/// Validate and percent-encode a user-supplied value for use in a URL path segment.
pub fn encode_url_path_segment(value: &str, param_name: &str) -> Result<String, RuntimeError> {
    if value.is_empty() {
        return Err(RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message: format!("Parameter '{param_name}' cannot be empty"),
            details: None,
            hint: Some(format!(
                "Provide a value for --{param_name}. Run with --help to see expected format."
            )),
            trace: None,
        });
    }
    if value.contains("..") {
        return Err(RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message: "Invalid parameter value: path traversal ('..') is not allowed.".to_string(),
            details: None,
            hint: None,
            trace: None,
        });
    }
    if value.contains('?') {
        return Err(RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message: "Invalid parameter value: '?' is not allowed.".to_string(),
            details: None,
            hint: None,
            trace: None,
        });
    }
    if value.contains('#') {
        return Err(RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message: "Invalid parameter value: '#' is not allowed.".to_string(),
            details: None,
            hint: None,
            trace: None,
        });
    }
    if value.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Err(RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message: "Invalid parameter value: control characters are not allowed.".to_string(),
            details: None,
            hint: None,
            trace: None,
        });
    }

    Ok(utf8_percent_encode(value, PATH_SEGMENT_ENCODE).to_string())
}

/// Strip terminal control sequences and control characters from a string before display.
pub fn strip_terminal_control_sequences(value: &str) -> String {
    let without_ansi = ANSI_RE.replace_all(value, "");
    without_ansi
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || (c >= ' ' && c != '\x7f'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CamelCase operation IDs become kebab-case CLI method names.
    #[test]
    fn test_kebab_simple_camel() {
        assert_eq!(to_kebab_case("GetUsers"), "get-users");
        assert_eq!(to_kebab_case("CreateUser"), "create-user");
    }

    /// Consecutive uppercase like "IDs" or "URLs" is normalised before splitting.
    #[test]
    fn test_kebab_consecutive_uppercase() {
        assert_eq!(to_kebab_case("GetUserIDs"), "get-user-ids");
        assert_eq!(to_kebab_case("ListURLs"), "list-urls");
    }

    /// Known acronyms are kept as single segments in kebab output.
    #[test]
    fn test_kebab_acronyms() {
        assert_eq!(to_kebab_case("QoSRegions"), "qos-regions");
        assert_eq!(to_kebab_case("S2SToken"), "s2s-token");
    }

    /// Lowercase connector words get their own kebab segment.
    #[test]
    fn test_kebab_connector_words() {
        assert_eq!(to_kebab_case("getUserbyId"), "get-user-by-id");
        assert_eq!(to_kebab_case("listItemsforUser"), "list-items-for-user");
    }

    /// Digit-to-uppercase transitions insert a hyphen.
    #[test]
    fn test_kebab_digits() {
        assert_eq!(to_kebab_case("AdminV3"), "admin-v3");
    }

    /// Already-lowercase input passes through unchanged.
    #[test]
    fn test_kebab_already_lowercase() {
        assert_eq!(to_kebab_case("list"), "list");
    }

    /// snake_case names are converted to kebab-case.
    #[test]
    fn test_to_kebab_case_handles_snake_case() {
        assert_eq!(to_kebab_case("user_id"), "user-id");
        assert_eq!(to_kebab_case("client_id"), "client-id");
        assert_eq!(to_kebab_case("response_type"), "response-type");
    }

    /// Common English plurals reduce to singular.
    #[test]
    fn test_singularize() {
        assert_eq!(singularize("clients"), "client");
        assert_eq!(singularize("policies"), "policy");
        assert_eq!(singularize("statuses"), "status");
        assert_eq!(singularize("users"), "user");
        assert_eq!(singularize("access"), "access");
        assert_eq!(singularize("config"), "config");
    }

    /// Count-aware pluralization returns singular for 1, plural otherwise.
    #[test]
    fn test_pluralize() {
        assert_eq!(pluralize("user", 0), "users");
        assert_eq!(pluralize("user", 1), "user");
        assert_eq!(pluralize("user", 2), "users");
        assert_eq!(pluralize("users", 3), "users");
        assert_eq!(pluralize("policy", 1), "policy");
        assert_eq!(pluralize("country", 2), "countries");
        assert_eq!(pluralize("policy", 2), "policies");
        assert_eq!(pluralize("key", 2), "keys");
        assert_eq!(pluralize("match", 2), "matches");
        assert_eq!(pluralize("status", 2), "statuses");
        assert_eq!(pluralize("batch", 2), "batches");
        assert_eq!(pluralize("box", 2), "boxes");
    }

    /// Kebab-case resource names are rendered with spaces when falling back to the resource name.
    #[test]
    fn test_derive_noun_from_method_strips_kebab_in_resource_fallback() {
        assert_eq!(
            derive_noun_from_method("list", "stat-config"),
            "stat config"
        );
        assert_eq!(
            derive_noun_from_method("get", "global-achievement"),
            "global achievement"
        );
        assert_eq!(
            derive_noun_from_method("list-by-user", "game-record"),
            "game record"
        );
    }

    /// The "-my" marker on /me endpoints is stripped.
    #[test]
    fn test_derive_noun_from_method_strips_my_prefix() {
        assert_eq!(
            derive_noun_from_method("get-my-info", "user-profile"),
            "info"
        );
        assert_eq!(
            derive_noun_from_method("update-my-zip-code", "user-profile"),
            "zip code"
        );
        assert_eq!(
            derive_noun_from_method("list-my-joined", "member"),
            "joined"
        );
        assert_eq!(
            derive_noun_from_method("get-my-rewards", "player-reward"),
            "rewards"
        );
    }

    /// Verb-only "-my" methods fall back to the resource name.
    #[test]
    fn test_derive_noun_from_method_verb_plus_my_falls_back_to_resource() {
        assert_eq!(
            derive_noun_from_method("subscribe-my", "notification-subscription"),
            "notification subscription"
        );
        assert_eq!(
            derive_noun_from_method("create-my", "user-profile"),
            "user profile"
        );
        assert_eq!(
            derive_noun_from_method("update-my", "user-profile"),
            "user profile"
        );
    }

    /// Short text within the limit passes through without truncation.
    #[test]
    fn test_truncate_summary_short() {
        assert_eq!(truncate_summary("hello", 10), "hello");
    }

    /// Text exceeding the limit is cut and suffixed with "...".
    #[test]
    fn test_truncate_summary_long() {
        assert_eq!(
            truncate_summary("this is a very long summary", 15),
            "this is a ve..."
        );
    }

    /// Resources with 3+ CRUD verbs get a "Manage" prefix.
    #[test]
    fn test_auto_draft_crud_resource() {
        assert_eq!(
            draft_resource_description("users", &["list", "create", "get", "update", "delete"]),
            "Manage users"
        );
    }

    /// Read-only resources get a "Query" prefix.
    #[test]
    fn test_auto_draft_read_only_resource() {
        assert_eq!(
            draft_resource_description("users", &["list", "get"]),
            "Query users"
        );
    }

    /// Resources ending in "config" get a "Configure" prefix.
    #[test]
    fn test_auto_draft_config_resource() {
        assert_eq!(
            draft_resource_description("session-config", &["get", "update"]),
            "Configure session config"
        );
    }

    /// Empty values are rejected with a message naming the parameter.
    #[test]
    fn test_rejects_empty_value() {
        let err = encode_url_path_segment("", "userId").unwrap_err();
        assert!(err.message.contains("userId"));
    }

    /// Alphanumeric values pass through without encoding.
    #[test]
    fn test_normal_value_unchanged() {
        assert_eq!(encode_url_path_segment("abc123", "id").unwrap(), "abc123");
    }

    /// UUIDs with hyphens are preserved.
    #[test]
    fn test_uuid_with_hyphens_preserved() {
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(encode_url_path_segment(uuid, "id").unwrap(), uuid);
    }

    /// Underscores and dots are unreserved and pass through.
    #[test]
    fn test_underscores_and_dots_preserved() {
        assert_eq!(
            encode_url_path_segment("foo_bar.baz", "id").unwrap(),
            "foo_bar.baz"
        );
    }

    /// Forward slashes are percent-encoded.
    #[test]
    fn test_slashes_encoded() {
        let result = encode_url_path_segment("foo/bar", "id").unwrap();
        assert_eq!(result, "foo%2Fbar");
    }

    /// Spaces are percent-encoded.
    #[test]
    fn test_spaces_encoded() {
        let result = encode_url_path_segment("hello world", "id").unwrap();
        assert!(result.contains("%20"));
    }

    /// Path traversal sequences are rejected.
    #[test]
    fn test_rejects_path_traversal() {
        assert!(encode_url_path_segment("../../admin", "id").is_err());
        assert!(encode_url_path_segment("foo/../bar", "id").is_err());
    }

    /// A bare ".." is also rejected.
    #[test]
    fn test_rejects_single_dot_dot() {
        assert!(encode_url_path_segment("..", "id").is_err());
    }

    /// Question marks are rejected.
    #[test]
    fn test_rejects_question_mark() {
        assert!(encode_url_path_segment("foo?x=1", "id").is_err());
    }

    /// Hash characters are rejected.
    #[test]
    fn test_rejects_hash() {
        assert!(encode_url_path_segment("foo#bar", "id").is_err());
    }

    /// Null bytes are rejected.
    #[test]
    fn test_rejects_null_byte() {
        assert!(encode_url_path_segment("foo\x00bar", "id").is_err());
    }

    /// Control characters are rejected.
    #[test]
    fn test_rejects_control_chars() {
        assert!(encode_url_path_segment("foo\x01bar", "id").is_err());
        assert!(encode_url_path_segment("foo\x1fbar", "id").is_err());
    }

    /// DEL is rejected alongside other control characters.
    #[test]
    fn test_rejects_del() {
        assert!(encode_url_path_segment("foo\x7fbar", "id").is_err());
    }

    /// Printable ASCII text passes through unchanged.
    #[test]
    fn test_normal_text_unchanged() {
        let input = "Hello, world! 123 @#$%";
        assert_eq!(strip_terminal_control_sequences(input), input);
    }

    /// ANSI color escape sequences are stripped.
    #[test]
    fn test_strips_ansi_color_codes() {
        assert_eq!(
            strip_terminal_control_sequences("\x1b[31mred\x1b[0m"),
            "red"
        );
    }

    /// ANSI cursor movement sequences are stripped.
    #[test]
    fn test_strips_ansi_cursor_movement() {
        assert_eq!(strip_terminal_control_sequences("\x1b[2Aup two"), "up two");
    }

    /// OSC title-setting sequences are stripped.
    #[test]
    fn test_strips_osc_sequences() {
        assert_eq!(
            strip_terminal_control_sequences("\x1b]0;terminal title\x07hello"),
            "hello"
        );
    }

    /// Tabs and newlines are preserved while other control chars are removed.
    #[test]
    fn test_preserves_tabs_and_newlines() {
        assert_eq!(strip_terminal_control_sequences("a\tb\nc\r"), "a\tb\nc");
    }

    /// Display length capping respects UTF-8 boundaries.
    #[test]
    fn test_truncate_display_text_respects_utf8_boundaries() {
        assert_eq!(truncate_display_text("hello", 10), "hello");
        assert_eq!(
            truncate_display_text("hello world this is long", 10),
            "hello worl... (truncated)"
        );
        assert_eq!(
            truncate_display_text("cafe\u{301}", 4),
            "cafe... (truncated)"
        );
    }

    /// Empty strings must return empty without panicking.
    #[test]
    fn test_capitalize_first_empty() {
        assert_eq!(capitalize_first(""), "");
    }

    /// Lowercase first character must be uppercased while preserving the rest.
    #[test]
    fn test_capitalize_first_lowercase() {
        assert_eq!(capitalize_first("hello"), "Hello");
    }
}
