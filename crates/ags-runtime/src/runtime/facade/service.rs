//! Service facade — `Runtime` methods for executing API calls:
//! `run_command`, `dry_run_command`, and the preview/confirmation pipeline.

use ags_protocol::catalogue::OperationSchema;
use ags_protocol::error::{RuntimeError, RuntimeErrorKind};
use ags_protocol::event::ProgressSink;
use ags_protocol::output::CommandOutput;
use ags_protocol::request::CommandRequest;

impl crate::runtime::Runtime {
    /// Execute a command through the runtime dispatch pipeline.
    pub async fn run_command(
        &mut self,
        request: &CommandRequest,
        sink: &mut dyn ProgressSink,
    ) -> Result<CommandOutput, RuntimeError> {
        let (resource_name, operation) = self.find_operation(request)?;

        // Derive display service_name from registry
        let service_name = crate::catalogue::Catalogue::display_name(request.service.as_str())
            .unwrap_or(request.service.as_str())
            .to_string();

        // Build resolution trace from context source fields when verbose
        let resolution_trace = if request.verbosity.is_verbose() {
            let spec_source_label = format!("{} loaded from cache", service_name.to_uppercase());
            let token_expiry_label = self
                .context
                .access_token_expiry
                .as_ref()
                .map(|d| format!("expires in {d}"));
            Some(ags_protocol::output::ResolutionTrace {
                spec_source: spec_source_label,
                profile: (
                    self.context.profile.clone(),
                    self.context.profile_source.label().to_string(),
                ),
                base_url: (
                    self.context.base_url.clone(),
                    self.context.base_url_source.label().to_string(),
                ),
                namespace: self
                    .context
                    .namespace
                    .as_ref()
                    .zip(self.context.namespace_source.as_ref())
                    .map(|(namespace, source)| (namespace.clone(), source.label().to_string())),
                token_source: self.context.access_token_source.label().to_string(),
                token_expiry: token_expiry_label,
            })
        } else {
            None
        };

        let dispatch = crate::runtime::dispatch::ApiCallContext {
            client: self.http_client.as_ref(),
            base_url: &self.context.base_url,
            token: &self.context.access_token,
            service_name: &service_name,
            resource_name: &resource_name,
            resolution_trace,
        };
        crate::runtime::dispatch::execute_operation(&dispatch, &operation, request, sink).await
    }

    /// Return a preview of what a command will do: method, interpolated URL, and
    /// whether the user must confirm before execution.
    pub fn preview_command(
        &mut self,
        request: &CommandRequest,
    ) -> Result<ags_protocol::result::CommandPreview, RuntimeError> {
        use crate::runtime::dispatch::requires_confirmation;
        use ags_protocol::result::CommandPreview;

        let (_resource_name, operation) = self.find_operation(request)?;

        // Substitute known path parameters to produce the display URL.
        // Unreplaced `{tokens}` are left as-is — execute_operation validates them.
        // Path values can be user- or API-sourced and this URL is rendered into
        // the confirm card, so strip terminal control sequences first
        // (CONTRIBUTING § Security). Percent-encoding stays the dispatch path's job.
        let mut path = operation.path_template.clone();
        for (name, value) in &request.path_params {
            let safe = crate::support::strings::strip_terminal_control_sequences(value);
            path = path.replace(&format!("{{{name}}}"), &safe);
        }
        let url = self.build_request_url(&path);

        let http_method = operation.http_method;
        let confirmation_required = requires_confirmation(http_method, &operation.name);

        Ok(CommandPreview {
            service: request.service.clone(),
            operation_id: request.operation_id.clone(),
            summary: format!("This will issue a {} request", http_method.as_str()),
            http_method,
            url,
            mutation_class: operation.mutation_class,
            confirmation_required,
            warnings: vec![],
        })
    }

