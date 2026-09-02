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
