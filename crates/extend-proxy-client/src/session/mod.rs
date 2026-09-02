//! Session/stream state machine (client role) on top of `protocol`. Ported
//! from Go's `pkg/tunnel`.

mod core;
mod idgen;
mod rpc;
mod stream;

pub use core::{
    CommandHandler, DialFn, HandshakeError, InterceptTargetToLocalMapping, RpcError, Session,
    SessionAckError, SessionRole, TargetAllowlist,
};
pub use idgen::{IdGenerator, IdParity};
pub use rpc::{call_rpc_typed, register_handler_typed};
pub use stream::{BoxFuture, DuplexConn, Stream};
