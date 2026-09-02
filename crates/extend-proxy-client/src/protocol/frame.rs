//! Frame types exchanged over the extend-proxy tunnel. Ported from Go's
//! `pkg/protocol/frame.go`, `session.go`, and `stream.go`.
//!
//! `Frame` is modeled as a tagged union (`stream_id` + `FrameBody`) rather
//! than a struct-of-optional-pointers mirroring Go's `Frame` — see the port
//! plan's AI-assisted porting workflow section. Payloads stay `Option` where
//! Go's are pointers (nil-able) so `decode` can still produce a `Frame` for
//! malformed wire input and defer rejection to `validate`, matching Go's
//! `Decode`/`Validate` split exactly (`TestValidate` constructs several
//! frames with missing payloads directly, without going through `decode`).

use serde::{Deserialize, Serialize};

use super::error::ValidationError;

/// Discriminates the 12 frame kinds, plus an `Unknown` catch-all for
/// forward-compatibility. Mirrors Go's `FrameType` string constants.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FrameType {
    SessionInit,
    SessionAck,
    Open,
    OpenAck,
    Nack,
    Data,
    Fin,
    Rst,
    Ping,
    Pong,
    Cmd,
    CmdResponse,
    /// A `type` string this crate doesn't recognize. `decode` never rejects
    /// one (mirrors Go's `Decode`, which doesn't inspect `Type` at all);
    /// only `Frame::validate` does, matching `TestValidate`'s "unknown type" case.
    Unknown(String),
}

impl FrameType {
    /// The wire string for this frame type, e.g. `"SESSION_INIT"`.
    pub fn as_wire_str(&self) -> &str {
        match self {
            Self::SessionInit => "SESSION_INIT",
            Self::SessionAck => "SESSION_ACK",
            Self::Open => "OPEN",
            Self::OpenAck => "OPEN_ACK",
            Self::Nack => "NACK",
            Self::Data => "DATA",
            Self::Fin => "FIN",
            Self::Rst => "RST",
            Self::Ping => "PING",
            Self::Pong => "PONG",
            Self::Cmd => "CMD",
            Self::CmdResponse => "CMD_RESPONSE",
            Self::Unknown(s) => s,
        }
    }

    /// Parses a wire `type` string, falling back to `Unknown` for anything
    /// unrecognized rather than failing — mirrors Go's `Decode` never
    /// rejecting a frame based on its `Type` value.
    pub(super) fn from_wire_str(s: &str) -> Self {
        match s {
            "SESSION_INIT" => Self::SessionInit,
            "SESSION_ACK" => Self::SessionAck,
            "OPEN" => Self::Open,
            "OPEN_ACK" => Self::OpenAck,
            "NACK" => Self::Nack,
            "DATA" => Self::Data,
            "FIN" => Self::Fin,
            "RST" => Self::Rst,
            "PING" => Self::Ping,
            "PONG" => Self::Pong,
            "CMD" => Self::Cmd,
            "CMD_RESPONSE" => Self::CmdResponse,
            other => Self::Unknown(other.to_string()),
        }
    }
}

impl std::fmt::Display for FrameType {
    /// Formats like Go's `%q` on the wire string (e.g. `"SESSION_INIT"`), so
    /// [`ValidationError`] messages read the same as Go's
    /// `fmt.Errorf("...%q...", f.Type)` instead of Rust's bare enum Debug name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_wire_str())
    }
}

/// Payload of a SESSION_INIT frame. Mirrors `protocol.SessionInitPayload`.
///
/// `client_version`/`server_version` (below) are `i64`, not `u32`, even
/// though [`PROTOCOL_VERSION`](super::PROTOCOL_VERSION) is always small and
/// positive today: Go's `int` field has no range check in `Validate()`, so a
/// negative or out-of-`u32`-range value decodes and validates fine on the Go
/// side. A narrower Rust type would reject those same wire bytes at
/// decode-time instead of matching Go's decode-then-compare behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInitPayload {
    pub client_version: i64,
    /// e.g. `["flow_control"]`. `deserialize_with` accepts an explicit JSON
    /// `null` the same way Go's `encoding/json` does for a bare `[]string`.
    #[serde(
        default,
        deserialize_with = "super::serde_helpers::null_as_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub capabilities: Vec<String>,
    /// Ports the sidecar should tunnel; all others passthrough. See
    /// `capabilities` above for why `null` is accepted alongside a missing key.
    #[serde(default, deserialize_with = "super::serde_helpers::null_as_default")]
    pub intercept_ports: Vec<i32>,
}

