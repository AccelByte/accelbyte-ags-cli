//! Tree node model for the structured JSON body editor.
//!
//! Each [`Node`] holds its schema, its current value (when scalar), and an
//! expand/collapse flag for the renderer. Builds from a schema + value
//! via [`from_schema`]; the inverse [`to_value`] lives next door.

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub schema: Value,
    pub required: bool,
    pub kind: NodeKind,
    pub expanded: bool,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Object {
        children: Vec<Node>,
    },
    Array {
        children: Vec<Node>,
    },
    Scalar {
        value: Option<ScalarValue>,
    },
    OneOf {
        variants: Vec<Node>,
        selected: Option<usize>,
    },
    /// `additionalProperties` map: arbitrary user-named keys onto a single
    /// value schema. Stored as `(key, value-node)` pairs to preserve insertion
    /// order in the renderer.
    FreeMap {
        entries: Vec<(String, Node)>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScalarValue {
    String(String),
    Integer(i64),
    Number(f64),
    Boolean(bool),
    Enum(String),
}

/// Build a minimal JSON-schema fragment describing the shape of `value`.
/// Used as a fallback when the field's declared schema lacks the structure
/// needed for the tree editor (e.g. a property-less `{"type":"object"}` whose
/// concrete value carries keys we still want to surface).
fn infer_schema_from_value(value: &Value) -> Value {
    match value {
        Value::String(_) => serde_json::json!({"type": "string"}),
        Value::Bool(_) => serde_json::json!({"type": "boolean"}),
        Value::Number(n) if n.is_i64() || n.is_u64() => {
            serde_json::json!({"type": "integer"})
        }
        Value::Number(_) => serde_json::json!({"type": "number"}),
        Value::Array(items) => {
            let item_schema = items
                .first()
                .map(infer_schema_from_value)
                .unwrap_or_else(|| serde_json::json!({}));
            serde_json::json!({"type": "array", "items": item_schema})
        }
        Value::Object(_) => serde_json::json!({"type": "object"}),
        Value::Null => serde_json::json!({}),
    }
}

/// Build a node tree from a JSON schema plus an existing value (often
/// `Value::Null` for fresh forms). Required-flag inheritance comes from
/// the parent object's `required: [...]` array, mirroring OpenAPI.
pub fn from_schema(name: &str, schema: &Value, value: &Value, required: bool) -> Node {
    let kind = match schema.get("type").and_then(|t| t.as_str()) {
        Some("object") => {
            let mut children = Vec::new();
            if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                let required_set: std::collections::HashSet<String> = schema
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                for (prop_name, prop_schema) in props {
                    let child_value = value.get(prop_name).cloned().unwrap_or(Value::Null);
                    let child_required = required_set.contains(prop_name);
                    children.push(from_schema(
                        prop_name,
                        prop_schema,
                        &child_value,
                        child_required,
                    ));
                }
            } else if let Value::Object(map) = value {
                // Schema has no `properties`, but the current value has keys
                // (e.g. a literal-bound JSON object) — surface those keys with
                // types inferred from their values, so the user can edit them
                // structurally instead of dropping to raw mode.
                for (key, child_value) in map {
                    let inferred = infer_schema_from_value(child_value);
                    children.push(from_schema(key, &inferred, child_value, false));
                }
            }
            NodeKind::Object { children }
        }
        Some("array") => {
            let item_schema = schema.get("items").cloned().unwrap_or(Value::Null);
            let mut children = Vec::new();
            if let Value::Array(items) = value {
                for (i, item) in items.iter().enumerate() {
                    children.push(from_schema(&format!("[{i}]"), &item_schema, item, false));
                }
            }
            NodeKind::Array { children }
        }
        Some("string") | Some("integer") | Some("number") | Some("boolean") => NodeKind::Scalar {
            value: scalar_from_value(value, schema),
        },
        _ => {
            // Type unset or unknown: oneOf, additionalProperties, or plain free-form.
            if let Some(variants) = schema.get("oneOf").and_then(|v| v.as_array()) {
                let variant_nodes = variants
                    .iter()
                    .enumerate()
                    .map(|(i, vs)| from_schema(&format!("variant {i}"), vs, &Value::Null, false))
                    .collect();
                NodeKind::OneOf {
                    variants: variant_nodes,
                    selected: None,
                }
            } else if schema.get("additionalProperties").is_some() {
                NodeKind::FreeMap {
                    entries: Vec::new(),
                }
            } else {
                NodeKind::Scalar {
                    value: scalar_from_value(value, schema),
                }
            }
        }
    };
    Node {
        name: name.into(),
        schema: schema.clone(),
        required,
        kind,
        expanded: true,
    }
}

