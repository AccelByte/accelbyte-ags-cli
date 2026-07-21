//! Dynamic-enum option resolution: run a catalogue list operation and project
//! its raw response body into `(label, value)` choices. v1 backs string-typed
//! workflow inputs declaring an `options_source`. Side-effect free — the
//! referenced operation is validated to be HTTP GET at compile time.

use std::collections::BTreeMap;

use ags_protocol::catalogue::{OperationSchema, ParameterLocation};
use ags_protocol::error::RuntimeError;
use ags_protocol::event::{ProgressEvent, ProgressSink};
use ags_protocol::request::{CommandRequest, OutputFormat, PaginationHint, Verbosity};
use ags_protocol::workflow::{
    LabelDetail, OptionChoice, OptionParameterBinding, OptionsSource, ResolvedOptions,
};

use crate::runtime::workflows::auto_derive::find_operation_or_error;
use crate::runtime::workflows::jsonpath::apply_jsonpath_subset;
use crate::runtime::workflows::resolve::json_to_param_string;
use crate::runtime::Runtime;

/// Maximum number of projected choices returned. Above this the list is marked
/// truncated and the picker shows a "type a value if yours isn't shown" hint.
/// A tuning constant, comfortably above real image/instance counts.
pub const OPTION_ITEM_CAP: usize = 500;

/// A no-op `ProgressSink` — option resolution drives its own UI spinner in the
/// CLI bridge, so the runtime fetch emits no progress.
#[derive(Default)]
struct NullProgressSink;
impl ProgressSink for NullProgressSink {
    fn on_event(&mut self, _event: ProgressEvent) {}
}

/// Run the `source` operation and project its raw response body into choices.
///
/// `runtime` is the resolver's **private clone** (never the executor's). `inputs`
/// carries the current workflow input values; the caller guarantees every
/// `FromInput` dependency is present and canonically typed.
pub async fn resolve_options(
    runtime: &mut Runtime,
    source: &OptionsSource,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> Result<ResolvedOptions, RuntimeError> {
    // The resolver's Runtime holds the workflow prologue's token snapshot — under
    // `--dry-run` that is a placeholder, and in a long interactive session it can
    // be stale. This fetch is a real read-only GET, so re-resolve a fresh token
    // first (best-effort; mirrors what a normal command does on each invocation).
    runtime.refresh_access_token_best_effort().await;

    let service_schema = runtime
        .catalogue_mut()
        .get_or_load(source.operation.service.as_str())?
        .clone();
    let operation = find_operation_or_error(&service_schema, &source.operation, "options_source")?;

    let request = build_options_request(operation, source, inputs);
    // Diagnostic context appended to any fetch failure so the picker hint shows
    // *which* request/profile/token was used (helps distinguish a wrong-profile,
    // wrong-base-URL, or genuinely-expired-token failure from an otherwise
    // identical-looking direct command).
    let display_path = operation.path_template.replace(
        "{namespace}",
        inputs
            .get("namespace")
            .and_then(|v| v.as_str())
            .unwrap_or("?"),
    );
    let mut sink = NullProgressSink;
    let body = match runtime
        .fetch_options_body(request, &source.items_path, &mut sink)
        .await
    {
        Ok(body) => body,
        Err(err) => {
            let ctx = runtime.context();
            // Lead with profile/base-URL/token so it's visible even if the TUI
            // hint truncates the trailing upstream message.
            return Err(RuntimeError::internal(format!(
                "profile={} token={:?} GET {}{} :: {}",
                ctx.profile,
                ctx.access_token_source,
                ctx.base_url.trim_end_matches('/'),
                display_path,
                err.message,
            )));
        }
    };

    let array = apply_jsonpath_subset(&body, &source.items_path)
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    // Resolve the label-detail JSONPath once: a fixed path, or one chosen by the
    // current value of a workflow input so the bracketed detail reflects what the
    // user searched by.
    let detail_path: Option<&str> = match &source.label_detail {
        Some(LabelDetail::Path(p)) => Some(p.as_str()),
        Some(LabelDetail::ByInput { input, paths }) => inputs
            .get(input)
            .and_then(|v| v.as_str())
            .and_then(|val| paths.get(val))
            .map(String::as_str),
        None => None,
    };

    let mut choices: Vec<OptionChoice> = Vec::new();
    for elem in &array {
        // Client-side equality filter: keep only elements whose node at
        // `filter.path` equals `filter.equals`. A path that resolves to nothing
        // counts as non-matching. Runs before projection + the truncation cap.
        if let Some(filter) = &source.filter {
            if apply_jsonpath_subset(elem, &filter.path).as_ref() != Some(&filter.equals) {
                continue;
            }
        }
        let Some(value_node) = apply_jsonpath_subset(elem, &source.value) else {
            continue;
        };
        let Some(value) = scalar_to_string(&value_node) else {
            continue; // non-scalar value → skip element
        };
        let label = source
            .label
            .as_ref()
            .and_then(|p| apply_jsonpath_subset(elem, p))
            .and_then(|n| scalar_to_string(&n))
            // An empty/whitespace label (e.g. a user with no display name set)
            // would render as a blank picker row, so fall back to the value.
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| value.clone());
        // Append a secondary detail in brackets when present and non-empty,
        // e.g. `afifdev01 (afif@example.com)` when searching by email.
        let label = match detail_path
            .and_then(|p| apply_jsonpath_subset(elem, p))
            .and_then(|n| scalar_to_string(&n))
            .filter(|s| !s.trim().is_empty())
        {
            Some(detail) => format!("{label} ({detail})"),
            None => label,
        };
        choices.push(OptionChoice { label, value });
    }

    // Upstream list order is arbitrary, which makes the picker hard to scan.
    // Present a stable alphabetical order by the displayed label (case-
    // insensitive), tie-breaking on value for determinism. Sort before the cap
    // so a truncated list keeps the alphabetically-leading entries.
    choices.sort_by(|a, b| {
        a.label
            .to_lowercase()
            .cmp(&b.label.to_lowercase())
            .then_with(|| a.value.cmp(&b.value))
    });

    let truncated = choices.len() > OPTION_ITEM_CAP;
    if truncated {
        choices.truncate(OPTION_ITEM_CAP);
    }
    Ok(ResolvedOptions { choices, truncated })
}

