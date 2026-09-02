//! Client connection lifecycle (reconnect/backoff) on top of `session`.
//! Ported from Go's `pkg/client`.

mod agent;
mod session_holder;

pub use agent::{Agent, AgentError, Config, PortMapping, TokenProvider};
pub use session_holder::SessionHolder;
