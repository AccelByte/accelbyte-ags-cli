//! Dispatch execution pipeline for service operations.

use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
use crate::support::output_sink::OutputSink;
use crate::support::strings::{singularize, strip_terminal_control_sequences};
use ags_protocol::catalogue::{HttpMethod, MutationClass, OperationSchema};
use ags_protocol::error::{RuntimeError, RuntimeErrorKind};
use ags_protocol::event::{ProgressEvent, ProgressSink};
use ags_protocol::output::{
    ApiBody, ApiOutput, ApiSuccess, CommandOutput, ExecutionTrace, RequestTrace, ResponseTrace,
};
use ags_protocol::request::{CommandRequest, OutputFormat, PaginationHint};

/// Short-lived per-call context threaded into `execute_operation`.
pub(crate) struct ApiCallContext<'a> {
    pub client: &'a dyn HttpClient,
    pub base_url: &'a str,
    pub token: &'a str,
    pub service_name: &'a str,
    pub resource_name: &'a str,
    pub resolution_trace: Option<ags_protocol::output::ResolutionTrace>,
    /// Whether the scope entry that resolved this operation has more than one
    /// API version. Threaded from the catalogue lookup to `ApiOutput` so the
    /// human renderer can decide whether to show the version label.
    pub has_alternate_versions: bool,
}