/// Payload of a SESSION_ACK frame. Mirrors `protocol.SessionAckPayload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionAckPayload {
    pub server_version: i64,
    pub session_id: String,
    pub idle_timeout_sec: i64,
}

/// Payload of an OPEN frame. Mirrors `protocol.OpenPayload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenPayload {
    /// e.g. `"127.0.0.1"` for inbound.
    pub target_host: String,
    /// Port on the receiving side.
    pub target_port: i32,
    /// Original caller address, for logging.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub remote_addr: String,
}

/// Payload of a NACK frame. Mirrors `protocol.NackPayload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NackPayload {
    pub stream_id: u64,
    pub reason_code: String,
    /// `None` means implementation default; `Some(false)` means do not retry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

/// Payload of a CMD frame. `params` is arbitrary JSON so callers can use any
/// schema without coupling the protocol to specific command types. Mirrors
/// `protocol.CmdPayload` (whose `Params` is `json.RawMessage`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CmdPayload {
    /// Unique request id (monotonic, > 0).
    pub cmd_id: u64,
    /// Command name to invoke.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// Payload of a CMD_RESPONSE frame. Mirrors `protocol.CmdResponsePayload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CmdResponsePayload {
    /// Matches the originating CMD frame.
    pub cmd_id: u64,
    /// Callee response, set on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<serde_json::Value>,
    /// Callee error message, set when the callee returns an error.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
}

/// The frame-type-specific portion of a `Frame`, minus `stream_id`.
#[derive(Debug, Clone, PartialEq)]
pub enum FrameBody {
    SessionInit(Option<SessionInitPayload>),
    SessionAck(Option<SessionAckPayload>),
    Open(Option<OpenPayload>),
    OpenAck,
    Nack(Option<NackPayload>),
    Data(Vec<u8>),
    Fin,
    /// Used by RST frames.
    Rst(String),
    Ping,
    Pong,
    Cmd(Option<CmdPayload>),
    CmdResponse(Option<CmdResponsePayload>),
    /// See `FrameType::Unknown`.
    Unknown(String),
}

/// The top-level message unit exchanged over the WebSocket tunnel. One
/// WebSocket message = one frame; maximum size is
/// [`MAX_FRAME_SIZE_BYTES`](super::MAX_FRAME_SIZE_BYTES). Mirrors Go's `Frame` struct.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// 0 for session-level and CMD/CMD_RESPONSE frames.
    pub stream_id: u64,
    pub body: FrameBody,
}

impl Frame {
    /// Constructs a SESSION_INIT frame.
    pub fn new_session_init(
        client_version: i64,
        capabilities: Vec<String>,
        intercept_ports: Vec<i32>,
    ) -> Self {
        Self {
            stream_id: 0,
            body: FrameBody::SessionInit(Some(SessionInitPayload {
                client_version,
                capabilities,
                intercept_ports,
            })),
        }
    }

    /// Constructs a SESSION_ACK frame.
    pub fn new_session_ack(
        server_version: i64,
        session_id: impl Into<String>,
        idle_timeout_sec: i64,
    ) -> Self {
        Self {
            stream_id: 0,
            body: FrameBody::SessionAck(Some(SessionAckPayload {
                server_version,
                session_id: session_id.into(),
                idle_timeout_sec,
            })),
        }
    }

