//! JSON wire codec. Mirrors Go's `pkg/protocol/codec.go`: one WebSocket
//! message = one flat JSON object with a `type` discriminator, a `stream_id`
//! (omitted when zero), and a set of mutually-exclusive `omitempty` payload
//! fields. `Frame`/`FrameBody` (an idiomatic tagged enum, see `frame.rs`) is
//! converted to/from this flat `WireFrame` at the encode/decode boundary so
//! the wire shape stays byte-compatible with the Go sidecar without forcing
//! the same struct-of-optionals shape on the rest of this crate.

use serde::{Deserialize, Serialize};

use super::error::{DecodeError, EncodeError};
use super::frame::{
    CmdPayload, CmdResponsePayload, Frame, FrameBody, FrameType, NackPayload, OpenPayload,
    SessionAckPayload, SessionInitPayload,
};

/// Encodes a `Frame` to wire bytes (JSON text in protocol v1). Mirrors `protocol.Encode`.
pub fn encode(frame: &Frame) -> Result<Vec<u8>, EncodeError> {
    let wire = WireFrame::from(frame);
    Ok(serde_json::to_vec(&wire)?)
}

/// Decodes wire bytes into a `Frame`. Never fails on an unrecognized `type`
/// string (mirrors Go's `Decode`, which doesn't inspect `Type` at all); only
/// fails on malformed JSON. Mirrors `protocol.Decode`.
pub fn decode(bytes: &[u8]) -> Result<Frame, DecodeError> {
    let wire: WireFrame = serde_json::from_slice(bytes)?;
    Ok(Frame::from(wire))
}

/// The flat wire shape. Mirrors Go's `Frame` struct field-for-field.
#[derive(Debug, Default, Serialize, Deserialize)]
struct WireFrame {
    #[serde(rename = "type")]
    frame_type: String,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    stream_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_init: Option<SessionInitPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_ack: Option<SessionAckPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    open: Option<OpenPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_ack: Option<WireOpenAck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nack: Option<NackPayload>,
    /// Go's `encoding/json` serializes `[]byte` as a standard-base64 string.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "opt_base64")]
    data: Option<Vec<u8>>,
    /// Accepts an explicit JSON `null` alongside a missing key, matching
    /// Go's `encoding/json` tolerance for `null` on a bare `string` field.
    #[serde(
        default,
        deserialize_with = "super::serde_helpers::null_as_default",
        skip_serializing_if = "String::is_empty"
    )]
    reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cmd: Option<CmdPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cmd_response: Option<CmdResponsePayload>,
}

/// Mirrors `protocol.OpenAckPayload`. `stream_id` here is always redundant
/// with the top-level `Frame.stream_id` (Go sets both identically in
/// `NewOpenAck`); `decode` discards it and trusts the top-level field only,
/// same as every Go call site that reads `f.StreamID` instead of
/// `f.OpenAck.StreamID`. Kept on encode for wire-shape parity.
#[derive(Debug, Serialize, Deserialize)]
struct WireOpenAck {
    stream_id: u64,
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

impl From<&Frame> for WireFrame {
    fn from(frame: &Frame) -> Self {
        let mut wire = WireFrame {
            frame_type: frame.frame_type().as_wire_str().to_string(),
            stream_id: frame.stream_id,
            ..Default::default()
        };
        match &frame.body {
            FrameBody::SessionInit(p) => wire.session_init = p.clone(),
            FrameBody::SessionAck(p) => wire.session_ack = p.clone(),
            FrameBody::Open(p) => wire.open = p.clone(),
            FrameBody::OpenAck => {
                wire.open_ack = Some(WireOpenAck {
                    stream_id: frame.stream_id,
                })
            }
            FrameBody::Nack(p) => wire.nack = p.clone(),
            FrameBody::Data(d) => {
                if !d.is_empty() {
                    wire.data = Some(d.clone());
                }
            }
            FrameBody::Fin => {}
            FrameBody::Rst(reason) => wire.reason = reason.clone(),
            FrameBody::Ping | FrameBody::Pong => {}
            FrameBody::Cmd(p) => wire.cmd = p.clone(),
            FrameBody::CmdResponse(p) => wire.cmd_response = p.clone(),
            FrameBody::Unknown(_) => {}
        }
        wire
    }
}

impl From<WireFrame> for Frame {
    fn from(wire: WireFrame) -> Self {
        let body = match FrameType::from_wire_str(&wire.frame_type) {
            FrameType::SessionInit => FrameBody::SessionInit(wire.session_init),
            FrameType::SessionAck => FrameBody::SessionAck(wire.session_ack),
            FrameType::Open => FrameBody::Open(wire.open),
            FrameType::OpenAck => FrameBody::OpenAck,
            FrameType::Nack => FrameBody::Nack(wire.nack),
            FrameType::Data => FrameBody::Data(wire.data.unwrap_or_default()),
            FrameType::Fin => FrameBody::Fin,
            FrameType::Rst => FrameBody::Rst(wire.reason),
            FrameType::Ping => FrameBody::Ping,
            FrameType::Pong => FrameBody::Pong,
            FrameType::Cmd => FrameBody::Cmd(wire.cmd),
            FrameType::CmdResponse => FrameBody::CmdResponse(wire.cmd_response),
            FrameType::Unknown(s) => FrameBody::Unknown(s),
        };
        Frame {
            stream_id: wire.stream_id,
            body,
        }
    }
}

/// `serde(with = "opt_base64")` — standard-base64 string on the wire,
/// matching Go's default `[]byte` JSON encoding (`base64.StdEncoding`).
mod opt_base64 {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serializes `Some(bytes)` as a standard-base64 string; never called
    /// for `None` in practice since the field's `skip_serializing_if` short-circuits first.
    pub fn serialize<S>(value: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(bytes) => serializer.serialize_str(&STANDARD.encode(bytes)),
            None => serializer.serialize_none(),
        }
    }

