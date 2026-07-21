//! Synchronous resolver seam the interactive Phase-1 form calls when a
//! dynamic-enum field is activated. The form depends only on this sync trait
//! and is agnostic to how resolution happens: the production impl bridges to
//! the async runtime resolver off-thread (Task 15); tests inject a canned
//! double with no I/O.

use std::collections::BTreeMap;

use ags_protocol::workflow::{OptionsSource, ResolvedOptions};

use crate::errors::CliError;

/// Resolve a dynamic-enum field's choices. Returning `Err` (or a cancellation,
/// surfaced as `Err`) leaves the field on its free-text fallback; it is never
/// fatal to the run.
pub trait DynamicOptionResolver {
    /// Resolve a dynamic-enum field's choices from `source` and the current
    /// `inputs`.
    fn resolve(
        &self,
        source: &OptionsSource,
        inputs: &BTreeMap<String, serde_json::Value>,
    ) -> Result<ResolvedOptions, CliError>;
}

// ── Async fetch seam (inline picker) ───────────────────────────────────────
//
// The sync `DynamicOptionResolver` cannot be `Handle::spawn`ed (no `Send`; the
// canned double holds `RefCell`). The inline picker sub-loop needs a fetch it
// can start, poll, and abort while it animates the spinner through the terminal
// it already holds, so it uses this seam over the async `resolve_options`.

/// A started, pollable, abortable options fetch.
pub struct FetchTask {
    /// Receives the fetch result exactly once.
    pub rx: mpsc::Receiver<Result<ResolvedOptions, ags_protocol::error::RuntimeError>>,
    /// Production tasks carry a join handle so the caller can abort the real
    /// fetch on cancel; the test double leaves this `None`.
    abort: Option<tokio::task::JoinHandle<()>>,
}

impl FetchTask {
    /// Abort the in-flight fetch (best-effort). No-op for the test double.
    pub fn abort(&mut self) {
        if let Some(handle) = self.abort.take() {
            handle.abort();
        }
    }
}

/// Start an options fetch. Production spawns `resolve_options`; tests return a
/// ready (or perpetually-pending) channel with no I/O.
pub trait OptionsFetch {
    fn start(
        &self,
        source: &OptionsSource,
        inputs: &BTreeMap<String, serde_json::Value>,
    ) -> FetchTask;
}

/// Production fetch: spawns `resolve_options` on the async runtime and returns
/// its join handle (for `abort`) plus the result receiver. Mirrors the spawn in
/// `ProductionResolver::resolve` (lines 74-84) but without any drawing.
pub struct RuntimeOptionsFetch {
    runtime: Arc<tokio::sync::Mutex<Runtime>>,
    handle: Handle,
}

impl RuntimeOptionsFetch {
    pub fn new(runtime: Runtime, handle: Handle) -> Self {
        Self {
            runtime: Arc::new(tokio::sync::Mutex::new(runtime)),
            handle,
        }
    }
}

impl OptionsFetch for RuntimeOptionsFetch {
    fn start(
        &self,
        source: &OptionsSource,
        inputs: &BTreeMap<String, serde_json::Value>,
    ) -> FetchTask {
        let runtime = Arc::clone(&self.runtime);
        let source = source.clone();
        let inputs = inputs.clone();
        let (tx, rx) =
            mpsc::channel::<Result<ResolvedOptions, ags_protocol::error::RuntimeError>>();
        let join = self.handle.spawn(async move {
            let mut guard = runtime.lock().await;
            let result = resolve_options(&mut guard, &source, &inputs).await;
            let _ = tx.send(result);
        });
        FetchTask {
            rx,
            abort: Some(join),
        }
    }
}

#[cfg(test)]
pub struct CannedFetch {
    result: std::cell::RefCell<Option<Result<ResolvedOptions, ags_protocol::error::RuntimeError>>>,
}

#[cfg(test)]
impl CannedFetch {
    pub fn ok(choices: Vec<ags_protocol::workflow::OptionChoice>) -> Self {
        Self {
            result: std::cell::RefCell::new(Some(Ok(ResolvedOptions {
                choices,
                truncated: false,
            }))),
        }
    }

    /// A fetch that never delivers — for exercising Esc/abort.
    pub fn pending() -> Self {
        Self {
            result: std::cell::RefCell::new(None),
        }
    }
}

#[cfg(test)]
impl OptionsFetch for CannedFetch {
    fn start(
        &self,
        _source: &OptionsSource,
        _inputs: &BTreeMap<String, serde_json::Value>,
    ) -> FetchTask {
        let (tx, rx) = mpsc::channel();
        // Decide forget-vs-drop at the branch point: an `ok()` double sends and
        // then drops `tx` (channel closes, a second recv sees Disconnected); a
        // `pending()` double leaks `tx` so the receiver keeps reporting Empty,
        // matching an in-flight production fetch.
        match self.result.borrow_mut().take() {
            Some(result) => {
                let _ = tx.send(result);
            }
            None => std::mem::forget(tx),
        }
        FetchTask { rx, abort: None }
    }
}

