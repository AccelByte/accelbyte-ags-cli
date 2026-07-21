//! Dispatch runtime for executing API operations and classifying responses.

pub mod classify;
mod confirmation;
mod error_codes;
mod execute;
pub mod http;
mod pagination;
mod path;
pub mod shape;

pub(crate) use confirmation::requires_confirmation;
pub(crate) use execute::fetch_raw_body;
pub(crate) use execute::{execute_operation, ApiCallContext};
pub(crate) use path::substitute_path_params;

use ags_protocol::error::{RuntimeError, RuntimeErrorKind};

/// Canonical error for a command or workflow step that uploads a file
/// (`multipart/form-data`), which the CLI cannot construct yet. Shared by the
/// service route, the workflow route, and the dispatch-layer guard so the
/// message and hint stay in sync.
pub fn file_upload_not_supported_error() -> RuntimeError {
    RuntimeError {
        kind: RuntimeErrorKind::Validation,
        message:
            "This command uploads a file (multipart/form-data), which the CLI does not yet support"
                .to_string(),
        details: None,
        hint: Some("Upload the file through the AccelByte Admin Portal.".to_string()),
        trace: None,
    }
}
