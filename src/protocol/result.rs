//! Shaped service results and execution previews.
//!
//! This module defines the serializable payloads produced by service execution
//! and dry-run/preview flows before they are wrapped in a top-level
//! [`crate::protocol::output::CommandOutput`].
//!
//! In the protocol split:
//! - `result` owns service-shaped payloads such as `CommandResult`,
//!   `DryRunResult`, and `CommandPreview`
//! - `output` owns command-level envelopes and non-service command outputs
//!
//! Keeping that split lets frontends consume a stable payload taxonomy for API
//! results while still handling auth/config/profile/version-style commands
//! through a single top-level output enum.

use crate::protocol::catalogue::{HttpMethod, MutationClass};
use serde::{Deserialize, Serialize};

/// A preview of what a command will do, before it actually runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandPreview {
    pub service: crate::catalogue::ServiceId,
    pub operation_id: crate::protocol::catalogue::OperationId,
    /// One-line "what this will do" message.
    pub summary: String,
    pub http_method: HttpMethod,
    /// Fully resolved request URL with all `{placeholders}` filled in.
    pub url: String,
    pub mutation_class: MutationClass,
    pub confirmation_required: bool,
    pub warnings: Vec<String>,
}

/// What would be sent over the wire if the command ran, without running it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DryRunResult {
    pub http_method: HttpMethod,
    pub url: String,
    /// Auth header is masked or elided; never emitted in cleartext.
    pub headers: Vec<(String, String)>,
    pub query: Vec<(String, String)>,
    pub body: Option<serde_json::Value>,
}

/// The top-level return value from a successful command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CommandResult {
    Entity(EntityResult),
    Collection(CollectionResult),
    Empty(EmptyResult),
    Raw(RawResult),
}

/// A single-entity result (e.g. "get user by id") rendered as a label/value list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityResult {
    /// Human label for the entity, e.g. "User".
    pub kind: String,
    /// Optional stable identifier (e.g. user ID) shown in the headline.
    pub identifier: Option<String>,
    /// How the top line should be rendered.
    pub heading_style: HeadingStyle,
    pub fields: Vec<Field>,
    /// Nested sub-groups rendered after the top-level fields.
    pub sections: Vec<FieldGroup>,
}

/// How the top line of an `EntityResult` should be rendered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HeadingStyle {
    /// Renders as "<kind>: <identifier>" — identifier is a human-friendly name.
    Named,
    /// Renders as "<kind> (<identifier>)" — identifier is an opaque ID.
    Identified,
    /// Renders as just "<kind>" — identifier is None.
    Bare,
}

/// A named group of scalar fields, rendered as an indented section under an
/// `EntityResult`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldGroup {
    pub heading: String,
    pub fields: Vec<Field>,
}

/// A collection result (e.g. "list users") rendered as a table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CollectionResult {
    /// Plural human label, e.g. "Users".
    pub kind: String,
    pub columns: Vec<ColumnSpec>,
    pub rows: Vec<Row>,
    pub page_info: Option<PageInfo>,
}

/// A successful-but-empty result (e.g. "delete user" succeeded).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmptyResult {
    pub operation_id: crate::protocol::catalogue::OperationId,
    pub status: String,
}

/// A result that could not be shaped into any of the taxonomy above.
/// Falls through to a JSON pretty-print renderer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawResult {
    pub value: serde_json::Value,
}

/// Label + typed value, used inside `EntityResult`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub label: String,
    pub value: FieldValue,
}

/// A concrete cell value — what a renderer actually formats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
#[non_exhaustive]
pub enum FieldValue {
    Text(String),
    Number(f64),
    Bool(bool),
    List(Vec<String>),
    Null,
}

/// Column header metadata for a collection result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnSpec {
    /// Display header, e.g. "Email".
    pub label: String,
    /// Stable key referenced by row cells, e.g. "email".
    pub key: String,
}

/// One row of a collection, with cells aligned to the column list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub cells: Vec<FieldValue>,
}

