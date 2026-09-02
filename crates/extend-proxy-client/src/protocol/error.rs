//! Errors surfaced by the protocol codec and frame validation. Mirrors the
//! error paths of Go's `protocol.Encode`, `protocol.Decode`, and
//! `protocol.Validate`.

use thiserror::Error;

use super::frame::FrameType;

/// Failure encoding a `Frame` to wire bytes. Mirrors `protocol.Encode`'s error path.
#[derive(Debug, Error)]
pub enum EncodeError {
    #[error("encode frame: {0}")]
    Json(#[from] serde_json::Error),
}

/// Failure decoding wire bytes into a `Frame`. Mirrors `protocol.Decode`'s error path.
#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("decode frame: {0}")]
    Json(#[from] serde_json::Error),
}

/// A decoded frame that violates a protocol invariant. Mirrors every error
/// branch of Go's `protocol.Validate`.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ValidationError {
    #[error("session frame {frame_type} must have stream_id=0, got {stream_id}")]
    SessionFrameHasStreamId {
        frame_type: FrameType,
        stream_id: u64,
    },
    #[error("OPEN frame must have stream_id > 0")]
    OpenMissingStreamId,
    #[error("OPEN frame missing open payload")]
    OpenMissingPayload,
    #[error("OPEN frame invalid target_port {0}")]
    OpenInvalidTargetPort(i32),
    #[error("OPEN frame missing target_host")]
    OpenMissingTargetHost,
    #[error("OPEN_ACK frame must have stream_id > 0")]
    OpenAckMissingStreamId,
    #[error("NACK frame must have stream_id > 0")]
    NackMissingStreamId,
    #[error("NACK frame missing nack payload")]
    NackMissingPayload,
    #[error("{frame_type} frame must have stream_id > 0")]
    DataLikeFrameMissingStreamId { frame_type: FrameType },
    #[error("CMD frame must have stream_id=0, got {0}")]
    CmdHasStreamId(u64),
    #[error("CMD frame missing cmd payload")]
    CmdMissingPayload,
    #[error("CMD frame missing or zero cmd_id")]
    CmdMissingCmdId,
    #[error("CMD frame missing name")]
    CmdMissingName,
    #[error("CMD_RESPONSE frame must have stream_id=0, got {0}")]
    CmdResponseHasStreamId(u64),
    #[error("CMD_RESPONSE frame missing cmd_response payload")]
    CmdResponseMissingPayload,
    #[error("CMD_RESPONSE frame missing or zero cmd_id")]
    CmdResponseMissingCmdId,
    #[error("unknown frame type {0:?}")]
    UnknownFrameType(String),
}
