//! Parser for nested-field binding paths.
//!
//! Wire form: `field: "data.matching_rule[0].attribute"`.
//! Parses to a typed `FieldPath` of dotted segments and array indices.

use ags_protocol::catalogue::{BodyField, BodyFieldType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldPath {
    pub segments: Vec<PathSegment>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FieldPathError {
    #[error("Field path is empty")]
    Empty,
    #[error("Field path '{path}' has unbalanced brackets at byte {at}")]
    UnbalancedBrackets { path: String, at: usize },
    #[error("Field path '{path}' has invalid index at byte {at}")]
    InvalidIndex { path: String, at: usize },
    #[error("Field path '{path}' has empty key segment at byte {at}")]
    EmptyKey { path: String, at: usize },
}

impl FieldPath {
    /// Parse a nested-field binding path (e.g. `data.items[0].id`) into key and
    /// index segments. Rejects empty paths, empty key segments, and invalid indices.
    pub fn parse(input: &str) -> Result<Self, FieldPathError> {
        if input.is_empty() {
            return Err(FieldPathError::Empty);
        }
        let mut segments = Vec::new();
        let bytes = input.as_bytes();
        let mut i = 0;
        let mut start = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'.' => {
                    if i == start {
                        return Err(FieldPathError::EmptyKey {
                            path: input.into(),
                            at: i,
                        });
                    }
                    segments.push(PathSegment::Key(input[start..i].into()));
                    i += 1;
                    start = i;
                }
                b'[' => {
                    if i > start {
                        segments.push(PathSegment::Key(input[start..i].into()));
                    }
                    let bracket_start = i;
                    i += 1;
                    let idx_start = i;
                    while i < bytes.len() && bytes[i] != b']' {
                        i += 1;
                    }
                    if i >= bytes.len() {
                        return Err(FieldPathError::UnbalancedBrackets {
                            path: input.into(),
                            at: bracket_start,
                        });
                    }
                    let idx_str = &input[idx_start..i];
                    let idx =
                        idx_str
                            .parse::<usize>()
                            .map_err(|_| FieldPathError::InvalidIndex {
                                path: input.into(),
                                at: idx_start,
                            })?;
                    segments.push(PathSegment::Index(idx));
                    i += 1; // consume ']'
                    if i < bytes.len() && bytes[i] == b'.' {
                        i += 1;
                    }
                    start = i;
                }
                _ => i += 1,
            }
        }
        if bytes.last() == Some(&b'.') {
            return Err(FieldPathError::EmptyKey {
                path: input.into(),
                at: bytes.len() - 1,
            });
        }
        if start < bytes.len() {
            segments.push(PathSegment::Key(input[start..].into()));
        }
        if segments.is_empty() {
            return Err(FieldPathError::Empty);
        }
        Ok(FieldPath { segments })
    }

    /// Resolve this path against an operation's top-level body fields.
    ///
    /// Returns `SchemaLeaf` if every ancestor is schema-typed and the leaf
    /// exists; `StructuralOnly` if a free-form ancestor short-circuits the
    /// walk; `Err` if a typed ancestor lacks the requested segment.
    pub fn resolve_against_schema<'a>(
        &self,
        root_fields: &'a [BodyField],
    ) -> Result<PathResolution<'a>, PathResolveError> {
        let mut current_fields = root_fields;
        let mut parent_label = "<root>".to_string();
        let mut iter = self.segments.iter().peekable();
        while let Some(seg) = iter.next() {
            match seg {
                PathSegment::Key(name) => {
                    let field = current_fields.iter().find(|f| &f.name == name);
                    let Some(field) = field else {
                        return Err(PathResolveError::UnknownSegment {
                            parent: parent_label,
                            segment: name.clone(),
                        });
                    };
                    if iter.peek().is_none() {
                        return Ok(PathResolution::SchemaLeaf(field));
                    }
                    // Free-form object (no children) or unresolved $ref: fall
                    // back to structural validation. The parser flattens nested
                    // objects inline when it can; a `Reference(...)` survives
                    // when depth limits or cycles prevent inlining, and the
                    // referenced definition isn't reachable from this BodyField
                    // alone — so any sub-leaf inside it can't be schema-checked.
                    if matches!(field.field_type, BodyFieldType::Object)
                        && field.children.is_empty()
                    {
                        return Ok(PathResolution::StructuralOnly);
                    }
                    if matches!(field.field_type, BodyFieldType::Reference(_)) {
                        return Ok(PathResolution::StructuralOnly);
                    }
                    if !matches!(
                        field.field_type,
                        BodyFieldType::Object | BodyFieldType::Array(_)
                    ) {
                        return Err(PathResolveError::SegmentUnderNonObject {
                            parent: parent_label,
                            segment: name.clone(),
                        });
                    }
                    current_fields = &field.children;
                    parent_label = field.name.clone();
                }
                PathSegment::Index(_) => {
                    // Path ends on an index — treat element binding as structural.
                    if iter.peek().is_none() {
                        return Ok(PathResolution::StructuralOnly);
                    }
                    // Array index: if no children on current context, fall back to structural.
                    if current_fields.is_empty() {
                        return Ok(PathResolution::StructuralOnly);
                    }
                }
            }
        }
        unreachable!("segments is non-empty by construction")
    }
}

