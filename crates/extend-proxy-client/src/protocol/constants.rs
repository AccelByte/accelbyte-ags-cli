//! Protocol-wide constants ported from Go's scattered `const` declarations,
//! centralized here since this crate owns the protocol layer even though the
//! Go side keeps them spread across `pkg/client`/`pkg/sidecar`/`pkg/protocol`.

use std::time::Duration;

/// Maximum size of a single WebSocket message / encoded frame (64 KiB).
/// Mirrors `maxWsMessageBytes`, duplicated identically in Go's `pkg/client`
/// and `pkg/sidecar`; centralized here since this crate owns the protocol layer.
pub const MAX_FRAME_SIZE_BYTES: usize = 64 * 1024;

/// Bound on frames buffered in a session's outbound channel (shared by every
/// stream's `read_pump` plus the session's own control frames) before a
/// sender has to wait. At `MAX_FRAME_SIZE_BYTES` each, this caps worst-case
/// buffered memory for a single stalled session to ~8 MiB — enough to absorb
/// a burst without unbounded growth, while still applying backpressure into
/// `read_pump`'s local reads once the peer's write side stalls.
pub const OUTBOUND_CHANNEL_CAPACITY: usize = 128;

/// Wire protocol version. Mirrors `tunnel.ProtocolVersion`.
pub const PROTOCOL_VERSION: u32 = 1;

/// Timeout waiting for the SESSION_INIT/SESSION_ACK handshake to complete.
/// Mirrors `tunnel.SessionInitTimeout`.
pub const SESSION_INIT_TIMEOUT: Duration = Duration::from_secs(15);

/// NACK/RST reason codes. Mirrors the `Reason*` string constants in
/// `pkg/protocol/stream.go`.
pub mod reason {
    pub const PORT_NOT_ALLOWED: &str = "port_not_allowed";
    pub const NO_LOCAL_LISTENER: &str = "no_local_listener";
    pub const NO_CLUSTER_LISTENER: &str = "no_cluster_listener";
    pub const LOCAL_READ_ERROR: &str = "local_read_error";
    pub const LOCAL_WRITE_ERROR: &str = "local_write_error";
    pub const WS_WRITE_ERROR: &str = "ws_write_error";
    pub const SESSION_TIMEOUT: &str = "session_timeout";
    pub const STREAM_UNKNOWN: &str = "stream_unknown";
    pub const INVALID_STREAM_ID: &str = "invalid_stream_id";
    pub const STREAM_ID_CONFLICT: &str = "stream_id_conflict";
    pub const INVALID_FRAME: &str = "invalid_frame";
}
