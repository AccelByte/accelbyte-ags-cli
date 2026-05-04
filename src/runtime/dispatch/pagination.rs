//! Automatic pagination: fetch multiple pages and merge results.

use serde_json::{json, Value};

use crate::protocol::catalogue::{HttpMethod, OperationSchema, ParameterLocation};
use crate::protocol::error::{RuntimeError, RuntimeErrorKind};
use crate::protocol::event::{ProgressEvent, ProgressSink};
use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
use crate::support::strings::pluralize;

/// Hard cap on pages to prevent runaway requests
const HARD_PAGE_CAP: u64 = 100;

// ── Public entry point ──

/// Fetch all pages of a paginated response and merge results.
///
/// Takes the already-fetched first page. If it has a next cursor and the operation
/// supports pagination, fetches subsequent pages and merges all data arrays.
/// Returns the original response unchanged if not paginated.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn fetch_all_pages(
    client: &dyn HttpClient,
    resolved_url: &str,
    token: &str,
    operation: &OperationSchema,
    initial_response: Value,
    initial_query_params: &[(String, String)],
    page_limit: u64,
    sink: &mut dyn ProgressSink,
) -> Result<Value, RuntimeError> {
    let next_cursor = match extract_next_cursor(&initial_response) {
        Some(cursor) => cursor,
        None => return Ok(initial_response),
    };

    let strategy = match detect_strategy(operation) {
        Some(s) => s,
        None => return Ok(initial_response),
    };

    let first_page_items = extract_data_array(&initial_response)
        .cloned()
        .unwrap_or_default();
    let first_page_paging = initial_response.get("paging").cloned().unwrap_or(json!({}));
    let page_size = page_size_from_query(initial_query_params);

    paginate_loop(
        client,
        resolved_url,
        token,
        &strategy,
        initial_query_params,
        page_size,
        page_limit,
        sink,
        next_cursor,
        first_page_items,
        first_page_paging,
    )
    .await
}

// ── Core loop ──

/// Fetch subsequent pages and accumulate data items.
/// Called after the first page has already been fetched and validated.
#[allow(clippy::too_many_arguments)]
async fn paginate_loop(
    client: &dyn HttpClient,
    resolved_url: &str,
    token: &str,
    strategy: &PaginationStrategy,
    base_query: &[(String, String)],
    page_size: u64,
    page_limit: u64,
    sink: &mut dyn ProgressSink,
    first_cursor: String,
    first_page_items: Vec<Value>,
    first_page_paging: Value,
) -> Result<Value, RuntimeError> {
    let effective_limit = effective_page_limit(page_limit);
    let mut all_items = first_page_items;
    let mut last_paging = first_page_paging;
    let mut current_cursor = first_cursor;
    let mut pages_fetched: u64 = 1;

    loop {
        if pages_fetched >= effective_limit {
            sink.on_event(ProgressEvent::Message {
                text: limit_message_text(page_limit, pages_fetched, all_items.len()),
            });
            break;
        }

        let next_page_number = pages_fetched + 1;
        sink.on_event(ProgressEvent::Page {
            current: next_page_number as usize,
            total: None,
        });

        let next_query = build_next_query(
            base_query,
            strategy,
            &current_cursor,
            pages_fetched,
            page_size,
        );

        let page_response =
            fetch_next_page(client, resolved_url, token, &next_query, next_page_number).await?;

        match extract_data_array(&page_response) {
            Some(data) if !data.is_empty() => all_items.extend(data.clone()),
            _ => break,
        }

        last_paging = page_response.get("paging").cloned().unwrap_or(json!({}));
        pages_fetched += 1;

        match extract_next_cursor(&page_response) {
            Some(cursor) => current_cursor = cursor,
            None => break,
        }
    }

    Ok(build_merged_response(all_items, last_paging))
}

// ── HTTP ──