/// Build the fetch `CommandRequest` from the operation + resolved parameter
/// bindings. Each binding's value is placed at the matching operation
/// parameter's location (path/query/header); an unknown parameter name (which
/// compile-time validation rejects) defaults to a query parameter.
fn build_options_request(
    operation: &OperationSchema,
    source: &OptionsSource,
    inputs: &BTreeMap<String, serde_json::Value>,
) -> CommandRequest {
    let mut path_params: BTreeMap<String, String> = BTreeMap::new();
    let mut query_params: BTreeMap<String, String> = BTreeMap::new();
    let mut header_params: BTreeMap<String, String> = BTreeMap::new();

    for (name, binding) in &source.parameters {
        let value = match binding {
            OptionParameterBinding::FromInput(input_name)
            | OptionParameterBinding::FromInputOptional(input_name) => match inputs.get(input_name)
            {
                Some(v) => v.clone(),
                None => continue, // optional (or defensively absent) params are skipped
            },
            OptionParameterBinding::Literal(v) => v.clone(),
        };
        let as_string = json_to_param_string(&value);
        let location = operation
            .parameters
            .iter()
            .find(|p| &p.name == name)
            .map(|p| p.location)
            .unwrap_or(ParameterLocation::Query);
        match location {
            ParameterLocation::Path => {
                path_params.insert(name.clone(), as_string);
            }
            ParameterLocation::Header => {
                header_params.insert(name.clone(), as_string);
            }
            // Query / Body / FormData all fall to query for a GET list op.
            _ => {
                query_params.insert(name.clone(), as_string);
            }
        }
    }

    let namespace = inputs
        .get("namespace")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    CommandRequest {
        service: source.operation.service.clone(),
        operation_id: source.operation.operation.clone(),
        namespace,
        path_params,
        query_params,
        header_params,
        body: None,
        output_format: OutputFormat::Json,
        pagination: PaginationHint::All,
        verbosity: Verbosity::Quiet,
        output: None,
    }
}

