//! Bridge helpers between parsed CLI commands and the workflow executor.
//!
//! `route_service` in `routes::service` synthesises a 1-step workflow from a service operation;
//! these helpers translate an already-parsed `CommandRequest` (or registered-
//! workflow flags) into the `pre_supplied` input map the executor seeds
//! `workflow_supplied` from.

use std::collections::BTreeMap;

use ags_protocol::request::CommandRequest;
use ags_protocol::workflow::CompiledWorkflow;
use ags_runtime::support::strings::to_kebab_case;
use serde_json::Value;

/// Build a clap command for a registered workflow: one optional
/// `--<kebab-name>` flag per declared input. Every flag is a plain `String`
/// at the clap level — schema-typed coercion happens post-parse via
/// [`coerce_cli_value`].
pub fn build_registered_workflow_clap(compiled: &CompiledWorkflow) -> clap::Command {
    let mut command =
        clap::Command::new(compiled.id.as_str().to_string()).disable_help_subcommand(true);
    for spec in &compiled.inputs {
        let mut arg = clap::Arg::new(spec.name.clone())
            .long(to_kebab_case(&spec.name))
            .required(false);
        if let Some(description) = &spec.description {
            arg = arg.help(description.clone());
        }
        command = command.arg(arg);
    }
    command
}

/// Build the executor's `pre_supplied` input map from a parsed
/// `CommandRequest`.
///
/// Namespace and path/query/header parameters arrive as strings (clap parses
/// every dynamic arg as a string) and are coerced against each input's
/// declared schema via [`coerce_cli_value`]. Body object properties are
/// already typed JSON and pass through verbatim. Names not declared as
/// workflow inputs in `compiled` are dropped.
pub fn cli_flags_matching_workflow_inputs(
    compiled: &CompiledWorkflow,
    request: &CommandRequest,
) -> BTreeMap<String, Value> {
    let schemas: BTreeMap<&str, Option<&Value>> = compiled
        .inputs
        .iter()
        .map(|spec| (spec.name.as_str(), spec.schema.as_ref()))
        .collect();
    let mut supplied: BTreeMap<String, Value> = BTreeMap::new();

    // String-valued flags: path/query/header parameters plus namespace.
    let string_params = request
        .path_params
        .iter()
        .chain(request.query_params.iter())
        .chain(request.header_params.iter());
    for (name, raw) in string_params {
        if let Some(schema) = schemas.get(name.as_str()) {
            supplied.insert(name.clone(), coerce_cli_value(raw, *schema));
        }
    }
    if let Some(namespace) = &request.namespace {
        if let Some(schema) = schemas.get("namespace") {
            supplied.insert(
                "namespace".to_string(),
                coerce_cli_value(namespace, *schema),
            );
        }
    }

    // Body properties are already typed JSON — pass through verbatim.
    if let Some(Value::Object(body)) = &request.body {
        for (name, value) in body {
            if schemas.contains_key(name.as_str()) {
                supplied.insert(name.clone(), value.clone());
            }
        }
    }

    supplied
}

/// Best-effort schema-typed parsing of a raw CLI string.
///
/// The `schema`'s `"type"` field selects the target JSON type. On any parse
/// miss the raw value is preserved as a JSON string rather than rejected —
/// the server is the authority on value validity (project policy: no
/// client-side validation of int/number/bool params).
pub fn coerce_cli_value(raw: &str, schema: Option<&Value>) -> Value {
    let type_hint = schema
        .and_then(|s| s.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("string");
    match type_hint {
        "integer" => raw
            .parse::<i64>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(raw.to_string())),
        "number" => raw
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(raw.to_string())),
        "boolean" => match raw.to_ascii_lowercase().as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => Value::String(raw.to_string()),
        },
        "array" | "object" => {
            serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
        }
        _ => Value::String(raw.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coerce_integer_parses() {
        let schema = serde_json::json!({"type": "integer"});
        assert_eq!(coerce_cli_value("42", Some(&schema)), Value::from(42));
    }

    #[test]
    fn test_coerce_integer_parse_miss_falls_back_to_string() {
        let schema = serde_json::json!({"type": "integer"});
        assert_eq!(
            coerce_cli_value("not-a-number", Some(&schema)),
            Value::String("not-a-number".to_string())
        );
    }

    #[test]
    fn test_coerce_boolean_parses() {
        let schema = serde_json::json!({"type": "boolean"});
        assert_eq!(coerce_cli_value("TRUE", Some(&schema)), Value::Bool(true));
        assert_eq!(coerce_cli_value("false", Some(&schema)), Value::Bool(false));
    }

    #[test]
    fn test_coerce_no_schema_is_string() {
        assert_eq!(
            coerce_cli_value("hello", None),
            Value::String("hello".to_string())
        );
    }

    #[test]
    fn test_coerce_array_parses_json() {
        let schema = serde_json::json!({"type": "array"});
        assert_eq!(
            coerce_cli_value("[1,2]", Some(&schema)),
            serde_json::json!([1, 2])
        );
    }
}