/// Execute an API operation.
pub(crate) async fn execute_operation(
    ctx: &ApiCallContext<'_>,
    operation: &OperationSchema,
    request: &CommandRequest,
    sink: &mut dyn ProgressSink,
) -> Result<CommandOutput, RuntimeError> {
    let (url, query_params, mut http_request) = build_base_request(ctx, operation, request)?;
    http_request.body = request.body.clone();

    sink.on_event(ProgressEvent::Started {
        message: format!(
            "{}...",
            progress_verb_for_operation(operation.http_method.as_str(), operation.mutation_class)
        ),
    });

    let HttpResponse { status, body } = ctx.client.send(http_request).await?;
    let body_text = match body {
        HttpBody::Text(s) => s,
        HttpBody::Binary {
            content_type,
            bytes,
        } => return write_binary_response(status, content_type, bytes, request, sink),
    };

    if (200..300).contains(&status) {
        if request.output.is_some() && matches!(request.output_format, OutputFormat::Human) {
            return write_text_response(&body_text, operation, request, sink);
        }

        let is_mutating_operation = operation.mutation_class == MutationClass::Mutating;
        let success = if is_mutating_operation && !request.verbosity.is_quiet() {
            let verb = match operation.http_method {
                HttpMethod::Post => "Created",
                HttpMethod::Put | HttpMethod::Patch => "Updated",
                HttpMethod::Delete => "Deleted",
                _ => "Completed",
            };
            let noun = derive_success_noun(&operation.name, ctx.resource_name);
            Some(ApiSuccess {
                // No trailing full stop: this is a status line, not a sentence,
                // matching the other success lines ("Token refreshed", etc.).
                summary: format!("{} {}", verb, noun),
                api_version: operation.api_version,
            })
        } else {
            None
        };

        let (body, raw_body) =
            if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&body_text) {
                let final_value = if matches!(
                    request.pagination,
                    PaginationHint::All | PaginationHint::Limit(_)
                ) {
                    let page_limit_value = match request.pagination {
                        PaginationHint::Limit(limit) => limit,
                        PaginationHint::All => 0,
                        _ => 0,
                    };
                    crate::runtime::dispatch::pagination::fetch_all_pages(
                        ctx.client,
                        &url,
                        ctx.token,
                        operation,
                        json_value,
                        &query_params,
                        page_limit_value,
                        sink,
                    )
                    .await?
                } else {
                    json_value
                };
                let shaped = if matches!(request.output_format, OutputFormat::Json) {
                    ags_protocol::result::CommandResult::Raw(ags_protocol::result::RawResult {
                        value: final_value.clone(),
                    })
                } else {
                    crate::runtime::dispatch::shape::shape_response(
                        &final_value,
                        operation,
                        ctx.resource_name,
                        request.verbosity.is_verbose(),
                    )
                };
                // Retain the raw JSON for workflow captures (see ApiOutput.raw_body).
                (ApiBody::Shaped(Box::new(shaped)), Some(final_value))
            } else if !body_text.is_empty() {
                (
                    ApiBody::Text(strip_terminal_control_sequences(&body_text)),
                    None,
                )
            } else {
                (ApiBody::Empty, None)
            };

        let trace = if request.verbosity.is_verbose() {
            let request_body_size = request.body.as_ref().map(|body| match body {
                ags_protocol::request::RequestBody::Json(value) => {
                    serde_json::to_string(value).unwrap_or_default().len()
                }
                ags_protocol::request::RequestBody::Multipart(parts) => parts
                    .iter()
                    .map(|part| match part {
                        ags_protocol::request::FormPart::Text { value, .. } => value.len(),
                        ags_protocol::request::FormPart::File { path, .. } => {
                            std::fs::metadata(path)
                                .map(|m| m.len() as usize)
                                .unwrap_or(0)
                        }
                    })
                    .sum(),
            });
            Some(ExecutionTrace {
                resolution: ctx.resolution_trace.clone(),
                request: RequestTrace {
                    http_method: operation.http_method.as_str().to_string(),
                    url,
                    query_params: query_params.into_iter().collect(),
                    has_auth_header: true,
                    body_size: request_body_size,
                },
                response: Some(ResponseTrace {
                    status,
                    reason: None,
                    body_size: Some(body_text.len()),
                }),
            })
        } else {
            None
        };

        sink.on_event(ProgressEvent::Finished);
        Ok(CommandOutput::Service(Box::new(ApiOutput {
            operation: operation.clone(),
            resource_name: ctx.resource_name.to_string(),
            body,
            success,
            trace,
            raw_body,
            has_alternate_versions: ctx.has_alternate_versions,
        })))
    } else {
        sink.on_event(ProgressEvent::Finished);
        let mut runtime_error = classify_error_body(
            status,
            &body_text,
            ctx.service_name,
            ctx.resource_name,
            &operation.name,
        );
        // Attach the verbose trace so the frontend can render the same
        // request/response diagnostic block on the error path that it does
        // on success. Without this, `--verbose` is silent on failures.
        if request.verbosity.is_verbose() {
            runtime_error.trace = Some(Box::new(ExecutionTrace {
                resolution: ctx.resolution_trace.clone(),
                request: RequestTrace {
                    http_method: operation.http_method.as_str().to_string(),
                    url,
                    query_params: query_params.into_iter().collect(),
                    has_auth_header: true,
                    body_size: None,
                },
                response: Some(ResponseTrace {
                    status,
                    reason: None,
                    body_size: Some(body_text.len()),
                }),
            }));
        }
        Err(runtime_error)
    }
}

/// Write a binary response body to the resolved output sink (file or stdout)
/// and return a `BinaryWritten` output. A non-2xx status is surfaced as an
/// upstream error; sink/write failures are mapped to runtime errors. Emits the
/// terminal `Finished` progress event on every path.
fn write_binary_response(
    status: u16,
    content_type: String,
    bytes: Vec<u8>,
    request: &CommandRequest,
    sink: &mut dyn ProgressSink,
) -> Result<CommandOutput, RuntimeError> {
    use ags_protocol::output::{BinaryWrittenDestination, BinaryWrittenOutput};

    // The body never touches the progress sink; `Finished` is emitted exactly
    // once, after it, on both the success and error paths.
    let result = (move || {
        if !(200..300).contains(&status) {
            return Err(RuntimeError {
                kind: RuntimeErrorKind::Upstream { status, code: None },
                message: format!(
                    "HTTP {status} with binary body ({content_type}, {} bytes).",
                    bytes.len()
                ),
                details: None,
                hint: None,
                trace: None,
            });
        }
        let bytes_written = bytes.len();
        let output_sink = OutputSink::resolve(request.output.as_ref(), true)
            .map_err(map_output_sink_error_to_runtime_error)?;
        output_sink
            .write(&bytes)
            .map_err(map_output_sink_error_to_runtime_error)?;
        let destination = match &output_sink {
            OutputSink::Stdout => BinaryWrittenDestination::Stdout,
            OutputSink::File(path) => BinaryWrittenDestination::File(path.clone()),
        };
        Ok(CommandOutput::BinaryWritten(BinaryWrittenOutput {
            destination,
            bytes_written,
            content_type,
        }))
    })();

    sink.on_event(ProgressEvent::Finished);
    result
}

