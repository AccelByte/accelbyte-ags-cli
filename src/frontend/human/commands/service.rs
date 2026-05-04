//! Human-readable rendering for API service responses.

use crate::errors::CliError;
use crate::frontend::human::templates;
use crate::frontend::presenters::service as service_presenter;
use crate::frontend::style;
use crate::frontend::PaginationHint;
use crate::frontend::RenderOptions;
use crate::frontend::RenderedOutput;
use crate::protocol::catalogue::OperationSchema;
use crate::protocol::output::{
    ApiBody, ApiOutput, ApiSuccess, CommandIntent, ExecutionTrace, FieldEntry, ResolutionTrace,
    Section,
};

/// Render a full API output to stdout (body) and stderr (trace, success message)
pub(crate) fn render_api_output(
    output: &ApiOutput,
    options: &RenderOptions,
) -> Result<RenderedOutput, CliError> {
    let stdout = match &output.body {
        ApiBody::Shaped(result) => {
            let text =
                render_command_result(result, &output.operation, &output.resource_name, options)?;
            Some(text)
        }
        ApiBody::Text(body) => Some(body.clone()),
        ApiBody::Empty => None,
    };

    let mut stderr_lines = Vec::new();
    if let Some(trace) = &output.trace {
        stderr_lines.push(render_execution_trace(trace));
    }
    if let Some(ApiSuccess { summary }) = &output.success {
        stderr_lines.push(style::success(summary, style::is_stderr_enabled()));
    }

    Ok(RenderedOutput {
        stdout,
        stderr: Some(stderr_lines.join("\n")).filter(|s| !s.is_empty()),
        ..Default::default()
    })
}

/// Render a dry-run output showing the HTTP request that would be sent
pub(crate) fn render_dry_run_output(
    report: &crate::protocol::result::DryRunResult,
) -> Result<RenderedOutput, CliError> {
    let request_view = service_presenter::present_dry_run(report);
    let mut lines = vec![format!("{} {}", request_view.http_method, request_view.url)];
    if !request_view.query.is_empty() {
        let pairs: Vec<String> = request_view
            .query
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        lines.push(format!("  Query: {}", pairs.join(", ")));
    }
    for (key, value) in &request_view.headers {
        lines.push(format!("{key}: {value}"));
    }
    if let Some(body) = &request_view.body {
        lines.push(format!(
            "Body: {}",
            serde_json::to_string(body).expect("serializing serde_json::Value is infallible")
        ));
    }
    Ok(RenderedOutput {
        stdout: Some(lines.join("\n")),
        stderr: None,
        ..Default::default()
    })
}

/// Render resolution trace lines for verbose output.
pub(crate) fn render_resolution_trace(resolution: &ResolutionTrace) -> String {
    let color_enabled = style::is_stderr_enabled();
    let mut lines = Vec::new();
    lines.push(style::apply_tone(
        &format!("  Spec: {}", resolution.spec_source),
        style::Tone::Dim,
        color_enabled,
    ));
    lines.push(style::apply_tone(
        &format!(
            "  Profile: {} ({})",
            resolution.profile.0, resolution.profile.1
        ),
        style::Tone::Dim,
        color_enabled,
    ));
    lines.push(style::apply_tone(
        &format!(
            "  Base URL: {} ({})",
            resolution.base_url.0, resolution.base_url.1
        ),
        style::Tone::Dim,
        color_enabled,
    ));
    if let Some((namespace, source)) = &resolution.namespace {
        lines.push(style::apply_tone(
            &format!("  Namespace: {namespace} ({source})"),
            style::Tone::Dim,
            color_enabled,
        ));
    }
    let token_detail = match &resolution.token_expiry {
        Some(expiry) => format!("{}, {expiry}", resolution.token_source),
        None => resolution.token_source.clone(),
    };
    lines.push(style::apply_tone(
        &format!("  Token: {token_detail}"),
        style::Tone::Dim,
        color_enabled,
    ));
    lines.join("\n")
}

/// Render a `CommandResult` as a human-readable string.
fn render_command_result(
    result: &crate::protocol::result::CommandResult,
    operation: &OperationSchema,
    resource_name: &str,
    options: &RenderOptions,
) -> Result<String, CliError> {
    use crate::protocol::result::CommandResult;
    let color_enabled = style::is_stdout_enabled();
    match result {
        CommandResult::Collection(collection) => {
            Ok(render_collection(collection, options, color_enabled))
        }
        CommandResult::Entity(entity) => Ok(render_entity(
            entity,
            CommandIntent::from_operation(operation),
            resource_name,
            operation,
            options,
            color_enabled,
        )),
        CommandResult::Empty(_) => Ok(String::new()),
        CommandResult::Raw(raw) => crate::frontend::json::format_json(&raw.value),
    }
}