/// Outcome of resolving a `FieldPath` against an operation's body schema.
#[derive(Debug)]
pub enum PathResolution<'a> {
    /// Path fully resolves to a schema-typed leaf.
    SchemaLeaf(&'a BodyField),
    /// Path passes through a free-form ancestor; structural validation only.
    StructuralOnly,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PathResolveError {
    #[error("Path segment '{segment}' not found under '{parent}'")]
    UnknownSegment { parent: String, segment: String },
    #[error("Path segment '{segment}' addresses a non-object field '{parent}'")]
    SegmentUnderNonObject { parent: String, segment: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_key() {
        assert_eq!(
            FieldPath::parse("name").unwrap().segments,
            vec![PathSegment::Key("name".into())]
        );
    }

    #[test]
    fn test_parse_dotted_keys() {
        assert_eq!(
            FieldPath::parse("dsHostConfiguration.instanceId")
                .unwrap()
                .segments,
            vec![
                PathSegment::Key("dsHostConfiguration".into()),
                PathSegment::Key("instanceId".into()),
            ]
        );
    }

    #[test]
    fn test_parse_indexed_segment() {
        assert_eq!(
            FieldPath::parse("regions[0].region").unwrap().segments,
            vec![
                PathSegment::Key("regions".into()),
                PathSegment::Index(0),
                PathSegment::Key("region".into()),
            ]
        );
    }

    #[test]
    fn test_parse_deeply_nested() {
        assert_eq!(
            FieldPath::parse("data.matching_rule[0].attribute")
                .unwrap()
                .segments,
            vec![
                PathSegment::Key("data".into()),
                PathSegment::Key("matching_rule".into()),
                PathSegment::Index(0),
                PathSegment::Key("attribute".into()),
            ]
        );
    }

    #[test]
    fn test_parse_empty_is_error() {
        assert!(matches!(FieldPath::parse(""), Err(FieldPathError::Empty)));
    }

    #[test]
    fn test_parse_unbalanced_bracket_is_error() {
        assert!(matches!(
            FieldPath::parse("regions[0"),
            Err(FieldPathError::UnbalancedBrackets { .. })
        ));
    }

    #[test]
    fn test_parse_invalid_index_is_error() {
        assert!(matches!(
            FieldPath::parse("regions[x]"),
            Err(FieldPathError::InvalidIndex { .. })
        ));
    }

    #[test]
    fn test_parse_trailing_dot_after_key_is_error() {
        assert!(matches!(
            FieldPath::parse("name."),
            Err(FieldPathError::EmptyKey { .. })
        ));
    }

    #[test]
    fn test_parse_trailing_dot_after_index_is_error() {
        assert!(matches!(
            FieldPath::parse("regions[0]."),
            Err(FieldPathError::EmptyKey { .. })
        ));
    }

    #[test]
    fn test_parse_leading_dot_is_error() {
        assert!(matches!(
            FieldPath::parse(".name"),
            Err(FieldPathError::EmptyKey { .. })
        ));
    }

    #[test]
    fn test_parse_consecutive_dots_is_error() {
        assert!(matches!(
            FieldPath::parse("a..b"),
            Err(FieldPathError::EmptyKey { .. })
        ));
    }

    // ---------------------------------------------------------------------------
    // resolve_against_schema tests
    // ---------------------------------------------------------------------------

    /// Build a nested `BodyField` fixture.
    fn make_field(name: &str, field_type: BodyFieldType, children: Vec<BodyField>) -> BodyField {
        BodyField {
            name: name.into(),
            field_type,
            required: false,
            description: None,
            children,
            default: None,
        }
    }

    #[test]
    fn test_resolve_schema_leaf() {
        // dsHostConfiguration.instanceId — both segments typed.
        let fields = vec![make_field(
            "dsHostConfiguration",
            BodyFieldType::Object,
            vec![make_field("instanceId", BodyFieldType::String, vec![])],
        )];
        let path = FieldPath::parse("dsHostConfiguration.instanceId").unwrap();
        let result = path.resolve_against_schema(&fields).unwrap();
        assert!(
            matches!(result, PathResolution::SchemaLeaf(f) if f.name == "instanceId"),
            "expected SchemaLeaf(instanceId)"
        );
    }

    #[test]
    fn test_resolve_structural_only_under_freeform_object() {
        // "data" is an Object with no declared children — free-form.
        let fields = vec![make_field("data", BodyFieldType::Object, vec![])];
        let path = FieldPath::parse("data.matching_rule").unwrap();
        let result = path.resolve_against_schema(&fields).unwrap();
        assert!(
            matches!(result, PathResolution::StructuralOnly),
            "expected StructuralOnly for free-form object ancestor"
        );
    }

    #[test]
    fn test_resolve_structural_only_under_reference_field() {
        // A `Reference("...")` field survives when the parser can't inline
        // the referenced definition (depth limits or cycles). The leaf
        // schema isn't reachable, so any sub-path falls back to structural.
        let fields = vec![make_field(
            "dsHostConfiguration",
            BodyFieldType::Reference("DSHostConfiguration".into()),
            vec![],
        )];
        let path = FieldPath::parse("dsHostConfiguration.instanceId").unwrap();
        let result = path.resolve_against_schema(&fields).unwrap();
        assert!(
            matches!(result, PathResolution::StructuralOnly),
            "expected StructuralOnly for unresolved Reference ancestor"
        );
    }

    #[test]
    fn test_resolve_structural_only_deep_under_freeform_object() {
        // "data" is free-form — deeper path still resolves StructuralOnly.
        let fields = vec![make_field("data", BodyFieldType::Object, vec![])];
        let path = FieldPath::parse("data.matching_rule[0].attribute").unwrap();
        let result = path.resolve_against_schema(&fields).unwrap();
        assert!(
            matches!(result, PathResolution::StructuralOnly),
            "expected StructuralOnly for deep path under free-form object"
        );
    }

    #[test]
    fn test_resolve_unknown_segment_errors() {
        // Typed parent without the named child.
        let fields = vec![make_field(
            "dsHostConfiguration",
            BodyFieldType::Object,
            vec![make_field("instanceId", BodyFieldType::String, vec![])],
        )];
        let path = FieldPath::parse("dsHostConfiguration.notReal").unwrap();
        let err = path.resolve_against_schema(&fields).unwrap_err();
        assert!(
            matches!(err, PathResolveError::UnknownSegment { ref segment, .. } if segment == "notReal"),
            "expected UnknownSegment for notReal: {err:?}"
        );
    }

    #[test]
    fn test_resolve_segment_under_non_object_errors() {
        // "instanceId" is a String — cannot traverse into it.
        let fields = vec![make_field(
            "dsHostConfiguration",
            BodyFieldType::Object,
            vec![make_field("instanceId", BodyFieldType::String, vec![])],
        )];
        let path = FieldPath::parse("dsHostConfiguration.instanceId.nested").unwrap();
        let err = path.resolve_against_schema(&fields).unwrap_err();
        assert!(
            matches!(err, PathResolveError::SegmentUnderNonObject { ref segment, .. } if segment == "instanceId"),
            "expected SegmentUnderNonObject for instanceId: {err:?}"
        );
    }

    #[test]
    fn test_resolve_index_at_end_of_path_is_structural() {
        // Path ends with an Index segment — element bindings are structural.
        let fields = vec![make_field(
            "regions",
            BodyFieldType::Array(Box::new(BodyFieldType::Object)),
            vec![make_field("region", BodyFieldType::String, vec![])],
        )];
        let path = FieldPath::parse("regions[0]").unwrap();
        let result = path.resolve_against_schema(&fields).unwrap();
        assert!(
            matches!(result, PathResolution::StructuralOnly),
            "expected StructuralOnly for Index-ending path"
        );
    }

    #[test]
    fn test_resolve_array_index_under_empty_children_is_structural() {
        // Array field with no children — index traversal is structural-only.
        let fields = vec![make_field(
            "regions",
            BodyFieldType::Array(Box::new(BodyFieldType::Object)),
            vec![],
        )];
        let path = FieldPath::parse("regions[0].region").unwrap();
        let result = path.resolve_against_schema(&fields).unwrap();
        assert!(
            matches!(result, PathResolution::StructuralOnly),
            "expected StructuralOnly for array with no children"
        );
    }
}
