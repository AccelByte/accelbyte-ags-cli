use clap::{Arg, Command};

use crate::catalogue::Catalogue;
use crate::frontend::style;
use crate::invocation::flags::LeafSelectors;
use crate::invocation::resolve::{resolve, ResolvedContract};
use crate::protocol::catalogue::{
    BodyField, BodyFieldType, BodySchema, MethodSchema, OperationSchema, ParameterSchema,
    ScopeEntry, ServiceSchema, ValueType,
};
use crate::support::strings::{to_kebab_case, truncate_summary};

/// Build a service command with resource subcommands.
pub(crate) fn build_service_command_tree(
    schema: &ServiceSchema,
    selectors: &LeafSelectors,
) -> Command {
    let service_name = Catalogue::display_name(&schema.name).unwrap_or(&schema.name);
    let non_leaf_help_template = "{about-with-newline}\n\
        {usage-heading}\n  {usage}\n\n\
        {all-args}"
        .to_string();

    let mut service_command = Command::new(service_name.to_string())
        .help_template(non_leaf_help_template.clone())
        .about(schema.description.clone())
        .term_width(120)
        .override_usage(format!(
            "{} <RESOURCE> <METHOD> [OPTIONS]",
            style::styled_literal(&format!("ags {service_name}"))
        ))
        .next_display_order(None)
        .subcommand_required(true)
        .subcommand_help_heading("Resources")
        .arg_required_else_help(true)
        .disable_help_subcommand(true);

    for resource in &schema.resources {
        let resource_name = &resource.name;
        let mut resource_command = Command::new(resource_name.to_string())
            .help_template(non_leaf_help_template.clone())
            .about(resource.description.clone())
            .term_width(120)
            .override_usage(format!(
                "{} <METHOD> [OPTIONS]",
                style::styled_literal(&format!("ags {service_name} {resource_name}"))
            ))
            .next_display_order(None)
            .subcommand_required(true)
            .subcommand_help_heading("Methods")
            .arg_required_else_help(true)
            .disable_help_subcommand(true);

        for method in &resource.methods {
            let command_path = format!("ags {service_name} {} {}", resource.name, method.name);
            let (operation, resolved_scope_name, resolved_version) = match resolve(
                &command_path,
                method,
                selectors.api_scope.as_deref(),
                selectors.api_version.as_deref(),
            ) {
                Ok(ResolvedContract {
                    scope,
                    api_version,
                    operation,
                }) => (operation, scope, api_version),
                Err(_) => match fallback_default_contract(method) {
                    Some((operation, scope, version)) => (operation.clone(), scope, version),
                    None => continue,
                },
            };
            let operation = &operation;
            let method_name = &operation.name;
            let summary = truncate_summary(&operation.summary, 80);
            let operation_help_template = "{about-with-newline}\n\
                {usage-heading}\n  {usage}\n\n\
                {all-args}\
                {after-help}"
                .to_string();

            let mut operation_command = Command::new(method_name.to_string())
                .help_template(operation_help_template)
                .about(summary.clone())
                .term_width(120)
                .bin_name(format!("ags {service_name} {resource_name} {method_name}"));

            let raw_description = operation.description.as_deref().unwrap_or("");
            let body = if raw_description.is_empty() {
                to_sentence_case(&operation.summary)
            } else {
                let normalized = normalize_description(raw_description);
                split_long_sentences(&normalized)
            };
            let permission_block = if !operation.permissions.is_empty() {
                let permission_lines: Vec<String> = operation
                    .permissions
                    .iter()
                    .map(|permission| format_permission(permission))
                    .collect();
                format!("\n\nRequires permission:\n{}", permission_lines.join("\n"))
            } else {
                String::new()
            };
            let long_about = format!("{body}{permission_block}");

            let resolved_scope_entry: Option<&ScopeEntry> = method
                .scopes
                .iter()
                .find(|scope_entry| scope_entry.scope == resolved_scope_name);
            let multi_scope = method.scopes.len() > 1;
            let multi_version = resolved_scope_entry
                .map(|scope_entry| scope_entry.contracts.len() > 1)
                .unwrap_or(false);

            let contract_block = format!(
                "Default contract:\n  scope:   {resolved_scope_name}\n  version: {resolved_version}"
            );
            let long_about = if long_about.is_empty() {
                contract_block
            } else {
                format!("{long_about}\n\n{contract_block}")
            };
            if !long_about.trim().is_empty() {
                operation_command = operation_command.long_about(long_about);
            }

            for parameter in &operation.parameters {
                let arg_name = to_kebab_case(&parameter.name);
                let help_text =
                    normalize_description(parameter.description.as_deref().unwrap_or(""));
                let mut arg = Arg::new(parameter.name.clone())
                    .long(arg_name.clone())
                    .value_name(arg_name.clone())
                    .help(help_text);

                if parameter.required {
                    arg = arg.required(true);
                }

                let (_, enum_values) = extract_value_type(&parameter.value_type);
                if let Some(values) = enum_values {
                    arg = arg.value_parser(PermissiveEnumParser::new(values));
                }

                operation_command = operation_command.arg(arg);
            }

            let has_request_body = operation.request_body.is_some();
            if has_request_body {
                let mut json_arg = Arg::new("json").long("json").required(true);

                if let Some(ref body_schema) = operation.request_body {
                    let short_name = display_type(&body_schema.definition_name);
                    json_arg = json_arg
                        .help(format!(
                            "JSON request body ({short_name}; use --help for schema)"
                        ))
                        .long_help(format_json_long_help(body_schema));
                } else {
                    json_arg = json_arg.help("JSON request body");
                }

                operation_command = operation_command.arg(json_arg);
            }

            if multi_scope {
                let scopes: Vec<String> = method
                    .scopes
                    .iter()
                    .map(|scope_entry| scope_entry.scope.clone())
                    .collect();
                let default_scope = method
                    .default_scope
                    .clone()
                    .or_else(|| scopes.first().cloned())
                    .unwrap_or_default();
                let arg = Arg::new("api-scope")
                    .long("api-scope")
                    .help("Select the CLI API scope for this command")
                    .value_parser(scopes)
                    .default_value(default_scope);
                operation_command = operation_command.arg(arg);
            }
            if multi_version {
                if let Some(scope_entry) = resolved_scope_entry {
                    let versions: Vec<String> = scope_entry
                        .contracts
                        .iter()
                        .map(|contract| contract.api_version.to_string())
                        .collect();
                    let default_version = scope_entry.default_version.to_string();
                    let arg = Arg::new("api-version")
                        .long("api-version")
                        .help("Select the CLI API version for this command")
                        .value_parser(versions)
                        .default_value(default_version);
                    operation_command = operation_command.arg(arg);
                }
            }

            let after_help = build_operation_example(
                service_name,
                resource_name,
                method_name,
                &operation.parameters,
                has_request_body,
            );
            if !after_help.is_empty() {
                operation_command = operation_command.after_help(after_help);
            }

            resource_command = resource_command.subcommand(operation_command);
        }

        service_command = service_command.subcommand(resource_command);
    }

    service_command
}