/// Fetch one page from the API and parse the JSON response
async fn fetch_next_page(
    client: &dyn HttpClient,
    resolved_url: &str,
    token: &str,
    query_params: &[(String, String)],
    page_number: u64,
) -> Result<Value, RuntimeError> {
    let request = HttpRequest {
        method: HttpMethod::Get,
        url: resolved_url.to_string(),
        headers: vec![("Authorization".to_string(), format!("Bearer {token}"))],
        query: query_params.to_vec(),
        body: None,
    };
    let HttpResponse { status, body } = client.send(request).await?;
    let body_text = match body {
        HttpBody::Text(s) => s,
        HttpBody::Binary {
            content_type,
            bytes,
        } => {
            return Err(RuntimeError {
                kind: RuntimeErrorKind::Upstream { status, code: None },
                message: format!(
                    "Paginated endpoint returned binary body ({content_type}, {} bytes) — unsupported.",
                    bytes.len()
                ),
                details: None,
                hint: None,
                trace: None,
            });
        }
    };

    if !(200..300).contains(&status) {
        return Err(RuntimeError {
            kind: RuntimeErrorKind::Upstream { status, code: None },
            message: format!("HTTP {status} error on page {page_number}"),
            details: None,
            hint: None,
            trace: None,
        });
    }

    serde_json::from_str(&body_text).map_err(|e| RuntimeError {
        kind: RuntimeErrorKind::Internal,
        message: format!("Failed to parse page {page_number} response: {e}"),
        details: None,
        hint: None,
        trace: None,
    })
}

// ── Strategy detection ──

/// Pagination strategy detected from the operation's parameter list
#[derive(Debug, Clone, PartialEq)]
enum PaginationStrategy {
    /// Increment offset by page size each request
    Offset,
    /// Pass paging.next cursor as the `after` parameter
    Cursor,
}

/// Detect which pagination strategy an operation supports.
fn detect_strategy(operation: &OperationSchema) -> Option<PaginationStrategy> {
    let is_after_supported = operation.parameters.iter().any(|parameter| {
        parameter.name == "after" && parameter.location == ParameterLocation::Query
    });
    let is_offset_supported = operation.parameters.iter().any(|parameter| {
        parameter.name == "offset" && parameter.location == ParameterLocation::Query
    });

    if is_after_supported {
        Some(PaginationStrategy::Cursor)
    } else if is_offset_supported {
        Some(PaginationStrategy::Offset)
    } else {
        None
    }
}

// ── Response parsing ──