/// Pagination state for a collection result, if known.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageInfo {
    pub current_page: usize,
    pub total_pages: Option<usize>,
    pub total_items: Option<usize>,
    pub has_next: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialise `value` to JSON, parse it back, and assert equality — the contract test for protocol types.
    fn round_trip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let parsed: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, &parsed);
    }

    #[test]
    fn test_command_preview_round_trip() {
        round_trip(&CommandPreview {
            service: crate::catalogue::Catalogue::find_id("iam").expect("iam in manifest"),
            operation_id: crate::protocol::catalogue::OperationId::new("AdminDeleteUserV3"),
            summary: "Delete user abc123".to_string(),
            http_method: HttpMethod::Delete,
            url: "https://demo.accelbyte.io/iam/v3/admin/namespaces/accelbyte/users/abc123"
                .to_string(),
            mutation_class: MutationClass::Mutating,
            confirmation_required: true,
            warnings: vec!["This cannot be undone.".to_string()],
        });
    }

    #[test]
    fn test_dry_run_result_round_trip() {
        round_trip(&DryRunResult {
            http_method: HttpMethod::Post,
            url: "https://demo.accelbyte.io/iam/v4/namespaces/accelbyte/users".to_string(),
            headers: vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("Authorization".to_string(), "Bearer <redacted>".to_string()),
            ],
            query: vec![],
            body: Some(serde_json::json!({"emailAddress": "a@b.com"})),
        });
    }

    #[test]
    fn test_command_result_entity_round_trip() {
        round_trip(&CommandResult::Entity(EntityResult {
            kind: "User".to_string(),
            identifier: Some("abc123".to_string()),
            heading_style: HeadingStyle::Named,
            fields: vec![
                Field {
                    label: "Email".to_string(),
                    value: FieldValue::Text("alice@example.com".to_string()),
                },
                Field {
                    label: "Active".to_string(),
                    value: FieldValue::Bool(true),
                },
            ],
            sections: vec![FieldGroup {
                heading: "Contact".to_string(),
                fields: vec![Field {
                    label: "Phone".to_string(),
                    value: FieldValue::Text("+1-555-0100".to_string()),
                }],
            }],
        }));
    }

    #[test]
    fn test_heading_style_variants_round_trip() {
        round_trip(&HeadingStyle::Named);
        round_trip(&HeadingStyle::Identified);
        round_trip(&HeadingStyle::Bare);
    }

    #[test]
    fn test_field_group_round_trip() {
        round_trip(&FieldGroup {
            heading: "Profile".to_string(),
            fields: vec![
                Field {
                    label: "Country".to_string(),
                    value: FieldValue::Text("US".to_string()),
                },
                Field {
                    label: "DOB".to_string(),
                    value: FieldValue::Null,
                },
            ],
        });
    }

    #[test]
    fn test_entity_result_with_sections_round_trip() {
        round_trip(&CommandResult::Entity(EntityResult {
            kind: "User".to_string(),
            identifier: Some("abc123".to_string()),
            heading_style: HeadingStyle::Identified,
            fields: vec![Field {
                label: "Email".to_string(),
                value: FieldValue::Text("a@b.com".to_string()),
            }],
            sections: vec![
                FieldGroup {
                    heading: "Contact".to_string(),
                    fields: vec![],
                },
                FieldGroup {
                    heading: "Profile".to_string(),
                    fields: vec![Field {
                        label: "Country".to_string(),
                        value: FieldValue::Text("US".to_string()),
                    }],
                },
            ],
        }));
    }

    #[test]
    fn test_entity_result_bare_heading_no_identifier_round_trip() {
        round_trip(&CommandResult::Entity(EntityResult {
            kind: "User".to_string(),
            identifier: None,
            heading_style: HeadingStyle::Bare,
            fields: vec![],
            sections: vec![],
        }));
    }

    #[test]
    fn test_command_result_collection_round_trip() {
        round_trip(&CommandResult::Collection(CollectionResult {
            kind: "Users".to_string(),
            columns: vec![
                ColumnSpec {
                    label: "ID".to_string(),
                    key: "id".to_string(),
                },
                ColumnSpec {
                    label: "Email".to_string(),
                    key: "email".to_string(),
                },
            ],
            rows: vec![Row {
                cells: vec![
                    FieldValue::Text("abc123".to_string()),
                    FieldValue::Text("alice@example.com".to_string()),
                ],
            }],
            page_info: Some(PageInfo {
                current_page: 1,
                total_pages: Some(10),
                total_items: Some(1000),
                has_next: true,
            }),
        }));
    }

    #[test]
    fn test_command_result_empty_round_trip() {
        round_trip(&CommandResult::Empty(EmptyResult {
            operation_id: crate::protocol::catalogue::OperationId::new("AdminDeleteUserV3"),
            status: "deleted".to_string(),
        }));
    }

    #[test]
    fn test_command_result_raw_round_trip() {
        round_trip(&CommandResult::Raw(RawResult {
            value: serde_json::json!({"arbitrary": ["nested", {"json": 42}]}),
        }));
    }

    #[test]
    fn test_field_value_all_variants_round_trip() {
        round_trip(&FieldValue::Text("hello".to_string()));
        round_trip(&FieldValue::Number(2.5));
        round_trip(&FieldValue::Bool(false));
        round_trip(&FieldValue::List(vec!["a".to_string(), "b".to_string()]));
        round_trip(&FieldValue::Null);
    }

    #[test]
    fn test_column_spec_round_trip() {
        round_trip(&ColumnSpec {
            label: "Created".to_string(),
            key: "created_at".to_string(),
        });
    }

    #[test]
    fn test_row_round_trip() {
        round_trip(&Row {
            cells: vec![FieldValue::Null, FieldValue::Number(0.0)],
        });
    }

    #[test]
    fn test_page_info_round_trip_unknown_totals() {
        round_trip(&PageInfo {
            current_page: 3,
            total_pages: None,
            total_items: None,
            has_next: true,
        });
    }
}
