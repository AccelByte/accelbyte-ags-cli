//! Response shaping — decide which fields to show, in what order, with what
//! labels. Pure function of the parsed JSON body plus the operation schema.
//!
//! No I/O, no rendering, no terminal awareness. The render layer consumes the
//! resulting `CommandResult` and emits bytes.

use serde_json::{Map, Value};

use crate::protocol::catalogue::OperationSchema;
use crate::protocol::output::{CommandIntent, FieldEntry, Section};
use crate::protocol::result::{
    CollectionResult, ColumnSpec, CommandResult, EntityResult, Field, FieldGroup, FieldValue,
    HeadingStyle, PageInfo, RawResult, Row,
};
use crate::support::strings::{
    capitalize_first, derive_noun_from_method, singularize, strip_terminal_control_sequences,
};

// ── Field Extraction And Selection ──

/// Convert a camelCase key to "Human Label" with special cases.
pub(crate) fn normalize_label(key: &str) -> String {
    match key {
        "id" => return "ID".to_string(),
        "userId" => return "User ID".to_string(),
        "clientId" => return "Client ID".to_string(),
        "roleId" => return "Role ID".to_string(),
        "namespaceId" => return "Namespace ID".to_string(),
        "createdAt" => return "Created".to_string(),
        "updatedAt" => return "Updated".to_string(),
        "deletedAt" => return "Deleted".to_string(),
        "baseUrl" => return "Base URL".to_string(),
        "emailAddress" => return "Email".to_string(),
        "displayName" => return "Display Name".to_string(),
        "userName" => return "Username".to_string(),
        "appName" => return "App Name".to_string(),
        "namespace" => return "Namespace".to_string(),
        "enable" => return "Enabled".to_string(),
        _ => {}
    }

    let mut result = String::new();
    for (i, character) in key.chars().enumerate() {
        if character.is_uppercase() && i > 0 {
            result.push(' ');
            result.push(character.to_lowercase().next().unwrap_or(character));
        } else if i == 0 {
            result.push(character.to_uppercase().next().unwrap_or(character));
        } else {
            result.push(character);
        }
    }
    result
}

/// Normalize a JSON value to a display string. Returns `None` for values that
/// should be omitted from human-readable output.
pub(crate) fn normalize_value(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Bool(true) => Some("yes".to_string()),
        Value::Bool(false) => Some("no".to_string()),
        Value::String(string) if string.is_empty() => None,
        Value::String(string) => Some(strip_terminal_control_sequences(string)),
        Value::Number(number) => Some(number.to_string()),
        Value::Array(array) if array.is_empty() => None,
        Value::Array(array) => {
            let n = array.len();
            if n == 1 {
                Some("1 item".to_string())
            } else {
                Some(format!("{n} items"))
            }
        }
        Value::Object(object) if object.is_empty() => None,
        Value::Object(_) => None,
    }
}

/// Assign a sort priority to a field key. Lower = more important.
fn field_priority(key: &str) -> u8 {
    match key {
        "id" | "userId" | "clientId" | "roleId" | "namespaceId" => 0,
        "name" | "displayName" | "userName" | "emailAddress" | "appName" => 1,
        "status" | "enabled" | "active" | "verified" | "emailVerified" | "banned" => 2,
        "namespace" | "platform" | "platformId" => 3,
        "createdBy" | "updatedBy" | "deletedBy" => 4,
        "createdAt" | "updatedAt" | "deletedAt" | "lastLogin" | "expiresAt" => 5,
        "description" | "country" | "dateOfBirth" | "language" | "timezone" => 6,
        "code" | "errorCode" | "errorMessage" => 7,
        _ => 8,
    }
}

/// Returns true if a field should be suppressed from output.
fn should_suppress_field(key: &str, value: &Value) -> bool {
    if value.is_null() {
        return true;
    }
    if let Some(string) = value.as_str() {
        if string.is_empty() {
            return true;
        }
    }
    if let Some(array) = value.as_array() {
        if array.is_empty() {
            return true;
        }
    }
    if let Some(object) = value.as_object() {
        if object.is_empty() {
            return true;
        }
    }

    let noise_keys = [
        "paging",
        "pagination",
        "links",
        "_links",
        "self",
        "next",
        "previous",
        "first",
        "last",
        "totalPages",
        "totalData",
        "headers",
        "metadata",
        "_metadata",
    ];
    noise_keys.contains(&key)
}