/// Collapse a `ValueType` into the `(type_name, optional_enum_values)` view
/// the builder's help-text generator consumes. The generator was originally
/// written against that tuple shape; collapsing the richer enum here keeps
/// the generator unchanged.
fn extract_value_type(value_type: &ValueType) -> (&'static str, Option<&[String]>) {
    match value_type {
        ValueType::String => ("string", None),
        ValueType::Integer => ("integer", None),
        ValueType::Number => ("number", None),
        ValueType::Boolean => ("boolean", None),
        ValueType::Enum(values) => ("string", Some(values.as_slice())),
        ValueType::Array(_) => ("array", None),
    }
}

/// Derive the default contract for a method without running the resolver.
/// Used as a best-effort fallback when the explicit selectors don't match the
/// method's matrix: we still want to expose some leaf so `--help` works and
/// the downstream resolve call can produce the real error message.
fn fallback_default_contract(
    method: &MethodSchema,
) -> Option<(
    &OperationSchema,
    String,
    crate::protocol::catalogue::ApiVersion,
)> {
    let scope_entry = method
        .default_scope
        .as_deref()
        .and_then(|name| {
            method
                .scopes
                .iter()
                .find(|scope_entry| scope_entry.scope == name)
        })
        .or_else(|| method.scopes.first())?;
    let contract = scope_entry
        .contracts
        .iter()
        .find(|contract| contract.api_version == scope_entry.default_version)
        .or_else(|| {
            scope_entry
                .contracts
                .iter()
                .max_by_key(|contract| contract.api_version)
        })?;
    Some((contract, scope_entry.scope.clone(), contract.api_version))
}

