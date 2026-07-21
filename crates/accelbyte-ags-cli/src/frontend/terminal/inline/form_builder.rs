//! Build a Form's field list from workflow gather slots + already-supplied inputs.
//!
//! `build_form_fields` turns the runtime's `WorkflowInputNeeded` slots (things
//! the user must fill in) and `SuppliedInputView` values (things already known
//! but editable) into the surface-agnostic `FormField` list that `Form` renders.
//!
//! Field **type** and schema always come from the schema, never the value, so
//! that coercion on submit is correct regardless of how the value was pre-filled.

use ags_protocol::workflow::{
    GatherResult, SuppliedInputView, SuppliedSource, WorkflowInputNeeded, WorkflowInputSpec,
};
use ags_runtime::support::strings::to_kebab_case;
use std::collections::BTreeMap;

use crate::frontend::terminal::inline::form::{
    DynamicEnumState, FieldKey, FieldSource, FieldType, FieldValue, FormField,
};

/// Build the `FormField` list from the runtime's gather request.
///
/// `needed` slots become `FieldKey::Slot` fields (empty, awaiting user input).
/// `supplied` inputs become `FieldKey::Input` fields (pre-filled, editable).
/// Field type and schema always derive from the schema, never the value.
pub fn build_form_fields(
    needed: &[WorkflowInputNeeded],
    supplied: &[SuppliedInputView],
) -> Vec<FormField> {
    let mut fields = Vec::new();
    for slot in needed {
        let field_type = schema_to_field_type(&slot.schema);
        // Strict: pre-fill only when the spec defines a default. Fields without
        // a spec default (incl. required booleans/enums) start unset — the user
        // must make a deliberate choice before submit. No fabricated values.
        let value = match slot.default.as_ref() {
            Some(v) => value_to_field_value(v),
            None => FieldValue::Empty,
        };
        fields.push(FormField {
            label: slot.label.clone(),
            field_type,
            required: slot.required,
            value,
            description: slot.description.clone().unwrap_or_default(),
            source: FieldSource::UserInput,
            key: FieldKey::Slot(slot.id),
            schema: slot.schema.clone(),
            read_only: false,
            dynamic: None,
        });
    }
    for sup in supplied {
        fields.push(FormField {
            // Field type + schema come from the input's schema, NOT the
            // value — supplied fields are editable and must coerce edits
            // per type (integer/bool/object), and a JsonBody supplied
            // field must re-open the tree editor.
            label: sup.label.clone(),
            field_type: schema_to_field_type(&sup.schema),
            // Supplied fields are always editable-but-optional: they already
            // hold a value, so required-input validation applies only to the
            // missing `needed` slots above.
            required: false,
            value: value_to_field_value(&sup.value),
            description: sup.description.clone().unwrap_or_default(),
            source: match sup.source {
                SuppliedSource::FromFlag => FieldSource::FromFlag,
                SuppliedSource::Default => FieldSource::Default,
            },
            key: FieldKey::Input(sup.label.clone()),
            schema: sup.schema.clone(),
            read_only: false,
            dynamic: None,
        });
    }
    fields
}