    /// Build a dry-run report for the given command request without executing it.
    /// Returns the HTTP method, fully-interpolated URL, masked auth header,
    /// query parameters, and request body that would be sent if the command
    /// were executed.
    pub fn dry_run_command(
        &mut self,
        request: &CommandRequest,
    ) -> Result<ags_protocol::result::DryRunResult, RuntimeError> {
        use ags_protocol::result::DryRunResult;

        let (_resource_name, operation) = self.find_operation(request)?;

        // Substitute path parameters with sanitized values. Shares the helper
        // with `execute_operation` so dry-run rejects the same malicious
        // inputs (`#`, `?`, `..`, etc.) that real execution does.
        let path = crate::runtime::dispatch::substitute_path_params(
            &operation.path_template,
            &request.path_params,
        )?;
        let url = self.build_request_url(&path);

        let query: Vec<(String, String)> = request
            .query_params
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let headers = vec![("Authorization".to_string(), "Bearer <token>".to_string())];

        let body = request.body.clone();

        Ok(DryRunResult {
            http_method: operation.http_method,
            url,
            headers,
            query,
            body,
        })
    }

    /// Look up the dispatched contract by `operation_id` and return the resource it lives in.
    ///
    /// `operation_id` is unique across every scope/version, so a flat scan over
    /// all contracts correctly returns the specific contract the user dispatched.
    /// The returned `OperationSchema` is owned (cloned) so callers can release
    /// the catalogue borrow.
    fn find_operation(
        &mut self,
        request: &CommandRequest,
    ) -> Result<(String, OperationSchema), RuntimeError> {
        let schema = self.catalogue.get_or_load(request.service.as_str())?;

        schema
            .resources
            .iter()
            .find_map(|resource| {
                resource
                    .operations()
                    .find(|operation| operation.id == request.operation_id)
                    .map(|operation| (resource.name.clone(), operation.clone()))
            })
            .ok_or_else(|| RuntimeError {
                kind: RuntimeErrorKind::Validation,
                message: format!(
                    "Operation '{}' not found in service '{}'",
                    request.operation_id, request.service
                ),
                details: None,
                hint: None,
                trace: None,
            })
    }

    /// Dispatch a read operation and return its **raw** aggregated response body
    /// for dynamic-enum option resolution. Mirrors `run_command`'s operation
    /// lookup + dispatch-context assembly but routes to `fetch_raw_body`
    /// (no shaping, no traces), merging the paginated array at `items_path`. The
    /// operation is validated to be `GET` at workflow compile time, so this is
    /// side-effect free.
    pub async fn fetch_options_body(
        &mut self,
        request: CommandRequest,
        items_path: &str,
        // `+ Send`: awaited inside an off-thread `Handle::spawn` task by the
        // dynamic-enum option resolver, so the held sink must be `Send`.
        sink: &mut (dyn ProgressSink + Send),
    ) -> Result<serde_json::Value, RuntimeError> {
        let (resource_name, operation) = self.find_operation(&request)?;
        let service_name = crate::catalogue::Catalogue::display_name(request.service.as_str())
            .unwrap_or(request.service.as_str())
            .to_string();
        let dispatch = crate::runtime::dispatch::ApiCallContext {
            client: self.http_client.as_ref(),
            base_url: &self.context.base_url,
            token: &self.context.access_token,
            service_name: &service_name,
            resource_name: &resource_name,
            resolution_trace: None,
        };
        crate::runtime::dispatch::fetch_raw_body(&dispatch, &operation, &request, items_path, sink)
            .await
    }

    /// Combine the runtime base URL with `path`, trimming any trailing slash from the base.
    fn build_request_url(&self, path: &str) -> String {
        format!("{}{}", self.context.base_url.trim_end_matches('/'), path)
    }
}