/// Stringify a JSON scalar deterministically. Returns `None` for object / array
/// / null (not scalars), which the caller treats as "skip this element".
fn scalar_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) | serde_json::Value::Null => {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::execution::ExecutionContext;
    use crate::runtime::Runtime;
    use ags_protocol::catalogue::{OperationId, ServiceId};
    use ags_protocol::error::RuntimeError;
    use ags_protocol::workflow::{
        OperationReference, OptionFilter, OptionParameterBinding, OptionsSource,
    };
    use async_trait::async_trait;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    struct CannedClient {
        body: String,
        seen: Arc<Mutex<Option<HttpRequest>>>,
    }
    #[async_trait]
    impl HttpClient for CannedClient {
        async fn send(&self, request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            *self.seen.lock().unwrap() = Some(request);
            Ok(HttpResponse {
                status: 200,
                body: HttpBody::Text(self.body.clone()),
            })
        }
    }

    fn runtime_with(body: &str) -> (Runtime, Arc<Mutex<Option<HttpRequest>>>) {
        let ctx = ExecutionContext {
            base_url: "https://example.com".to_string(),
            access_token: "t".to_string(),
            ..ExecutionContext::default()
        };
        let seen = Arc::new(Mutex::new(None));
        let client = CannedClient {
            body: body.to_string(),
            seen: Arc::clone(&seen),
        };
        (
            Runtime::new(ctx, Box::new(client), reqwest::Client::new()),
            seen,
        )
    }

    fn images_source() -> OptionsSource {
        OptionsSource {
            operation: OperationReference {
                service: ServiceId::new("ams"),
                operation: OperationId::new("ams/admin/images/v1/list"),
            },
            parameters: BTreeMap::from([(
                "namespace".to_string(),
                OptionParameterBinding::FromInput("namespace".to_string()),
            )]),
            items_path: "$.images".to_string(),
            value: "$.id".to_string(),
            label: Some("$.name".to_string()),
            label_detail: None,
            fallback_description: None,
            filter: None,
        }
    }

    fn store_source_filtered(equals: serde_json::Value) -> OptionsSource {
        OptionsSource {
            operation: OperationReference {
                service: ServiceId::new("platform"),
                operation: OperationId::new("platform/admin/stores/v1/list"),
            },
            parameters: BTreeMap::from([(
                "namespace".to_string(),
                OptionParameterBinding::FromInput("namespace".to_string()),
            )]),
            items_path: "$".to_string(),
            value: "$.storeId".to_string(),
            label: Some("$.title".to_string()),
            label_detail: None,
            fallback_description: None,
            filter: Some(OptionFilter {
                path: "$.published".to_string(),
                equals,
            }),
        }
    }

    const STORES_BODY: &str = r#"[{"storeId":"draft-1","title":"In-game store","published":false},{"storeId":"pub-1","title":"published","published":true}]"#;

    #[tokio::test]
    async fn test_resolve_options_filter_keeps_only_matching_elements() {
        let (mut rt, _) = runtime_with(STORES_BODY);
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("ns"))]);
        let resolved = resolve_options(
            &mut rt,
            &store_source_filtered(serde_json::json!(false)),
            &inputs,
        )
        .await
        .unwrap();
        let values: Vec<&str> = resolved.choices.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(
            values,
            vec!["draft-1"],
            "only the published==false store is kept"
        );
    }

    #[tokio::test]
    async fn test_resolve_options_filter_matches_by_json_value_type() {
        // The filter compares by JSON value: `false` (bool) matches the bool field,
        // and there is exactly one draft store.
        let (mut rt, _) = runtime_with(STORES_BODY);
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("ns"))]);
        // Filtering on the published==true side keeps only the published store.
        let resolved = resolve_options(
            &mut rt,
            &store_source_filtered(serde_json::json!(true)),
            &inputs,
        )
        .await
        .unwrap();
        let values: Vec<&str> = resolved.choices.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, vec!["pub-1"]);
    }

    #[tokio::test]
    async fn test_resolve_options_no_filter_keeps_all() {
        // images_source() has filter: None → both elements retained (backward compat).
        let (mut rt, _) = runtime_with(
            r#"{"images":[{"id":"img-1","name":"Prod"},{"id":"img-2","name":"Staging"}]}"#,
        );
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let resolved = resolve_options(&mut rt, &images_source(), &inputs)
            .await
            .unwrap();
        assert_eq!(resolved.choices.len(), 2);
    }

    #[tokio::test]
    async fn test_resolve_options_filter_matches_string_field() {
        // Proves the filter is NOT boolean-only: keep currencies where the
        // string field `$.currencyType == "VIRTUAL"`.
        // (OperationReference / OptionParameterBinding are already imported in
        // this test module's `use ags_protocol::workflow::{...}` line.)
        let source = OptionsSource {
            operation: OperationReference {
                service: ServiceId::new("platform"),
                operation: OperationId::new("platform/admin/currencies/v1/list"),
            },
            parameters: BTreeMap::from([(
                "namespace".to_string(),
                OptionParameterBinding::FromInput("namespace".to_string()),
            )]),
            items_path: "$".to_string(),
            value: "$.currencyCode".to_string(),
            label: Some("$.currencyCode".to_string()),
            label_detail: None,
            fallback_description: None,
            filter: Some(OptionFilter {
                path: "$.currencyType".to_string(),
                equals: serde_json::json!("VIRTUAL"),
            }),
        };
        let (mut rt, _) = runtime_with(
            r#"[{"currencyCode":"GOLD","currencyType":"VIRTUAL"},{"currencyCode":"USD","currencyType":"REAL"}]"#,
        );
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("ns"))]);
        let resolved = resolve_options(&mut rt, &source, &inputs).await.unwrap();
        let values: Vec<&str> = resolved.choices.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, vec!["GOLD"], "only the VIRTUAL currency is kept");
    }

    #[tokio::test]
    async fn test_resolve_options_filter_excludes_element_missing_path() {
        // One element lacks `published`; a `published==false` filter excludes it
        // (a path that resolves to nothing counts as non-matching).
        let (mut rt, _) = runtime_with(
            r#"[{"storeId":"a","title":"has","published":false},{"storeId":"b","title":"missing"}]"#,
        );
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("ns"))]);
        let resolved = resolve_options(
            &mut rt,
            &store_source_filtered(serde_json::json!(false)),
            &inputs,
        )
        .await
        .unwrap();
        let values: Vec<&str> = resolved.choices.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, vec!["a"]);
    }

    #[tokio::test]
    async fn test_resolve_options_filter_all_excluded_is_ok_and_empty() {
        // All stores published → the draft filter yields an empty (non-panicking)
        // choice list, matching the existing empty-array behaviour. The picker
        // degrades to its type-your-own fallback (no draft to offer).
        let (mut rt, _) =
            runtime_with(r#"[{"storeId":"p1","title":"published","published":true}]"#);
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("ns"))]);
        let resolved = resolve_options(
            &mut rt,
            &store_source_filtered(serde_json::json!(false)),
            &inputs,
        )
        .await
        .unwrap();
        assert!(resolved.choices.is_empty());
    }

    #[tokio::test]
    async fn test_resolve_options_projects_label_and_value() {
        let (mut rt, _) = runtime_with(
            r#"{"images":[{"id":"img-1","name":"Prod"},{"id":"img-2","name":"Staging"}]}"#,
        );
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let resolved = resolve_options(&mut rt, &images_source(), &inputs)
            .await
            .unwrap();
        assert_eq!(resolved.choices.len(), 2);
        assert_eq!(resolved.choices[0].label, "Prod");
        assert_eq!(resolved.choices[0].value, "img-1");
        assert!(!resolved.truncated);
    }

    #[tokio::test]
    async fn test_resolve_options_sorts_choices_by_label_case_insensitively() {
        let (mut rt, _) = runtime_with(
            r#"{"images":[{"id":"3","name":"Zeta"},{"id":"1","name":"alpha"},{"id":"2","name":"Mike"}]}"#,
        );
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let resolved = resolve_options(&mut rt, &images_source(), &inputs)
            .await
            .unwrap();
        let labels: Vec<&str> = resolved.choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["alpha", "Mike", "Zeta"]);
    }

    #[tokio::test]
    async fn test_resolve_options_label_falls_back_to_value() {
        let (mut rt, _) = runtime_with(r#"{"images":[{"id":"img-1"}]}"#);
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let resolved = resolve_options(&mut rt, &images_source(), &inputs)
            .await
            .unwrap();
        assert_eq!(resolved.choices[0].label, "img-1");
        assert_eq!(resolved.choices[0].value, "img-1");
    }

    #[tokio::test]
    async fn test_resolve_options_empty_label_falls_back_to_value() {
        // An empty-string label (e.g. a user with no display name) would render
        // as a blank picker row; it must fall back to the value instead.
        let (mut rt, _) = runtime_with(r#"{"images":[{"id":"img-1","name":""}]}"#);
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let resolved = resolve_options(&mut rt, &images_source(), &inputs)
            .await
            .unwrap();
        assert_eq!(resolved.choices[0].label, "img-1");
    }

    #[tokio::test]
    async fn test_resolve_options_label_detail_renders_in_brackets() {
        // label_detail shows in brackets after the label when present, and is
        // omitted when empty.
        let mut src = images_source();
        src.label = Some("$.id".to_string());
        src.label_detail = Some(LabelDetail::Path("$.name".to_string()));
        let (mut rt, _) =
            runtime_with(r#"{"images":[{"id":"img-1","name":"Prod"},{"id":"img-2","name":""}]}"#);
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let resolved = resolve_options(&mut rt, &src, &inputs).await.unwrap();
        let labels: Vec<&str> = resolved.choices.iter().map(|c| c.label.as_str()).collect();
        assert!(
            labels.contains(&"img-1 (Prod)"),
            "detail in brackets: {labels:?}"
        );
        assert!(
            labels.contains(&"img-2"),
            "empty detail omitted: {labels:?}"
        );
    }

    #[tokio::test]
    async fn test_resolve_options_label_detail_by_input_picks_path_from_input_value() {
        // The bracketed detail path is chosen by a workflow input's value, so it
        // reflects what the user searched by. Here `searchBy = email` selects the
        // `$.email` path; a value absent from the map yields no detail.
        let mut src = images_source();
        src.label = Some("$.id".to_string());
        src.label_detail = Some(LabelDetail::ByInput {
            input: "searchBy".to_string(),
            paths: BTreeMap::from([
                ("email".to_string(), "$.email".to_string()),
                ("name".to_string(), "$.name".to_string()),
            ]),
        });
        let (mut rt, _) =
            runtime_with(r#"{"images":[{"id":"img-1","name":"Prod","email":"a@b.com"}]}"#);
        let inputs = BTreeMap::from([
            ("namespace".to_string(), serde_json::json!("dev")),
            ("searchBy".to_string(), serde_json::json!("email")),
        ]);
        let resolved = resolve_options(&mut rt, &src, &inputs).await.unwrap();
        assert_eq!(
            resolved.choices[0].label, "img-1 (a@b.com)",
            "detail uses the path selected by searchBy"
        );

        // A searchBy value with no mapped path → no bracket.
        let inputs = BTreeMap::from([
            ("namespace".to_string(), serde_json::json!("dev")),
            ("searchBy".to_string(), serde_json::json!("phone")),
        ]);
        let resolved = resolve_options(&mut rt, &src, &inputs).await.unwrap();
        assert_eq!(resolved.choices[0].label, "img-1");
    }

    #[tokio::test]
    async fn test_resolve_options_stringifies_scalar_value() {
        let mut src = images_source();
        src.value = "$.id".to_string();
        let (mut rt, _) = runtime_with(r#"{"images":[{"id":42},{"id":true}]}"#);
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let resolved = resolve_options(&mut rt, &src, &inputs).await.unwrap();
        assert_eq!(resolved.choices[0].value, "42");
        assert_eq!(resolved.choices[1].value, "true");
    }

    #[tokio::test]
    async fn test_resolve_options_skips_non_scalar_and_absent_value() {
        let (mut rt, _) =
            runtime_with(r#"{"images":[{"id":{"nested":1}},{"name":"no-id"},{"id":"ok"}]}"#);
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let resolved = resolve_options(&mut rt, &images_source(), &inputs)
            .await
            .unwrap();
        assert_eq!(resolved.choices.len(), 1);
        assert_eq!(resolved.choices[0].value, "ok");
    }

    #[tokio::test]
    async fn test_resolve_options_empty_array_is_ok() {
        let (mut rt, _) = runtime_with(r#"{"images":[]}"#);
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let resolved = resolve_options(&mut rt, &images_source(), &inputs)
            .await
            .unwrap();
        assert!(resolved.choices.is_empty());
        assert!(!resolved.truncated);
    }

    #[tokio::test]
    async fn test_resolve_options_binds_from_input_into_path_param() {
        let (mut rt, seen) = runtime_with(r#"{"images":[]}"#);
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("my-ns"))]);
        let _ = resolve_options(&mut rt, &images_source(), &inputs)
            .await
            .unwrap();
        let req = seen.lock().unwrap().clone().unwrap();
        assert!(
            req.url.contains("/namespaces/my-ns/images"),
            "url: {}",
            req.url
        );
    }

    #[tokio::test]
    async fn test_resolve_options_literal_parameter_binds() {
        let mut src = images_source();
        src.parameters.insert(
            "count".to_string(),
            OptionParameterBinding::Literal(serde_json::json!(5)),
        );
        let (mut rt, seen) = runtime_with(r#"{"images":[]}"#);
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let _ = resolve_options(&mut rt, &src, &inputs).await.unwrap();
        let req = seen.lock().unwrap().clone().unwrap();
        assert!(
            req.query.iter().any(|(k, v)| k == "count" && v == "5"),
            "query: {:?}",
            req.query
        );
    }

    #[tokio::test]
    async fn test_resolve_options_dispatch_error_is_err() {
        struct Failing;
        #[async_trait]
        impl HttpClient for Failing {
            async fn send(&self, _r: HttpRequest) -> Result<HttpResponse, RuntimeError> {
                Ok(HttpResponse {
                    status: 404,
                    body: HttpBody::Text("{}".to_string()),
                })
            }
        }
        let ctx = ExecutionContext {
            base_url: "https://example.com".to_string(),
            access_token: "t".to_string(),
            ..ExecutionContext::default()
        };
        let mut rt = Runtime::new(ctx, Box::new(Failing), reqwest::Client::new());
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let err = resolve_options(&mut rt, &images_source(), &inputs).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_resolve_options_caps_and_marks_truncated() {
        let items: Vec<String> = (0..600).map(|i| format!(r#"{{"id":"img-{i}"}}"#)).collect();
        let body = format!(r#"{{"images":[{}]}}"#, items.join(","));
        let (mut rt, _) = runtime_with(&body);
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let resolved = resolve_options(&mut rt, &images_source(), &inputs)
            .await
            .unwrap();
        assert_eq!(resolved.choices.len(), OPTION_ITEM_CAP);
        assert!(resolved.truncated);
    }

    /// A paginated `$.images` response (the AMS shape) merges all pages — the
    /// regression guard for the `fetch_all_pages`-only-merges-`data` bug. The
    /// bundled `ams` images op carries an `offset` query param, so pagination is
    /// detected; the `paging.next` cursor drives the loop.
    #[tokio::test]
    async fn test_resolve_options_merges_paginated_images() {
        struct Pages(Arc<Mutex<Vec<String>>>);
        #[async_trait]
        impl HttpClient for Pages {
            async fn send(&self, _r: HttpRequest) -> Result<HttpResponse, RuntimeError> {
                let body = self.0.lock().unwrap().pop().expect("ran out of pages");
                Ok(HttpResponse {
                    status: 200,
                    body: HttpBody::Text(body),
                })
            }
        }
        let ctx = ExecutionContext {
            base_url: "https://example.com".to_string(),
            access_token: "t".to_string(),
            ..ExecutionContext::default()
        };
        let pages = Arc::new(Mutex::new(vec![
            r#"{"images":[{"id":"c","name":"C"}],"paging":{"next":""}}"#.to_string(),
            r#"{"images":[{"id":"b","name":"B"}],"paging":{"next":"cursor2"}}"#.to_string(),
            r#"{"images":[{"id":"a","name":"A"}],"paging":{"next":"cursor1"}}"#.to_string(),
        ]));
        let mut rt = Runtime::new(
            ctx,
            Box::new(Pages(Arc::clone(&pages))),
            reqwest::Client::new(),
        );
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let resolved = resolve_options(&mut rt, &images_source(), &inputs)
            .await
            .unwrap();
        let ids: Vec<&str> = resolved.choices.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"], "all paginated images merged");
    }

    #[tokio::test]
    async fn test_resolve_options_projects_from_root_array() {
        // A non-paginated list endpoint returns a top-level array; items_path "$"
        // projects it directly (the store-picker shape).
        let (mut rt, _) =
            runtime_with(r#"[{"storeId":"s-1","title":"Draft"},{"storeId":"s-2","title":"Live"}]"#);
        let mut src = images_source();
        src.items_path = "$".to_string();
        src.value = "$.storeId".to_string();
        src.label = Some("$.title".to_string());
        let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
        let resolved = resolve_options(&mut rt, &src, &inputs).await.unwrap();
        let values: Vec<&str> = resolved.choices.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, ["s-1", "s-2"]); // sorted by label: Draft, Live
        assert_eq!(resolved.choices[0].label, "Draft");
    }

    #[tokio::test]
    async fn test_item_picker_fetch_carries_store_id_from_input() {
        use ags_protocol::catalogue::{OperationId, ServiceId};
        use ags_protocol::workflow::{OperationReference, OptionParameterBinding, OptionsSource};
        // The store picker has already been resolved: storeId is in `inputs` when the
        // item picker's options are fetched. The fetch must carry storeId as a query
        // param (the data flow the store-scoped item pickers depend on).
        let (mut rt, seen) = runtime_with(r#"{"data":[{"itemId":"i-1","name":"Pass"}]}"#);
        let src = OptionsSource {
            operation: OperationReference {
                service: ServiceId::new("platform"),
                operation: OperationId::new("platform/admin/items/v1/list"),
            },
            parameters: BTreeMap::from([
                (
                    "namespace".to_string(),
                    OptionParameterBinding::FromInput("namespace".to_string()),
                ),
                (
                    "storeId".to_string(),
                    OptionParameterBinding::FromInput("storeId".to_string()),
                ),
            ]),
            items_path: "$.data".to_string(),
            value: "$.itemId".to_string(),
            label: Some("$.name".to_string()),
            label_detail: None,
            fallback_description: None,
            filter: None,
        };
        let inputs = BTreeMap::from([
            ("namespace".to_string(), serde_json::json!("dev")),
            ("storeId".to_string(), serde_json::json!("store-1")),
        ]);
        let resolved = resolve_options(&mut rt, &src, &inputs).await.unwrap();
        assert_eq!(resolved.choices.len(), 1);
        let req = seen.lock().unwrap().clone().unwrap();
        assert!(
            req.query
                .iter()
                .any(|(k, v)| k == "storeId" && v == "store-1"),
            "item picker fetch must carry storeId from the storeId input: {:?}",
            req.query
        );
    }
}