    /// Decodes a base64 string into `Some(bytes)`; absent field deserializes as `None`.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw: Option<String> = Option::deserialize(deserializer)?;
        match raw {
            Some(s) => STANDARD
                .decode(s.as_bytes())
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Translated from `codec_test.go`'s `TestEncodeDecodeRoundTrip`: every
    /// frame variant survives an encode→decode round trip with its `type`
    /// and `stream_id` intact.
    #[test]
    fn test_encode_decode_round_trip() {
        let cmd = Frame::new_cmd(1, "echo", Some(serde_json::json!({"msg": "hi"})));
        let cmd_resp = Frame::new_cmd_response(1, Some(serde_json::json!({"result": "ok"})));

        let frames = vec![
            Frame::new_session_init(1, vec!["flow_control".to_string()], vec![8080, 9090]),
            Frame::new_session_ack(1, "abc123", 1800),
            Frame::new_open(1, "127.0.0.1", 8080, "10.0.0.1:54321"),
            Frame::new_open_ack(1),
            Frame::new_nack(1, crate::protocol::reason::PORT_NOT_ALLOWED, Some(false)),
            Frame::new_data(3, b"hello".to_vec()),
            Frame::new_fin(3),
            Frame::new_rst(5, crate::protocol::reason::LOCAL_READ_ERROR),
            Frame::new_ping(),
            Frame::new_pong(),
            cmd,
            cmd_resp,
            Frame::new_cmd_error_response(2, "something went wrong"),
        ];

        for frame in frames {
            let bytes =
                encode(&frame).unwrap_or_else(|e| panic!("encode({:?}): {e}", frame.frame_type()));
            let got =
                decode(&bytes).unwrap_or_else(|e| panic!("decode({:?}): {e}", frame.frame_type()));
            assert_eq!(got.frame_type(), frame.frame_type(), "type mismatch");
            assert_eq!(
                got.stream_id,
                frame.stream_id,
                "stream_id mismatch for {:?}",
                frame.frame_type()
            );
        }
    }

    /// DATA frames base64-encode their payload on the wire, matching Go's
    /// default `[]byte` JSON encoding — not a raw JSON byte array.
    #[test]
    fn test_data_frame_wire_shape_uses_base64() {
        let frame = Frame::new_data(7, b"hello".to_vec());
        let bytes = encode(&frame).expect("encode");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("parse json");
        assert_eq!(value["data"], serde_json::json!("aGVsbG8="));
    }

    /// An empty DATA payload is omitted from the wire, matching Go's
    /// `omitempty` on `[]byte` (true for both nil and zero-length slices).
    #[test]
    fn test_empty_data_frame_omits_data_field() {
        let frame = Frame::new_data(7, Vec::new());
        let bytes = encode(&frame).expect("encode");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("parse json");
        assert!(value.get("data").is_none());
    }
}
