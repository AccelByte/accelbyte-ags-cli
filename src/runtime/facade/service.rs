//! Service facade — `Runtime` methods for executing API calls:
//! `run_command`, `dry_run_command`, and the preview/confirmation pipeline.

use crate::protocol::catalogue::OperationSchema;
use crate::protocol::error::{RuntimeError, RuntimeErrorKind};
use crate::protocol::event::ProgressSink;
use crate::protocol::output::CommandOutput;
use crate::protocol::request::CommandRequest;

impl crate::runtime::Runtime {
    /// Execute a command through the runtime dispatch pipeline.
    pub async fn run_command(
        &mut self,
        request: CommandRequest,
        sink: &mut dyn ProgressSink,
    ) -> Result<CommandOutput, RuntimeError> {
        let (resource_name, operation) = self.find_operation(&request)?;

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
            Some(crate::protocol::output::ResolutionTrace {
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
        crate::runtime::dispatch::execute_operation(&dispatch, &operation, &request, sink).await
    }

    /// Return a preview of what a command will do: method, interpolated URL, and
    /// whether the user must confirm before execution.
    pub fn preview_command(
        &mut self,
        request: &CommandRequest,
    ) -> Result<crate::protocol::result::CommandPreview, RuntimeError> {
        use crate::protocol::result::CommandPreview;
        use crate::runtime::dispatch::requires_confirmation;

        let (_resource_name, operation) = self.find_operation(request)?;

        // Substitute known path parameters to produce the display URL.
        // Unreplaced `{tokens}` are left as-is — execute_operation validates them.
        let mut path = operation.path_template.clone();
        for (name, value) in &request.path_params {
            path = path.replace(&format!("{{{name}}}"), value);
        }
        let url = self.build_request_url(&path);

        let http_method = operation.http_method;
        let confirmation_required = requires_confirmation(http_method, &operation.name);

        Ok(CommandPreview {
            service: request.service,
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
    ) -> Result<crate::protocol::result::DryRunResult, RuntimeError> {
        use crate::protocol::result::DryRunResult;

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

        let body = if operation.request_body.is_some() {
            request.body.clone()
        } else {
            None
        };

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

    /// Combine the runtime base URL with `path`, trimming any trailing slash from the base.
    fn build_request_url(&self, path: &str) -> String {
        format!("{}{}", self.context.base_url.trim_end_matches('/'), path)
    }
}
