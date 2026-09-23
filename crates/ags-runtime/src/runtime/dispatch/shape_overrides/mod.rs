//! Per-operation output overrides: hand-picked column sets for specific
//! operations whose responses the generic field-priority shaper
//! (`shape.rs::prioritize_fields`) can't render well — e.g. it prioritizes
//! the wrong fields, caps non-verbose lists at 4 columns, or an array sits
//! next to sibling scalars that defeats `detect_shape`'s single-array
//! auto-unwrap heuristic.
//!
//! `shape_response` calls `find` once. To add a new override: write a
//! sibling module exposing `OPERATION_ID` and `shape` (see
//! `security_assessment_list` for the pattern), then add one row to
//! `OVERRIDES` below — `shape.rs` itself never needs to change again.
//!
//! `find` sanitizes every `CommandResult` before returning it, stripping
//! terminal control sequences from every display string. Override authors
//! do not need to strip individually — the single exit handles it.

mod security_assessment_endpoints;
mod security_assessment_list;

use ags_protocol::result::{
    CollectionResult, ColumnSpec, CommandResult, EntityResult, FieldValue, Row,
};
use serde_json::{Map, Value};

/// A single table column: its label, and how to pull the cell value out of
/// one item's JSON object.
type Column = (&'static str, fn(&Map<String, Value>) -> FieldValue);

/// One registered override: which operation it applies to, and how to shape
/// that operation's response body.
struct Override {
    operation_id: &'static str,
    shape: fn(&Value) -> Option<CommandResult>,
}

/// Every registered override. Add a row here for each new one.
const OVERRIDES: &[Override] = &[
    Override {
        operation_id: security_assessment_list::OPERATION_ID,
        shape: security_assessment_list::shape,
    },
    Override {
        operation_id: security_assessment_endpoints::OPERATION_ID,
        shape: security_assessment_endpoints::shape,
    },
];

/// Look up and run the override for `operation_id`, if one is registered.
/// `None` means either no override is registered, or the one that matched
/// didn't recognize the body's shape — either way the caller falls back to
/// the generic shaper rather than losing data.
///
/// Every result is passed through [`sanitize_result`] before returning, so
/// individual override modules do not need to strip terminal control
/// sequences from the values they build.
pub(crate) fn find(operation_id: &str, body: &Value) -> Option<CommandResult> {
    OVERRIDES
        .iter()
        .find(|o| o.operation_id == operation_id)
        .and_then(|o| (o.shape)(body))
        .map(sanitize_result)
}

/// Build a `CollectionResult` from a named array in `body`, mapping each
/// item through a fixed, ordered `(label, extractor)` column list. Returns
/// `None` if `array_key` is absent (shape mismatch → fall back to the
/// generic shaper); a JSON `null` at that key is treated as zero rows, not
/// a mismatch, since AccelByte services use `null` to mean "empty" for some
/// array fields.
fn fixed_columns_from_array(
    body: &Value,
    array_key: &str,
    kind: &str,
    columns: &[Column],
) -> Option<CommandResult> {
    let items: &[Value] = match body.as_object()?.get(array_key)? {
        Value::Array(items) => items,
        Value::Null => &[],
        _ => return None,
    };

    let column_specs = columns
        .iter()
        .map(|(label, _)| ColumnSpec {
            label: label.to_string(),
            key: label.to_string(),
        })
        .collect();

    let rows = items
        .iter()
        .filter_map(Value::as_object)
        .map(|item| Row {
            cells: columns.iter().map(|(_, extract)| extract(item)).collect(),
        })
        .collect();

    Some(CommandResult::Collection(CollectionResult {
        kind: kind.to_string(),
        columns: column_specs,
        rows,
        page_info: None,
        notes: vec![],
    }))
}

const MISSING: &str = "—";

/// Render a scalar JSON field (string or number) as display text, `—` if
/// missing, null, or empty. Shared by every override's column extractors.
/// Strings are stripped of terminal control sequences, matching
/// `shape.rs::normalize_value`.
fn text_field(value: Option<&Value>) -> FieldValue {
    let text = match value {
        Some(Value::String(s)) if !s.is_empty() => {
            crate::support::strings::strip_terminal_control_sequences(s)
        }
        Some(Value::Number(n)) => n.to_string(),
        _ => return FieldValue::Text(MISSING.to_string()),
    };
    FieldValue::Text(text)
}

/// Strip terminal control sequences from every display string inside a
/// `CommandResult`. Called on the single exit path of [`find`] so that no
/// override can return unstripped data to the caller.
fn sanitize_result(result: CommandResult) -> CommandResult {
    use crate::support::strings::strip_terminal_control_sequences;

    fn strip_field_value(fv: FieldValue) -> FieldValue {
        match fv {
            FieldValue::Text(s) => FieldValue::Text(strip_terminal_control_sequences(&s)),
            FieldValue::List(items) => FieldValue::List(
                items
                    .into_iter()
                    .map(|s| strip_terminal_control_sequences(&s))
                    .collect(),
            ),
            other => other,
        }
    }

    fn strip_field(f: ags_protocol::result::Field) -> ags_protocol::result::Field {
        ags_protocol::result::Field {
            label: strip_terminal_control_sequences(&f.label),
            value: strip_field_value(f.value),
        }
    }

    fn strip_field_group(g: ags_protocol::result::FieldGroup) -> ags_protocol::result::FieldGroup {
        ags_protocol::result::FieldGroup {
            heading: strip_terminal_control_sequences(&g.heading),
            fields: g.fields.into_iter().map(strip_field).collect(),
        }
    }

    match result {
        CommandResult::Collection(c) => CommandResult::Collection(CollectionResult {
            kind: strip_terminal_control_sequences(&c.kind),
            columns: c
                .columns
                .into_iter()
                .map(|col| ColumnSpec {
                    label: strip_terminal_control_sequences(&col.label),
                    key: strip_terminal_control_sequences(&col.key),
                })
                .collect(),
            rows: c
                .rows
                .into_iter()
                .map(|row| Row {
                    cells: row.cells.into_iter().map(strip_field_value).collect(),
                })
                .collect(),
            page_info: c.page_info,
            notes: c
                .notes
                .into_iter()
                .map(|n| strip_terminal_control_sequences(&n))
                .collect(),
        }),
        CommandResult::Entity(e) => CommandResult::Entity(EntityResult {
            kind: strip_terminal_control_sequences(&e.kind),
            identifier: e.identifier.map(|id| strip_terminal_control_sequences(&id)),
            heading_style: e.heading_style,
            fields: e.fields.into_iter().map(strip_field).collect(),
            sections: e.sections.into_iter().map(strip_field_group).collect(),
        }),
        // EmptyResult: `status` is stripped so the sanitization guarantee
        // still holds if a renderer ever prints it. `operation_id` is left
        // as-is because it is a catalogue identifier, not response data,
        // and no renderer reads it.
        CommandResult::Empty(empty) => CommandResult::Empty(ags_protocol::result::EmptyResult {
            operation_id: empty.operation_id,
            status: strip_terminal_control_sequences(&empty.status),
        }),
        // RawResult holds a serde_json::Value rendered as pretty-printed
        // JSON. The JSON serializer escapes control characters already, so
        // no string-level strip is needed.
        CommandResult::Raw(_) => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_columns_and_rows_from_named_array() {
        let body = json!({ "items": [{"a": "x", "b": 1}, {"a": "y"}] });
        let columns: &[Column] = &[
            ("A", |item| text_field(item.get("a"))),
            ("B", |item| text_field(item.get("b"))),
        ];
        let CommandResult::Collection(collection) =
            fixed_columns_from_array(&body, "items", "things", columns).expect("shape matches")
        else {
            panic!("expected Collection");
        };
        assert_eq!(collection.kind, "things");
        assert_eq!(
            collection.rows[0].cells,
            vec![
                FieldValue::Text("x".to_string()),
                FieldValue::Text("1".to_string())
            ]
        );
        assert_eq!(
            collection.rows[1].cells,
            vec![
                FieldValue::Text("y".to_string()),
                FieldValue::Text(MISSING.to_string())
            ]
        );
    }

    #[test]
    fn null_array_key_is_empty_not_a_mismatch() {
        let body = json!({ "items": null });
        let result = fixed_columns_from_array(&body, "items", "things", &[]);
        let CommandResult::Collection(collection) = result.expect("null is a match") else {
            panic!("expected Collection");
        };
        assert!(collection.rows.is_empty());
    }

    #[test]
    fn missing_array_key_falls_back_to_generic_shaper() {
        assert!(fixed_columns_from_array(&json!({}), "items", "things", &[]).is_none());
    }

    #[test]
    fn text_field_strips_terminal_control_sequences() {
        assert_eq!(
            text_field(Some(&json!("\x1b[31mred\x1b[0m"))),
            FieldValue::Text("red".to_string())
        );
        assert_eq!(
            text_field(Some(&json!("\x1b]0;PWNED\x07path"))),
            FieldValue::Text("path".to_string())
        );
    }

    /// `sanitize_result` strips terminal control sequences from every
    /// display-string position in a CommandResult. Covers positions that
    /// no current override reaches (Entity fields, sections, identifiers,
    /// List items, notes).
    #[test]
    fn sanitize_result_strips_every_position() {
        use ags_protocol::result::{EntityResult, Field, FieldGroup, HeadingStyle, PageInfo};

        let esc = "\x1b]0;PWNED\x07";

        // Collection: cells (Text + List), notes, kind
        let collection = CommandResult::Collection(CollectionResult {
            kind: format!("{esc}things"),
            columns: vec![ColumnSpec {
                label: format!("{esc}Col"),
                key: format!("{esc}col"),
            }],
            rows: vec![Row {
                cells: vec![
                    FieldValue::Text(format!("{esc}hello")),
                    FieldValue::List(vec![format!("{esc}a"), format!("{esc}b")]),
                    FieldValue::Number(42.0),
                    FieldValue::Bool(true),
                    FieldValue::Null,
                ],
            }],
            page_info: Some(PageInfo {
                current_page: 1,
                total_pages: None,
                total_items: None,
                has_next: false,
            }),
            notes: vec![format!("{esc}note1")],
        });

        let CommandResult::Collection(c) = sanitize_result(collection) else {
            panic!("expected Collection");
        };
        assert_eq!(c.kind, "things");
        assert_eq!(c.columns[0].label, "Col");
        assert_eq!(c.columns[0].key, "col");
        assert_eq!(c.rows[0].cells[0], FieldValue::Text("hello".to_string()));
        assert_eq!(
            c.rows[0].cells[1],
            FieldValue::List(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(c.rows[0].cells[2], FieldValue::Number(42.0));
        assert_eq!(c.rows[0].cells[3], FieldValue::Bool(true));
        assert_eq!(c.rows[0].cells[4], FieldValue::Null);
        assert_eq!(c.notes, vec!["note1".to_string()]);

        // Entity: identifier, fields, section fields
        let entity = CommandResult::Entity(EntityResult {
            kind: format!("{esc}User"),
            identifier: Some(format!("{esc}id-123")),
            heading_style: HeadingStyle::Identified,
            fields: vec![Field {
                label: "Name".to_string(),
                value: FieldValue::Text(format!("{esc}Alice")),
            }],
            sections: vec![FieldGroup {
                heading: format!("{esc}Contact"),
                fields: vec![Field {
                    label: "Phone".to_string(),
                    value: FieldValue::Text(format!("{esc}555")),
                }],
            }],
        });

        let CommandResult::Entity(e) = sanitize_result(entity) else {
            panic!("expected Entity");
        };
        assert_eq!(e.kind, "User");
        assert_eq!(e.identifier.as_deref(), Some("id-123"));
        assert_eq!(e.fields[0].value, FieldValue::Text("Alice".to_string()));
        assert_eq!(e.sections[0].heading, "Contact");
        assert_eq!(
            e.sections[0].fields[0].value,
            FieldValue::Text("555".to_string())
        );
    }
}