impl Node {
    /// Serialise the node back to JSON for request submission. Unset
    /// scalars and unselected `oneOf` variants emit `null` at the leaf;
    /// parent objects/maps drop any `null` children so optional fields
    /// the user left untouched don't appear in the output.
    pub fn to_value(&self) -> Value {
        match &self.kind {
            NodeKind::Object { children } => {
                let mut map = serde_json::Map::new();
                for child in children {
                    let v = child.to_value();
                    if !v.is_null() {
                        map.insert(child.name.clone(), v);
                    }
                }
                Value::Object(map)
            }
            NodeKind::Array { children } => {
                Value::Array(children.iter().map(|c| c.to_value()).collect())
            }
            NodeKind::Scalar { value: Some(s) } => match s {
                ScalarValue::String(s) | ScalarValue::Enum(s) => Value::String(s.clone()),
                ScalarValue::Integer(i) => Value::Number((*i).into()),
                ScalarValue::Number(f) => serde_json::Number::from_f64(*f)
                    .map(Value::Number)
                    .unwrap_or(Value::Null),
                ScalarValue::Boolean(b) => Value::Bool(*b),
            },
            NodeKind::Scalar { value: None } => Value::Null,
            NodeKind::OneOf {
                variants,
                selected: Some(i),
            } => variants.get(*i).map(Node::to_value).unwrap_or(Value::Null),
            NodeKind::OneOf { selected: None, .. } => Value::Null,
            NodeKind::FreeMap { entries } => {
                let mut map = serde_json::Map::new();
                for (k, v) in entries {
                    let val = v.to_value();
                    if !val.is_null() {
                        map.insert(k.clone(), val);
                    }
                }
                Value::Object(map)
            }
        }
    }
}

impl Node {
    /// If this node is an array, append a new entry built from the
    /// items schema. Returns the new entry's index on success.
    pub fn array_append_default(&mut self) -> Result<usize, &'static str> {
        let NodeKind::Array { children } = &mut self.kind else {
            return Err("not an array");
        };
        let items_schema = self.schema.get("items").cloned().unwrap_or(Value::Null);
        let new_index = children.len();
        children.push(from_schema(
            &format!("[{new_index}]"),
            &items_schema,
            &Value::Null,
            false,
        ));
        Ok(new_index)
    }

    /// Delete the entry at `index`, renumbering the remaining visual
    /// labels so they stay contiguous.
    pub fn array_delete_at(&mut self, index: usize) -> Result<(), &'static str> {
        let NodeKind::Array { children } = &mut self.kind else {
            return Err("not an array");
        };
        if index >= children.len() {
            return Err("out of bounds");
        }
        children.remove(index);
        for (i, child) in children.iter_mut().enumerate() {
            child.name = format!("[{i}]");
        }
        Ok(())
    }
}