/// Build the Phase-1 inputs form: one field per declared workflow input.
/// Required inputs stay `required: true` even when prefilled (so clearing one
/// re-blocks submit); optional inputs are prefilled with their current value
/// (flag or default) and editable.
pub(crate) fn build_inputs_form(
    specs: &[WorkflowInputSpec],
    current: &BTreeMap<String, serde_json::Value>,
    dynamic_enums: bool,
) -> Vec<FormField> {
    specs
        .iter()
        .map(|spec| {
            let schema = spec
                .schema
                .clone()
                .unwrap_or_else(|| serde_json::json!({"type": "string"}));

            // A string-typed input with an options_source becomes a DynamicEnum
            // (a modal picker), but only when the surface can drive one — the
            // fullscreen surface passes `dynamic_enums: true`. The inline surface
            // has no picker, so it passes `false` and the input falls back to a
            // plain editable text field. Every other input keeps its
            // schema-derived field type. The DynamicEnum carrier is
            // FieldValue::Enum (holds the value string).
            let is_string = schema.get("type").and_then(|t| t.as_str()) == Some("string");
            let (field_type, dynamic, value) = if let (true, true, Some(source)) =
                (dynamic_enums, is_string, spec.options_source.as_ref())
            {
                let mut deps: Vec<String> = Vec::new();
                let mut optional_deps: Vec<String> = Vec::new();
                for b in source.parameters.values() {
                    match b {
                        ags_protocol::workflow::OptionParameterBinding::FromInput(n) => {
                            deps.push(n.clone())
                        }
                        ags_protocol::workflow::OptionParameterBinding::FromInputOptional(n) => {
                            optional_deps.push(n.clone())
                        }
                        ags_protocol::workflow::OptionParameterBinding::Literal(_) => {}
                    }
                }
                let value = match current.get(&spec.name) {
                    Some(serde_json::Value::String(s)) => FieldValue::Enum(Some(s.clone())),
                    _ => FieldValue::Enum(None),
                };
                (
                    FieldType::DynamicEnum,
                    Some(DynamicEnumState {
                        source: source.clone(),
                        deps,
                        optional_deps,
                        resolved: None,
                    }),
                    value,
                )
            } else {
                let field_type = schema_to_field_type(&schema);
                // A declared workflow input authored as `format: date-time`
                // gets the segmented date widget. This gate lives here (built
                // from the WorkflowInputSpec), NOT in the shared
                // `schema_to_field_type`, so service commands and auto-derived
                // fields with the same format are unaffected.
                let field_type = if is_string
                    && schema.get("format").and_then(|f| f.as_str()) == Some("date-time")
                {
                    FieldType::DateTime
                } else {
                    field_type
                };
                let value = match current.get(&spec.name) {
                    // Coerce to the field-type carrier so cycle/toggle work on
                    // the first key press. A bool/enum default arrives as a JSON
                    // string/bool; stored as a plain `Scalar`, the first space
                    // press would look like a no-op (toggle/cycle treats a
                    // non-`Bool`/`Enum` carrier as unset).
                    Some(v) => match (&field_type, v) {
                        (FieldType::Bool, serde_json::Value::Bool(b)) => FieldValue::Bool(Some(*b)),
                        (FieldType::Enum { .. }, serde_json::Value::String(s)) => {
                            FieldValue::Enum(Some(s.clone()))
                        }
                        _ => value_to_field_value(v),
                    },
                    None => match field_type {
                        FieldType::JsonBody => FieldValue::JsonBody(String::new()),
                        _ => FieldValue::Empty,
                    },
                };
                (field_type, None, value)
            };

            FormField {
                // Display label is kebab-cased for consistency with per-step
                // review labels and CLI flags; the FieldKey::Input below keeps
                // the raw camelCase name so the projection still matches the
                // workflow input spec.
                label: to_kebab_case(&spec.name),
                field_type,
                required: spec.required,
                value,
                description: spec.description.clone().unwrap_or_default(),
                source: FieldSource::UserInput,
                key: FieldKey::Input(spec.name.clone()),
                schema,
                read_only: false,
                dynamic,
            }
        })
        .collect()
}