/// A `ValueParser` that accepts any string while still advertising a closed
/// set of values for shell-completion and help display. Used for OpenAPI
/// `enum`-typed flags: we want `<TAB>` to suggest known values, but the
/// server is the source of truth for enum membership, so parse must not
/// reject unknown values.
#[derive(Clone)]
struct PermissiveEnumParser {
    values: Vec<clap::builder::PossibleValue>,
}

impl PermissiveEnumParser {
    /// Build a Clap value parser that publishes its enum values for completion but accepts any string at parse time.
    fn new(values: &[String]) -> Self {
        Self {
            values: values
                .iter()
                .map(|value| clap::builder::PossibleValue::new(value.clone()))
                .collect(),
        }
    }
}

impl clap::builder::TypedValueParser for PermissiveEnumParser {
    type Value = String;

    fn parse_ref(
        &self,
        _command: &clap::Command,
        _arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        Ok(value.to_string_lossy().into_owned())
    }

    fn possible_values(
        &self,
    ) -> Option<Box<dyn Iterator<Item = clap::builder::PossibleValue> + '_>> {
        Some(Box::new(self.values.iter().cloned()))
    }
}

/// Normalize description text from OpenAPI specs for clean terminal display.
fn normalize_description(text: &str) -> String {
    let text = text
        .replace("&#39;", "'")
        .replace("&#34;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"");
    let text = text.replace("**", "").replace("__", "");
    let text = text.replace('\t', "  ");

    let text = text
        .split("\n\n")
        .map(|paragraph| {
            let mut result_lines: Vec<String> = Vec::new();
            let mut prose_buffer: Vec<String> = Vec::new();

            for line in paragraph.lines() {
                let trimmed = line.trim();
                let trimmed = trim_heading_prefix(trimmed);
                let trimmed = convert_markdown_links(&trimmed);

                if trimmed.starts_with("- ") {
                    if !prose_buffer.is_empty() {
                        result_lines.push(prose_buffer.join(" "));
                        prose_buffer.clear();
                    }
                    result_lines.push(trimmed);
                } else {
                    prose_buffer.push(trimmed);
                }
            }
            if !prose_buffer.is_empty() {
                result_lines.push(prose_buffer.join(" "));
            }
            result_lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let text = text
        .lines()
        .map(collapse_spaces)
        .collect::<Vec<_>>()
        .join("\n");

    strip_action_code(&text)
}

/// Strip a leading Markdown heading marker (`#`, `##`, …) from a line so spec descriptions render flat.
fn trim_heading_prefix(line: &str) -> String {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        let after_hashes = trimmed.trim_start_matches('#');
        if after_hashes.starts_with(' ') {
            after_hashes.trim_start().to_string()
        } else {
            line.to_string()
        }
    } else {
        line.to_string()
    }
}

/// Rewrite `[label](url)` Markdown links as `label (url)` for plain-terminal display.
fn convert_markdown_links(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        if ch == '[' {
            if let Some(bracket_end) = text[index + 1..].find(']') {
                let bracket_end = index + 1 + bracket_end;
                if text.get(bracket_end + 1..bracket_end + 2) == Some("(") {
                    if let Some(paren_end) = text[bracket_end + 2..].find(')') {
                        let paren_end = bracket_end + 2 + paren_end;
                        let link_text = &text[index + 1..bracket_end];
                        let url = &text[bracket_end + 2..paren_end];
                        result.push_str(link_text);
                        result.push_str(" (");
                        result.push_str(url);
                        result.push(')');
                        while let Some(&(current_index, _)) = chars.peek() {
                            if current_index <= paren_end {
                                chars.next();
                            } else {
                                break;
                            }
                        }
                        continue;
                    }
                }
            }
            result.push(ch);
        } else {
            result.push(ch);
        }
    }
    result
}

/// Collapse runs of spaces within a line, preserving leading indentation on bullet items.
fn collapse_spaces(line: &str) -> String {
    let trimmed = line.trim_start();
    if trimmed.starts_with("- ") {
        let indent = &line[..line.len() - trimmed.len()];
        let collapsed: String = collapse_inner_spaces(trimmed);
        format!("{indent}{collapsed}")
    } else {
        collapse_inner_spaces(line)
    }
}

/// Collapse every run of spaces in `text` down to a single space.
fn collapse_inner_spaces(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_space = false;
    for ch in text.chars() {
        if ch == ' ' {
            if !prev_space {
                result.push(' ');
            }
            prev_space = true;
        } else {
            prev_space = false;
            result.push(ch);
        }
    }
    result
}

/// Drop the AccelByte `Action Code: …` trailer that appears in many spec descriptions.
fn strip_action_code(text: &str) -> String {
    if let Some(position) = text.to_lowercase().find("action code") {
        text[..position].trim_end().to_string()
    } else {
        text.to_string()
    }
}

/// Capitalise only the first character of `text`, leaving the rest untouched.
fn to_sentence_case(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            format!("{upper}{}", chars.as_str())
        }
    }
}

