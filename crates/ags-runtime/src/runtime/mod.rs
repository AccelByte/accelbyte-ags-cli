//! Execution core — the Runtime facade and its supporting modules.

pub mod auth;
mod cleanup;
pub mod config;
pub mod diagnostics;
pub mod dispatch;
pub mod execution;
mod facade;
pub mod workflows;

use crate::catalogue::Catalogue;
use crate::runtime::dispatch::http::HttpClient;
use crate::runtime::execution::ExecutionContext;

/// Run all process-scoped runtime startup side-effects. Called once from
/// `invocation::run()` before any command is dispatched.
pub fn bootstrap() {
    cleanup::cleanup_stale_temp_files();
}

/// Top-level runtime facade. Holds process-scoped state and delegates
/// to per-concern facade modules.
pub struct Runtime {
    pub(crate) catalogue: Catalogue,
    pub(crate) context: ExecutionContext,
    pub(crate) http_client: Box<dyn HttpClient>,
    /// Concrete reqwest client used by auth flows (token exchange, probe,
    /// refresh). Shares the user-configured timeout with `http_client` so
    /// `--timeout` applies uniformly across dispatch and auth.
    pub(crate) reqwest_client: reqwest::Client,
}

impl Runtime {
    /// Build a runtime from a resolved execution context, an `HttpClient`
    /// implementation, and the concrete `reqwest::Client` used for auth.
    /// Use this for test injection or alternate dispatch transports;
    /// production callers typically use [`Runtime::from_reqwest`].
    pub fn new(
        context: ExecutionContext,
        http_client: Box<dyn HttpClient>,
        reqwest_client: reqwest::Client,
    ) -> Self {
        Self {
            catalogue: Catalogue::new(),
            context,
            http_client,
            reqwest_client,
        }
    }

    /// Convenience constructor that wraps a real `reqwest::Client` in the
    /// production `ReqwestHttpClient` adapter and retains the original for
    /// auth flows that need the concrete type.
    pub fn from_reqwest(context: ExecutionContext, http_client: reqwest::Client) -> Self {
        Self::new(
            context,
            Box::new(crate::runtime::dispatch::http::ReqwestHttpClient::new(
                http_client.clone(),
            )),
            http_client,
        )
    }

    /// Mutable borrow of the OpenAPI catalogue. Workflow compile and resolve
    /// helpers need this to walk operation schemas.
    pub fn catalogue_mut(&mut self) -> &mut Catalogue {
        &mut self.catalogue
    }

    /// Shared borrow of the execution context (profile, namespace, base URL,
    /// auth state). Workflow code reads namespace/profile to populate
    /// `CommandRequest.namespace` and to log resolution traces.
    pub fn context(&self) -> &ExecutionContext {
        &self.context
    }

    /// Re-resolve a real access token for this runtime's profile, replacing the
    /// token snapshot taken at the workflow prologue. Under `--dry-run` that
    /// snapshot is a placeholder (`"dry-run-token"`), and in a long interactive
    /// session it can go stale — so a read-only side-fetch (e.g. a dynamic-enum
    /// options picker) must re-resolve, exactly as a normal command would.
    ///
    /// Best-effort: on any failure the existing token is kept (the subsequent
    /// fetch surfaces the auth error rather than this aborting). Skipped when
    /// there is no configured profile (the empty default used in tests), which
    /// keeps the credential store / keychain out of the unit-test path.
    pub async fn refresh_access_token_best_effort(&mut self) {
        if self.context.profile.is_empty() {
            return;
        }
        if let Ok(resolution) = crate::runtime::auth::session::resolve_access_token(
            &self.reqwest_client,
            &self.context.profile,
        )
        .await
        {
            self.context.access_token = resolution.token;
        }
    }
}

#[cfg(test)]
mod constructor_tests {
    use super::*;
    use crate::runtime::dispatch::http::{HttpClient, HttpRequest, HttpResponse};
    use ags_protocol::error::RuntimeError;

    struct DummyClient;

    #[async_trait::async_trait]
    impl HttpClient for DummyClient {
        async fn send(&self, _request: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            unimplemented!("dummy client used only for type-check test")
        }
    }

    /// `Runtime::new` accepts any `Box<dyn HttpClient>`, enabling test
    /// injection of a fake transport without spinning up reqwest.
    #[test]
    fn test_runtime_new_accepts_dyn_http_client() {
        let ctx = ExecutionContext::default();
        let _runtime = Runtime::new(ctx, Box::new(DummyClient), reqwest::Client::new());
    }

    /// `refresh_access_token_best_effort` is a no-op when the profile is empty
    /// (the default used in tests), so it never touches the credential store —
    /// the token is left exactly as injected.
    #[tokio::test]
    async fn test_refresh_access_token_best_effort_skips_empty_profile() {
        let ctx = ExecutionContext {
            access_token: "injected".to_string(),
            ..ExecutionContext::default()
        };
        let mut runtime = Runtime::new(ctx, Box::new(DummyClient), reqwest::Client::new());
        runtime.refresh_access_token_best_effort().await;
        assert_eq!(runtime.context().access_token, "injected");
    }
}