/// Build the inline single-command form's full request surface: one field per
/// declared workflow input (every param + top-level body property, required and
/// optional). Values seed from `supplied` (flag/default), else the input's own
/// declared default, else empty. Every field is `FieldKey::Input(name)` so edits
/// project to `input_overrides`, which the executor folds into `workflow_supplied`
/// (synthesised single-command steps have only WorkflowInput-scoped fields, so no
/// slot routing is needed). Labels are kebab-cased; the key keeps the raw name.
///
/// Unlike `build_inputs_form`, this does not special-case `options_source` into a
/// `DynamicEnum` field: synthesised single-command inputs never carry an
/// `options_source` (it is only set on authored registry workflows), so every
/// field here takes its schema-derived type with `dynamic: None`.
pub(crate) fn build_full_surface_fields(
    full_inputs: &[ags_protocol::workflow::WorkflowInputSpec],
    supplied: &[SuppliedInputView],
) -> Vec<FormField> {
    let supplied_by_name: BTreeMap<&str, &SuppliedInputView> =
        supplied.iter().map(|s| (s.label.as_str(), s)).collect();

    full_inputs
        .iter()
        .map(|spec| {
            let schema = spec
                .schema
                .clone()
                .unwrap_or_else(|| serde_json::json!({"type": "string"}));
            let field_type = schema_to_field_type(&schema);
            // Single lookup: value seed and source provenance both come from the
            // same supplied entry, so resolve it once.
            let entry = supplied_by_name.get(spec.name.as_str()).copied();
            // A flag-supplied value always seeds. The spec default seeds scalars,
            // but NOT JSON bodies: a body's declared default is often a one-element
            // template (e.g. `[{}]`) that would pre-fill the field with a value the
            // user never chose. The fullscreen gather leaves body fields empty, so
            // match it here — a body field starts empty unless a flag supplied one.
            let seeded = entry.map(|s| s.value.clone()).or_else(|| {
                if matches!(field_type, FieldType::JsonBody) {
                    None
                } else {
                    spec.default.clone()
                }
            });
            let value = match seeded {
                Some(v) => value_to_field_value(&v),
                None => match field_type {
                    FieldType::JsonBody => FieldValue::JsonBody(String::new()),
                    _ => FieldValue::Empty,
                },
            };
            let source = match entry.map(|s| s.source) {
                Some(SuppliedSource::FromFlag) => FieldSource::FromFlag,
                Some(SuppliedSource::Default) => FieldSource::Default,
                None => FieldSource::UserInput,
            };
            FormField {
                label: to_kebab_case(&spec.name),
                field_type,
                required: spec.required,
                value,
                description: spec.description.clone().unwrap_or_default(),
                source,
                key: FieldKey::Input(spec.name.clone()),
                schema,
                read_only: false,
                dynamic: None,
            }
        })
        .collect()
}

/// Map a JSON schema object to the `FieldType` used for rendering and coercion.
///
/// Rules (in priority order):
/// - presence of `"enum"` array → `Enum { variants }` (string elements only)
/// - `"type": "object"` or `"array"`, or presence of `"properties"` → `JsonBody`
/// - `"type": "boolean"` → `Bool`
/// - everything else (string, integer, number, unknown) → `Scalar`
pub(crate) fn schema_to_field_type(schema: &serde_json::Value) -> FieldType {
    // Enum check first — an enum may also carry "type": "string".
    if let Some(enum_arr) = schema.get("enum").and_then(|v| v.as_array()) {
        let variants = enum_arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        return FieldType::Enum { variants };
    }
    let type_str = schema.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if type_str == "object" || type_str == "array" || schema.get("properties").is_some() {
        return FieldType::JsonBody;
    }
    if type_str == "boolean" {
        return FieldType::Bool;
    }
    FieldType::Scalar
}

/// Map a `serde_json::Value` to a pre-filled `FieldValue`.
///
/// Type always comes from the schema (see `schema_to_field_type`); this helper
/// only handles the *value* pre-fill for display purposes.
pub(crate) fn value_to_field_value(value: &serde_json::Value) -> FieldValue {
    match value {
        serde_json::Value::String(s) => FieldValue::Scalar(s.clone()),
        serde_json::Value::Bool(b) => FieldValue::Bool(Some(*b)),
        serde_json::Value::Number(n) => FieldValue::Scalar(n.to_string()),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
            FieldValue::JsonBody(serde_json::to_string_pretty(value).unwrap_or_default())
        }
        serde_json::Value::Null => FieldValue::Empty,
    }
}