    /// Constructs an OPEN frame for a new stream.
    pub fn new_open(
        stream_id: u64,
        target_host: impl Into<String>,
        target_port: i32,
        remote_addr: impl Into<String>,
    ) -> Self {
        Self {
            stream_id,
            body: FrameBody::Open(Some(OpenPayload {
                target_host: target_host.into(),
                target_port,
                remote_addr: remote_addr.into(),
            })),
        }
    }

    /// Constructs an OPEN_ACK frame.
    pub fn new_open_ack(stream_id: u64) -> Self {
        Self {
            stream_id,
            body: FrameBody::OpenAck,
        }
    }

    /// Constructs a NACK frame. `retryable` may be `None` (field omitted on the wire).
    pub fn new_nack(
        stream_id: u64,
        reason_code: impl Into<String>,
        retryable: Option<bool>,
    ) -> Self {
        Self {
            stream_id,
            body: FrameBody::Nack(Some(NackPayload {
                stream_id,
                reason_code: reason_code.into(),
                retryable,
            })),
        }
    }

    /// Constructs a DATA frame carrying a byte payload.
    pub fn new_data(stream_id: u64, payload: Vec<u8>) -> Self {
        Self {
            stream_id,
            body: FrameBody::Data(payload),
        }
    }

    /// Constructs a FIN frame (half-close: sender will send no more DATA).
    pub fn new_fin(stream_id: u64) -> Self {
        Self {
            stream_id,
            body: FrameBody::Fin,
        }
    }

    /// Constructs an RST frame (abort stream immediately).
    pub fn new_rst(stream_id: u64, reason: impl Into<String>) -> Self {
        Self {
            stream_id,
            body: FrameBody::Rst(reason.into()),
        }
    }

    /// Constructs a PING keepalive frame.
    pub fn new_ping() -> Self {
        Self {
            stream_id: 0,
            body: FrameBody::Ping,
        }
    }

    /// Constructs a PONG keepalive frame.
    pub fn new_pong() -> Self {
        Self {
            stream_id: 0,
            body: FrameBody::Pong,
        }
    }

    /// Constructs a CMD frame. Pass `None` for commands with no parameters.
    pub fn new_cmd(
        cmd_id: u64,
        name: impl Into<String>,
        params: Option<serde_json::Value>,
    ) -> Self {
        Self {
            stream_id: 0,
            body: FrameBody::Cmd(Some(CmdPayload {
                cmd_id,
                name: name.into(),
                params,
            })),
        }
    }

    /// Constructs a CMD_RESPONSE frame carrying a successful response.
    /// Pass `None` if there is no response body.
    pub fn new_cmd_response(cmd_id: u64, response: Option<serde_json::Value>) -> Self {
        Self {
            stream_id: 0,
            body: FrameBody::CmdResponse(Some(CmdResponsePayload {
                cmd_id,
                response,
                error: String::new(),
            })),
        }
    }

    /// Constructs a CMD_RESPONSE frame carrying a callee error.
    pub fn new_cmd_error_response(cmd_id: u64, error_message: impl Into<String>) -> Self {
        Self {
            stream_id: 0,
            body: FrameBody::CmdResponse(Some(CmdResponsePayload {
                cmd_id,
                response: None,
                error: error_message.into(),
            })),
        }
    }

    /// The frame's `FrameType`. Cheap except for `Unknown`, which clones its wire string.
    pub fn frame_type(&self) -> FrameType {
        match &self.body {
            FrameBody::SessionInit(_) => FrameType::SessionInit,
            FrameBody::SessionAck(_) => FrameType::SessionAck,
            FrameBody::Open(_) => FrameType::Open,
            FrameBody::OpenAck => FrameType::OpenAck,
            FrameBody::Nack(_) => FrameType::Nack,
            FrameBody::Data(_) => FrameType::Data,
            FrameBody::Fin => FrameType::Fin,
            FrameBody::Rst(_) => FrameType::Rst,
            FrameBody::Ping => FrameType::Ping,
            FrameBody::Pong => FrameType::Pong,
            FrameBody::Cmd(_) => FrameType::Cmd,
            FrameBody::CmdResponse(_) => FrameType::CmdResponse,
            FrameBody::Unknown(s) => FrameType::Unknown(s.clone()),
        }
    }

