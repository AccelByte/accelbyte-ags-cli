//! Execution core — the Runtime facade and its supporting modules.

pub mod auth;
mod cleanup;
pub mod completions;
pub mod config;
pub mod diagnostics;
pub mod dispatch;
pub mod execution;
mod facade;

use crate::catalogue::Catalogue;
use crate::runtime::dispatch::http::HttpClient;
use crate::runtime::execution::ExecutionContext;

/// Run all process-scoped runtime startup side-effects. Called once from
/// `invocation::run()` before any command is dispatched.
pub(crate) fn bootstrap() {
    cleanup::cleanup_stale_temp_files();
}

/// Top-level runtime facade. Holds process-scoped state and delegates
/// to per-concern facade modules.
pub struct Runtime {
    pub(crate) catalogue: Catalogue,
    pub(crate) context: ExecutionContext,
    pub(crate) http_client: Box<dyn HttpClient>,
}

impl Runtime {
    /// Build a runtime from a resolved execution context and any `HttpClient`
    /// implementation. Use this constructor for test injection or alternate
    /// transports; production callers typically use [`Runtime::from_reqwest`].
    pub(crate) fn new(context: ExecutionContext, http_client: Box<dyn HttpClient>) -> Self {
        Self {
            catalogue: Catalogue::new(),
            context,
            http_client,
        }
    }

    /// Convenience constructor that wraps a real `reqwest::Client` in the
    /// production `ReqwestHttpClient` adapter.
    pub fn from_reqwest(context: ExecutionContext, http_client: reqwest::Client) -> Self {
        Self::new(
            context,
            Box::new(crate::runtime::dispatch::http::ReqwestHttpClient::new(
                http_client,
            )),
        )
    }
}

#[cfg(test)]
mod constructor_tests {
    use super::*;
    use crate::protocol::error::RuntimeError;
    use crate::runtime::dispatch::http::{HttpClient, HttpRequest, HttpResponse};

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
        let _runtime = Runtime::new(ctx, Box::new(DummyClient));
    }
}
