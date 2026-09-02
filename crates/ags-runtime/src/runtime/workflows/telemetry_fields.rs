//! Builds a failed step's `input_fields` telemetry facts (§4.1 of the CLI
//! telemetry observability design). Turns a resolved `StepFieldPlan` into
//! the transmittable `Vec<StepInputField>`, applying the redaction boundary
//! that is always enforced regardless of workflow origin, and withholding
//! every value outright for an external (user-installed) workflow.

use std::collections::BTreeSet;

use ags_protocol::workflow::{StepField, StepFieldSource, StepInputField};

/// Cap, in bytes, on one field's serialized value before it is replaced by
/// [`OVERSIZED_VALUE_MARKER`]. Keeps a single failed-step event well clear of
/// PostHog's per-event size limits even with several large request bodies.
const MAX_INPUT_FIELD_VALUE_BYTES: usize = 2048;

/// Placeholder substituted for a field value whose serialized size exceeds
/// `MAX_INPUT_FIELD_VALUE_BYTES`. Deliberately not shaped like real data, so
/// it can never be misread as a truncated-but-genuine value.
const OVERSIZED_VALUE_MARKER: &str = "<oversized-value-omitted>";

/// Case-insensitive substrings that mark a field name as sensitive, checked
/// regardless of workflow origin or declared schema. Widened 2026-08-20 (see
/// §4.1 of the design doc) from the original six —
/// `secret|password|token|key|credential|auth` — to fourteen, adding
/// `passwd|pwd|bearer|signature|session|cookie|salt|private`.
///
/// Matching is a plain case-insensitive **substring** check (see
/// [`is_sensitive_name`]) — not a whole-word or exact match — and that is
/// deliberate, not an oversight to "fix" by narrowing it later. A wider
/// needle list means more incidental over-matching, and that is accepted as
/// the safe direction for a security boundary:
///
/// - `key` also matches `keyword` and `monkey`.
/// - `private` also matches `privateNote`.
/// - `session` also matches `sessionRegion`.
///
/// The consequence is more than extra top-level withholding. When a matching
/// name belongs to an object or array (not a scalar), [`redact_nested`]
/// hands its *entire subtree* to [`redact_all_leaves`], so every leaf beneath
/// it is redacted regardless of that leaf's own name — see
/// `test_redact_nested_container_named_session_redacts_whole_subtree` for a
/// worked example (a body field named `session` holding `{"id": 1, "region":
/// "us"}` now transmits neither `id` nor `region`, where a name that didn't
/// match would have sent both untouched). A future reader who finds a whole
/// object going dark because its container name happens to substring-match
/// should read that as this constant working as designed, not as a bug to
/// narrow away: losing some diagnostic detail from an over-matched container
/// is the accepted trade for redacting every credential-shaped key we did not
/// anticipate by exact name.
const SENSITIVE_NAME_NEEDLES: &[&str] = &[
    "secret",
    "password",
    "token",
    "key",
    "credential",
    "auth",
    "passwd",
    "pwd",
    "bearer",
    "signature",
    "session",
    "cookie",
    "salt",
    "private",
];

/// Placeholder substituted in place of a sensitive value found nested inside
/// an otherwise-transmitted body object or array. Top-level withholding uses
/// `StepInputField.value = None` instead; this marker is only for a redacted
/// key found while recursing into structure that is otherwise sent whole.
const NESTED_REDACTED_MARKER: &str = "<redacted>";

/// True when `name` matches the sensitive-name vocabulary, case-insensitively.
fn is_sensitive_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    SENSITIVE_NAME_NEEDLES
        .iter()
        .any(|needle| lower.contains(needle))
}

/// True when a JSON schema marks its value as write-only (`writeOnly: true`
/// or `format: "password"`) — the schema half of the redaction rule.
fn schema_marks_sensitive(schema: &serde_json::Value) -> bool {
    let write_only = schema
        .get("writeOnly")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let is_password_format =
        schema.get("format").and_then(serde_json::Value::as_str) == Some("password");
    write_only || is_password_format
}