/// Render a list-style API response as a column table, honouring quiet mode and pagination hints.
fn render_collection(
    collection: &crate::protocol::result::CollectionResult,
    options: &RenderOptions,
    color_enabled: bool,
) -> String {
    if collection.rows.is_empty() {
        if options.verbosity.is_quiet() {
            return String::new();
        }
        return style::info(&format!("No {} found", collection.kind), color_enabled);
    }

    let headers: Vec<String> = collection
        .columns
        .iter()
        .map(|column| column.label.clone())
        .collect();

    let rows: Vec<Vec<String>> = collection
        .rows
        .iter()
        .map(|row| row.cells.iter().map(field_value_to_display).collect())
        .collect();

    let pagination_hint = collection.page_info.as_ref().map(|info| PaginationHint {
        total: info.total_items.map(|t| t as u64),
        has_next: info.has_next,
    });

    templates::render_list_text(
        collection.rows.len(),
        &collection.kind,
        &headers,
        &rows,
        pagination_hint,
        options.is_page_all,
        options.verbosity.is_quiet(),
        color_enabled,
    )
}

/// Render a single-entity API response — heading, fields, and grouped sections — for action, inspect, or list intents.
fn render_entity(
    entity: &crate::protocol::result::EntityResult,
    intent: CommandIntent,
    resource_name: &str,
    operation: &OperationSchema,
    options: &RenderOptions,
    color_enabled: bool,
) -> String {
    use crate::protocol::result::HeadingStyle;

    let field_entries: Vec<FieldEntry> =
        entity.fields.iter().map(protocol_field_to_entry).collect();

    let section_entries: Vec<Section> = entity
        .sections
        .iter()
        .map(|group| Section {
            heading: group.heading.clone(),
            fields: group.fields.iter().map(protocol_field_to_entry).collect(),
        })
        .collect();

    match intent {
        CommandIntent::Action => {
            // The verb-specific success line ("✔ Updated stat-definition.")
            // is rendered separately to stderr from the `ApiSuccess` summary,
            // so the stdout block here is just the field rows. A second
            // generic "✔ <resource> operation completed" heading would be
            // redundant.
            if field_entries.is_empty() {
                return String::new();
            }
            let rows: Vec<(&str, String)> = field_entries
                .iter()
                .map(|f| (f.label.as_str(), f.value.clone()))
                .collect();
            templates::render_label_value_block_text(&rows, style::Tone::Plain, color_enabled)
        }
        CommandIntent::Inspect | CommandIntent::List => {
            if field_entries.is_empty() && section_entries.is_empty() {
                let noun = crate::support::strings::derive_noun_from_method(
                    &operation.name,
                    resource_name,
                );
                if options.verbosity.is_quiet() {
                    return String::new();
                }
                return style::info(&format!("No {noun} found"), color_enabled);
            }

            let heading = match entity.heading_style {
                HeadingStyle::Named => format!(
                    "{}: {}",
                    entity.kind,
                    entity.identifier.as_deref().unwrap_or("")
                ),
                HeadingStyle::Identified => format!(
                    "{} ({})",
                    entity.kind,
                    entity.identifier.as_deref().unwrap_or("")
                ),
                HeadingStyle::Bare => entity.kind.clone(),
            };

            templates::render_inspect_text(
                &heading,
                &field_entries,
                &section_entries,
                options.verbosity.is_quiet(),
                color_enabled,
            )
        }
    }
}

/// Convert a protocol-level `Field` into the `FieldEntry` shape the human templates expect.
fn protocol_field_to_entry(field: &crate::protocol::result::Field) -> FieldEntry {
    FieldEntry {
        label: field.label.clone(),
        value: field_value_to_display(&field.value),
    }
}

/// Render a single typed field value as the human-friendly string used in tables and inspect views.
fn field_value_to_display(value: &crate::protocol::result::FieldValue) -> String {
    use crate::protocol::result::FieldValue;
    match value {
        FieldValue::Text(string) => string.clone(),
        FieldValue::Number(number) => {
            if number.fract() == 0.0 {
                format!("{number:.0}")
            } else {
                number.to_string()
            }
        }
        FieldValue::Bool(true) => "yes".to_string(),
        FieldValue::Bool(false) => "no".to_string(),
        FieldValue::List(items) => {
            let n = items.len();
            if n == 1 {
                "1 item".to_string()
            } else {
                format!("{n} items")
            }
        }
        FieldValue::Null => String::new(),
    }
}