/// Extract the next page cursor from a paginated response
fn extract_next_cursor(response: &Value) -> Option<String> {
    response
        .get("paging")
        .and_then(|p| p.get("next"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Extract the data array from a paginated response
fn extract_data_array(response: &Value) -> Option<&Vec<Value>> {
    response.get("data").and_then(|d| d.as_array())
}

/// Merge accumulated data items into a single response envelope
fn build_merged_response(all_items: Vec<Value>, last_paging: Value) -> Value {
    json!({
        "data": all_items,
        "paging": last_paging,
    })
}

// ── Query building ──

/// Build query params for the next page based on the pagination strategy
fn build_next_query(
    base_query: &[(String, String)],
    strategy: &PaginationStrategy,
    cursor: &str,
    pages_fetched: u64,
    page_size: u64,
) -> Vec<(String, String)> {
    let mut query: Vec<(String, String)> = base_query.to_vec();

    match strategy {
        PaginationStrategy::Cursor => {
            query.retain(|(k, _)| k != "after");
            query.push(("after".to_string(), cursor.to_string()));
        }
        PaginationStrategy::Offset => {
            let new_offset = pages_fetched * page_size;
            query.retain(|(k, _)| k != "offset");
            query.push(("offset".to_string(), new_offset.to_string()));
        }
    }

    query
}

/// Extract the per-page limit from query params (defaults to 100)
fn page_size_from_query(query_params: &[(String, String)]) -> u64 {
    query_params
        .iter()
        .find(|(k, _)| k == "limit")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(100)
}

// ── Limits ──

/// Resolve the effective page limit: user value capped to hard cap
fn effective_page_limit(user_limit: u64) -> u64 {
    if user_limit == 0 {
        HARD_PAGE_CAP
    } else {
        user_limit.min(HARD_PAGE_CAP)
    }
}

/// Build the warning text shown when the page limit is reached.
///
/// Returned as a `String` so the caller can emit it through the sink as a
/// `ProgressEvent::Message`.
fn limit_message_text(page_limit: u64, pages_fetched: u64, item_count: usize) -> String {
    if page_limit == 0 {
        format!(
            "Stopped after {} {}. For larger datasets, use the API directly or process in batches",
            HARD_PAGE_CAP,
            pluralize("page", HARD_PAGE_CAP as usize),
        )
    } else {
        format!(
            "Stopped after {} {} ({} {}). Use --page-limit to increase",
            pages_fetched,
            pluralize("page", pages_fetched as usize),
            item_count,
            pluralize("item", item_count),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::catalogue::{
        ApiVersion, HttpMethod, MutationClass, OperationSchema, ParameterLocation, ParameterSchema,
        ValueType,
    };

    /// Build a minimal optional string parameter at the given location for use in test fixtures.
    fn make_param(name: &str, location: ParameterLocation) -> ParameterSchema {
        ParameterSchema {
            name: name.to_string(),
            location,
            required: false,
            value_type: ValueType::String,
            description: None,
        }
    }

    /// limit_message_text must pluralize "page" and "item" correctly when
    /// pages_fetched or item_count is 1 — otherwise users see "1 pages (1 items)".
    #[test]
    fn test_limit_message_text_pluralizes_page_and_item_at_one() {
        assert_eq!(
            limit_message_text(1, 1, 1),
            "Stopped after 1 page (1 item). Use --page-limit to increase"
        );
        assert_eq!(
            limit_message_text(2, 2, 50),
            "Stopped after 2 pages (50 items). Use --page-limit to increase"
        );
        assert_eq!(
            limit_message_text(5, 1, 42),
            "Stopped after 1 page (42 items). Use --page-limit to increase"
        );
        assert_eq!(
            limit_message_text(5, 3, 1),
            "Stopped after 3 pages (1 item). Use --page-limit to increase"
        );
    }

    /// Build a list-style GET `/items` operation carrying the supplied parameters for tests.
    fn make_operation(parameters: Vec<ParameterSchema>) -> OperationSchema {
        OperationSchema {
            id: crate::protocol::catalogue::OperationId::new("listItems"),
            name: "list".to_string(),
            summary: String::new(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/items".to_string(),
            parameters,
            request_body: None,
            response: None,
            permissions: vec![],
            scope: String::new(),
            api_version: ApiVersion(0),
            deprecated: false,
            response_content_type: None,
        }
    }

    /// `after` + `limit` query params imply cursor-based pagination
    #[test]
    fn test_detect_cursor_strategy() {
        let operation = make_operation(vec![
            make_param("after", ParameterLocation::Query),
            make_param("limit", ParameterLocation::Query),
        ]);
        assert_eq!(
            detect_strategy(&operation),
            Some(PaginationStrategy::Cursor)
        );
    }

    /// `offset` + `limit` query params imply offset-based pagination
    #[test]
    fn test_detect_offset_strategy() {
        let operation = make_operation(vec![
            make_param("offset", ParameterLocation::Query),
            make_param("limit", ParameterLocation::Query),
        ]);
        assert_eq!(
            detect_strategy(&operation),
            Some(PaginationStrategy::Offset)
        );
    }

    /// When both cursor and offset params are present, cursor wins
    #[test]
    fn test_cursor_preferred_over_offset() {
        let operation = make_operation(vec![
            make_param("after", ParameterLocation::Query),
            make_param("offset", ParameterLocation::Query),
            make_param("limit", ParameterLocation::Query),
        ]);
        assert_eq!(
            detect_strategy(&operation),
            Some(PaginationStrategy::Cursor)
        );
    }

    /// Without `after` or `offset`, the operation has no detectable pagination strategy
    #[test]
    fn test_no_strategy_without_params() {
        let operation = make_operation(vec![make_param("limit", ParameterLocation::Query)]);
        assert_eq!(detect_strategy(&operation), None);
    }

    /// `extract_next_cursor` returns the `paging.next` value when it is non-empty
    #[test]
    fn test_extract_next_cursor_from_response() {
        let response = json!({
            "data": [],
            "paging": {"next": "abc123", "previous": ""}
        });
        assert_eq!(extract_next_cursor(&response), Some("abc123".to_string()));
    }

    /// An empty `paging.next` string means pagination is complete
    #[test]
    fn test_extract_next_cursor_empty_means_no_more() {
        let response = json!({
            "data": [],
            "paging": {"next": "", "previous": ""}
        });
        assert_eq!(extract_next_cursor(&response), None);
    }

    /// A missing `paging` field means pagination is complete (single-page response)
    #[test]
    fn test_extract_next_cursor_missing_means_no_more() {
        let response = json!({"data": []});
        assert_eq!(extract_next_cursor(&response), None);
    }

    /// `build_merged_response` concatenates per-page items and carries paging through
    #[test]
    fn test_merge_response_combines_data() {
        let items = vec![json!({"id": "a"}), json!({"id": "b"})];
        let paging = json!({"next": ""});
        let merged = build_merged_response(items, paging);
        assert_eq!(merged["data"].as_array().unwrap().len(), 2);
        assert_eq!(merged["paging"]["next"], "");
    }

    // ── Sink emission test ──

    use crate::protocol::event::{ProgressEvent, ProgressSink};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    /// Test sink that records every event into a Vec for assertion.
    #[derive(Default)]
    struct RecordingSink {
        events: Vec<ProgressEvent>,
    }

    impl ProgressSink for RecordingSink {
        fn on_event(&mut self, event: ProgressEvent) {
            self.events.push(event);
        }
    }

    /// A minimal fake `HttpClient` that returns canned responses keyed by call order.
    ///
    /// Each call pops the next response. Panics if called more times than responses
    /// are available — the test fixture is expected to size its responses exactly.
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

    /// Pagination through 3 pages (initial + 2 follow-ups) emits two `Page`
    /// events in order. No `Message` event fires because the limit is not reached.
    #[tokio::test]
    async fn test_fetch_all_pages_emits_page_events_in_order() {
        // Canned pages are popped in reverse order — `Vec::pop` returns from the
        // end. So push them in reverse of the consumption order.
        //   page 2 (final): data has items, paging.next is empty → loop ends
        //   page 1 (middle): data has items, paging.next present → continue
        let pages = vec![
            r#"{"data": [{"id": "c"}], "paging": {"next": ""}}"#.to_string(),
            r#"{"data": [{"id": "b"}], "paging": {"next": "cursor2"}}"#.to_string(),
        ];
        let fake = FakePages {
            responses: Arc::new(Mutex::new(pages)),
        };

        let initial_response = serde_json::json!({
            "data": [{"id": "a"}],
            "paging": {"next": "cursor1"},
        });

        let operation = make_operation(vec![
            make_param("after", ParameterLocation::Query),
            make_param("limit", ParameterLocation::Query),
        ]);

        let mut sink = RecordingSink::default();
        let result = fetch_all_pages(
            &fake,
            "https://example.com/items",
            "fake-token",
            &operation,
            initial_response,
            &[],
            100,
            &mut sink,
        )
        .await
        .expect("fetch_all_pages should succeed");

        // Three pages of items were merged
        let merged_items = result["data"].as_array().expect("data is array");
        assert_eq!(merged_items.len(), 3);

        // Exactly two Page events were emitted, for pages 2 and 3
        let page_events: Vec<&ProgressEvent> = sink
            .events
            .iter()
            .filter(|event| matches!(event, ProgressEvent::Page { .. }))
            .collect();
        assert_eq!(page_events.len(), 2);
        assert!(matches!(
            page_events[0],
            ProgressEvent::Page {
                current: 2,
                total: None
            }
        ));
        assert!(matches!(
            page_events[1],
            ProgressEvent::Page {
                current: 3,
                total: None
            }
        ));

        // No Message events (limit not reached)
        assert!(!sink
            .events
            .iter()
            .any(|event| matches!(event, ProgressEvent::Message { .. })));
    }
}