/// Prioritize and partition fields from a JSON object for display.
///
/// Filters noise, ranks scalars by importance, and separates nested sections.
pub(crate) fn prioritize_fields(
    data: &Map<String, Value>,
    intent: &CommandIntent,
    is_verbose: bool,
) -> (Vec<FieldEntry>, Vec<Section>) {
    let mut scalar_entries: Vec<(u8, FieldEntry)> = Vec::new();
    let mut candidate_sections: Vec<Section> = Vec::new();

    for (key, value) in data {
        if should_suppress_field(key, value) {
            continue;
        }

        match value {
            Value::Object(nested) if !is_verbose => {
                let scalar_children: Vec<FieldEntry> = nested
                    .iter()
                    .filter(|(_, nested_value)| {
                        !nested_value.is_object()
                            && !nested_value.is_array()
                            && !nested_value.is_null()
                    })
                    .filter_map(|(nested_key, nested_value)| {
                        normalize_value(nested_value).map(|display_value| FieldEntry {
                            label: normalize_label(nested_key),
                            value: display_value,
                        })
                    })
                    .collect();

                if !scalar_children.is_empty() && scalar_children.len() <= 6 {
                    candidate_sections.push(Section {
                        heading: normalize_label(key),
                        fields: scalar_children,
                    });
                }
            }
            Value::Object(nested) if is_verbose => {
                let section_fields: Vec<FieldEntry> = nested
                    .iter()
                    .filter_map(|(nested_key, nested_value)| {
                        normalize_value(nested_value).map(|display_value| FieldEntry {
                            label: normalize_label(nested_key),
                            value: display_value,
                        })
                    })
                    .collect();

                if !section_fields.is_empty() {
                    candidate_sections.push(Section {
                        heading: normalize_label(key),
                        fields: section_fields,
                    });
                }
            }
            _ => {
                if let Some(display_value) = normalize_value(value) {
                    let priority = field_priority(key);
                    scalar_entries.push((
                        priority,
                        FieldEntry {
                            label: normalize_label(key),
                            value: display_value,
                        },
                    ));
                }
            }
        }
    }

    scalar_entries.sort_by_key(|(priority, _)| *priority);

    let limit = match (intent, is_verbose) {
        (_, false) => match intent {
            CommandIntent::Action => 5,
            CommandIntent::Inspect => 8,
            CommandIntent::List => 4,
        },
        (CommandIntent::List, true) => 8,
        (_, true) => usize::MAX,
    };

    let fields: Vec<FieldEntry> = scalar_entries
        .into_iter()
        .take(limit)
        .map(|(_, entry)| entry)
        .collect();

    let section_limit = if is_verbose { usize::MAX } else { 2 };
    candidate_sections.truncate(section_limit);

    (fields, candidate_sections)
}

/// Pagination metadata extracted from the response envelope.
#[derive(Debug, Clone)]
pub(crate) struct PaginationHint {
    /// Total number of items across all pages (from `totalData`).
    pub total: Option<u64>,
    /// Whether more pages exist (from non-empty `paging.next`).
    pub is_next_page_available: bool,
}

/// Classified shape of a JSON response body for choosing the right renderer.
pub(crate) enum ResponseShape<'a> {
    /// An array of items, plus the object key it was unwrapped from (e.g.
    /// `regions` for `{"regions": [...]}`), or `None` for a bare top-level
    /// array. The key names the single column when items are scalars.
    Array(&'a Vec<Value>, Option<&'a str>),
    PaginatedList(&'a Vec<Value>, PaginationHint),
    SingleObject(&'a serde_json::Map<String, Value>),
    Scalar,
}

/// Classify a JSON body as array, paginated list, single object, or scalar.
pub(crate) fn detect_shape(body: &Value) -> ResponseShape<'_> {
    match body {
        Value::Array(items) => ResponseShape::Array(items, None),
        Value::Object(object) => {
            if let Some(Value::Array(items)) = object.get("data") {
                let total = object.get("totalData").and_then(|v| v.as_u64());
                let is_next_page_available = object
                    .get("paging")
                    .and_then(|p| p.get("next"))
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.is_empty());
                return ResponseShape::PaginatedList(
                    items,
                    PaginationHint {
                        total,
                        is_next_page_available,
                    },
                );
            }
            let scalar_count = object
                .iter()
                .filter(|(_, v)| !v.is_array() && !v.is_object() && !v.is_null())
                .count();
            let arrays: Vec<_> = object
                .iter()
                .filter(|(_, v)| matches!(v, Value::Array(a) if !a.is_empty()))
                .collect();
            if arrays.len() == 1 && scalar_count <= 2 {
                let (key, value) = arrays[0];
                if let Value::Array(items) = value {
                    return ResponseShape::Array(items, Some(key.as_str()));
                }
            }

            ResponseShape::SingleObject(object)
        }
        _ => ResponseShape::Scalar,
    }
}

