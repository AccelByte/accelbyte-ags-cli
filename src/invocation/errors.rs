//! Domain-specific error types for the invocation module.

use crate::errors::CliError;

pub use crate::invocation::resolve::ResolutionError;

/// Errors raised during API request construction and execution.
#[derive(thiserror::Error, Debug)]
pub enum ExecutorError {
    #[error("This operation requires confirmation. Use --yes to skip, or run interactively.")]
    ConfirmationRequired,
    #[allow(dead_code)]
    #[error("Output format '{0}' is not yet supported. Supported formats: json")]
    UnsupportedOutputFormat(String),
}

impl From<ExecutorError> for CliError {
    fn from(e: ExecutorError) -> Self {
        CliError::Usage {
            message: e.to_string(),
            metadata: None,
        }
    }
}

impl From<ResolutionError> for CliError {
    fn from(e: ResolutionError) -> Self {
        CliError::Usage {
            message: e.to_string(),
            metadata: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unsupported_scope_message_matches_spec() {
        let err = ResolutionError::UnsupportedScope {
            command: "ags iam users create".to_string(),
            requested: "public".to_string(),
            supported: vec!["admin".to_string()],
        };
        let rendered = err.to_string();
        assert_eq!(
            rendered,
            "api scope 'public' is not supported for 'ags iam users create'\n\
             Supported scopes: admin"
        );
    }

    #[test]
    fn test_unsupported_version_message_matches_spec() {
        let err = ResolutionError::UnsupportedVersion {
            command: "ags iam users get".to_string(),
            scope: "public".to_string(),
            requested: "v1".to_string(),
            supported: vec![
                crate::protocol::catalogue::ApiVersion(2),
                crate::protocol::catalogue::ApiVersion(3),
            ],
        };
        let rendered = err.to_string();
        assert_eq!(
            rendered,
            "api version 'v1' is not supported for 'ags iam users get' with --api-scope public\n\
             Supported public versions: v2, v3"
        );
    }

    #[test]
    fn test_missing_scope_message_lists_options() {
        let err = ResolutionError::MissingScope {
            command: "ags svc res get".to_string(),
            supported: vec!["public".to_string(), "server".to_string()],
        };
        let rendered = err.to_string();
        assert_eq!(
            rendered,
            "no default --api-scope for 'ags svc res get'\n\
             Supported scopes: public, server"
        );
    }

    #[test]
    fn test_cli_error_from_resolution_error_is_usage_variant() {
        let err = ResolutionError::UnsupportedScope {
            command: "ags x y z".to_string(),
            requested: "public".to_string(),
            supported: vec!["admin".to_string()],
        };
        let cli: CliError = err.into();
        assert!(matches!(cli, CliError::Usage { .. }));
    }
}