/// Write a (2xx) text response body to the resolved output sink for the
/// `--output` + human path, returning a `BinaryWritten` output tagged with the
/// operation's declared content type. Emits the terminal `Finished` event.
fn write_text_response(
    body_text: &str,
    operation: &OperationSchema,
    request: &CommandRequest,
    sink: &mut dyn ProgressSink,
) -> Result<CommandOutput, RuntimeError> {
    use ags_protocol::output::{BinaryWrittenDestination, BinaryWrittenOutput};

    // The body never touches the progress sink; `Finished` is emitted exactly
    // once, after it, on both the success and error paths.
    let result = (|| {
        let bytes_written = body_text.len();
        let output_sink = OutputSink::resolve(request.output.as_ref(), false)
            .map_err(map_output_sink_error_to_runtime_error)?;
        output_sink
            .write(body_text.as_bytes())
            .map_err(map_output_sink_error_to_runtime_error)?;
        let destination = match &output_sink {
            OutputSink::Stdout => BinaryWrittenDestination::Stdout,
            OutputSink::File(path) => BinaryWrittenDestination::File(path.clone()),
        };
        let declared = operation
            .response_content_type
            .clone()
            .unwrap_or_else(|| "application/json".to_string());
        Ok(CommandOutput::BinaryWritten(BinaryWrittenOutput {
            destination,
            bytes_written,
            content_type: declared,
        }))
    })();

    sink.on_event(ProgressEvent::Finished);
    result
}

/// Dispatch a read operation and return its **raw** aggregated response body
/// (`serde_json::Value`, post-pagination, pre-shaping). Unlike
/// `execute_operation`, this performs no response shaping, no output-sink
/// writing, and no trace assembly — it is the entry the dynamic-enum option
/// resolver projects JSONPaths against. `items_path` names the array to merge
/// across pages. The caller guarantees the operation is a read (`GET`, enforced
/// at workflow compile time), so no request body is sent.
pub(crate) async fn fetch_raw_body(
    ctx: &ApiCallContext<'_>,
    operation: &OperationSchema,
    request: &CommandRequest,
    items_path: &str,
    // `+ Send`: awaited inside an off-thread `Handle::spawn` task by the
    // dynamic-enum option resolver, so the held sink must be `Send`.
    sink: &mut (dyn ProgressSink + Send),
) -> Result<serde_json::Value, RuntimeError> {
    let (url, query_params, http_request) = build_base_request(ctx, operation, request)?;

    let HttpResponse { status, body } = ctx.client.send(http_request).await?;
    let body_text = match body {
        HttpBody::Text(s) => s,
        HttpBody::Binary {
            content_type,
            bytes,
        } => {
            return Err(RuntimeError {
                kind: RuntimeErrorKind::Upstream { status, code: None },
                message: format!(
                    "options fetch returned a binary body ({content_type}, {} bytes)",
                    bytes.len()
                ),
                details: None,
                hint: None,
                trace: None,
            });
        }
    };

    if !(200..300).contains(&status) {
        return Err(classify_error_body(
            status,
            &body_text,
            ctx.service_name,
            ctx.resource_name,
            &operation.name,
        ));
    }

    let json_value: serde_json::Value =
        serde_json::from_str(&body_text).map_err(|err| RuntimeError {
            kind: RuntimeErrorKind::Internal,
            message: format!("options fetch returned a non-JSON body: {err}"),
            details: None,
            hint: None,
            trace: None,
        })?;

    crate::runtime::dispatch::pagination::fetch_all_pages_at_path(
        ctx.client,
        &url,
        ctx.token,
        operation,
        json_value,
        &query_params,
        0,
        items_path,
        sink,
    )
    .await
}