/// Column headers and cell values for a tabular list view.
pub(crate) struct ListTable {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Build column headers and rows from an array of items. Arrays of objects
/// derive their columns from the objects' fields; an array of bare scalars
/// renders as a single column titled `scalar_header`.
pub(crate) fn build_list_table(
    items: &[Value],
    scalar_header: &str,
    intent: &CommandIntent,
    is_verbose: bool,
) -> ListTable {
    // Headers come from the first non-empty item.
    let mut headers: Vec<String> = Vec::new();
    for item in items {
        if let Value::Object(object) = item {
            let (fields, _) = prioritize_fields(object, intent, is_verbose);
            if !fields.is_empty() {
                headers = fields.into_iter().map(|f| f.label).collect();
                break;
            }
        }
    }

    if headers.is_empty() {
        // No object fields anywhere — render the items as a single scalar
        // column (e.g. a list of region strings). An array of objects that
        // all suppress their fields, or a genuinely empty list, still yields
        // no rows and renders as the empty-collection view.
        let rows: Vec<Vec<String>> = items
            .iter()
            .filter_map(|item| normalize_value(item).map(|value| vec![value]))
            .collect();
        if rows.is_empty() {
            return ListTable {
                headers,
                rows: Vec::new(),
            };
        }
        return ListTable {
            headers: vec![scalar_header.to_string()],
            rows,
        };
    }

    // Each row is built by label lookup so a missing field never shifts later columns.
    let mut rows: Vec<Vec<String>> = Vec::new();
    for item in items {
        if let Value::Object(object) = item {
            let (fields, _) = prioritize_fields(object, intent, is_verbose);
            if fields.is_empty() {
                continue;
            }
            let by_label: std::collections::HashMap<&str, &str> = fields
                .iter()
                .map(|f| (f.label.as_str(), f.value.as_str()))
                .collect();
            let row = headers
                .iter()
                .map(|h| by_label.get(h.as_str()).copied().unwrap_or("—").to_string())
                .collect();
            rows.push(row);
        } else if let Some(value) = normalize_value(item) {
            rows.push(vec![value]);
        }
    }

    ListTable { headers, rows }
}

/// Remove the field already shown in the heading to avoid repetition.
pub(crate) fn dedupe_heading_field(
    fields: Vec<FieldEntry>,
    used_key: Option<&'static str>,
) -> Vec<FieldEntry> {
    if let Some(key) = used_key {
        let used_label = normalize_label(key);
        fields
            .into_iter()
            .filter(|field| field.label != used_label)
            .collect()
    } else {
        fields
    }
}

// ── Heading Derivation ──

const HEADING_NAME_KEYS: &[&str] = &[
    "displayName",
    "name",
    "appName",
    "clientName",
    "userName",
    "emailAddress",
];

/// Extract and capitalize the noun portion of a method name for inspect headings.
pub(crate) fn derive_heading_from_method(method_name: &str) -> Option<String> {
    let pos = method_name.find('-')?;
    let after = &method_name[pos + 1..];
    let heading = after.replace('-', " ");
    Some(capitalize_first(&heading))
}

// ── Public Shaping Entry Point ──

/// Convert a parsed JSON response body into a structured `CommandResult`.
///
/// For collection-shaped bodies (arrays, `{data: [...]}`), returns `Collection`.
/// For single-object bodies, returns `Entity`. For scalar or unrecognised
/// shapes, returns `Raw` with the original value.
///
/// Does not handle empty bodies or non-JSON text — the caller constructs
/// `Empty` or `Raw` directly for those.
pub fn shape_response(
    body: &Value,
    operation: &OperationSchema,
    resource_name: &str,
    is_verbose: bool,
) -> CommandResult {
    let intent = CommandIntent::from_operation(operation);

    match detect_shape(body) {
        ResponseShape::Array(items, scalar_key) => shape_collection(
            items,
            None,
            scalar_key,
            &intent,
            is_verbose,
            operation,
            resource_name,
        ),
        ResponseShape::PaginatedList(items, pagination) => shape_collection(
            items,
            Some(pagination),
            None,
            &intent,
            is_verbose,
            operation,
            resource_name,
        ),
        ResponseShape::SingleObject(object) => {
            shape_entity(object, &intent, is_verbose, operation, resource_name)
        }
        ResponseShape::Scalar => CommandResult::Raw(RawResult {
            value: body.clone(),
        }),
    }
}

// ── Collection And Entity Rendering ──

/// Build a `CommandResult::Collection` from a list of items, derived columns, and pagination state.
fn shape_collection(
    items: &[Value],
    pagination: Option<PaginationHint>,
    scalar_key: Option<&str>,
    intent: &CommandIntent,
    is_verbose: bool,
    operation: &OperationSchema,
    resource_name: &str,
) -> CommandResult {
    let noun = derive_noun_from_method(&operation.name, resource_name);
    // When the items are bare scalars (e.g. `{"regions": ["us-east-1", ...]}`)
    // there are no object fields to derive columns from. Label the single
    // column after the key it was unwrapped from, falling back to the noun.
    let scalar_header = scalar_key
        .map(normalize_label)
        .unwrap_or_else(|| capitalize_first(&noun));
    let table = build_list_table(items, &scalar_header, intent, is_verbose);

    let columns = table
        .headers
        .iter()
        .map(|label| ColumnSpec {
            label: label.clone(),
            key: label.clone(),
        })
        .collect::<Vec<_>>();

    let rows = table
        .rows
        .into_iter()
        .map(|cells| Row {
            cells: cells.into_iter().map(FieldValue::Text).collect(),
        })
        .collect();

    let page_info = pagination.map(|hint| PageInfo {
        current_page: 1,
        total_pages: None,
        total_items: hint.total.map(|t| t as usize),
        has_next: hint.is_next_page_available,
    });

    CommandResult::Collection(CollectionResult {
        kind: noun,
        columns,
        rows,
        page_info,
    })
}

/// Build a `CommandResult::Entity` from a JSON object, deriving heading and prioritized fields.
fn shape_entity(
    object: &Map<String, Value>,
    intent: &CommandIntent,
    is_verbose: bool,
    operation: &OperationSchema,
    resource_name: &str,
) -> CommandResult {
    let (entries, section_entries) = prioritize_fields(object, intent, is_verbose);

    let (heading_kind, identifier, heading_style, used_key) =
        derive_entity_heading(object, resource_name, &operation.name);

    let kept_entries = dedupe_heading_field(entries, used_key);

    let fields = kept_entries
        .into_iter()
        .map(field_entry_to_protocol)
        .collect();

    let sections = section_entries
        .into_iter()
        .map(|section| FieldGroup {
            heading: section.heading,
            fields: section
                .fields
                .into_iter()
                .map(field_entry_to_protocol)
                .collect(),
        })
        .collect();

    CommandResult::Entity(EntityResult {
        kind: heading_kind,
        identifier,
        heading_style,
        fields,
        sections,
    })
}

/// Convert an internal `FieldEntry` (label + already-rendered text) to a protocol `Field`.
fn field_entry_to_protocol(entry: FieldEntry) -> Field {
    Field {
        label: entry.label,
        value: FieldValue::Text(entry.value),
    }
}

/// Derive the heading kind, identifier, and style for an entity result.
///
/// Emits a structured triple instead of a pre-rendered string. The caller uses
/// `heading_style` to decide the render format: Named → "<kind>: <id>",
/// Identified → "<kind> (<id>)", Bare → just "<kind>".
pub(crate) fn derive_entity_heading(
    object: &Map<String, Value>,
    resource_name: &str,
    method_name: &str,
) -> (String, Option<String>, HeadingStyle, Option<&'static str>) {
    let singular = singularize(resource_name);

    for &key in HEADING_NAME_KEYS {
        if let Some(name) = object.get(key).and_then(|value| value.as_str()) {
            if !name.is_empty() {
                return (
                    normalize_label(&singular),
                    Some(name.to_string()),
                    HeadingStyle::Named,
                    Some(key),
                );
            }
        }
    }

    let heading_base =
        derive_heading_from_method(method_name).unwrap_or_else(|| normalize_label(&singular));

    if let Some(id) = object
        .get("id")
        .or_else(|| object.get("userId"))
        .and_then(|value| value.as_str())
    {
        (
            heading_base,
            Some(id.to_string()),
            HeadingStyle::Identified,
            None,
        )
    } else {
        (heading_base, None, HeadingStyle::Bare, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A bare JSON array body is detected as `ResponseShape::Array`
    #[test]
    fn test_detect_shape_array() {
        let body = json!([1, 2, 3]);
        assert!(matches!(detect_shape(&body), ResponseShape::Array(..)));
    }

    /// `{ data: [...], paging: ... }` envelopes are detected as a paginated list
    #[test]
    fn test_detect_shape_paginated_list() {
        let body = json!({"data": [{"id": "a"}], "paging": {}});
        assert!(matches!(
            detect_shape(&body),
            ResponseShape::PaginatedList(_, _)
        ));
    }

    /// Objects without a `data` key are treated as a single entity
    #[test]
    fn test_detect_shape_object_without_data_key() {
        let body = json!({"id": "abc", "name": "test"});
        assert!(matches!(
            detect_shape(&body),
            ResponseShape::SingleObject(_)
        ));
    }

    /// A `data` field that is not an array does not trigger paginated-list detection
    #[test]
    fn test_detect_shape_object_with_non_array_data() {
        let body = json!({"data": "not-an-array"});
        assert!(matches!(
            detect_shape(&body),
            ResponseShape::SingleObject(_)
        ));
    }

    /// A bare JSON string body is detected as `ResponseShape::Scalar`
    #[test]
    fn test_detect_shape_scalar_string() {
        let body = json!("hello");
        assert!(matches!(detect_shape(&body), ResponseShape::Scalar));
    }

    /// A bare JSON number body is detected as `ResponseShape::Scalar`
    #[test]
    fn test_detect_shape_scalar_number() {
        let body = json!(42);
        assert!(matches!(detect_shape(&body), ResponseShape::Scalar));
    }

    /// A JSON null body is detected as `ResponseShape::Scalar`
    #[test]
    fn test_detect_shape_scalar_null() {
        let body = json!(null);
        assert!(matches!(detect_shape(&body), ResponseShape::Scalar));
    }

    /// An entity with multiple scalar fields and one nested array is still a single object, not a list
    #[test]
    fn test_detect_shape_rich_object_with_one_array() {
        let body = json!({
            "userId": "abc-123",
            "displayName": "Jane",
            "emailAddress": "jane@example.com",
            "enabled": true,
            "namespace": "test",
            "namespaceRoles": [
                {"roleId": "role-1", "namespace": "*"},
                {"roleId": "role-2", "namespace": "test"}
            ]
        });
        assert!(matches!(
            detect_shape(&body),
            ResponseShape::SingleObject(_)
        ));
    }

    /// An envelope with a single array field (e.g. `{ bans: [...] }`) is unwrapped to an array
    #[test]
    fn test_detect_shape_single_array_envelope() {
        let body = json!({"bans": [{"id": "b1"}, {"id": "b2"}]});
        assert!(matches!(detect_shape(&body), ResponseShape::Array(..)));
    }

    /// A method like `get-ban` strips the verb prefix to derive the noun "ban"
    #[test]
    fn test_derive_noun_from_method_with_verb_prefix() {
        assert_eq!(derive_noun_from_method("get-ban", "bans"), "ban");
        assert_eq!(derive_noun_from_method("list-roles", "roles"), "roles");
    }

    /// Compound method names like `update-ip-ban` produce multi-word nouns ("ip ban")
    #[test]
    fn test_derive_noun_from_method_compound() {
        assert_eq!(derive_noun_from_method("update-ip-ban", "bans"), "ip ban");
    }

    /// Methods without a hyphen fall back to the resource name as the noun
    #[test]
    fn test_derive_noun_from_method_no_hyphen() {
        assert_eq!(derive_noun_from_method("list", "users"), "users");
        assert_eq!(derive_noun_from_method("get", "clients"), "clients");
    }

    /// When the suffix after the first hyphen starts with a preposition, the
    /// suffix describes *how* the lookup works (a query qualifier) rather
    /// than the returned entity. Fall back to the resource name so the list
    /// header reads naturally: "Found N clients" not "Found N by namespaces".
    #[test]
    fn test_derive_noun_from_method_prepositional_suffix_uses_resource_name() {
        assert_eq!(
            derive_noun_from_method("get-by-namespace", "clients"),
            "clients"
        );
        assert_eq!(derive_noun_from_method("get-by-user-id", "users"), "users");
        assert_eq!(
            derive_noun_from_method("get-by-email-address", "users"),
            "users"
        );
        assert_eq!(
            derive_noun_from_method("get-by-platform-id", "users"),
            "users"
        );
    }

    /// Hyphenated methods like `get-user` produce a capitalized heading ("User", "Ip ban")
    #[test]
    fn test_derive_heading_from_method_with_hyphen() {
        assert_eq!(
            derive_heading_from_method("get-user"),
            Some("User".to_string())
        );
        assert_eq!(
            derive_heading_from_method("update-ip-ban"),
            Some("Ip ban".to_string())
        );
    }

    /// Methods without a hyphen produce no heading (`None`) — caller falls back to the resource name
    #[test]
    fn test_derive_heading_from_method_no_hyphen() {
        assert_eq!(derive_heading_from_method("list"), None);
        assert_eq!(derive_heading_from_method("get"), None);
    }

    /// When a later row is missing a field that the first row established as a header,
    /// the row must still have the same number of cells — missing cells must be "—"
    /// rather than shifting subsequent fields left into the wrong column.
    #[test]
    fn test_build_list_table_missing_field_does_not_shift_columns() {
        let items = vec![
            json!({
                "userId": "user-1",
                "displayName": "Alice",
                "emailAddress": "alice@example.com"
            }),
            json!({
                "userId": "user-2",
                "displayName": "",        // empty → filtered by is_noise
                "emailAddress": "bob@example.com"
            }),
        ];
        let table = build_list_table(&items, "Value", &CommandIntent::List, false);

        assert_eq!(table.headers, vec!["User ID", "Display Name", "Email"]);
        assert_eq!(table.rows[0], vec!["user-1", "Alice", "alice@example.com"]);
        // Display Name must be "—", not "bob@example.com" shifted left
        assert_eq!(table.rows[1], vec!["user-2", "—", "bob@example.com"]);
    }

    #[test]
    fn test_build_list_table_app_name_precedes_status_and_timestamps() {
        let items = vec![json!({
            "appId": "app-001",
            "appName": "my-service",
            "appStatus": "running",
            "createdAt": "2026-01-01T00:00:00Z",
            "namespace": "my-game"
        })];
        let table = build_list_table(&items, "App", &CommandIntent::List, false);
        let app_name_pos = table
            .headers
            .iter()
            .position(|h| h == "App Name")
            .expect("App Name header present");
        let created_pos = table
            .headers
            .iter()
            .position(|h| h == "Created")
            .expect("Created header present");
        assert!(
            app_name_pos < created_pos,
            "appName should sort before createdAt"
        );
    }

    /// An array of bare scalars (the `{"regions": [...]}` shape, unwrapped) is
    /// rendered as a single column named after the source key, not dropped.
    #[test]
    fn test_build_list_table_scalar_array_uses_named_column() {
        let items = vec![json!("us-east-1"), json!("eu-west-1")];
        let table = build_list_table(&items, "Regions", &CommandIntent::List, false);
        assert_eq!(table.headers, vec!["Regions"]);
        assert_eq!(table.rows, vec![vec!["us-east-1"], vec!["eu-west-1"]]);
    }

    /// End-to-end: a `{"regions": [strings]}` body shapes into a Collection
    /// with a single "Regions" column — the regression guard for the empty
    /// list rendering bug.
    #[test]
    fn test_shape_response_regions_scalar_array_collection() {
        use crate::protocol::catalogue::{ApiVersion, HttpMethod, MutationClass, OperationId};
        let body = json!({ "regions": ["us-east-1", "eu-west-1"] });
        let operation = OperationSchema {
            id: OperationId::new("ams/admin/info/v1/list-regions"),
            name: "list-regions".to_string(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/regions".to_string(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        };
        let result = shape_response(&body, &operation, "region", false);
        let CommandResult::Collection(collection) = result else {
            panic!("expected Collection, got {result:?}");
        };
        assert_eq!(collection.columns.len(), 1);
        assert_eq!(collection.columns[0].label, "Regions");
        assert_eq!(collection.rows.len(), 2);
        assert_eq!(
            collection.rows[0].cells,
            vec![FieldValue::Text("us-east-1".to_string())]
        );
    }
}