/// Map a JSON value into a [`ScalarValue`], honouring `enum` in the schema
/// (which wins over the value's underlying JSON type).
fn scalar_from_value(value: &Value, schema: &Value) -> Option<ScalarValue> {
    if schema.get("enum").is_some() {
        return value.as_str().map(|s| ScalarValue::Enum(s.into()));
    }
    match value {
        Value::String(s) => Some(ScalarValue::String(s.clone())),
        Value::Bool(b) => Some(ScalarValue::Boolean(*b)),
        Value::Number(n) if n.is_i64() => Some(ScalarValue::Integer(n.as_i64().unwrap())),
        Value::Number(n) => n.as_f64().map(ScalarValue::Number),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_schema_builds_object_with_required_props() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" }, "b": { "type": "integer" } },
            "required": ["a"]
        });
        let value = serde_json::json!({ "a": "hi" });
        let node = from_schema("root", &schema, &value, true);
        let NodeKind::Object { children } = &node.kind else {
            panic!("expected object")
        };
        assert_eq!(children.len(), 2);
        let a = children.iter().find(|c| c.name == "a").unwrap();
        let b = children.iter().find(|c| c.name == "b").unwrap();
        assert!(a.required);
        assert!(!b.required);
    }

    #[test]
    fn test_from_schema_builds_array_from_existing_value() {
        let schema = serde_json::json!({
            "type": "array",
            "items": { "type": "string" }
        });
        let value = serde_json::json!(["a", "b", "c"]);
        let node = from_schema("items", &schema, &value, false);
        let NodeKind::Array { children } = &node.kind else {
            panic!("expected array")
        };
        assert_eq!(children.len(), 3);
    }

    #[test]
    fn test_from_schema_extracts_oneof_variants() {
        let schema = serde_json::json!({
            "oneOf": [{ "type": "string" }, { "type": "integer" }]
        });
        let node = from_schema("variant", &schema, &serde_json::Value::Null, false);
        let NodeKind::OneOf { variants, selected } = &node.kind else {
            panic!("expected oneOf")
        };
        assert_eq!(variants.len(), 2);
        assert!(selected.is_none());
    }

    #[test]
    fn test_from_schema_treats_enum_as_scalar_enum() {
        let schema = serde_json::json!({
            "type": "string",
            "enum": ["red", "green", "blue"]
        });
        let value = serde_json::json!("red");
        let node = from_schema("color", &schema, &value, true);
        let NodeKind::Scalar {
            value: Some(ScalarValue::Enum(s)),
        } = &node.kind
        else {
            panic!("expected scalar enum, got {:?}", node.kind)
        };
        assert_eq!(s, "red");
    }

    #[test]
    fn test_from_schema_seeds_scalar_value_from_existing_json() {
        let schema = serde_json::json!({ "type": "integer" });
        let value = serde_json::json!(42);
        let node = from_schema("count", &schema, &value, false);
        let NodeKind::Scalar {
            value: Some(ScalarValue::Integer(n)),
        } = &node.kind
        else {
            panic!("expected scalar integer")
        };
        assert_eq!(*n, 42);
    }

    #[test]
    fn test_roundtrip_simple_object() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "name": { "type": "string" }, "age": { "type": "integer" } },
            "required": ["name"]
        });
        let original = serde_json::json!({ "name": "alice", "age": 30 });
        let node = from_schema("root", &schema, &original, true);
        assert_eq!(node.to_value(), original);
    }

    #[test]
    fn test_roundtrip_array_of_objects() {
        let schema = serde_json::json!({
            "type": "array",
            "items": {
                "type": "object",
                "properties": { "sku": { "type": "string" } }
            }
        });
        let original = serde_json::json!([
            { "sku": "a" },
            { "sku": "b" }
        ]);
        let node = from_schema("items", &schema, &original, false);
        assert_eq!(node.to_value(), original);
    }

    #[test]
    fn test_unset_optional_fields_omitted_from_output() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "a": { "type": "string" }, "b": { "type": "string" } }
        });
        let original = serde_json::json!({ "a": "set" });
        let node = from_schema("root", &schema, &original, false);
        assert_eq!(node.to_value(), serde_json::json!({ "a": "set" }));
    }

    #[test]
    fn test_unset_scalar_node_emits_null() {
        let schema = serde_json::json!({ "type": "string" });
        let node = from_schema("name", &schema, &serde_json::Value::Null, false);
        assert_eq!(node.to_value(), serde_json::Value::Null);
    }

    #[test]
    fn test_oneof_with_no_selection_emits_null() {
        let schema = serde_json::json!({
            "oneOf": [{ "type": "string" }, { "type": "integer" }]
        });
        let node = from_schema("v", &schema, &serde_json::Value::Null, false);
        assert_eq!(node.to_value(), serde_json::Value::Null);
    }

    #[test]
    fn test_array_append_default_grows_by_one() {
        let schema = serde_json::json!({
            "type": "array",
            "items": { "type": "string" }
        });
        let mut node = from_schema("items", &schema, &serde_json::Value::Null, false);
        let new_index = node.array_append_default().unwrap();
        assert_eq!(new_index, 0);
        let NodeKind::Array { children } = &node.kind else {
            panic!("expected array")
        };
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "[0]");
    }

    #[test]
    fn test_array_delete_renumbers_remaining_entries() {
        let schema = serde_json::json!({
            "type": "array",
            "items": { "type": "string" }
        });
        let value = serde_json::json!(["a", "b", "c"]);
        let mut node = from_schema("items", &schema, &value, false);
        node.array_delete_at(1).unwrap();
        let NodeKind::Array { children } = &node.kind else {
            panic!("expected array")
        };
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name, "[0]");
        assert_eq!(children[1].name, "[1]");
    }

    #[test]
    fn test_array_delete_out_of_bounds_returns_err() {
        let schema = serde_json::json!({
            "type": "array",
            "items": { "type": "string" }
        });
        let mut node = from_schema("items", &schema, &serde_json::Value::Null, false);
        assert!(node.array_delete_at(0).is_err());
    }

    #[test]
    fn test_array_methods_on_non_array_return_err() {
        let schema = serde_json::json!({ "type": "string" });
        let mut node = from_schema("name", &schema, &serde_json::Value::Null, false);
        assert!(node.array_append_default().is_err());
        assert!(node.array_delete_at(0).is_err());
    }

    #[test]
    fn test_from_schema_free_map_when_additional_properties_set() {
        let schema = serde_json::json!({
            "additionalProperties": { "type": "string" }
        });
        let node = from_schema("attrs", &schema, &serde_json::Value::Null, false);
        let NodeKind::FreeMap { entries } = &node.kind else {
            panic!("expected free map")
        };
        assert!(entries.is_empty());
    }
}
