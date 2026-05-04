//! Config-layer error helpers.

use crate::protocol::error::{RuntimeError, RuntimeErrorKind};

/// Build an internal config `RuntimeError` with the given message.
pub(crate) fn internal_error(message: impl Into<String>) -> RuntimeError {
    RuntimeError {
        kind: RuntimeErrorKind::Internal,
        message: message.into(),
        details: None,
        hint: None,
            trace: None,
    }
}
