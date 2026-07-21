//! JSONPath subset for workflow transforms and capture paths.

/// Apply the v2 JSONPath subset to a value.
///
/// Supported expressions:
/// - `$`               — return the value as-is
/// - `$.field`         — descend one object level
/// - `$.field.nested`  — descend repeatedly
/// - `$[N]`            — index into an array at position N
/// - `$.field[N]`      — combine the two; arbitrary chaining is supported
///
/// Returns `None` for missing fields, out-of-range indices, type
/// mismatches, or unsupported expressions (wildcards, filters, etc.).
pub fn apply_jsonpath_subset(
    value: &serde_json::Value,
    expression: &str,
) -> Option<serde_json::Value> {
    if expression == "$" {
        return Some(value.clone());
    }
    let mut remaining = expression.strip_prefix('$')?;
    let mut cursor = value;
    while !remaining.is_empty() {
        if let Some(after_dot) = remaining.strip_prefix('.') {
            let end = after_dot.find(['.', '[']).unwrap_or(after_dot.len());
            let (name, rest_after) = after_dot.split_at(end);
            if name.is_empty() {
                return None;
            }
            cursor = cursor.get(name)?;
            remaining = rest_after;
        } else {
            let after_open = remaining.strip_prefix('[')?;
            let close = after_open.find(']')?;
            let index_str = &after_open[..close];
            let index: usize = index_str.parse().ok()?;
            cursor = cursor.get(index)?;
            remaining = &after_open[close + 1..];
        }
    }
    Some(cursor.clone())
}

/// Whether `expression` is a syntactically valid expression in the supported
/// JSONPath subset (`$`, `$.field`, `$.a.b`, `$[N]`, `$.a[N].b`). Rejects
/// wildcards, filters, recursive descent, and malformed paths. Used by
/// compile-time `options_source` validation so an unparseable projection path
/// fails at author time rather than silently returning `None` at runtime.
pub fn jsonpath_is_valid(expression: &str) -> bool {
    if expression == "$" {
        return true;
    }
    let Some(mut remaining) = expression.strip_prefix('$') else {
        return false;
    };
    if remaining.is_empty() {
        return false;
    }
    while !remaining.is_empty() {
        if let Some(after_dot) = remaining.strip_prefix('.') {
            let end = after_dot.find(['.', '[']).unwrap_or(after_dot.len());
            let (name, rest) = after_dot.split_at(end);
            // A name segment must be a plain identifier-ish token: non-empty and
            // free of wildcard/filter characters.
            if name.is_empty() || name.contains(['*', '?', '@', '$']) {
                return false;
            }
            remaining = rest;
        } else if let Some(after_open) = remaining.strip_prefix('[') {
            let Some(close) = after_open.find(']') else {
                return false;
            };
            let index_str = &after_open[..close];
            if index_str.parse::<usize>().is_err() {
                return false;
            }
            remaining = &after_open[close + 1..];
        } else {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_dollar_root_returns_value() {
        let value = json!({"foo": 1});
        assert_eq!(apply_jsonpath_subset(&value, "$"), Some(value.clone()));
    }

    #[test]
    fn test_dot_field_descends_one_level() {
        let value = json!({"foo": "bar"});
        assert_eq!(apply_jsonpath_subset(&value, "$.foo"), Some(json!("bar")));
    }

    #[test]
    fn test_nested_dot_path() {
        let value = json!({"user": {"id": 42}});
        assert_eq!(apply_jsonpath_subset(&value, "$.user.id"), Some(json!(42)));
    }

    #[test]
    fn test_array_index() {
        let value = json!({"items": [10, 20, 30]});
        assert_eq!(apply_jsonpath_subset(&value, "$.items[1]"), Some(json!(20)));
    }

    #[test]
    fn test_chained_index_and_field() {
        let value = json!({"items": [{"id": "a"}, {"id": "b"}]});
        assert_eq!(
            apply_jsonpath_subset(&value, "$.items[1].id"),
            Some(json!("b"))
        );
    }

    #[test]
    fn test_missing_field_returns_none() {
        let value = json!({"foo": 1});
        assert_eq!(apply_jsonpath_subset(&value, "$.bar"), None);
    }

    #[test]
    fn test_out_of_range_index_returns_none() {
        let value = json!({"items": [10]});
        assert_eq!(apply_jsonpath_subset(&value, "$.items[5]"), None);
    }

    #[test]
    fn test_missing_leading_dollar_returns_none() {
        let value = json!({"foo": 1});
        assert_eq!(apply_jsonpath_subset(&value, "foo.bar"), None);
    }

    #[test]
    fn test_jsonpath_is_valid_accepts_supported_subset() {
        for expr in ["$", "$.images", "$.user.id", "$.items[1]", "$.items[1].id"] {
            assert!(jsonpath_is_valid(expr), "should accept {expr}");
        }
    }

    #[test]
    fn test_jsonpath_is_valid_rejects_unsupported() {
        for expr in [
            "images",
            "$.items[*]",
            "$..id",
            "$.items[?(@.x)]",
            "$.",
            "$.items[]",
        ] {
            assert!(!jsonpath_is_valid(expr), "should reject {expr}");
        }
    }
}
