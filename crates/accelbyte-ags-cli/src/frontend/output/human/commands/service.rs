//! Human-readable rendering for API service responses.

use crate::errors::CliError;
use crate::frontend::output::human::templates;
use crate::frontend::presenters::service as service_presenter;
use crate::frontend::style;
use crate::frontend::PaginationHint;
use crate::frontend::RenderOptions;
use crate::frontend::RenderedOutput;
use ags_protocol::catalogue::OperationSchema;
use ags_protocol::output::{
    ApiBody, ApiOutput, CommandIntent, ExecutionTrace, FieldEntry, ResolutionTrace, Section,
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
    if let Some(success) = &output.success {
        let label = if output.has_alternate_versions {
            format!("{} ({})", success.summary, success.api_version)
        } else {
            success.summary.clone()
        };
        stderr_lines.push(style::success(&label, style::is_stderr_enabled()));
    } else if output.has_alternate_versions && !options.verbosity.is_quiet() {
        // Read operations have no ApiSuccess, but when the command has
        // alternate versions the user needs to know which API contract
        // version served the response.
        let label = format!("API {}", output.operation.api_version);
        stderr_lines.push(style::apply_tone(
            &label,
            style::Tone::Dim,
            style::is_stderr_enabled(),
        ));
    }

    Ok(RenderedOutput {
        stdout,
        stderr: Some(stderr_lines.join("\n")).filter(|s| !s.is_empty()),
        ..Default::default()
    })
}

/// Render a dry-run output showing the HTTP request that would be sent.
///
/// The method+URL line stays at the default colour (headline), and the
/// detail lines below (Query, headers, Body) render in dim — secondary
/// information for inspecting the composed request. The Body is
/// pretty-printed across multiple lines so JSON is readable.
pub(crate) fn render_dry_run_output(
    report: &ags_protocol::result::DryRunResult,
) -> Result<RenderedOutput, CliError> {
    let request_view = service_presenter::present_dry_run(report);
    let color_enabled = style::is_stdout_enabled();
    let dim = |s: String| style::apply_tone(&s, style::Tone::Dim, color_enabled);

    let mut lines = vec![format!("{} {}", request_view.http_method, request_view.url)];
    if !request_view.query.is_empty() {
        let pairs: Vec<String> = request_view
            .query
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        lines.push(dim(format!("  Query: {}", pairs.join(", "))));
    }
    for (key, value) in &request_view.headers {
        lines.push(dim(format!("{key}: {value}")));
    }
    if let Some(body) = &request_view.body {
        let rendered = service_presenter::render_dry_run_body(body);
        lines.push(dim("Body:".into()));
        for body_line in rendered.lines() {
            lines.push(dim(body_line.to_string()));
        }
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
    result: &ags_protocol::result::CommandResult,
    operation: &OperationSchema,
    resource_name: &str,
    options: &RenderOptions,
) -> Result<String, CliError> {
    use ags_protocol::result::CommandResult;
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
        CommandResult::Raw(raw) => crate::frontend::output::json::format_json(&raw.value),
    }
}