/// Derive the noun used in a mutation success message.
fn derive_success_noun(method_name: &str, resource_name: &str) -> String {
    if let Some(pos) = method_name.find('-') {
        let after = &method_name[pos + 1..];
        return after.replace('-', " ");
    }
    singularize(resource_name)
}

/// Convert an output-sink error into a runtime error.
fn map_output_sink_error_to_runtime_error(
    err: crate::support::output_sink::OutputSinkError,
) -> RuntimeError {
    use crate::support::output_sink::OutputSinkError;
    match err {
        OutputSinkError::Usage(message) => RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message,
            details: None,
            hint: None,
            trace: None,
        },
        OutputSinkError::Internal(inner) => RuntimeError {
            kind: RuntimeErrorKind::Internal,
            message: inner.to_string(),
            details: None,
            hint: None,
            trace: None,
        },
    }
}

/// The URL, collected query params, and base auth'd request for an operation,
/// as produced by [`build_base_request`].
type BaseRequest = (String, Vec<(String, String)>, HttpRequest);

/// Build the URL, collected query params, and a base auth'd `HttpRequest`
/// (Bearer header, no body) for an operation. Shared by `execute_operation` and
/// `fetch_raw_body` so path substitution, URL assembly, the query projection,
/// and the auth header live in one place; callers attach a request body when the
/// operation takes one.
fn build_base_request(
    ctx: &ApiCallContext<'_>,
    operation: &OperationSchema,
    request: &CommandRequest,
) -> Result<BaseRequest, RuntimeError> {
    let path = crate::runtime::dispatch::path::substitute_path_params(
        &operation.path_template,
        &request.path_params,
    )?;
    let url = format!("{}{}", ctx.base_url.trim_end_matches('/'), path);
    let query_params: Vec<(String, String)> = request
        .query_params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let http_request = HttpRequest {
        method: operation.http_method,
        url: url.clone(),
        headers: vec![("Authorization".to_string(), format!("Bearer {}", ctx.token))],
        query: query_params.clone(),
        body: None,
    };
    Ok((url, query_params, http_request))
}

/// Classify a non-2xx response body into a `RuntimeError`. A JSON body is
/// classified directly; a non-JSON body is sanitised and wrapped as
/// `errorMessage` so classify still produces a status-based message and
/// suggestion instead of a bare "HTTP N" with an empty detail line. Shared by
/// `execute_operation` and `fetch_raw_body` so both sanitise identically.
fn classify_error_body(
    status: u16,
    body_text: &str,
    service_name: &str,
    resource_name: &str,
    method_name: &str,
) -> RuntimeError {
    let error_object = serde_json::from_str::<serde_json::Value>(body_text).unwrap_or_else(|_| {
        let cleaned = strip_terminal_control_sequences(body_text);
        if cleaned.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::json!({ "errorMessage": cleaned })
        }
    });
    crate::runtime::dispatch::classify::classify_to_runtime_error(
        status,
        &error_object,
        service_name,
        resource_name,
        method_name,
    )
}

/// Map an HTTP method and mutation class to a progress verb.
///
/// Read-classified operations (including POST list-by-ids and similar) report
/// "Fetching" rather than the verb the HTTP method alone would suggest.
fn progress_verb_for_operation(method: &str, mutation_class: MutationClass) -> &'static str {
    if matches!(
        mutation_class,
        MutationClass::ReadOnly | MutationClass::Diagnostic
    ) {
        return "Fetching";
    }
    match method {
        "GET" | "HEAD" | "OPTIONS" => "Fetching",
        "POST" => "Creating",
        "PUT" | "PATCH" => "Updating",
        "DELETE" => "Deleting",
        _ => "Requesting",
    }
}