/// Re-seed a form field list from a previous gather `result` so the user's
/// edits survive a "Back to edit" round trip. Starts from a fresh
/// [`build_form_fields`] list, then overwrites each field's value from
/// `result` by [`FieldKey`]: slot fields take `result.slot_values`, input
/// fields take `result.input_overrides`.
pub(crate) fn reseed_fields(
    needed: &[WorkflowInputNeeded],
    supplied: &[SuppliedInputView],
    result: &GatherResult,
) -> Vec<FormField> {
    let mut fields = build_form_fields(needed, supplied);
    for field in &mut fields {
        match &field.key {
            FieldKey::Slot(id) => {
                if let Some(v) = result.slot_values.get(id) {
                    field.value = value_to_field_value(v);
                }
            }
            FieldKey::Input(name) => {
                if let Some(v) = result.input_overrides.get(name) {
                    field.value = value_to_field_value(v);
                }
            }
            // Review fields are built/reseeded via the per-step review path,
            // not the gather path that drives `reseed_fields`.
            FieldKey::Review(_) => {}
        }
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::workflow::{
        AutoDeriveScope, GatherSlotId, SuppliedInputView, SuppliedSource, WorkflowInputNeeded,
    };

    #[test]
    fn test_build_inputs_form_keeps_required_even_when_prefilled() {
        use ags_protocol::workflow::WorkflowInputSpec;
        use std::collections::BTreeMap;
        let specs = vec![
            WorkflowInputSpec {
                name: "namespace".into(),
                description: None,
                schema: Some(serde_json::json!({"type":"string"})),
                required: true,
                default: None,
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
            },
            WorkflowInputSpec {
                name: "fleetName".into(),
                description: None,
                schema: Some(serde_json::json!({"type":"string"})),
                required: false,
                default: Some(serde_json::json!("ranked-fleet")),
                sensitive: false,
                options_source: None,
                location: ags_protocol::workflow::StepFieldLocation::Body,
            },
        ];
        let mut current = BTreeMap::new();
        current.insert("namespace".to_string(), serde_json::json!("dev")); // prefilled by flag
        current.insert("fleetName".to_string(), serde_json::json!("ranked-fleet")); // default
        let fields = build_inputs_form(&specs, &current, true);
        let ns = fields.iter().find(|f| f.label == "namespace").unwrap();
        assert!(
            ns.required,
            "required input stays required even when prefilled"
        );
        let fleet = fields.iter().find(|f| f.label == "fleet-name").unwrap();
        assert!(!fleet.required, "optional input is not required");
        // Label kebab-cases for display, but the key keeps the raw input name.
        assert!(matches!(&fleet.key, FieldKey::Input(n) if n == "fleetName"));
    }

    #[test]
    fn test_build_inputs_form_preserves_given_spec_order() {
        use ags_protocol::workflow::WorkflowInputSpec;
        use std::collections::BTreeMap;
        // The upfront Phase-1 form must keep inputs in the order it is GIVEN.
        // The workflow executor orders them by step-of-first-use
        // (`order_inputs_by_first_use`) so the form follows execution flow; a
        // sort here (alphabetical or by location) would silently override that.
        let spec = |name: &str| WorkflowInputSpec {
            name: name.into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
        };
        // Deliberately neither alphabetical nor location-sorted.
        let specs = vec![spec("namespace"), spec("fleetRegion"), spec("alpha")];
        let fields = build_inputs_form(&specs, &BTreeMap::new(), false);
        let order: Vec<&str> = fields
            .iter()
            .filter_map(|f| match &f.key {
                FieldKey::Input(n) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            order,
            vec!["namespace", "fleetRegion", "alpha"],
            "build_inputs_form must preserve the given spec order, not sort"
        );
    }

    #[test]
    fn test_build_full_surface_fields_includes_optional_and_seeds_supplied() {
        use ags_protocol::workflow::{
            StepFieldLocation, SuppliedInputView, SuppliedSource, WorkflowInputSpec,
        };
        let full_inputs = vec![
            WorkflowInputSpec {
                name: "namespace".into(),
                description: None,
                schema: Some(serde_json::json!({"type": "string"})),
                required: true,
                default: None,
                sensitive: false,
                options_source: None,
                location: StepFieldLocation::Path,
            },
            WorkflowInputSpec {
                name: "clientName".into(),
                description: None,
                schema: Some(serde_json::json!({"type": "string"})),
                required: false,
                default: None,
                sensitive: false,
                options_source: None,
                location: StepFieldLocation::Body,
            },
        ];
        // namespace already supplied via flag.
        let supplied = vec![SuppliedInputView {
            label: "namespace".into(),
            value: serde_json::json!("acme"),
            schema: serde_json::json!({"type": "string"}),
            description: None,
            source: SuppliedSource::FromFlag,
            location: StepFieldLocation::Path,
        }];
        let fields = build_full_surface_fields(&full_inputs, &supplied);
        assert_eq!(fields.len(), 2, "every input becomes a field");
        let ns = fields.iter().find(|f| f.label == "namespace").unwrap();
        assert!(matches!(&ns.value, FieldValue::Scalar(s) if s == "acme"));
        assert_eq!(ns.source, FieldSource::FromFlag);
        assert!(ns.required, "required flag preserved from spec");
        // optional clientName appears as an empty Input-keyed field.
        let client = fields.iter().find(|f| f.label == "client-name").unwrap();
        assert!(matches!(client.value, FieldValue::Empty));
        assert!(!client.required);
        assert!(matches!(&client.key, FieldKey::Input(n) if n == "clientName"));
    }

    #[test]
    fn test_build_full_surface_fields_does_not_seed_body_default_template() {
        use ags_protocol::workflow::{StepFieldLocation, WorkflowInputSpec};
        // A body field with a declared `[{}]` template default must start EMPTY,
        // matching the fullscreen gather (which drops body-field defaults). A
        // scalar default still seeds.
        let full_inputs = vec![
            WorkflowInputSpec {
                name: "regions".into(),
                description: None,
                schema: Some(serde_json::json!({"type": "array", "items": {"type": "object"}})),
                required: false,
                default: Some(serde_json::json!([{}])),
                sensitive: false,
                options_source: None,
                location: StepFieldLocation::Body,
            },
            WorkflowInputSpec {
                name: "limit".into(),
                description: None,
                schema: Some(serde_json::json!({"type": "integer"})),
                required: false,
                default: Some(serde_json::json!(20)),
                sensitive: false,
                options_source: None,
                location: StepFieldLocation::Query,
            },
        ];
        let fields = build_full_surface_fields(&full_inputs, &[]);
        let regions = fields.iter().find(|f| f.label == "regions").unwrap();
        assert!(
            matches!(&regions.value, FieldValue::JsonBody(s) if s.is_empty()),
            "body field must not be seeded with the [{{}}] template default: {:?}",
            regions.value
        );
        // A scalar default is still honoured.
        let limit = fields.iter().find(|f| f.label == "limit").unwrap();
        assert!(
            matches!(&limit.value, FieldValue::Scalar(s) if s == "20"),
            "scalar default still seeds: {:?}",
            limit.value
        );
    }

    #[test]
    fn test_build_form_fields_needed_is_slot_supplied_is_input() {
        let needed = vec![WorkflowInputNeeded {
            id: GatherSlotId(0),
            label: "title".into(),
            description: None,
            schema: serde_json::json!({"type": "string"}),
            default: None,
            required: true,
            sensitive: false,
            scope: AutoDeriveScope::StepLocal {
                field_name: "title".into(),
            },
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }];
        let supplied = vec![SuppliedInputView {
            label: "namespace".into(),
            value: serde_json::json!("my-ns"),
            schema: serde_json::json!({"type": "string"}),
            description: None,
            source: SuppliedSource::FromFlag,
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }];
        let fields = build_form_fields(&needed, &supplied);
        let title = fields.iter().find(|f| f.label == "title").unwrap();
        let ns = fields.iter().find(|f| f.label == "namespace").unwrap();
        assert!(matches!(title.key, FieldKey::Slot(_)));
        assert!(matches!(&ns.key, FieldKey::Input(n) if n == "namespace"));
        assert_eq!(ns.source, FieldSource::FromFlag);
        assert!(matches!(ns.value, FieldValue::Scalar(ref s) if s == "my-ns"));
    }

    #[test]
    fn test_supplied_integer_field_round_trips_as_integer() {
        let supplied = vec![SuppliedInputView {
            label: "count".into(),
            value: serde_json::json!(7),
            schema: serde_json::json!({"type": "integer"}),
            description: None,
            source: SuppliedSource::FromFlag,
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }];
        let fields = build_form_fields(&[], &supplied);
        let count = fields.iter().find(|f| f.label == "count").unwrap();
        assert!(matches!(count.field_type, FieldType::Scalar));
        // project via a Form: the value coerces back to an integer, not a string.
        let form = crate::frontend::terminal::inline::form::Form::new("t", fields);
        let r = form.project_gathered();
        assert_eq!(r.input_overrides.get("count"), Some(&serde_json::json!(7)));
    }

    #[test]
    fn test_schema_to_field_type_object_becomes_json_body() {
        assert!(matches!(
            schema_to_field_type(&serde_json::json!({"type": "object"})),
            FieldType::JsonBody
        ));
    }

    #[test]
    fn test_schema_to_field_type_properties_without_type_becomes_json_body() {
        assert!(matches!(
            schema_to_field_type(&serde_json::json!({"properties": {"x": {}}})),
            FieldType::JsonBody
        ));
    }

    #[test]
    fn test_schema_to_field_type_boolean_becomes_bool() {
        assert!(matches!(
            schema_to_field_type(&serde_json::json!({"type": "boolean"})),
            FieldType::Bool
        ));
    }

    #[test]
    fn test_schema_to_field_type_enum_extracts_variants() {
        let ft = schema_to_field_type(&serde_json::json!({"enum": ["RANKED", "CASUAL"]}));
        let FieldType::Enum { variants } = ft else {
            panic!("expected enum")
        };
        assert_eq!(variants, vec!["RANKED", "CASUAL"]);
    }

    #[test]
    fn test_schema_to_field_type_scalar_fallback() {
        for schema in [
            serde_json::json!({"type": "string"}),
            serde_json::json!({"type": "integer"}),
            serde_json::json!({"type": "number"}),
            serde_json::json!({}),
        ] {
            assert!(
                matches!(schema_to_field_type(&schema), FieldType::Scalar),
                "expected Scalar for {schema}"
            );
        }
    }

    #[test]
    fn test_schema_to_field_type_array_becomes_json_body() {
        assert!(matches!(
            schema_to_field_type(&serde_json::json!({"type": "array"})),
            FieldType::JsonBody
        ));
    }

    #[test]
    fn test_value_to_field_value_covers_all_variants() {
        assert!(matches!(
            value_to_field_value(&serde_json::json!("hello")),
            FieldValue::Scalar(s) if s == "hello"
        ));
        assert!(matches!(
            value_to_field_value(&serde_json::json!(true)),
            FieldValue::Bool(Some(true))
        ));
        assert!(matches!(
            value_to_field_value(&serde_json::json!(42)),
            FieldValue::Scalar(s) if s == "42"
        ));
        assert!(matches!(
            value_to_field_value(&serde_json::json!({"k": "v"})),
            FieldValue::JsonBody(_)
        ));
        assert!(matches!(
            value_to_field_value(&serde_json::json!([1, 2, 3])),
            FieldValue::JsonBody(_)
        ));
        assert!(matches!(
            value_to_field_value(&serde_json::Value::Null),
            FieldValue::Empty
        ));
    }

    #[test]
    fn test_reseed_fields_applies_slot_value() {
        let needed = vec![WorkflowInputNeeded {
            id: GatherSlotId(0),
            label: "username".into(),
            description: None,
            schema: serde_json::json!({"type": "string"}),
            default: None,
            required: true,
            sensitive: false,
            scope: AutoDeriveScope::StepLocal {
                field_name: "username".into(),
            },
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }];
        let mut slot_values = std::collections::BTreeMap::new();
        slot_values.insert(GatherSlotId(0), serde_json::json!("alice"));
        let result = GatherResult {
            slot_values,
            input_overrides: std::collections::BTreeMap::new(),
        };
        let fields = reseed_fields(&needed, &[], &result);
        assert_eq!(fields.len(), 1);
        assert!(matches!(&fields[0].key, FieldKey::Slot(GatherSlotId(0))));
        assert!(
            matches!(&fields[0].value, FieldValue::Scalar(s) if s == "alice"),
            "expected Scalar(alice), got {:?}",
            fields[0].value
        );
    }

    #[test]
    fn test_needed_slot_has_empty_value_and_user_input_source() {
        let needed = vec![WorkflowInputNeeded {
            id: GatherSlotId(5),
            label: "stat-code".into(),
            description: Some("The stat code".into()),
            schema: serde_json::json!({"type": "string"}),
            default: None,
            required: true,
            sensitive: false,
            scope: AutoDeriveScope::WorkflowInput {
                name: "stat-code".into(),
            },
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }];
        let fields = build_form_fields(&needed, &[]);
        let f = &fields[0];
        assert!(matches!(f.value, FieldValue::Empty));
        assert_eq!(f.source, FieldSource::UserInput);
        assert!(f.required);
        assert_eq!(f.description, "The stat code");
        assert!(matches!(f.key, FieldKey::Slot(GatherSlotId(5))));
    }

    #[test]
    fn test_supplied_default_source_maps_to_field_source_default() {
        let supplied = vec![SuppliedInputView {
            label: "mode".into(),
            value: serde_json::json!("CASUAL"),
            schema: serde_json::json!({"enum": ["RANKED", "CASUAL"]}),
            description: None,
            source: SuppliedSource::Default,
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }];
        let fields = build_form_fields(&[], &supplied);
        assert_eq!(fields[0].source, FieldSource::Default);
        assert!(matches!(&fields[0].key, FieldKey::Input(n) if n == "mode"));
    }

    #[test]
    fn test_build_inputs_form_dynamic_enum_for_options_source() {
        use crate::frontend::terminal::inline::form::{DynamicEnumState, FieldType};
        use ags_protocol::workflow::{
            OperationReference, OptionParameterBinding, OptionsSource, WorkflowInputSpec,
        };
        let spec = WorkflowInputSpec {
            name: "fleetImageId".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: Some(OptionsSource {
                operation: OperationReference {
                    service: ags_protocol::catalogue::ServiceId::new("ams"),
                    operation: ags_protocol::catalogue::OperationId::new(
                        "ams/admin/images/v1/list",
                    ),
                },
                parameters: std::collections::BTreeMap::from([(
                    "namespace".to_string(),
                    OptionParameterBinding::FromInput("namespace".to_string()),
                )]),
                items_path: "$.images".into(),
                value: "$.id".into(),
                label: Some("$.name".into()),
                label_detail: None,
                fallback_description: None,
                filter: None,
            }),
            location: ags_protocol::workflow::StepFieldLocation::Body,
        };
        let fields = build_inputs_form(&[spec], &std::collections::BTreeMap::new(), true);
        let f = &fields[0];
        assert!(matches!(f.field_type, FieldType::DynamicEnum));
        let state: &DynamicEnumState = f.dynamic.as_ref().expect("dynamic state present");
        assert_eq!(state.deps, vec!["namespace".to_string()]);
    }

    #[test]
    fn test_build_inputs_form_plain_text_for_options_source_when_dynamic_disabled() {
        // With `dynamic_enums: false` (the inline surface, which has no picker) an
        // options_source input falls back to a plain editable text field rather
        // than a DynamicEnum the surface couldn't drive.
        use crate::frontend::terminal::inline::form::FieldType;
        use ags_protocol::workflow::{
            OperationReference, OptionParameterBinding, OptionsSource, WorkflowInputSpec,
        };
        let spec = WorkflowInputSpec {
            name: "fleetImageId".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: Some(OptionsSource {
                operation: OperationReference {
                    service: ags_protocol::catalogue::ServiceId::new("ams"),
                    operation: ags_protocol::catalogue::OperationId::new(
                        "ams/admin/images/v1/list",
                    ),
                },
                parameters: std::collections::BTreeMap::from([(
                    "namespace".to_string(),
                    OptionParameterBinding::FromInput("namespace".to_string()),
                )]),
                items_path: "$.images".into(),
                value: "$.id".into(),
                label: Some("$.name".into()),
                label_detail: None,
                fallback_description: None,
                filter: None,
            }),
            location: ags_protocol::workflow::StepFieldLocation::Body,
        };
        let fields = build_inputs_form(&[spec], &std::collections::BTreeMap::new(), false);
        assert!(
            matches!(fields[0].field_type, FieldType::Scalar),
            "options_source input is plain text when dynamic enums are disabled"
        );
        assert!(
            fields[0].dynamic.is_none(),
            "no dynamic-enum state attached"
        );
    }

    #[test]
    fn test_build_inputs_form_plain_string_when_no_options_source() {
        use crate::frontend::terminal::inline::form::FieldType;
        use ags_protocol::workflow::WorkflowInputSpec;
        let spec = WorkflowInputSpec {
            name: "namespace".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string"})),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
        };
        let fields = build_inputs_form(&[spec], &std::collections::BTreeMap::new(), true);
        assert!(matches!(fields[0].field_type, FieldType::Scalar));
        assert!(fields[0].dynamic.is_none());
    }

    #[test]
    fn test_build_inputs_form_enum_value_uses_enum_carrier() {
        // A supplied/default enum value must be stored as FieldValue::Enum, not
        // a plain Scalar, or the first space-press would not cycle (it would
        // re-select the same variant). Regression guard.
        use crate::frontend::terminal::inline::form::FieldValue;
        use ags_protocol::workflow::WorkflowInputSpec;
        let spec = WorkflowInputSpec {
            name: "searchBy".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string", "enum": ["a", "b", "c"]})),
            required: false,
            default: Some(serde_json::json!("a")),
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
        };
        let current =
            std::collections::BTreeMap::from([("searchBy".to_string(), serde_json::json!("a"))]);
        let fields = build_inputs_form(&[spec], &current, true);
        assert!(
            matches!(&fields[0].value, FieldValue::Enum(Some(s)) if s == "a"),
            "enum value carried as Enum, got {:?}",
            fields[0].value
        );
    }

    #[test]
    fn test_build_inputs_form_unset_jsonbody_uses_empty_jsonbody_value() {
        use ags_protocol::workflow::WorkflowInputSpec;
        use std::collections::BTreeMap;
        let specs = vec![WorkflowInputSpec {
            name: "imageDeploymentProfile".into(),
            description: None,
            schema: Some(
                serde_json::json!({"type":"object","properties":{"imageId":{"type":"string"}}}),
            ),
            required: true,
            default: None,
            sensitive: false,
            options_source: None,
            location: ags_protocol::workflow::StepFieldLocation::Body,
        }];
        let current: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        let fields = build_inputs_form(&specs, &current, true);
        let f = &fields[0];
        assert!(
            matches!(&f.value, FieldValue::JsonBody(s) if s.is_empty()),
            "unset JsonBody-typed input gets FieldValue::JsonBody(\"\"): {:?}",
            f.value
        );
    }

    #[test]
    fn test_build_inputs_form_datetime_for_format_date_time() {
        use ags_protocol::workflow::{StepFieldLocation, WorkflowInputSpec};
        let spec = WorkflowInputSpec {
            name: "start".into(),
            description: None,
            schema: Some(serde_json::json!({"type": "string", "format": "date-time"})),
            required: false,
            default: Some(serde_json::json!("2020-01-01T00:00:00Z")),
            sensitive: false,
            options_source: None,
            location: StepFieldLocation::Body,
        };
        let current = std::collections::BTreeMap::from([(
            "start".to_string(),
            serde_json::json!("2020-01-01T00:00:00Z"),
        )]);
        let fields = build_inputs_form(&[spec], &current, true);
        assert_eq!(fields[0].field_type, FieldType::DateTime);
        assert!(
            matches!(&fields[0].value, FieldValue::Scalar(s) if s == "2020-01-01T00:00:00Z"),
            "DateTime carrier is the ISO Scalar"
        );
    }

    #[test]
    fn test_schema_to_field_type_leaves_date_time_as_scalar() {
        // The shared mapper (service commands / full surface) must NOT activate
        // the widget — a format:date-time field stays Scalar there.
        assert!(matches!(
            schema_to_field_type(&serde_json::json!({"type": "string", "format": "date-time"})),
            FieldType::Scalar
        ));
    }
}