/// Recursively redact every scalar leaf beneath `value` **in place**,
/// regardless of that leaf's own key name, while preserving the surrounding
/// object/array structure so the shape of what was withheld is still
/// visible. Invoked by [`redact_nested`] whenever a key's own name is
/// sensitive: a container key like `auth` or `credentials` is trusted to
/// mark everything nested beneath it as sensitive by association, so a
/// non-matching leaf under it (a bearer `blob`, a `user` field alongside a
/// `clientSecret`) must not slip through just because its own name is
/// harmless.
fn redact_all_leaves(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for nested in map.values_mut() {
                redact_all_leaves(nested);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                redact_all_leaves(item);
            }
        }
        _ => {
            *value = serde_json::Value::String(NESTED_REDACTED_MARKER.to_string());
        }
    }
}

/// Recursively redact sensitive keys within a JSON value **in place**,
/// walking every nested object and array. This is the load-bearing half of
/// the redaction boundary: because whole body objects and arrays are
/// transmitted, a sensitive key nested under an unrelated parent (or inside
/// an array of objects) must be caught too, or nesting becomes a trivial
/// bypass of the top-level name rule.
///
/// A matching key redacts **everything beneath it** via
/// [`redact_all_leaves`] — not just its own value if it happens to be a
/// scalar, and not only the leaves beneath it that themselves match the
/// vocabulary. A container is what it is named: once a key's own name marks
/// it sensitive (`auth`, `credentials`, ...), descending into it and
/// redacting only the leaves that separately re-match would transmit a
/// nested value like `{"auth": {"blob": "eyJ..."}}`'s `blob` in full,
/// because `blob` itself never matches the vocabulary. Covering strictly
/// more than the leaf-only rule is the safe direction for a security
/// boundary; a non-matching key keeps the narrower per-leaf rule applied by
/// recursing here instead.
fn redact_nested(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, nested) in map.iter_mut() {
                if is_sensitive_name(key) {
                    redact_all_leaves(nested);
                } else {
                    redact_nested(nested);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                redact_nested(item);
            }
        }
        _ => {}
    }
}

/// Replace `value` with [`OVERSIZED_VALUE_MARKER`] when its serialized size
/// exceeds the cap; otherwise return it unchanged.
fn cap_value_size(value: serde_json::Value) -> serde_json::Value {
    let size = serde_json::to_vec(&value)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    if size > MAX_INPUT_FIELD_VALUE_BYTES {
        serde_json::Value::String(OVERSIZED_VALUE_MARKER.to_string())
    } else {
        value
    }
}

/// Apply the full redaction pipeline to one field's resolved value: withhold
/// it entirely when the field's own name or schema is sensitive, otherwise
/// recurse into any nested structure and then size-cap what remains.
fn redact_field_value(
    name: &str,
    schema: &serde_json::Value,
    mut value: serde_json::Value,
) -> Option<serde_json::Value> {
    if is_sensitive_name(name) || schema_marks_sensitive(schema) {
        return None;
    }
    redact_nested(&mut value);
    Some(cap_value_size(value))
}

/// Map a resolved `StepFieldSource` to the closed telemetry provenance
/// vocabulary (`flag|prompt|default|prior_output|literal|derived|unset`).
/// `flag_names` distinguishes a CLI-flag-supplied workflow input from a
/// prompted one — both resolve to the same `StepFieldSource::WorkflowInput`
/// variant, since `SuppliedSource` has no `FromPrompt` case (see the design's
/// §6.5 note on why that's out of scope).
fn source_label(source: &StepFieldSource, flag_names: &BTreeSet<String>) -> &'static str {
    match source {
        StepFieldSource::WorkflowInput { name } => {
            if flag_names.contains(name) {
                "flag"
            } else {
                "prompt"
            }
        }
        StepFieldSource::Default { .. } => "default",
        StepFieldSource::PriorOutput => "prior_output",
        StepFieldSource::Literal => "literal",
        StepFieldSource::Unset => "unset",
        StepFieldSource::Derived { .. } => "derived",
    }
}