/// Insert line breaks between sentences in long help paragraphs so they wrap better in narrow terminals.
fn split_long_sentences(text: &str) -> String {
    let mut result = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("- ") || line.len() <= 120 {
            result.push(line.to_string());
            continue;
        }
        let mut current_line = String::new();
        let mut iter = line.char_indices().peekable();
        while let Some((_, ch)) = iter.next() {
            current_line.push(ch);
            if ch != '.' {
                continue;
            }
            let next_two: Vec<(usize, char)> = iter.clone().take(2).collect();
            if next_two.len() < 2 {
                continue;
            }
            if next_two[0].1 != ' ' || !next_two[1].1.is_uppercase() {
                continue;
            }
            if current_line.len() <= 40 {
                continue;
            }
            let remainder = &line[next_two[1].0..];
            if current_line.len() + distance_to_sentence_end(remainder) <= 120 {
                continue;
            }
            result.push(current_line);
            current_line = String::new();
            iter.next(); // consume the space we already inspected
        }
        if !current_line.is_empty() {
            result.push(current_line);
        }
    }
    result.join("\n")
}

/// Return the byte offset to the next sentence-ending `.` followed by a capitalised word.
fn distance_to_sentence_end(text: &str) -> usize {
    let mut iter = text.char_indices().peekable();
    while let Some((idx, ch)) = iter.next() {
        if ch != '.' {
            continue;
        }
        let next_two: Vec<(usize, char)> = iter.clone().take(2).collect();
        if next_two.len() == 2 && next_two[0].1 == ' ' && next_two[1].1.is_uppercase() {
            return idx + ch.len_utf8();
        }
    }
    text.len()
}

/// Strip Java-style package prefixes from a type name (e.g. `com.foo.Bar` → `Bar`) for help output.
fn display_type(type_str: &str) -> String {
    let mut result = type_str.to_string();
    while let Some(dot_pos) = result.find('.') {
        let start = result[..dot_pos]
            .rfind(|ch: char| !ch.is_alphanumeric() && ch != '_')
            .map(|position| position + 1)
            .unwrap_or(0);
        result = format!("{}{}", &result[..start], &result[dot_pos + 1..]);
    }
    result
}