// ── Production resolver ────────────────────────────────────────────────────

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use ags_runtime::runtime::workflows::resolve_options;
use ags_runtime::runtime::Runtime;
use tokio::runtime::Handle;

use crate::frontend::terminal::fullscreen::surface::FullscreenSurface;

use crate::frontend::terminal::dynamic_enums::SPINNER_FRAMES;

/// Production resolver: bridges the sync trait to the async runtime resolver via
/// `Handle::spawn` + a blocking `mpsc` poll loop. The cloned `Runtime` is the
/// resolver's private one (never the executor's). The poll loop animates a
/// spinner through the shared surface and polls crossterm for Esc / Ctrl-C so a
/// slow/hung fetch is cancellable.
pub struct ProductionResolver {
    runtime: Arc<tokio::sync::Mutex<Runtime>>,
    handle: Handle,
    surface: Rc<RefCell<FullscreenSurface>>,
}

impl ProductionResolver {
    /// Build a production resolver bound to a private `runtime` clone, the async
    /// `handle` it spawns fetches on, and the shared fullscreen surface it
    /// animates the spinner through.
    pub fn new(runtime: Runtime, handle: Handle, surface: Rc<RefCell<FullscreenSurface>>) -> Self {
        Self {
            runtime: Arc::new(tokio::sync::Mutex::new(runtime)),
            handle,
            surface,
        }
    }
}

