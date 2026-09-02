//! Typed convenience wrappers around `Session::call_rpc`/`register_command_handler`.
//! Ported from Go's `pkg/tunnel/rpc_typed.go`. Depends only on `Session`'s
//! public API — kept as a separate module for structural parity with Go's
//! separate `rpc_typed.go` file.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::core::{CommandHandler, RpcError, Session};

/// Sends a typed RPC call and deserializes the CMD_RESPONSE payload into
/// `R`. Mirrors Go's generic `tunnel.CallRPCTyped`. An empty/absent response
/// yields `R::default()` (the Rust analogue of Go's implicit zero value).
pub async fn call_rpc_typed<P, R>(
    session: &Session,
    name: &str,
    params: P,
    timeout: Duration,
) -> Result<R, RpcError>
where
    P: Serialize,
    R: DeserializeOwned + Default,
{
    let params_value = serde_json::to_value(&params).map_err(RpcError::Serialize)?;
    let raw = session.call_rpc(name, Some(params_value), timeout).await?;
    match raw {
        Some(value) => serde_json::from_value(value).map_err(RpcError::Deserialize),
        None => Ok(R::default()),
    }
}

/// Registers a strongly-typed command handler on a `Session`. Handles
/// unmarshalling the incoming raw JSON params into `P` and marshalling the
/// returned `R` back through the untyped `CommandHandler` pipeline. Mirrors
/// Go's generic `tunnel.RegisterHandler`.
pub fn register_handler_typed<P, R, F, Fut>(session: &Session, name: impl Into<String>, handler: F)
where
    P: DeserializeOwned + Default + Send + 'static,
    R: Serialize + Send + 'static,
    F: Fn(P) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<R, String>> + Send + 'static,
{
    let handler = Arc::new(handler);
    let wrapped: CommandHandler = Arc::new(move |raw: Option<serde_json::Value>| {
        let handler = handler.clone();
        Box::pin(async move {
            let params: P = match raw {
                Some(v) => {
                    serde_json::from_value(v).map_err(|e| format!("unmarshal params: {e}"))?
                }
                None => P::default(),
            };
            let result = handler(params).await?;
            let value = serde_json::to_value(result).map_err(|e| {
                tracing::warn!(error = %e, "CMD: failed to marshal handler response");
                format!("marshal response: {e}")
            })?;
            Ok(Some(value))
        })
            as Pin<Box<dyn Future<Output = Result<Option<serde_json::Value>, String>> + Send>>
    });
    session.register_command_handler(name, wrapped);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio_tungstenite::tungstenite::protocol::Role;
    use tokio_tungstenite::WebSocketStream;

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    struct AddReq {
        a: i32,
        b: i32,
    }

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    struct AddResp {
        sum: i32,
    }

    #[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
    struct Empty {}

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    struct GreetReq {
        name: String,
    }

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    struct GreetResp {
        greeting: String,
    }

    async fn websocket_pair() -> (
        WebSocketStream<tokio::io::DuplexStream>,
        WebSocketStream<tokio::io::DuplexStream>,
    ) {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let client = WebSocketStream::from_raw_socket(client_io, Role::Client, None).await;
        let server = WebSocketStream::from_raw_socket(server_io, Role::Server, None).await;
        (client, server)
    }

    async fn session_pair() -> (Arc<Session>, Arc<Session>) {
        let (client_ws, server_ws) = websocket_pair().await;

        let server_task = tokio::spawn(async move {
            let (session, _init) =
                Session::wait_session_from_client(server_ws, Duration::from_secs(3600), None)
                    .await
                    .expect("server handshake");
            session.send_session_ack().await.expect("send SESSION_ACK");
            session
        });

        let client_session =
            Session::send_session_to_server(client_ws, HashMap::new(), Duration::from_secs(3))
                .await
                .expect("client handshake");
        let server_session = server_task.await.expect("server task panicked");

        tokio::spawn(client_session.clone().run(Duration::ZERO, Duration::ZERO));
        tokio::spawn(server_session.clone().run(Duration::ZERO, Duration::ZERO));

        (client_session, server_session)
    }

    /// Translated from `rpc_test.go`'s `TestCallRPCTypedSuccess`.
    #[tokio::test]
    async fn test_call_rpc_typed_success() {
        let (caller, callee) = session_pair().await;
        register_handler_typed(&callee, "add", |req: AddReq| async move {
            Ok(AddResp { sum: req.a + req.b })
        });

        let resp: AddResp = call_rpc_typed(
            &caller,
            "add",
            AddReq { a: 3, b: 4 },
            Duration::from_secs(3),
        )
        .await
        .expect("CallRPCTyped");
        assert_eq!(resp.sum, 7);
    }

    /// Translated from `rpc_test.go`'s `TestCallRPCTypedCalleeError`.
    #[tokio::test]
    async fn test_call_rpc_typed_callee_error() {
        let (caller, callee) = session_pair().await;
        register_handler_typed(&callee, "boom", |_req: Empty| async move {
            Err::<Empty, _>("intentional failure".to_string())
        });

        let result: Result<Empty, RpcError> =
            call_rpc_typed(&caller, "boom", Empty {}, Duration::from_secs(3)).await;
        assert!(result.is_err());
    }

    /// Translated from `rpc_test.go`'s `TestCallRPCTypedTimeout`.
    #[tokio::test]
    async fn test_call_rpc_typed_timeout() {
        let (caller, callee) = session_pair().await;
        register_handler_typed(&callee, "slow2", |_req: Empty| async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            Ok(Empty {})
        });

        let result: Result<Empty, RpcError> =
            call_rpc_typed(&caller, "slow2", Empty {}, Duration::from_millis(80)).await;
        assert!(matches!(result, Err(RpcError::Timeout)));
    }

    /// Translated from `rpc_test.go`'s `TestRegisterHandlerTypedParams`.
    #[tokio::test]
    async fn test_register_handler_typed_params() {
        let (caller, callee) = session_pair().await;
        register_handler_typed(&callee, "greet", |req: GreetReq| async move {
            Ok(GreetResp {
                greeting: format!("hello, {}", req.name),
            })
        });

        let resp: GreetResp = call_rpc_typed(
            &caller,
            "greet",
            GreetReq {
                name: "world".to_string(),
            },
            Duration::from_secs(3),
        )
        .await
        .expect("CallRPCTyped");
        assert_eq!(resp.greeting, "hello, world");
    }
}