/// True when the operator kill switch
/// ([`crate::runtime::telemetry::ENV_TELEMETRY_NO_INPUT_VALUES`]) is set to a
/// non-empty value. Lets a security reviewer disable `input_fields` value
/// transmission without disabling telemetry outright — every other property
/// on `cli.workflow.step_completed` (and every other event) is unaffected.
fn input_values_disabled_by_env() -> bool {
    crate::runtime::config::is_env_var_set(crate::runtime::telemetry::ENV_TELEMETRY_NO_INPUT_VALUES)
}

/// Build the `input_fields` telemetry facts for one failed step's resolved
/// field plan. `bundled` gates real values: a bundled workflow's fields are
/// redacted-but-real; an external (user-installed) workflow's fields all
/// report `value: None`, since AccelByte does not author its schema. The
/// operator kill switch ([`input_values_disabled_by_env`]) overrides
/// `bundled` the same way: when set, every field reports `value: None`
/// exactly as an external workflow's would. The redaction rules in
/// `redact_field_value` apply unconditionally whenever a value is
/// transmitted at all.
pub(crate) fn build_step_input_fields(
    plan_fields: &[StepField],
    flag_names: &BTreeSet<String>,
    bundled: bool,
) -> Vec<StepInputField> {
    let transmit_values = bundled && !input_values_disabled_by_env();
    plan_fields
        .iter()
        .map(|field| {
            let value = if transmit_values {
                redact_field_value(&field.field, &field.schema, field.value.clone())
            } else {
                None
            };
            StepInputField {
                field: field.field.clone(),
                location: field.location,
                source: source_label(&field.source, flag_names),
                required: field.required,
                value,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::workflow::{StepFieldId, StepFieldLocation};

    /// Build a minimal `StepField` for a body field with the given name,
    /// value and source — the parts these tests vary.
    fn body_field(name: &str, value: serde_json::Value, source: StepFieldSource) -> StepField {
        StepField {
            id: StepFieldId(0),
            field: name.to_string(),
            label: name.to_string(),
            description: None,
            location: StepFieldLocation::Body,
            schema: serde_json::json!({"type": "string"}),
            value,
            source,
            required: false,
            workflow_input: None,
            body_overflow: false,
            show_in_review: false,
        }
    }

    #[test]
    /// A sensitive field name at the top level is withheld outright.
    fn test_redact_field_value_withholds_sensitive_name() {
        let schema = serde_json::json!({"type": "string"});
        let result = redact_field_value("clientSecret", &schema, serde_json::json!("abc123"));
        assert_eq!(result, None);
    }

    #[test]
    /// A name-matching check is case-insensitive, per the design.
    fn test_is_sensitive_name_is_case_insensitive() {
        assert!(is_sensitive_name("ApiToken"));
        assert!(is_sensitive_name("PASSWORD"));
        assert!(!is_sensitive_name("storeName"));
    }

    #[test]
    /// Each of the eight needles added in the 2026-08-20 widening
    /// (`passwd|pwd|bearer|signature|session|cookie|salt|private`) is
    /// individually recognized as sensitive — table-driven so a future
    /// widening that forgets one needle fails on that needle specifically,
    /// not just on "something" in the vocabulary.
    fn test_is_sensitive_name_recognizes_each_widened_needle() {
        let cases = [
            ("userPasswd", "passwd"),
            ("dbPwd", "pwd"),
            ("bearerHeader", "bearer"),
            ("requestSignature", "signature"),
            ("sessionId", "session"),
            ("cookieValue", "cookie"),
            ("saltValue", "salt"),
            ("privateNote", "private"),
        ];
        for (name, needle) in cases {
            assert!(
                is_sensitive_name(name),
                "expected {name:?} to match widened needle {needle:?}"
            );
        }
    }

    #[test]
    /// Pins a deliberate over-match called out in the constant's doc comment:
    /// `keyword` contains the `key` needle even though it names nothing
    /// secret. This is intentional-by-test, not accidental — a future reader
    /// must not "fix" this by narrowing `key` to an exact-word match.
    fn test_redact_field_value_over_matches_keyword_by_design() {
        let schema = serde_json::json!({"type": "string"});
        let result = redact_field_value("keyword", &schema, serde_json::json!("hello"));
        assert_eq!(result, None);
    }

    #[test]
    /// The container-amplification consequence of widening the vocabulary:
    /// a body field named `session` — which did not match before this
    /// widening — now matches `session` and, because its value is an object,
    /// has every leaf beneath it redacted by `redact_all_leaves`, not just a
    /// leaf that separately re-matches the vocabulary. Before the widening
    /// both `id` and `region` would have been transmitted untouched.
    fn test_redact_nested_container_named_session_redacts_whole_subtree() {
        let mut value = serde_json::json!({
            "session": {"id": 1, "region": "us"},
        });
        redact_nested(&mut value);
        assert_eq!(
            value,
            serde_json::json!({
                "session": {"id": "<redacted>", "region": "<redacted>"},
            })
        );
    }

    #[test]
    /// A `writeOnly: true` schema withholds the value even with a harmless name.
    fn test_redact_field_value_withholds_write_only_schema() {
        let schema = serde_json::json!({"type": "string", "writeOnly": true});
        let result = redact_field_value("pin", &schema, serde_json::json!("1234"));
        assert_eq!(result, None);
    }

    #[test]
    /// A `format: "password"` schema withholds the value even with a harmless name.
    fn test_redact_field_value_withholds_password_format_schema() {
        let schema = serde_json::json!({"type": "string", "format": "password"});
        let result = redact_field_value("pin", &schema, serde_json::json!("1234"));
        assert_eq!(result, None);
    }

    #[test]
    /// The name rule recurses one level deep: a sensitive key nested inside a
    /// transmitted object is redacted in place, leaving the rest untouched.
    fn test_redact_nested_redacts_one_level_deep() {
        let mut value = serde_json::json!({
            "credentials": {"clientSecret": "abc"},
            "storeName": "acme",
        });
        redact_nested(&mut value);
        assert_eq!(
            value,
            serde_json::json!({
                "credentials": {"clientSecret": "<redacted>"},
                "storeName": "acme",
            })
        );
    }

    #[test]
    /// A sensitive-named key whose value is itself a container (object or
    /// array) is never nuked wholesale — the walk keeps descending through
    /// it so the actual leaf (`password`, three levels down here) still gets
    /// redacted, and everything above it survives.
    fn test_redact_nested_descends_through_sensitively_named_containers() {
        let mut value = serde_json::json!({
            "auth": {
                "credentials": {"password": "hunter2"}
            },
            "displayName": "Acme Store",
        });
        redact_nested(&mut value);
        assert_eq!(
            value,
            serde_json::json!({
                "auth": {
                    "credentials": {"password": "<redacted>"}
                },
                "displayName": "Acme Store",
            })
        );
    }

    #[test]
    /// A sensitive key nested two levels deep under a non-sensitive parent
    /// chain is still redacted (no top-level match short-circuits the walk).
    fn test_redact_nested_redacts_two_levels_deep_under_safe_parents() {
        let mut value = serde_json::json!({
            "storeConfig": {
                "billing": {"apiKey": "sk-live-abc"}
            }
        });
        redact_nested(&mut value);
        assert_eq!(
            value,
            serde_json::json!({
                "storeConfig": {
                    "billing": {"apiKey": "<redacted>"}
                }
            })
        );
    }

    #[test]
    /// The name rule recurses into an array of objects — each element is
    /// walked independently.
    fn test_redact_nested_redacts_inside_array_of_objects() {
        let mut value = serde_json::json!([
            {"name": "alice", "token": "t-1"},
            {"name": "bob", "token": "t-2"},
        ]);
        redact_nested(&mut value);
        assert_eq!(
            value,
            serde_json::json!([
                {"name": "alice", "token": "<redacted>"},
                {"name": "bob", "token": "<redacted>"},
            ])
        );
    }

    #[test]
    /// The exact leak this fix closes: a sensitive container's non-matching
    /// leaf (`blob`) must not be transmitted just because `blob` itself never
    /// matches the sensitive-name vocabulary.
    fn test_redact_nested_redacts_blob_beneath_sensitive_auth_container() {
        let mut value = serde_json::json!({
            "auth": {"blob": "eyJhbGci..."},
        });
        redact_nested(&mut value);
        assert_eq!(
            value,
            serde_json::json!({
                "auth": {"blob": "<redacted>"},
            })
        );
    }

    #[test]
    /// A sensitive container redacts a non-matching sibling leaf (`user`)
    /// beneath it too, not only the leaf that separately matches
    /// (`clientSecret`) — everything under a sensitive key is withheld.
    fn test_redact_nested_redacts_non_matching_sibling_under_sensitive_container() {
        let mut value = serde_json::json!({
            "credentials": {"user": "x", "clientSecret": "y"},
        });
        redact_nested(&mut value);
        assert_eq!(
            value,
            serde_json::json!({
                "credentials": {"user": "<redacted>", "clientSecret": "<redacted>"},
            })
        );
    }

    #[test]
    /// Two levels of containers beneath a sensitive key: every leaf at every
    /// depth is redacted, not just the first level.
    fn test_redact_nested_redacts_all_leaves_two_containers_deep() {
        let mut value = serde_json::json!({
            "auth": {"session": {"cookie": "abc", "expiresIn": 3600}},
        });
        redact_nested(&mut value);
        assert_eq!(
            value,
            serde_json::json!({
                "auth": {"session": {"cookie": "<redacted>", "expiresIn": "<redacted>"}},
            })
        );
    }

    #[test]
    /// A sensitive key holding an array of objects: every leaf in every
    /// element is redacted, regardless of that leaf's own key name.
    fn test_redact_nested_redacts_all_leaves_in_array_under_sensitive_key() {
        let mut value = serde_json::json!({
            "auth": [
                {"t": "x", "scope": "read"},
                {"t": "y", "scope": "write"},
            ],
        });
        redact_nested(&mut value);
        assert_eq!(
            value,
            serde_json::json!({
                "auth": [
                    {"t": "<redacted>", "scope": "<redacted>"},
                    {"t": "<redacted>", "scope": "<redacted>"},
                ],
            })
        );
    }

    #[test]
    /// A non-sensitive container is unaffected by the new all-leaves rule:
    /// a matching leaf inside it still redacts individually, and a
    /// non-matching leaf inside it still transmits — today's narrower
    /// per-leaf behaviour is preserved outside a sensitive container.
    fn test_redact_nested_non_sensitive_container_keeps_per_leaf_behaviour() {
        let mut value = serde_json::json!({
            "storeConfig": {"apiKey": "sk-live-abc", "displayName": "Acme"},
        });
        redact_nested(&mut value);
        assert_eq!(
            value,
            serde_json::json!({
                "storeConfig": {"apiKey": "<redacted>", "displayName": "Acme"},
            })
        );
    }

    #[test]
    /// A value whose serialized size exceeds the cap is replaced by the
    /// marker rather than truncated into something that reads as real data.
    fn test_cap_value_size_replaces_oversized_value() {
        let huge = serde_json::Value::String("x".repeat(MAX_INPUT_FIELD_VALUE_BYTES + 1));
        let capped = cap_value_size(huge);
        assert_eq!(
            capped,
            serde_json::Value::String(OVERSIZED_VALUE_MARKER.to_string())
        );
    }

    #[test]
    /// A value within the cap passes through unchanged.
    fn test_cap_value_size_passes_through_small_value() {
        let small = serde_json::json!("ok");
        assert_eq!(cap_value_size(small.clone()), small);
    }

    #[test]
    /// `source_label` maps a flag-supplied workflow input to `"flag"`.
    fn test_source_label_maps_flag() {
        let mut flag_names = BTreeSet::new();
        flag_names.insert("storeName".to_string());
        let source = StepFieldSource::WorkflowInput {
            name: "storeName".to_string(),
        };
        assert_eq!(source_label(&source, &flag_names), "flag");
    }

    #[test]
    /// `source_label` maps a prompted workflow input (same enum variant, not
    /// in `flag_names`) to `"prompt"`.
    fn test_source_label_maps_prompt() {
        let flag_names = BTreeSet::new();
        let source = StepFieldSource::WorkflowInput {
            name: "storeName".to_string(),
        };
        assert_eq!(source_label(&source, &flag_names), "prompt");
    }

    #[test]
    /// `source_label` maps a declared-default-sourced field to `"default"`.
    fn test_source_label_maps_default() {
        let flag_names = BTreeSet::new();
        let source = StepFieldSource::Default {
            name: "region".to_string(),
        };
        assert_eq!(source_label(&source, &flag_names), "default");
    }

    #[test]
    /// The remaining sources each map to their own closed label.
    fn test_source_label_maps_remaining_sources() {
        let flag_names = BTreeSet::new();
        assert_eq!(
            source_label(&StepFieldSource::PriorOutput, &flag_names),
            "prior_output"
        );
        assert_eq!(
            source_label(&StepFieldSource::Literal, &flag_names),
            "literal"
        );
        assert_eq!(source_label(&StepFieldSource::Unset, &flag_names), "unset");
        assert_eq!(
            source_label(
                &StepFieldSource::Derived {
                    sources: vec!["a".to_string()]
                },
                &flag_names
            ),
            "derived"
        );
    }

    #[test]
    /// A bundled workflow's field carries its real (redacted-if-needed) value.
    fn test_build_step_input_fields_bundled_reports_real_values() {
        let flag_names = BTreeSet::new();
        let fields = vec![body_field(
            "storeName",
            serde_json::json!("acme"),
            StepFieldSource::Literal,
        )];
        let built = build_step_input_fields(&fields, &flag_names, true);
        assert_eq!(built.len(), 1);
        assert_eq!(built[0].field, "storeName");
        assert_eq!(built[0].value, Some(serde_json::json!("acme")));
    }

    #[test]
    /// An external workflow's fields all report `value: None`, even for a
    /// field whose name and schema are entirely harmless.
    fn test_build_step_input_fields_external_withholds_every_value() {
        let flag_names = BTreeSet::new();
        let fields = vec![
            body_field(
                "storeName",
                serde_json::json!("acme"),
                StepFieldSource::Literal,
            ),
            body_field("region", serde_json::json!("na"), StepFieldSource::Literal),
        ];
        let built = build_step_input_fields(&fields, &flag_names, false);
        assert!(built.iter().all(|f| f.value.is_none()));
    }

    #[test]
    /// A bundled workflow still redacts a sensitive-named field — origin
    /// only controls whether *non-sensitive* values are transmitted.
    fn test_build_step_input_fields_bundled_still_redacts_sensitive_field() {
        let flag_names = BTreeSet::new();
        let fields = vec![body_field(
            "clientSecret",
            serde_json::json!("abc123"),
            StepFieldSource::Literal,
        )];
        let built = build_step_input_fields(&fields, &flag_names, true);
        assert_eq!(built[0].value, None);
    }

    #[test]
    #[serial_test::serial]
    /// The operator kill switch (`AGS_TELEMETRY_NO_INPUT_VALUES`) forces
    /// every value to `None` for a bundled workflow, even for an entirely
    /// harmless field; leaving it unset transmits values as before.
    fn test_build_step_input_fields_kill_switch_withholds_bundled_values() {
        let flag_names = BTreeSet::new();
        let fields = vec![body_field(
            "storeName",
            serde_json::json!("acme"),
            StepFieldSource::Literal,
        )];

        let _unset = crate::support::test_helpers::TempEnvGuard::remove(
            crate::runtime::telemetry::ENV_TELEMETRY_NO_INPUT_VALUES,
        );
        let built_before = build_step_input_fields(&fields, &flag_names, true);
        assert_eq!(built_before[0].value, Some(serde_json::json!("acme")));

        let _set = crate::support::test_helpers::TempEnvGuard::set(
            crate::runtime::telemetry::ENV_TELEMETRY_NO_INPUT_VALUES,
            "1",
        );
        let built_after = build_step_input_fields(&fields, &flag_names, true);
        assert_eq!(built_after[0].value, None);
        // Non-value facts are untouched by the kill switch.
        assert_eq!(built_after[0].field, "storeName");
    }
}