/// Convert a `BodyFieldType` to the display string used in help text.
///
/// Applies `display_type` stripping on reference names so Java-style package
/// prefixes are removed consistently.
fn body_field_type_label(ft: &BodyFieldType) -> String {
    match ft {
        BodyFieldType::String => "string".to_string(),
        BodyFieldType::Integer => "integer".to_string(),
        BodyFieldType::Boolean => "boolean".to_string(),
        BodyFieldType::Number => "number".to_string(),
        BodyFieldType::Object => "object".to_string(),
        BodyFieldType::Enum(_) => "string".to_string(),
        BodyFieldType::Reference(name) => display_type(name),
        BodyFieldType::Array(inner) => format!("array[{}]", body_field_type_label(inner)),
    }
}

/// Format a permission string `RESOURCE [ACTION]` as the indented `  ACTION on RESOURCE` line shown in help.
fn format_permission(permission: &str) -> String {
    if let Some(bracket_pos) = permission.rfind('[') {
        let resource = permission[..bracket_pos].trim();
        let action = permission[bracket_pos + 1..].trim_end_matches(']').trim();
        format!("  {action} on {resource}")
    } else {
        format!("  {permission}")
    }
}

/// Render a JSON request-body schema as a `{}`-bracketed help block with type annotations and required markers.
fn render_schema(fields: &[BodyField], indent: usize) -> String {
    let padding = "  ".repeat(indent);
    let inner_padding = "  ".repeat(indent + 1);
    let mut output = format!("{padding}{{\n");

    for (index, field) in fields.iter().enumerate() {
        let marker = if field.required {
            style::styled_literal("*")
        } else {
            " ".to_string()
        };
        let name_display = format!("\"{}\"", field.name);
        let comma = if index < fields.len() - 1 { "," } else { "" };

        if !field.children.is_empty() {
            if matches!(field.field_type, BodyFieldType::Array(_)) {
                let type_label = body_field_type_label(&field.field_type);
                output.push_str(&format!(
                    "{inner_padding}{marker}{name_display}: [  <{type_label}>\n"
                ));
                output.push_str(&render_schema_inner(&field.children, indent + 2));
                output.push_str(&format!("{inner_padding}]{comma}\n"));
            } else {
                let type_label = body_field_type_label(&field.field_type);
                output.push_str(&format!(
                    "{inner_padding}{marker}{name_display}: {{  <{type_label}>\n"
                ));
                output.push_str(&render_schema_fields(&field.children, indent + 2));
                output.push_str(&format!("{inner_padding}}}{comma}\n"));
            }
        } else {
            let type_str = body_field_type_label(&field.field_type);
            let type_display = if let BodyFieldType::Enum(values) = &field.field_type {
                format!("<{} [{}]>", type_str, values.join("|"))
            } else {
                format!("<{type_str}>")
            };
            output.push_str(&format!(
                "{inner_padding}{marker}{name_display}: {type_display}{comma}\n"
            ));
        }
    }

    output.push_str(&format!("{padding}}}"));
    output
}

/// Render the inside of a schema block — the same fields as `render_schema` without the surrounding braces.
fn render_schema_fields(fields: &[BodyField], indent: usize) -> String {
    let inner_padding = "  ".repeat(indent);
    let mut output = String::new();

    for (index, field) in fields.iter().enumerate() {
        let marker = if field.required {
            style::styled_literal("*")
        } else {
            " ".to_string()
        };
        let name_display = format!("\"{}\"", field.name);
        let comma = if index < fields.len() - 1 { "," } else { "" };

        if !field.children.is_empty() {
            if matches!(field.field_type, BodyFieldType::Array(_)) {
                let type_label = body_field_type_label(&field.field_type);
                output.push_str(&format!(
                    "{inner_padding}{marker}{name_display}: [  <{type_label}>\n"
                ));
                output.push_str(&render_schema_inner(&field.children, indent + 1));
                output.push_str(&format!("{inner_padding}]{comma}\n"));
            } else {
                let type_label = body_field_type_label(&field.field_type);
                output.push_str(&format!(
                    "{inner_padding}{marker}{name_display}: {{  <{type_label}>\n"
                ));
                output.push_str(&render_schema_fields(&field.children, indent + 1));
                output.push_str(&format!("{inner_padding}}}{comma}\n"));
            }
        } else {
            let type_str = body_field_type_label(&field.field_type);
            let type_display = if let BodyFieldType::Enum(values) = &field.field_type {
                format!("<{} [{}]>", type_str, values.join("|"))
            } else {
                format!("<{type_str}>")
            };
            output.push_str(&format!(
                "{inner_padding}{marker}{name_display}: {type_display}{comma}\n"
            ));
        }
    }

    output
}