#[cfg(test)]
mod progress_verb_for_operation_tests {
    use super::*;

    /// GET maps to "Fetching" for the status line
    #[test]
    fn test_progress_verb_get() {
        assert_eq!(
            progress_verb_for_operation("GET", MutationClass::ReadOnly),
            "Fetching"
        );
    }

    /// HEAD maps to "Fetching" like GET since it is a read operation
    #[test]
    fn test_progress_verb_head() {
        assert_eq!(
            progress_verb_for_operation("HEAD", MutationClass::ReadOnly),
            "Fetching"
        );
    }

    /// OPTIONS maps to "Fetching" like GET since it is a read operation
    #[test]
    fn test_progress_verb_options() {
        assert_eq!(
            progress_verb_for_operation("OPTIONS", MutationClass::ReadOnly),
            "Fetching"
        );
    }

    /// Mutating POSTs map to "Creating"
    #[test]
    fn test_progress_verb_post_mutating() {
        assert_eq!(
            progress_verb_for_operation("POST", MutationClass::Mutating),
            "Creating"
        );
    }

    /// Read-classified POST (e.g. list-by-ids) maps to "Fetching"
    #[test]
    fn test_progress_verb_post_read_only() {
        assert_eq!(
            progress_verb_for_operation("POST", MutationClass::ReadOnly),
            "Fetching"
        );
    }

    /// Diagnostic POST also reads, so it maps to "Fetching"
    #[test]
    fn test_progress_verb_post_diagnostic() {
        assert_eq!(
            progress_verb_for_operation("POST", MutationClass::Diagnostic),
            "Fetching"
        );
    }

    /// PUT maps to "Updating" since it replaces a resource
    #[test]
    fn test_progress_verb_put() {
        assert_eq!(
            progress_verb_for_operation("PUT", MutationClass::Mutating),
            "Updating"
        );
    }

    /// PATCH maps to "Updating" since it partially modifies a resource
    #[test]
    fn test_progress_verb_patch() {
        assert_eq!(
            progress_verb_for_operation("PATCH", MutationClass::Mutating),
            "Updating"
        );
    }

    /// DELETE maps to "Deleting"
    #[test]
    fn test_progress_verb_delete() {
        assert_eq!(
            progress_verb_for_operation("DELETE", MutationClass::Mutating),
            "Deleting"
        );
    }

    /// Unknown HTTP methods fall back to the generic "Requesting"
    #[test]
    fn test_progress_verb_unknown() {
        assert_eq!(
            progress_verb_for_operation("TRACE", MutationClass::Mutating),
            "Requesting"
        );
    }
}

#[cfg(test)]
mod progress_event_order_tests {
    use super::*;
    use ags_protocol::catalogue::{
        ApiVersion, MutationClass, OperationId, ParameterLocation, ParameterSchema, ValueType,
    };
    use ags_protocol::request::Verbosity;
    use async_trait::async_trait;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct RecordingSink {
        events: Vec<ProgressEvent>,
    }

    impl ProgressSink for RecordingSink {
        fn on_event(&mut self, event: ProgressEvent) {
            self.events.push(event);
        }
    }

    struct FakePages {
        responses: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl HttpClient for FakePages {
        async fn send(
            &self,
            _request: HttpRequest,
        ) -> Result<HttpResponse, ags_protocol::error::RuntimeError> {
            let body = self
                .responses
                .lock()
                .unwrap()
                .pop()
                .expect("FakePages ran out of canned responses");
            Ok(HttpResponse {
                status: 200,
                body: HttpBody::Text(body),
            })
        }
    }

    /// Build a minimal optional string query parameter for use in test fixtures.
    fn make_query_param(name: &str) -> ParameterSchema {
        ParameterSchema {
            name: name.to_string(),
            location: ParameterLocation::Query,
            required: false,
            value_type: ValueType::String,
            is_file: false,
            description: None,
            default: None,
        }
    }