#[cfg(test)]
mod tests {
    use crate::runtime::execution::ExecutionContext;
    use crate::runtime::Runtime;
    use ags_protocol::catalogue::{OperationId, ServiceId};
    use ags_protocol::request::{FormPart, OutputFormat, PaginationHint, RequestBody, Verbosity};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// `dry_run_command` preserves a `RequestBody::Multipart` body for a
    /// real formData operation — regression for the bug where the body was
    /// nulled out whenever `operation.request_body.is_none()`, which is
    /// always true for a formData operation (body and formData are
    /// mutually exclusive per OAS2).
    #[test]
    fn test_dry_run_command_preserves_multipart_body_for_formdata_operation() {
        let mut runtime =
            Runtime::from_reqwest(ExecutionContext::default(), reqwest::Client::new());
        let mut path_params = BTreeMap::new();
        path_params.insert("namespace".to_string(), "dev".to_string());
        path_params.insert("appUiName".to_string(), "some-app".to_string());
        let request = ags_protocol::request::CommandRequest {
            service: ServiceId::new("csm"),
            operation_id: OperationId::new("csm/admin/app-ui/v1/upload-assets"),
            namespace: None,
            path_params,
            query_params: BTreeMap::new(),
            header_params: BTreeMap::new(),
            body: Some(RequestBody::Multipart(vec![FormPart::File {
                name: "file".to_string(),
                path: PathBuf::from("/tmp/asset.png"),
                filename: "asset.png".to_string(),
            }])),
            form_params: BTreeMap::new(),
            output_format: OutputFormat::Human,
            pagination: PaginationHint::Auto,
            verbosity: Verbosity::Normal,
            output: None,
        };

        let result = runtime.dry_run_command(&request).unwrap();
        assert!(matches!(result.body, Some(RequestBody::Multipart(_))));
    }
}

#[cfg(test)]
mod fetch_options_body_tests {
    use crate::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
    use crate::runtime::execution::ExecutionContext;
    use crate::runtime::Runtime;
    use ags_protocol::error::RuntimeError;
    use ags_protocol::event::{ProgressEvent, ProgressSink};
    use ags_protocol::request::{CommandRequest, OutputFormat, PaginationHint, Verbosity};
    use async_trait::async_trait;
    use std::collections::BTreeMap;

    struct CannedClient {
        body: String,
    }

    #[async_trait]
    impl HttpClient for CannedClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            Ok(HttpResponse {
                status: 200,
                body: HttpBody::Text(self.body.clone()),
            })
        }
    }

    #[derive(Default)]
    struct NoopSink;
    impl ProgressSink for NoopSink {
        fn on_event(&mut self, _event: ProgressEvent) {}
    }

    /// `fetch_options_body` looks up the operation by id and returns the raw
    /// body. Uses a real bundled service so `find_operation` resolves. `ams`
    /// `ams/admin/info/v1/list-supported-instances` is a single-page GET.
    #[tokio::test]
    async fn test_fetch_options_body_returns_raw_value() {
        let ctx = ExecutionContext {
            base_url: "https://example.com".to_string(),
            access_token: "t".to_string(),
            ..ExecutionContext::default()
        };
        let client = CannedClient {
            body: r#"{"availableInstanceTypes":[{"id":"c5.large","name":"C5 Large"}]}"#.to_string(),
        };
        let mut runtime = Runtime::new(ctx, Box::new(client), reqwest::Client::new());
        let request = CommandRequest {
            service: crate::catalogue::Catalogue::find_id("ams").expect("ams in manifest"),
            operation_id: ags_protocol::catalogue::OperationId::new(
                "ams/admin/info/v1/list-supported-instances",
            ),
            namespace: Some("dev".to_string()),
            path_params: BTreeMap::from([("namespace".to_string(), "dev".to_string())]),
            query_params: BTreeMap::new(),
            header_params: BTreeMap::new(),
            form_params: BTreeMap::new(),
            body: None,
            output_format: OutputFormat::Json,
            pagination: PaginationHint::All,
            verbosity: Verbosity::Quiet,
            output: None,
        };
        let mut sink = NoopSink;
        let body = runtime
            .fetch_options_body(request, "$.availableInstanceTypes", &mut sink)
            .await
            .expect("ok");
        let arr = body["availableInstanceTypes"].as_array().unwrap();
        assert_eq!(arr[0]["id"], "c5.large");
    }
}