/// Render a nested schema block at the given indent (recursive helper for `render_schema`).
fn render_schema_inner(fields: &[BodyField], indent: usize) -> String {
    let padding = "  ".repeat(indent);
    let mut output = format!("{padding}{{\n");
    output.push_str(&render_schema_fields(fields, indent + 1));
    output.push_str(&format!("{padding}}}\n"));
    output
}

/// Build the multi-section `--json` long-help text — input forms, schema, and a synthesised example.
fn format_json_long_help(schema: &BodySchema) -> String {
    let def_name = display_type(&schema.definition_name);
    let mut output = format!("JSON request body ({def_name})\n");
    output.push_str("\nInput:\n");
    output.push_str("  --json @path/to.json   read JSON from a file\n");
    output
        .push_str("  --json @-              read JSON from stdin (avoids shell quoting issues)\n");
    output.push_str("  --json '{...}'         inline JSON\n");
    output.push_str("\nSchema:\n");
    output.push_str(&render_schema(&schema.fields, 1));
    output.push('\n');

    let example_value = Catalogue::build_body_skeleton_from_schema(schema);
    let example_json = serde_json::to_string_pretty(&example_value).unwrap();
    let indented: String = example_json
        .lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    output.push_str("\nExample:\n");
    output.push_str(&indented);

    output
}

/// Synthesise a deterministic example value for a parameter, used in the help-text examples.
fn param_example_value(parameter: &ParameterSchema) -> String {
    let (parameter_type, enum_values) = extract_value_type(&parameter.value_type);
    if let Some(values) = enum_values {
        if let Some(first) = values.first() {
            return first.clone();
        }
    }
    let kebab = to_kebab_case(&parameter.name);
    match parameter_type {
        "integer" | "number" => format!("{}", stable_example_hash(&parameter.name) % 100),
        "boolean" => format!("{}", stable_example_hash(&parameter.name) % 2 == 0),
        _ => format!("my-{kebab}"),
    }
}

/// DJB2-style hash so example numbers and booleans stay stable across releases for the same parameter name.
fn stable_example_hash(name: &str) -> u64 {
    let mut hash_value: u64 = 5381;
    for byte in name.bytes() {
        hash_value = hash_value.wrapping_mul(33).wrapping_add(byte as u64);
    }
    hash_value
}

/// Build the synthesised `Example:` block shown in long help, populating each parameter with a sensible value.
fn build_operation_example(
    service_name: &str,
    resource_name: &str,
    method_name: &str,
    parameters: &[ParameterSchema],
    has_request_body: bool,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    for parameter in parameters {
        let flag = to_kebab_case(&parameter.name);
        let value = param_example_value(parameter);
        let (parameter_type, _) = extract_value_type(&parameter.value_type);
        let quoted = match parameter_type {
            "integer" | "number" | "boolean" => value,
            _ => format!("'{value}'"),
        };
        parts.push(format!("--{flag} {quoted}"));
    }

    if has_request_body {
        parts.push("--json '{...}'".to_string());
    }

    if parts.is_empty() {
        return String::new();
    }

    let args = parts.join(" ");
    let example_line = format!(
        "  ags {} {} {} {}",
        service_name, resource_name, method_name, args
    );
    format!("{}:\n{}", style::styled_header("Example"), example_line)
}