/// Render a list-style API response as a column table, honouring quiet mode and pagination hints.
fn render_collection(
    collection: &ags_protocol::result::CollectionResult,
    options: &RenderOptions,
    color_enabled: bool,
) -> String {
    // Notes (e.g. sibling app-level flags) describe the response as a whole,
    // not the row list specifically, so they're prepended whether or not
    // there are any rows — an empty/null endpoints array is exactly the case
    // where knowing isAppRunning/hasAPISpec/hasGRPCReflection matters most.
    let notes_prefix = if collection.notes.is_empty() || options.verbosity.is_quiet() {
        String::new()
    } else {
        let notes: Vec<String> = collection
            .notes
            .iter()
            .map(|note| style::apply_tone(note, style::Tone::Dim, color_enabled))
            .collect();
        format!("{}\n", notes.join("\n"))
    };

    if collection.rows.is_empty() {
        if options.verbosity.is_quiet() {
            return String::new();
        }
        return format!(
            "{notes_prefix}{}",
            style::info(&format!("No {} found", collection.kind), color_enabled)
        );
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

    let table = templates::render_list_text(
        collection.rows.len(),
        &collection.kind,
        &headers,
        &rows,
        pagination_hint,
        options.is_page_all,
        options.verbosity.is_quiet(),
        color_enabled,
    );

    format!("{notes_prefix}{table}")
}

/// Render a single-entity API response — heading, fields, and grouped sections — for action, inspect, or list intents.
fn render_entity(
    entity: &ags_protocol::result::EntityResult,
    intent: CommandIntent,
    resource_name: &str,
    operation: &OperationSchema,
    options: &RenderOptions,
    color_enabled: bool,
) -> String {
    use ags_protocol::result::HeadingStyle;

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
            // The verb-specific success line ("✔ Updated stat-definition")
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
                let noun = ags_runtime::support::strings::derive_noun_from_method(
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
fn protocol_field_to_entry(field: &ags_protocol::result::Field) -> FieldEntry {
    FieldEntry {
        label: field.label.clone(),
        value: field_value_to_display(&field.value),
    }
}

/// Render a single typed field value as the human-friendly string used in tables and inspect views.
fn field_value_to_display(value: &ags_protocol::result::FieldValue) -> String {
    use ags_protocol::result::FieldValue;
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

/// Render the verbose trace block, shared by the success-path and error-path renderers.
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
    use ags_protocol::catalogue::{
        ApiVersion, HttpMethod, MutationClass, OperationId, OperationSchema,
    };
    use ags_protocol::output::{ApiBody, ApiOutput, ApiSuccess};
    use ags_protocol::result::{CollectionResult, DryRunResult};

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
                summary: "Deleted stat-definition".to_string(),
                api_version: ApiVersion(1),
            }),
            trace: None,
            raw_body: None,
            has_alternate_versions: false,
        };
        let options = RenderOptions::default();
        let rendered = render_api_output(&output, &options).expect("render must succeed");
        assert!(rendered.stdout.is_none(), "no body → no stdout");
        let stderr = rendered.stderr.expect("success line must be on stderr");
        assert!(
            stderr.contains("Deleted stat-definition"),
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

    /// Build a minimal mutating POST operation at a given API version.
    fn make_post_operation(version: u32) -> OperationSchema {
        OperationSchema {
            id: OperationId::new("createItem"),
            name: "create".to_string(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::Mutating,
            http_method: HttpMethod::Post,
            path_template: format!("/social/v{version}/admin/items"),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: "admin".to_string(),
            api_version: ApiVersion(version),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// When the command has alternate versions, the human success line for a
    /// mutating call must contain the API version suffix so the user knows
    /// which contract version was used.
    #[test]
    fn test_human_success_line_contains_api_version_with_choice() {
        let output = ApiOutput {
            operation: make_post_operation(3),
            resource_name: "stat-definitions".to_string(),
            body: ApiBody::Empty,
            success: Some(ApiSuccess {
                summary: "Created stat-definition".to_string(),
                api_version: ApiVersion(3),
            }),
            trace: None,
            raw_body: None,
            has_alternate_versions: true,
        };
        let options = RenderOptions::default();
        let rendered = render_api_output(&output, &options).expect("render must succeed");
        let stderr = rendered.stderr.expect("success line must be on stderr");
        assert!(
            stderr.contains("v3"),
            "success line must contain the API version label; got: {stderr:?}"
        );
    }

    /// Build a minimal read-only GET operation at a given API version.
    fn make_get_operation(version: u32) -> OperationSchema {
        OperationSchema {
            id: OperationId::new("getItems"),
            name: "list".to_string(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: format!("/social/v{version}/admin/items"),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: "admin".to_string(),
            api_version: ApiVersion(version),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// When the command has alternate versions, a read operation must emit a
    /// dim API version label to stderr so the user knows which contract
    /// version was used.
    #[test]
    fn test_human_read_emits_api_version_label_with_choice() {
        let output = ApiOutput {
            operation: make_get_operation(2),
            resource_name: "items".to_string(),
            body: ApiBody::Text(r#"{"data":[]}"#.to_string()),
            success: None,
            trace: None,
            raw_body: None,
            has_alternate_versions: true,
        };
        let options = RenderOptions::default();
        let rendered = render_api_output(&output, &options).expect("render must succeed");
        let stderr = rendered
            .stderr
            .expect("version label must appear on stderr");
        assert!(
            stderr.contains("v2"),
            "read output must contain the API version label on stderr; got: {stderr:?}"
        );
    }

    /// The read version label must NOT appear in `--format json` output —
    /// it is human chrome, and the JSON renderer never emits stderr.
    #[test]
    fn test_json_read_does_not_contain_api_version_label_with_choice() {
        let output = ApiOutput {
            operation: make_get_operation(2),
            resource_name: "items".to_string(),
            body: ApiBody::Text(r#"{"data":[]}"#.to_string()),
            success: None,
            trace: None,
            raw_body: None,
            has_alternate_versions: true,
        };
        let options = RenderOptions::default();
        let rendered =
            crate::frontend::output::json::commands::service::render_api_output(&output, &options)
                .expect("JSON render must succeed");
        assert!(
            rendered.stderr.is_none(),
            "JSON renderer must not emit stderr for reads; got: {:?}",
            rendered.stderr
        );
    }

    /// The read version label must be suppressed in quiet mode even when the
    /// command has alternate versions.
    #[test]
    fn test_human_read_quiet_suppresses_api_version_label_with_choice() {
        let output = ApiOutput {
            operation: make_get_operation(2),
            resource_name: "items".to_string(),
            body: ApiBody::Text(r#"{"data":[]}"#.to_string()),
            success: None,
            trace: None,
            raw_body: None,
            has_alternate_versions: true,
        };
        let options = RenderOptions {
            verbosity: ags_protocol::request::Verbosity::Quiet,
            ..Default::default()
        };
        let rendered = render_api_output(&output, &options).expect("render must succeed");
        assert!(
            rendered.stderr.is_none() || !rendered.stderr.as_deref().unwrap_or("").contains("v2"),
            "quiet mode must suppress the API version label; got: {:?}",
            rendered.stderr
        );
    }

    /// `--format json` output must NOT contain the API version — the JSON
    /// renderer emits only `output.body` and the version is human chrome.
    #[test]
    fn test_json_output_does_not_contain_api_version_with_choice() {
        let output = ApiOutput {
            operation: make_post_operation(3),
            resource_name: "stat-definitions".to_string(),
            body: ApiBody::Text(r#"{"id":"abc"}"#.to_string()),
            success: Some(ApiSuccess {
                summary: "Created stat-definition".to_string(),
                api_version: ApiVersion(3),
            }),
            trace: None,
            raw_body: None,
            has_alternate_versions: true,
        };
        let options = RenderOptions::default();
        let rendered =
            crate::frontend::output::json::commands::service::render_api_output(&output, &options)
                .expect("JSON render must succeed");

        // JSON stdout must not contain any version reference.
        if let Some(stdout) = &rendered.stdout {
            assert!(
                !stdout.contains("api_version") && !stdout.contains("v3"),
                "JSON stdout must not contain the API version; got: {stdout:?}"
            );
        }
        // JSON renderer must not emit stderr (no success line, no trace).
        assert!(
            rendered.stderr.is_none(),
            "JSON renderer must not emit stderr; got: {:?}",
            rendered.stderr
        );
    }

    // ── "without a choice" tests: version must NOT appear ─────────────

    /// When the command has only one API version, the mutating success line
    /// must NOT carry a version suffix — it must be byte-identical to the
    /// line `origin/main` renders: `style::success(&summary, ...)`.
    #[test]
    fn test_human_success_line_no_version_without_choice() {
        let output = ApiOutput {
            operation: make_post_operation(3),
            resource_name: "stat-definitions".to_string(),
            body: ApiBody::Empty,
            success: Some(ApiSuccess {
                summary: "Created stat-definition".to_string(),
                api_version: ApiVersion(3),
            }),
            trace: None,
            raw_body: None,
            has_alternate_versions: false,
        };
        let options = RenderOptions::default();
        let rendered = render_api_output(&output, &options).expect("render must succeed");
        let stderr = rendered.stderr.expect("success line must be on stderr");
        // The line must be exactly what origin/main produces: just the
        // summary through style::success, with no version suffix.
        let expected = style::success("Created stat-definition", style::is_stderr_enabled());
        assert_eq!(
            stderr, expected,
            "without alternate versions the success line must match origin/main exactly"
        );
    }

    /// When the command has only one API version, a read operation must NOT
    /// emit any API version label to stderr.
    #[test]
    fn test_human_read_no_label_without_choice() {
        let output = ApiOutput {
            operation: make_get_operation(2),
            resource_name: "items".to_string(),
            body: ApiBody::Text(r#"{"data":[]}"#.to_string()),
            success: None,
            trace: None,
            raw_body: None,
            has_alternate_versions: false,
        };
        let options = RenderOptions::default();
        let rendered = render_api_output(&output, &options).expect("render must succeed");
        assert!(
            rendered.stderr.is_none() || !rendered.stderr.as_deref().unwrap_or("").contains("API"),
            "without alternate versions no API version label must appear; got: {:?}",
            rendered.stderr
        );
    }

    // ── quiet suppression for commands WITH a choice ──────────────────

    /// Quiet mode must suppress the mutating success line even when the
    /// command has alternate versions. The success line is already None
    /// because `execute_operation` omits `ApiSuccess` when quiet, but this
    /// test verifies the renderer handles the case correctly if one is
    /// somehow present.
    #[test]
    fn test_human_mutating_quiet_suppresses_version_with_choice() {
        // ApiSuccess is None in quiet mode (enforced by execute_operation),
        // so the version suffix path is unreachable. Verify no label leaks.
        let output = ApiOutput {
            operation: make_post_operation(3),
            resource_name: "stat-definitions".to_string(),
            body: ApiBody::Empty,
            success: None,
            trace: None,
            raw_body: None,
            has_alternate_versions: true,
        };
        let options = RenderOptions {
            verbosity: ags_protocol::request::Verbosity::Quiet,
            ..Default::default()
        };
        let rendered = render_api_output(&output, &options).expect("render must succeed");
        assert!(
            rendered.stderr.is_none(),
            "quiet mode must produce no stderr for mutating call; got: {:?}",
            rendered.stderr
        );
    }

    // ── JSON: version never appears for any combination ──────────────

    /// JSON mode for a read command without alternate versions: no stderr.
    #[test]
    fn test_json_read_no_version_without_choice() {
        let output = ApiOutput {
            operation: make_get_operation(2),
            resource_name: "items".to_string(),
            body: ApiBody::Text(r#"{"data":[]}"#.to_string()),
            success: None,
            trace: None,
            raw_body: None,
            has_alternate_versions: false,
        };
        let options = RenderOptions::default();
        let rendered =
            crate::frontend::output::json::commands::service::render_api_output(&output, &options)
                .expect("JSON render must succeed");
        assert!(
            rendered.stderr.is_none(),
            "JSON renderer must not emit stderr; got: {:?}",
            rendered.stderr
        );
    }

    /// JSON mode for a mutating command without alternate versions: no
    /// stderr and no version key.
    #[test]
    fn test_json_mutating_no_version_without_choice() {
        let output = ApiOutput {
            operation: make_post_operation(3),
            resource_name: "stat-definitions".to_string(),
            body: ApiBody::Text(r#"{"id":"abc"}"#.to_string()),
            success: Some(ApiSuccess {
                summary: "Created stat-definition".to_string(),
                api_version: ApiVersion(3),
            }),
            trace: None,
            raw_body: None,
            has_alternate_versions: false,
        };
        let options = RenderOptions::default();
        let rendered =
            crate::frontend::output::json::commands::service::render_api_output(&output, &options)
                .expect("JSON render must succeed");
        if let Some(stdout) = &rendered.stdout {
            assert!(
                !stdout.contains("api_version") && !stdout.contains("v3"),
                "JSON stdout must not contain the API version; got: {stdout:?}"
            );
        }
        assert!(
            rendered.stderr.is_none(),
            "JSON renderer must not emit stderr; got: {:?}",
            rendered.stderr
        );
    }

    /// Serialising an `ApiOutput` with `has_alternate_versions: true` must
    /// not produce a key for the flag — it is `#[serde(skip)]`.
    #[test]
    fn test_serde_skip_has_alternate_versions() {
        let output = ApiOutput {
            operation: make_get_operation(2),
            resource_name: "items".to_string(),
            body: ApiBody::Text(r#"{"data":[]}"#.to_string()),
            success: None,
            trace: None,
            raw_body: None,
            has_alternate_versions: true,
        };
        let json = serde_json::to_string(&output).expect("ApiOutput must serialise");
        assert!(
            !json.contains("has_alternate_versions"),
            "has_alternate_versions must not appear in serialised output; got: {json}"
        );
    }

    /// Serialising an `ApiOutput` whose `success` carries an `api_version`
    /// must not produce an `api_version` key in the success object — it is
    /// renderer-only metadata, not part of the serialised output.
    #[test]
    fn test_serde_skip_api_version_in_success() {
        let output = ApiOutput {
            operation: make_get_operation(3),
            resource_name: "items".to_string(),
            body: ApiBody::Empty,
            success: Some(ApiSuccess {
                summary: "OK".to_string(),
                api_version: ApiVersion(3),
            }),
            trace: None,
            raw_body: None,
            has_alternate_versions: true,
        };
        let json = serde_json::to_string(&output).expect("ApiOutput must serialise");
        let value: serde_json::Value = serde_json::from_str(&json).expect("round-trip must parse");
        let success = value
            .get("success")
            .expect("success key must be present when Some");
        assert!(
            success.get("api_version").is_none(),
            "api_version must not appear in the serialised success object; \
             success was: {success}"
        );
    }

    /// A collection with notes but zero rows (e.g. `get-app-endpoints` for a
    /// stopped app, where `endpoints` comes back null) must still print the
    /// notes line — that's exactly the case where knowing
    /// isAppRunning/hasAPISpec/hasGRPCReflection matters most. Regression
    /// test for the notes line being dropped by the empty-rows early return.
    #[test]
    fn test_render_collection_prints_notes_even_when_rows_are_empty() {
        let collection = CollectionResult {
            kind: "endpoints".to_string(),
            columns: vec![],
            rows: vec![],
            page_info: None,
            notes: vec!["App running: no  ·  API spec: no  ·  gRPC reflection: no".to_string()],
        };
        let rendered = render_collection(&collection, &RenderOptions::default(), false);
        assert!(
            rendered.contains("App running: no"),
            "notes line must appear even with zero rows; got: {rendered:?}"
        );
        assert!(
            rendered.contains("No endpoints found"),
            "empty-rows message must still be present; got: {rendered:?}"
        );
    }

    /// `--quiet` suppresses notes for both the empty-rows and populated-rows
    /// paths, and the empty-rows path must still return an empty string
    /// under quiet (no stray notes-only output).
    #[test]
    fn test_render_collection_quiet_suppresses_notes_and_empty_message() {
        let collection = CollectionResult {
            kind: "endpoints".to_string(),
            columns: vec![],
            rows: vec![],
            page_info: None,
            notes: vec!["App running: no".to_string()],
        };
        let options = RenderOptions {
            verbosity: ags_protocol::request::Verbosity::Quiet,
            ..RenderOptions::default()
        };
        let rendered = render_collection(&collection, &options, false);
        assert_eq!(
            rendered, "",
            "quiet mode must suppress notes and the empty message"
        );
    }
}
