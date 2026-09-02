//! Wire protocol for the extend-proxy tunnel: 12 frame types, JSON codec,
//! and validation rules. Ported from Go's `pkg/protocol`. Zero networking or
//! async dependencies by design — see the crate-level doc comment on why
//! this module is kept separate from `session`/`client`/`forwarder`.

mod codec;
mod constants;
mod error;
mod frame;
mod serde_helpers;

pub use codec::{decode, encode};
pub use constants::{
    reason, MAX_FRAME_SIZE_BYTES, OUTBOUND_CHANNEL_CAPACITY, PROTOCOL_VERSION, SESSION_INIT_TIMEOUT,
};
pub use error::{DecodeError, EncodeError, ValidationError};
pub use frame::{
    CmdPayload, CmdResponsePayload, Frame, FrameBody, FrameType, NackPayload, OpenPayload,
    SessionAckPayload, SessionInitPayload,
};