#[cfg(test)]
mod tests {
    use super::normalize_description;

    #[test]
    fn test_normalize_html_entities() {
        assert_eq!(
            normalize_description("account&#39;s data &amp; info"),
            "account's data & info"
        );
        assert_eq!(normalize_description("&lt;tag&gt;"), "<tag>");
        assert_eq!(normalize_description("&quot;hello&quot;"), "\"hello\"");
        assert_eq!(normalize_description("&#34;quoted&#34;"), "\"quoted\"");
    }

    #[test]
    fn test_normalize_markdown_bold() {
        assert_eq!(
            normalize_description("**This endpoint** does things"),
            "This endpoint does things"
        );
        assert_eq!(normalize_description("__also bold__"), "also bold");
    }

    #[test]
    fn test_normalize_markdown_links() {
        assert_eq!(
            normalize_description("See [docs](https://example.com) for info"),
            "See docs (https://example.com) for info"
        );
    }

    #[test]
    fn test_normalize_markdown_headers() {
        assert_eq!(normalize_description("## Header Text"), "Header Text");
        assert_eq!(normalize_description("### Sub Header"), "Sub Header");
    }

    #[test]
    fn test_normalize_tabs() {
        assert_eq!(normalize_description("\t- steam"), "- steam");
        assert_eq!(normalize_description("\t- ps4\n\t- xbox"), "- ps4\n- xbox");
        assert_eq!(normalize_description("a\tb"), "a b");
    }

    #[test]
    fn test_normalize_multiple_spaces() {
        assert_eq!(
            normalize_description("clients.  Specify the"),
            "clients. Specify the"
        );
    }

    #[test]
    fn test_normalize_mid_sentence_newlines() {
        assert_eq!(
            normalize_description("line one\nline two"),
            "line one line two"
        );
        assert_eq!(
            normalize_description("para one\n\npara two"),
            "para one\n\npara two"
        );
    }

    #[test]
    fn test_normalize_preserves_list_items() {
        let input = "Platforms:\n- steam\n- ps4\n- xbox";
        let expected = "Platforms:\n- steam\n- ps4\n- xbox";
        assert_eq!(normalize_description(input), expected);
    }

    #[test]
    fn test_normalize_combined() {
        let input = "**This endpoint** retrieves the account&#39;s data.\n\
                      It supports multiple platforms:\n\
                      \t- steam\n\
                      \t- ps4\n\n\
                      ## Notes\n\
                      See [docs](https://example.com) for  more info.";
        let result = normalize_description(input);
        assert!(!result.contains("**"));
        assert!(!result.contains("&#39;"));
        assert!(!result.contains('\t'));
        assert!(!result.contains("##"));
        assert!(result.contains("account's"));
        assert!(result.contains("- steam"));
        assert!(result.contains("- ps4"));
        assert!(result.contains("docs (https://example.com)"));
        assert!(!result.contains("  more"));
    }
}

#[cfg(test)]
mod sentence_splitting_tests {
    use super::split_long_sentences;

    /// Multi-byte UTF-8 chars (em-dash, smart quotes, CJK) must survive the
    /// sentence splitter unchanged. Earlier versions cast `bytes[i] as char`
    /// which corrupted any non-ASCII glyph.
    #[test]
    fn test_split_long_sentences_preserves_non_ascii_characters() {
        let input = "This sentence describes a feature \u{2014} using an em-dash, smart \u{201C}quotes\u{201D}, and a glyph: \u{4E2D}. \
                     This is a second sentence that pushes us over the wrap threshold so the splitter actually engages here.";
        let output = split_long_sentences(input);
        assert!(output.contains('\u{2014}'), "em-dash should survive");
        assert!(
            output.contains('\u{201C}'),
            "left smart quote should survive"
        );
        assert!(
            output.contains('\u{4E2D}'),
            "non-Latin glyph should survive"
        );
        assert!(
            !output.contains('\u{FFFD}'),
            "no replacement chars should appear"
        );
    }
}