/// Render the verbose `--trace` lines that show resolution, request, and response details on stderr.
pub(crate) fn render_execution_trace_string(trace: &ExecutionTrace) -> String {
    render_execution_trace(trace)
}

/// Internal: render the verbose trace block (private helper used by both the
/// success-path renderer and the error-path renderer in [`crate::frontend::human::frontend`]).
fn render_execution_trace(trace: &ExecutionTrace) -> String {
    let color_enabled = style::is_stderr_enabled();
    let mut lines = Vec::new();

    if let Some(resolution) = &trace.resolution {
        lines.push(render_resolution_trace(resolution));
    }

    lines.push(style::apply_tone(
        &format!("→ {} {}", trace.request.http_method, trace.request.url),
        style::Tone::Dim,
        color_enabled,
    ));
    if !trace.request.query_params.is_empty() {
        let pairs: Vec<String> = trace
            .request
            .query_params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        lines.push(style::apply_tone(
            &format!("  Query: {}", pairs.join(", ")),
            style::Tone::Dim,
            color_enabled,
        ));
    }
    if trace.request.has_auth_header {
        lines.push(style::apply_tone(
            "  Authorization: Bearer <token>",
            style::Tone::Dim,
            color_enabled,
        ));
    }
    if let Some(size) = trace.request.body_size {
        lines.push(style::apply_tone(
            &format!("  Body: {} bytes", size),
            style::Tone::Dim,
            color_enabled,
        ));
    }

    if let Some(response) = &trace.response {
        let size_suffix = response
            .body_size
            .map(|s| format!(" ({} bytes)", s))
            .unwrap_or_default();
        lines.push(style::apply_tone(
            &format!(
                "← {} {}{}",
                response.status,
                response.reason.as_deref().unwrap_or(""),
                size_suffix
            ),
            style::Tone::Dim,
            color_enabled,
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::catalogue::{
        ApiVersion, HttpMethod, MutationClass, OperationId, OperationSchema,
    };
    use crate::protocol::output::{ApiBody, ApiOutput, ApiSuccess};
    use crate::protocol::result::DryRunResult;

    /// Build a minimal DELETE operation schema for tests.
    fn make_delete_operation() -> OperationSchema {
        OperationSchema {
            id: OperationId::new("deleteStat"),
            name: "delete".to_string(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::Mutating,
            http_method: HttpMethod::Delete,
            path_template: "/social/v1/admin/namespaces/{namespace}/stats/{statCode}".to_string(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: "admin".to_string(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// Successful DELETE returns 204 (Empty body) but must still emit a
    /// "Deleted X" stderr line — otherwise the user gets zero feedback.
    /// Companion fix in `runtime/dispatch/http.rs` ensures empty bodies are
    /// classified as `Text` rather than `Binary`, so this render path is
    /// actually reached.
    #[test]
    fn test_render_api_output_emits_success_for_empty_body_delete() {
        let output = ApiOutput {
            operation: make_delete_operation(),
            resource_name: "stat-definitions".to_string(),
            body: ApiBody::Empty,
            success: Some(ApiSuccess {
                summary: "Deleted stat-definition.".to_string(),
            }),
            trace: None,
        };
        let options = RenderOptions::default();
        let rendered = render_api_output(&output, &options).expect("render must succeed");
        assert!(rendered.stdout.is_none(), "no body → no stdout");
        let stderr = rendered.stderr.expect("success line must be on stderr");
        assert!(
            stderr.contains("Deleted stat-definition."),
            "stderr should contain success summary; got: {stderr:?}"
        );
    }

    /// Multi-param dry-run output formats query as a single `Query: k=v, k=v`
    /// line, not one `?k=v` line per parameter (which read like multiple
    /// query separators).
    #[test]
    fn test_render_dry_run_output_renders_query_as_single_line() {
        let report = DryRunResult {
            http_method: HttpMethod::Get,
            url: "https://example.com/items".to_string(),
            headers: vec![("Authorization".to_string(), "Bearer <token>".to_string())],
            query: vec![
                ("limit".to_string(), "5".to_string()),
                ("offset".to_string(), "0".to_string()),
            ],
            body: None,
        };
        let rendered = render_dry_run_output(&report).expect("dry-run render must succeed");
        let stdout = rendered.stdout.expect("dry-run must produce stdout");
        assert!(
            stdout.contains("  Query: limit=5, offset=0"),
            "expected single canonical query line; got: {stdout:?}"
        );
        assert!(
            !stdout.contains("?limit") && !stdout.contains("?offset"),
            "old per-param ? prefix should be gone; got: {stdout:?}"
        );
    }
}