    /// Build a paginated GET `/items` operation with `after` and `limit` query params for tests.
    fn make_paginated_operation() -> OperationSchema {
        OperationSchema {
            id: ags_protocol::catalogue::OperationId::new("listItems"),
            name: "list".to_string(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/items".to_string(),
            parameters: vec![make_query_param("after"), make_query_param("limit")],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(0),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// A fake `HttpClient` that records the last request it was sent, so a
    /// test can assert on what `execute_operation` actually dispatched.
    #[derive(Default)]
    struct RecordingClient {
        sent: std::sync::Mutex<Option<HttpRequest>>,
    }

    #[async_trait]
    impl HttpClient for RecordingClient {
        async fn send(
            &self,
            request: HttpRequest,
        ) -> Result<HttpResponse, ags_protocol::error::RuntimeError> {
            *self.sent.lock().unwrap() = Some(request);
            Ok(HttpResponse {
                status: 200,
                body: HttpBody::Text(r#"{"ok":true}"#.to_string()),
            })
        }
    }

    /// Build a minimal mutating POST operation at a specific API version.
    fn make_mutating_post_operation(version: u32) -> OperationSchema {
        OperationSchema {
            id: OperationId::new("createItem"),
            name: "create".to_string(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::Mutating,
            http_method: HttpMethod::Post,
            path_template: "/items".to_string(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(version),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// `ApiSuccess` carries the operation's `api_version` so renderers can
    /// show which contract version was used without parsing the summary.
    #[tokio::test]
    async fn test_api_success_carries_api_version_for_mutating_call() {
        let client = RecordingClient::default();
        let operation = make_mutating_post_operation(3);
        let ctx = ApiCallContext {
            client: &client,
            base_url: "https://example.com",
            token: "fake-token",
            service_name: "social",
            resource_name: "stat-definitions",
            resolution_trace: None,
            has_alternate_versions: false,
        };
        let request = CommandRequest {
            service: crate::catalogue::Catalogue::find_id("social").expect("social in manifest"),
            operation_id: OperationId::new("createItem"),
            namespace: None,
            path_params: BTreeMap::new(),
            query_params: BTreeMap::new(),
            header_params: BTreeMap::new(),
            form_params: BTreeMap::new(),
            body: None,
            output_format: OutputFormat::Human,
            pagination: PaginationHint::Auto,
            verbosity: Verbosity::Normal,
            output: None,
        };
        let mut sink = RecordingSink::default();
        let result = execute_operation(&ctx, &operation, &request, &mut sink)
            .await
            .expect("execute_operation should succeed");

        let api_output = match result {
            CommandOutput::Service(ref output) => output,
            other => panic!("expected Service output, got: {other:?}"),
        };
        let success = api_output
            .success
            .as_ref()
            .expect("mutating call must produce ApiSuccess");

        assert_eq!(
            success.api_version,
            ApiVersion(3),
            "ApiSuccess must carry the operation's api_version"
        );
    }

    /// A formData operation with one text-only parameter — no path/query
    /// params, no body (this test exercises the body-wiring bug, which
    /// affects any formData operation, not just file-typed ones).
    fn make_formdata_text_operation() -> OperationSchema {
        OperationSchema {
            id: OperationId::new("importSomething"),
            name: "import".to_string(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::Mutating,
            http_method: HttpMethod::Post,
            path_template: "/import".to_string(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// A formData operation's resolved `RequestBody::Multipart` is actually
    /// attached to the outbound `HttpRequest` — regression for the bug where
    /// `execute_operation` only copied `request.body` when
    /// `operation.request_body.is_some()`, which is never true for a
    /// formData operation (body and formData are mutually exclusive per
    /// OAS2, so `operation.request_body` is always `None` here).
    #[tokio::test]
    async fn test_execute_operation_attaches_multipart_body_for_formdata_operation() {
        let client = RecordingClient::default();
        let operation = make_formdata_text_operation();
        let ctx = ApiCallContext {
            client: &client,
            base_url: "https://example.com",
            token: "fake-token",
            service_name: "svc",
            resource_name: "import",
            resolution_trace: None,
            has_alternate_versions: false,
        };
        let request = CommandRequest {
            service: crate::catalogue::Catalogue::find_id("csm").expect("csm in manifest"),
            operation_id: OperationId::new("importSomething"),
            namespace: None,
            path_params: BTreeMap::new(),
            query_params: BTreeMap::new(),
            header_params: BTreeMap::new(),
            body: Some(ags_protocol::request::RequestBody::Multipart(vec![
                ags_protocol::request::FormPart::Text {
                    name: "strategy".to_string(),
                    value: "REPLACE".to_string(),
                },
            ])),
            form_params: BTreeMap::new(),
            output_format: OutputFormat::Human,
            pagination: PaginationHint::Auto,
            verbosity: Verbosity::Normal,
            output: None,
        };
        let mut sink = RecordingSink::default();

        let _ = execute_operation(&ctx, &operation, &request, &mut sink).await;

        let sent = client.sent.lock().unwrap();
        let sent_request = sent
            .as_ref()
            .expect("execute_operation must call client.send");
        assert!(matches!(
            sent_request.body,
            Some(ags_protocol::request::RequestBody::Multipart(_))
        ));
    }

    /// `Finished` must be emitted after the last pagination `Page` event.
    #[tokio::test]
    async fn test_execute_operation_emits_finished_after_all_pages_when_page_all() {
        // Pages pop in reverse order from the Vec.
        let canned_pages = vec![
            // Final page: no next cursor → pagination loop ends.
            r#"{"data": [{"id": "c"}], "paging": {"next": ""}}"#.to_string(),
            // Middle page: has a next cursor → loop continues.
            r#"{"data": [{"id": "b"}], "paging": {"next": "cursor2"}}"#.to_string(),
            // Initial page: has a next cursor → triggers pagination.
            r#"{"data": [{"id": "a"}], "paging": {"next": "cursor1"}}"#.to_string(),
        ];
        let fake = FakePages {
            responses: Arc::new(Mutex::new(canned_pages)),
        };

        let operation = make_paginated_operation();

        let ctx = ApiCallContext {
            client: &fake,
            base_url: "https://example.com",
            token: "fake-token",
            service_name: "iam",
            resource_name: "roles",
            resolution_trace: None,
            has_alternate_versions: false,
        };

        let request = CommandRequest {
            service: crate::catalogue::Catalogue::find_id("iam").expect("iam in manifest"),
            operation_id: OperationId::new("listItems"),
            namespace: None,
            path_params: BTreeMap::new(),
            query_params: BTreeMap::new(),
            header_params: BTreeMap::new(),
            form_params: BTreeMap::new(),
            body: None,
            output_format: OutputFormat::Human,
            pagination: PaginationHint::All,
            verbosity: Verbosity::Normal,
            output: None,
        };

        let mut sink = RecordingSink::default();
        execute_operation(&ctx, &operation, &request, &mut sink)
            .await
            .expect("execute_operation should succeed");

        let started_index = sink
            .events
            .iter()
            .position(|event| matches!(event, ProgressEvent::Started { .. }))
            .expect("Started event missing");
        let finished_index = sink
            .events
            .iter()
            .position(|event| matches!(event, ProgressEvent::Finished))
            .expect("Finished event missing");
        let page_indexes: Vec<usize> = sink
            .events
            .iter()
            .enumerate()
            .filter_map(|(i, event)| {
                if matches!(event, ProgressEvent::Page { .. }) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();

        assert!(
            !page_indexes.is_empty(),
            "expected Page events during pagination, got none. Full sequence: {:?}",
            sink.events
        );
        assert!(
            started_index < page_indexes[0],
            "Started must fire before the first Page. Full sequence: {:?}",
            sink.events
        );
        for &page_index in &page_indexes {
            assert!(
                page_index < finished_index,
                "Finished fired before Page event at index {page_index}. The spinner is killed mid-pagination. Full sequence: {:?}",
                sink.events
            );
        }
    }

    /// `PaginationHint::All` must fetch beyond ten pages.
    #[tokio::test]
    async fn test_page_all_fetches_beyond_ten_pages() {
        let page_ids: Vec<char> = ('a'..='o').collect();
        let last_index = page_ids.len() - 1;
        let mut canned: Vec<String> = page_ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let next = if i == last_index {
                    String::new()
                } else {
                    format!("cursor{}", i + 1)
                };
                format!(r#"{{"data": [{{"id": "{id}"}}], "paging": {{"next": "{next}"}}}}"#)
            })
            .collect();
        canned.reverse();

        let fake = FakePages {
            responses: Arc::new(Mutex::new(canned)),
        };

        let operation = make_paginated_operation();
        let ctx = ApiCallContext {
            client: &fake,
            base_url: "https://example.com",
            token: "fake-token",
            service_name: "iam",
            resource_name: "roles",
            resolution_trace: None,
            has_alternate_versions: false,
        };
        let request = CommandRequest {
            service: crate::catalogue::Catalogue::find_id("iam").expect("iam in manifest"),
            operation_id: OperationId::new("listItems"),
            namespace: None,
            path_params: BTreeMap::new(),
            query_params: BTreeMap::new(),
            header_params: BTreeMap::new(),
            form_params: BTreeMap::new(),
            body: None,
            output_format: OutputFormat::Human,
            pagination: PaginationHint::All,
            verbosity: Verbosity::Quiet,
            output: None,
        };

        let mut sink = RecordingSink::default();
        execute_operation(&ctx, &operation, &request, &mut sink)
            .await
            .expect("execute_operation should succeed");

        assert!(
            fake.responses.lock().unwrap().is_empty(),
            "all 15 pages should be fetched under PaginationHint::All; \
             a 10-page cap would leave responses unconsumed"
        );
    }

    /// `fetch_raw_body` returns the merged raw JSON body (post-pagination,
    /// pre-shaping) for a paginated list, merging the array at `items_path`.
    #[tokio::test]
    async fn test_fetch_raw_body_merges_named_array_unshaped() {
        let canned_pages = vec![
            r#"{"images": [{"id": "c"}], "paging": {"next": ""}}"#.to_string(),
            r#"{"images": [{"id": "b"}], "paging": {"next": "cursor2"}}"#.to_string(),
            r#"{"images": [{"id": "a"}], "paging": {"next": "cursor1"}}"#.to_string(),
        ];
        let fake = FakePages {
            responses: Arc::new(Mutex::new(canned_pages)),
        };
        let operation = make_paginated_operation();
        let ctx = ApiCallContext {
            client: &fake,
            base_url: "https://example.com",
            token: "fake-token",
            service_name: "ams",
            resource_name: "images",
            resolution_trace: None,
            has_alternate_versions: false,
        };
        let request = CommandRequest {
            service: crate::catalogue::Catalogue::find_id("ams").expect("ams in manifest"),
            operation_id: OperationId::new("listItems"),
            namespace: None,
            path_params: BTreeMap::new(),
            query_params: BTreeMap::new(),
            header_params: BTreeMap::new(),
            form_params: BTreeMap::new(),
            body: None,
            output_format: OutputFormat::Json,
            pagination: PaginationHint::All,
            verbosity: Verbosity::Quiet,
            output: None,
        };
        let mut sink = RecordingSink::default();
        let body = super::fetch_raw_body(&ctx, &operation, &request, "$.images", &mut sink)
            .await
            .expect("fetch_raw_body ok");
        let imgs = body
            .get("images")
            .and_then(|v| v.as_array())
            .expect("images array survives");
        let ids: Vec<&str> = imgs.iter().filter_map(|e| e["id"].as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }
}