    /// Checks protocol invariants on a frame. Called by both sidecar and
    /// client on every decoded frame before dispatch. Mirrors `protocol.Validate`.
    pub fn validate(&self) -> Result<(), ValidationError> {
        match &self.body {
            FrameBody::SessionInit(_) | FrameBody::SessionAck(_) => {
                if self.stream_id != 0 {
                    return Err(ValidationError::SessionFrameHasStreamId {
                        frame_type: self.frame_type(),
                        stream_id: self.stream_id,
                    });
                }
            }
            FrameBody::Open(payload) => {
                if self.stream_id == 0 {
                    return Err(ValidationError::OpenMissingStreamId);
                }
                let payload = payload
                    .as_ref()
                    .ok_or(ValidationError::OpenMissingPayload)?;
                if payload.target_port <= 0 || payload.target_port > 65535 {
                    return Err(ValidationError::OpenInvalidTargetPort(payload.target_port));
                }
                if payload.target_host.is_empty() {
                    return Err(ValidationError::OpenMissingTargetHost);
                }
            }
            FrameBody::OpenAck => {
                if self.stream_id == 0 {
                    return Err(ValidationError::OpenAckMissingStreamId);
                }
            }
            FrameBody::Nack(payload) => {
                if self.stream_id == 0 {
                    return Err(ValidationError::NackMissingStreamId);
                }
                if payload.is_none() {
                    return Err(ValidationError::NackMissingPayload);
                }
            }
            FrameBody::Data(_) | FrameBody::Fin | FrameBody::Rst(_) => {
                if self.stream_id == 0 {
                    return Err(ValidationError::DataLikeFrameMissingStreamId {
                        frame_type: self.frame_type(),
                    });
                }
            }
            FrameBody::Ping | FrameBody::Pong => {}
            FrameBody::Cmd(payload) => {
                if self.stream_id != 0 {
                    return Err(ValidationError::CmdHasStreamId(self.stream_id));
                }
                let payload = payload.as_ref().ok_or(ValidationError::CmdMissingPayload)?;
                if payload.cmd_id == 0 {
                    return Err(ValidationError::CmdMissingCmdId);
                }
                if payload.name.is_empty() {
                    return Err(ValidationError::CmdMissingName);
                }
            }
            FrameBody::CmdResponse(payload) => {
                if self.stream_id != 0 {
                    return Err(ValidationError::CmdResponseHasStreamId(self.stream_id));
                }
                let payload = payload
                    .as_ref()
                    .ok_or(ValidationError::CmdResponseMissingPayload)?;
                if payload.cmd_id == 0 {
                    return Err(ValidationError::CmdResponseMissingCmdId);
                }
            }
            FrameBody::Unknown(type_str) => {
                return Err(ValidationError::UnknownFrameType(type_str.clone()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Translated 1:1 from `codec_test.go`'s `TestValidate` table.

    #[test]
    fn test_validate_ping_valid() {
        assert!(Frame::new_ping().validate().is_ok());
    }

    #[test]
    fn test_validate_pong_valid() {
        assert!(Frame::new_pong().validate().is_ok());
    }

    #[test]
    fn test_validate_session_init_valid() {
        assert!(Frame::new_session_init(1, Vec::new(), vec![8080])
            .validate()
            .is_ok());
    }

    #[test]
    fn test_validate_session_init_with_stream_id_nonzero() {
        let frame = Frame {
            stream_id: 1,
            body: FrameBody::SessionInit(None),
        };
        assert!(frame.validate().is_err());
    }

    #[test]
    fn test_validate_open_valid() {
        assert!(Frame::new_open(1, "127.0.0.1", 8080, "").validate().is_ok());
    }

    #[test]
    fn test_validate_open_missing_target_host() {
        assert!(Frame::new_open(1, "", 8080, "").validate().is_err());
    }

    #[test]
    fn test_validate_open_invalid_target_port_zero() {
        assert!(Frame::new_open(1, "127.0.0.1", 0, "").validate().is_err());
    }

    #[test]
    fn test_validate_open_stream_id_zero() {
        let frame = Frame {
            stream_id: 0,
            body: FrameBody::Open(Some(OpenPayload {
                target_host: "x".to_string(),
                target_port: 80,
                remote_addr: String::new(),
            })),
        };
        assert!(frame.validate().is_err());
    }

    #[test]
    fn test_validate_nack_valid() {
        assert!(
            Frame::new_nack(1, crate::protocol::reason::PORT_NOT_ALLOWED, Some(false))
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn test_validate_data_stream_id_zero() {
        let frame = Frame {
            stream_id: 0,
            body: FrameBody::Data(b"x".to_vec()),
        };
        assert!(frame.validate().is_err());
    }

    #[test]
    fn test_validate_data_valid() {
        assert!(Frame::new_data(1, b"hello".to_vec()).validate().is_ok());
    }

    #[test]
    fn test_validate_unknown_type() {
        let frame = Frame {
            stream_id: 0,
            body: FrameBody::Unknown("UNKNOWN".to_string()),
        };
        assert!(frame.validate().is_err());
    }

    #[test]
    fn test_validate_cmd_valid() {
        let frame = Frame {
            stream_id: 0,
            body: FrameBody::Cmd(Some(CmdPayload {
                cmd_id: 1,
                name: "ping".to_string(),
                params: None,
            })),
        };
        assert!(frame.validate().is_ok());
    }

    #[test]
    fn test_validate_cmd_missing_payload() {
        let frame = Frame {
            stream_id: 0,
            body: FrameBody::Cmd(None),
        };
        assert!(frame.validate().is_err());
    }

    #[test]
    fn test_validate_cmd_zero_cmd_id() {
        let frame = Frame {
            stream_id: 0,
            body: FrameBody::Cmd(Some(CmdPayload {
                cmd_id: 0,
                name: "ping".to_string(),
                params: None,
            })),
        };
        assert!(frame.validate().is_err());
    }

    #[test]
    fn test_validate_cmd_missing_name() {
        let frame = Frame {
            stream_id: 0,
            body: FrameBody::Cmd(Some(CmdPayload {
                cmd_id: 1,
                name: String::new(),
                params: None,
            })),
        };
        assert!(frame.validate().is_err());
    }

    #[test]
    fn test_validate_cmd_non_zero_stream_id_rejected() {
        let frame = Frame {
            stream_id: 1,
            body: FrameBody::Cmd(Some(CmdPayload {
                cmd_id: 1,
                name: "ping".to_string(),
                params: None,
            })),
        };
        assert!(frame.validate().is_err());
    }

    #[test]
    fn test_validate_cmd_response_valid() {
        let frame = Frame {
            stream_id: 0,
            body: FrameBody::CmdResponse(Some(CmdResponsePayload {
                cmd_id: 1,
                response: None,
                error: String::new(),
            })),
        };
        assert!(frame.validate().is_ok());
    }

    #[test]
    fn test_validate_cmd_response_missing_payload() {
        let frame = Frame {
            stream_id: 0,
            body: FrameBody::CmdResponse(None),
        };
        assert!(frame.validate().is_err());
    }

    #[test]
    fn test_validate_cmd_response_zero_cmd_id() {
        let frame = Frame {
            stream_id: 0,
            body: FrameBody::CmdResponse(Some(CmdResponsePayload {
                cmd_id: 0,
                response: None,
                error: String::new(),
            })),
        };
        assert!(frame.validate().is_err());
    }

    #[test]
    fn test_validate_cmd_response_non_zero_stream_id_rejected() {
        let frame = Frame {
            stream_id: 1,
            body: FrameBody::CmdResponse(Some(CmdResponsePayload {
                cmd_id: 1,
                response: None,
                error: String::new(),
            })),
        };
        assert!(frame.validate().is_err());
    }
}