impl DynamicOptionResolver for ProductionResolver {
    fn resolve(
        &self,
        source: &OptionsSource,
        inputs: &BTreeMap<String, serde_json::Value>,
    ) -> Result<ResolvedOptions, CliError> {
        use crossterm::event::{self, Event, KeyCode, KeyModifiers};

        let runtime = Arc::clone(&self.runtime);
        let source = source.clone();
        let inputs = inputs.clone();
        let (tx, rx) =
            mpsc::channel::<Result<ResolvedOptions, ags_protocol::error::RuntimeError>>();

        let join = self.handle.spawn(async move {
            let mut guard = runtime.lock().await;
            let result = resolve_options(&mut guard, &source, &inputs).await;
            let _ = tx.send(result);
        });

        let mut frame = 0usize;
        loop {
            match rx.try_recv() {
                Ok(Ok(resolved)) => {
                    self.surface.borrow_mut().set_options_loading(None);
                    return Ok(resolved);
                }
                Ok(Err(err)) => {
                    self.surface.borrow_mut().set_options_loading(None);
                    return Err(err.into());
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.surface.borrow_mut().set_options_loading(None);
                    return Err(CliError::Usage {
                        message: "options fetch ended unexpectedly".into(),
                        metadata: None,
                    });
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }

            {
                let mut s = self.surface.borrow_mut();
                s.set_options_loading(Some(format!(
                    "Loading choices… {}",
                    SPINNER_FRAMES[frame % SPINNER_FRAMES.len()]
                )));
                let _ = s.render();
            }
            frame += 1;

            if event::poll(Duration::from_millis(80)).unwrap_or(false) {
                if let Ok(Event::Key(k)) = event::read() {
                    // Windows emits Press + Release per keystroke; act on Press
                    // only so a key release doesn't spuriously cancel the fetch.
                    if k.kind != crossterm::event::KeyEventKind::Press {
                        continue;
                    }
                    let cancel = k.code == KeyCode::Esc
                        || (k.code == KeyCode::Char('c')
                            && k.modifiers.contains(KeyModifiers::CONTROL));
                    if cancel {
                        join.abort();
                        self.surface.borrow_mut().set_options_loading(None);
                        return Err(CliError::Usage {
                            message: "options fetch cancelled".into(),
                            metadata: None,
                        });
                    }
                }
            }
        }
    }
}

// ── Test double ────────────────────────────────────────────────────────────

/// Test double: returns a scripted result with no I/O and no spawn. The form↔
/// picker flow is fully unit-testable with this + `TestBackend` + scripted keys.
#[cfg(test)]
pub struct CannedResolver {
    pub result: std::cell::RefCell<Result<ResolvedOptions, CliError>>,
    pub calls: std::cell::RefCell<usize>,
}

#[cfg(test)]
impl CannedResolver {
    pub fn ok(choices: Vec<ags_protocol::workflow::OptionChoice>) -> Self {
        Self {
            result: std::cell::RefCell::new(Ok(ResolvedOptions {
                choices,
                truncated: false,
            })),
            calls: std::cell::RefCell::new(0),
        }
    }
}

#[cfg(test)]
impl DynamicOptionResolver for CannedResolver {
    fn resolve(
        &self,
        _source: &OptionsSource,
        _inputs: &BTreeMap<String, serde_json::Value>,
    ) -> Result<ResolvedOptions, CliError> {
        *self.calls.borrow_mut() += 1;
        match &*self.result.borrow() {
            Ok(r) => Ok(r.clone()),
            Err(_) => Err(CliError::Usage {
                message: "canned error".into(),
                metadata: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ags_protocol::workflow::{OperationReference, OptionChoice, OptionsSource};

    fn dummy_source() -> OptionsSource {
        OptionsSource {
            operation: OperationReference {
                service: ags_protocol::catalogue::ServiceId::new("ams"),
                operation: ags_protocol::catalogue::OperationId::new("ams/admin/images/v1/list"),
            },
            parameters: BTreeMap::new(),
            items_path: "$.images".into(),
            value: "$.id".into(),
            label: None,
            label_detail: None,
            fallback_description: None,
            filter: None,
        }
    }

    #[test]
    fn test_canned_resolver_returns_choices_and_counts_calls() {
        let resolver = CannedResolver::ok(vec![OptionChoice {
            label: "Prod".into(),
            value: "img-1".into(),
        }]);
        let out = resolver.resolve(&dummy_source(), &BTreeMap::new()).unwrap();
        assert_eq!(out.choices.len(), 1);
        assert_eq!(*resolver.calls.borrow(), 1);
    }
}

#[cfg(test)]
mod production_tests {
    use super::*;
    use ags_protocol::error::RuntimeError;
    use ags_protocol::workflow::{OperationReference, OptionParameterBinding, OptionsSource};
    use ags_runtime::runtime::dispatch::http::{HttpBody, HttpClient, HttpRequest, HttpResponse};
    use ags_runtime::runtime::execution::ExecutionContext;
    use ags_runtime::runtime::Runtime;
    use async_trait::async_trait;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    struct CannedClient(String);
    #[async_trait]
    impl HttpClient for CannedClient {
        async fn send(&self, _r: HttpRequest) -> Result<HttpResponse, RuntimeError> {
            Ok(HttpResponse {
                status: 200,
                body: HttpBody::Text(self.0.clone()),
            })
        }
    }

    fn source() -> OptionsSource {
        OptionsSource {
            operation: OperationReference {
                service: ags_protocol::catalogue::ServiceId::new("ams"),
                operation: ags_protocol::catalogue::OperationId::new("ams/admin/images/v1/list"),
            },
            parameters: BTreeMap::from([(
                "namespace".to_string(),
                OptionParameterBinding::FromInput("namespace".to_string()),
            )]),
            items_path: "$.images".into(),
            value: "$.id".into(),
            label: Some("$.name".into()),
            label_detail: None,
            fallback_description: None,
            filter: None,
        }
    }

    /// A completed fetch returns choices through the spawn-and-channel bridge.
    #[test]
    fn test_production_resolver_returns_choices_on_completion() {
        use crate::frontend::terminal::fullscreen::surface::FullscreenSurface;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let ctx = ExecutionContext {
                base_url: "https://example.com".to_string(),
                access_token: "t".to_string(),
                ..ExecutionContext::default()
            };
            let runtime = Runtime::new(
                ctx,
                Box::new(CannedClient(
                    r#"{"images":[{"id":"img-1","name":"Prod"}]}"#.into(),
                )),
                reqwest::Client::new(),
            );
            let surface = Rc::new(RefCell::new(FullscreenSurface::without_terminal()));
            let resolver =
                ProductionResolver::new(runtime, tokio::runtime::Handle::current(), surface);
            let inputs = BTreeMap::from([("namespace".to_string(), serde_json::json!("dev"))]);
            let resolved = resolver.resolve(&source(), &inputs).expect("ok");
            assert_eq!(resolved.choices.len(), 1);
            assert_eq!(resolved.choices[0].label, "Prod");
        });
    }
}

#[cfg(test)]
mod fetch_tests {
    use super::*;
    use ags_protocol::workflow::{OptionChoice, OptionsSource};
    use std::collections::BTreeMap;

    fn source() -> OptionsSource {
        OptionsSource {
            operation: ags_protocol::workflow::OperationReference {
                service: ags_protocol::catalogue::ServiceId::new("iam"),
                operation: ags_protocol::catalogue::OperationId::new("iam/admin/users/v3/search"),
            },
            parameters: BTreeMap::new(),
            items_path: "$.data".into(),
            value: "$.userId".into(),
            label: None,
            label_detail: None,
            fallback_description: None,
            filter: None,
        }
    }

    #[test]
    fn test_canned_fetch_delivers_ready_choices() {
        let fetch = CannedFetch::ok(vec![OptionChoice {
            label: "Ada".into(),
            value: "u-1".into(),
        }]);
        let task = fetch.start(&source(), &BTreeMap::new());
        // Ready channel: the result is immediately receivable.
        let got = task.rx.recv().expect("canned result present");
        let resolved = got.expect("Ok result");
        assert_eq!(resolved.choices.len(), 1);
        assert_eq!(resolved.choices[0].value, "u-1");
    }

    #[test]
    fn test_canned_fetch_pending_stays_empty_until_dropped() {
        // A "pending" double never sends — exercises the sub-loop's Esc/abort path.
        let fetch = CannedFetch::pending();
        let task = fetch.start(&source(), &BTreeMap::new());
        assert!(matches!(
            task.rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
    }
}
