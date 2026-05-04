//! Dispatch execution pipeline for service operations.

use crate::protocol::catalogue::{HttpMethod, MutationClass, OperationSchema};
use crate::protocol::error::{RuntimeError, RuntimeErrorKind};
use crate::protocol::event::{ProgressEvent, ProgressSink};
use crate::protocol::output::{
    ApiBody, ApiOutput, ApiSuccess, CommandOutput, ExecutionTrace, RequestTrace, ResponseTrace,
};
use crate::protocol::request::{CommandRequest, OutputFormat, PaginationHint};
use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
use crate::support::output_sink::OutputSink;
use crate::support::strings::{singularize, strip_terminal_control_sequences};

/// Short-lived per-call context threaded into `execute_operation`.
pub(crate) struct ApiCallContext<'a> {
    pub client: &'a dyn HttpClient,
    pub base_url: &'a str,
    pub token: &'a str,
    pub service_name: &'a str,
    pub resource_name: &'a str,
    pub resolution_trace: Option<crate::protocol::output::ResolutionTrace>,
}

/// Execute an API operation.
pub(crate) async fn execute_operation(
    ctx: &ApiCallContext<'_>,
    operation: &OperationSchema,
    request: &CommandRequest,
    sink: &mut dyn ProgressSink,
) -> Result<CommandOutput, RuntimeError> {
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

    let has_request_body = operation.request_body.is_some();

    let body_value = if has_request_body {
        request.body.clone()
    } else {
        None
    };
    let http_request = HttpRequest {
        method: operation.http_method,
        url: url.clone(),
        headers: vec![("Authorization".to_string(), format!("Bearer {}", ctx.token))],
        query: query_params.clone(),
        body: body_value,
    };

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
        } => {
            use crate::protocol::output::{BinaryWrittenDestination, BinaryWrittenOutput};

            if !(200..300).contains(&status) {
                sink.on_event(ProgressEvent::Finished);
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
            let output_sink = match OutputSink::resolve(request.output.as_ref(), true) {
                Ok(sink) => sink,
                Err(err) => {
                    sink.on_event(ProgressEvent::Finished);
                    return Err(map_cli_error_to_runtime_error(err));
                }
            };
            if let Err(err) = output_sink.write(&bytes) {
                sink.on_event(ProgressEvent::Finished);
                return Err(map_cli_error_to_runtime_error(err));
            }

            let destination = match &output_sink {
                OutputSink::Stdout => BinaryWrittenDestination::Stdout,
                OutputSink::File(path) => BinaryWrittenDestination::File(path.clone()),
            };

            sink.on_event(ProgressEvent::Finished);
            return Ok(CommandOutput::BinaryWritten(BinaryWrittenOutput {
                destination,
                bytes_written,
                content_type,
            }));
        }
    };

    if (200..300).contains(&status) {
        if request.output.is_some() && matches!(request.output_format, OutputFormat::Human) {
            use crate::protocol::output::{BinaryWrittenDestination, BinaryWrittenOutput};

            let bytes_written = body_text.len();
            let output_sink = match OutputSink::resolve(request.output.as_ref(), false) {
                Ok(sink) => sink,
                Err(err) => {
                    sink.on_event(ProgressEvent::Finished);
                    return Err(map_cli_error_to_runtime_error(err));
                }
            };
            if let Err(err) = output_sink.write(body_text.as_bytes()) {
                sink.on_event(ProgressEvent::Finished);
                return Err(map_cli_error_to_runtime_error(err));
            }

            let destination = match &output_sink {
                OutputSink::Stdout => BinaryWrittenDestination::Stdout,
                OutputSink::File(path) => BinaryWrittenDestination::File(path.clone()),
            };

            let declared = operation
                .response_content_type
                .clone()
                .unwrap_or_else(|| "application/json".to_string());

            sink.on_event(ProgressEvent::Finished);
            return Ok(CommandOutput::BinaryWritten(BinaryWrittenOutput {
                destination,
                bytes_written,
                content_type: declared,
            }));
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
                summary: format!("{} {}.", verb, noun),
            })
        } else {
            None
        };

        let body = if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&body_text) {
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
                crate::protocol::result::CommandResult::Raw(crate::protocol::result::RawResult {
                    value: final_value,
                })
            } else {
                crate::runtime::dispatch::shape::shape_response(
                    &final_value,
                    operation,
                    ctx.resource_name,
                    request.verbosity.is_verbose(),
                )
            };
            ApiBody::Shaped(Box::new(shaped))
        } else if !body_text.is_empty() {
            ApiBody::Text(strip_terminal_control_sequences(&body_text))
        } else {
            ApiBody::Empty
        };

        let trace = if request.verbosity.is_verbose() {
            let request_body_size = if has_request_body {
                request
                    .body
                    .as_ref()
                    .map(|value| serde_json::to_string(value).unwrap_or_default().len())
            } else {
                None
            };
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
        })))
    } else {
        sink.on_event(ProgressEvent::Finished);
        // If the body is JSON, hand it straight to classify. Otherwise wrap
        // the raw body as `errorMessage` so classify still produces the right
        // status-based message + suggestion (instead of falling back to a
        // bare "HTTP N error." with an empty Detail line).
        let error_object = serde_json::from_str::<serde_json::Value>(&body_text).unwrap_or_else(
            |_| {
                let cleaned = strip_terminal_control_sequences(&body_text);
                if cleaned.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::json!({ "errorMessage": cleaned })
                }
            },
        );
        let mut runtime_error = crate::runtime::dispatch::classify::classify_to_runtime_error(
            status,
            &error_object,
            ctx.service_name,
            ctx.resource_name,
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

/// Derive the noun used in a mutation success message.
fn derive_success_noun(method_name: &str, resource_name: &str) -> String {
    if let Some(pos) = method_name.find('-') {
        let after = &method_name[pos + 1..];
        return after.replace('-', " ");
    }
    singularize(resource_name)
}

/// Convert an output error into a runtime error.
fn map_cli_error_to_runtime_error(err: crate::errors::CliError) -> RuntimeError {
    match err {
        crate::errors::CliError::Usage { message, .. } => RuntimeError {
            kind: RuntimeErrorKind::Validation,
            message,
            details: None,
            hint: None,
                    trace: None,
        },
        other => RuntimeError {
            kind: RuntimeErrorKind::Internal,
            message: other.to_string(),
            details: None,
            hint: None,
                    trace: None,
        },
    }
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
    use crate::protocol::catalogue::{
        ApiVersion, MutationClass, OperationId, ParameterLocation, ParameterSchema, ValueType,
    };
    use crate::protocol::request::Verbosity;
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
        ) -> Result<HttpResponse, crate::protocol::error::RuntimeError> {
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
            description: None,
        }
    }

    /// Build a paginated GET `/items` operation with `after` and `limit` query params for tests.
    fn make_paginated_operation() -> OperationSchema {
        OperationSchema {
            id: crate::protocol::catalogue::OperationId::new("listItems"),
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
        };

        let request = CommandRequest {
            service: crate::catalogue::Catalogue::find_id("iam").expect("iam in manifest"),
            operation_id: OperationId::new("listItems"),
            namespace: None,
            path_params: BTreeMap::new(),
            query_params: BTreeMap::new(),
            header_params: BTreeMap::new(),
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
        };
        let request = CommandRequest {
            service: crate::catalogue::Catalogue::find_id("iam").expect("iam in manifest"),
            operation_id: OperationId::new("listItems"),
            namespace: None,
            path_params: BTreeMap::new(),
            query_params: BTreeMap::new(),
            header_params: BTreeMap::new(),
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
}
