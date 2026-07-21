//! Surface-neutral helpers for dynamic-enum (picker) fields: dependency checks,
//! the resolution cache key, choice storage, and applying a picked value. Shared
//! by the fullscreen picker modal and the inline picker sub-loop so both reason
//! about `DynamicEnumState` identically.

use std::collections::BTreeMap;

use ags_protocol::workflow::{OptionChoice, ResolvedOptions};

use crate::frontend::coerce_to_schema;
use crate::frontend::terminal::inline::form::{
    is_field_filled, FieldKey, FieldSource, FieldValue, Form, FormField, ResolvedChoices,
};

/// Braille spinner frames for the "loading choices…" animation, shared by the
/// fullscreen resolver (`ProductionResolver`) and the inline picker sub-loop so
/// the two animate identically.
pub(crate) const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Whether the picker can open, and with what choices.
pub(crate) enum PickerAction {
    /// Resolved (empty or not) → open the modal. An empty result opens a
    /// "no matches" picker so the user can revise their search or type a value.
    Open {
        choices: Vec<OptionChoice>,
        truncated: bool,
    },
    /// Unresolved (deps-missing or a fetch error) → do nothing; the validation
    /// note set by `resolve_dynamic_enum_field` stands.
    Blocked,
}

/// Build the dependency-value tuple for a `DynamicEnum`'s `deps`, coerced with
/// the same `coerce_to_schema` as `project_inputs` so the key is canonical
/// (never a `"4"`/`4` mismatch). Missing/empty dependency fields are omitted.
pub(crate) fn compute_dep_key(form: &Form, deps: &[String]) -> BTreeMap<String, serde_json::Value> {
    let mut key = BTreeMap::new();
    for dep in deps {
        if let Some(field) = form.fields.iter().find(|f| match &f.key {
            FieldKey::Input(n) => n == dep,
            _ => false,
        }) {
            if let Some(raw) = field_buffer(field) {
                key.insert(dep.clone(), coerce_to_schema(&raw, &field.schema));
            }
        }
    }
    key
}

/// The editable string of a field, if filled.
// Intentional local mirror of form.rs::field_value_string (kept here to avoid
// widening that module's API). Keep the two in sync if FieldValue gains variants.
pub(crate) fn field_buffer(field: &FormField) -> Option<String> {
    match &field.value {
        FieldValue::Scalar(s) | FieldValue::JsonBody(s) if !s.is_empty() => Some(s.clone()),
        FieldValue::Enum(Some(s)) => Some(s.clone()),
        FieldValue::Bool(Some(b)) => Some(b.to_string()),
        _ => None,
    }
}

/// Whether every dependency field is filled (`is_field_filled`).
pub(crate) fn deps_satisfied(form: &Form, deps: &[String]) -> bool {
    deps.iter().all(|dep| {
        form.fields
            .iter()
            .find(|f| matches!(&f.key, FieldKey::Input(n) if n == dep))
            .map(is_field_filled)
            .unwrap_or(false)
    })
}

/// Apply a resolver result to a `DynamicEnum` field's cache for the given
/// dependency key. Only `Ok` results are cached; the caller drops `Err`/cancel.
pub(crate) fn store_resolved(
    field: &mut FormField,
    dep_key: BTreeMap<String, serde_json::Value>,
    resolved: ResolvedOptions,
) {
    if let Some(state) = field.dynamic.as_mut() {
        state.resolved = Some(ResolvedChoices {
            dep_key,
            choices: resolved.choices,
            truncated: resolved.truncated,
        });
    }
}

/// Decide what to do when opening the picker for field `idx`: open the modal
/// (even for an empty result, so the dialog shows "no matches" rather than
/// silently never appearing), or stay blocked when the field is unresolved.
pub(crate) fn picker_action(form: &Form, idx: usize) -> PickerAction {
    match form.fields[idx]
        .dynamic
        .as_ref()
        .and_then(|d| d.resolved.as_ref())
    {
        Some(r) => PickerAction::Open {
            choices: r.choices.clone(),
            truncated: r.truncated,
        },
        None => PickerAction::Blocked,
    }
}

/// Apply a modal outcome to field `idx`: write the chosen value (a choice value
/// or a typed custom value), or leave the field untouched on cancel (`None`).
pub(crate) fn apply_picker_result(form: &mut Form, idx: usize, result: Option<String>) {
    if let Some(value) = result {
        form.fields[idx].value = FieldValue::Enum(Some(value));
        form.fields[idx].source = FieldSource::UserInput;
    }
}
