//! Domain-specific error types for the auth module.

/// Storage backend resource being acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthResource {
    ClientSecret,
    Token,
}

/// Storage backend operation being attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageOperation {
    Read,
    Write,
    Delete,
}

/// Prefix string for a keychain operation failure (does not include the trailing ": {source}").
fn keychain_operation_message(resource: AuthResource, operation: StorageOperation) -> &'static str {
    match (resource, operation) {
        (AuthResource::ClientSecret, StorageOperation::Write) => {
            "Failed to store client secret in OS keychain"
        }
        (AuthResource::ClientSecret, StorageOperation::Read) => {
            "Failed to read client secret from OS keychain"
        }
        (AuthResource::ClientSecret, StorageOperation::Delete) => {
            "Failed to delete client secret from OS keychain"
        }
        (AuthResource::Token, StorageOperation::Write) => "Failed to store tokens in OS keychain",
        (AuthResource::Token, StorageOperation::Read) => "Failed to read tokens from OS keychain",
        (AuthResource::Token, StorageOperation::Delete) => {
            "Failed to delete tokens from OS keychain"
        }
    }
}

/// Prefix string for a file storage operation failure (does not include the trailing ": {source}").
fn file_operation_message(resource: AuthResource, operation: StorageOperation) -> &'static str {
    match (resource, operation) {
        (AuthResource::ClientSecret, StorageOperation::Write) => "Failed to write secret file",
        (AuthResource::ClientSecret, StorageOperation::Read) => "Failed to read secret file",
        (AuthResource::ClientSecret, StorageOperation::Delete) => "Failed to delete secret file",
        (AuthResource::Token, StorageOperation::Write) => "Failed to write token file",
        (AuthResource::Token, StorageOperation::Read) => "Failed to read token file",
        (AuthResource::Token, StorageOperation::Delete) => "Failed to delete token file",
    }
}

/// Full sentence for a missing-directory error.
fn dir_message(resource: AuthResource) -> &'static str {
    match resource {
        AuthResource::ClientSecret => "Cannot determine secret file directory.",
        AuthResource::Token => "Cannot determine token file directory.",
    }
}

/// Suggestion text shown when a keychain operation fails. Read/Write share the
/// fallback hint; Delete suggests retrying logout.
fn keychain_runtime_suggestion(operation: StorageOperation) -> &'static str {
    match operation {
        StorageOperation::Read | StorageOperation::Write => {
            "Check OS keychain access or set AGS_NO_KEYCHAIN=1 to use file storage."
        }
        StorageOperation::Delete => "Check OS keychain access and retry 'ags auth logout'.",
    }
}

/// Authentication and credential errors surfaced from OAuth flows, token storage, and session management.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    // --- Credential resolution ---
    #[error("Not authenticated.")]
    BaseUrlMissing,
    #[error("Invalid base URL: '{0}'.")]
    InvalidBaseUrl(String),
    #[error("Not authenticated.")]
    ClientIdMissing,
    #[error("Not authenticated.")]
    ClientSecretMissing,

    // --- Session expiry ---
    #[error("Session expired and token refresh failed.")]
    SessionExpiredRefreshFailed(String),
    #[error("Session expired — refresh token expired.")]
    SessionExpiredRefreshTokenExpired,
    #[error("Session expired — no refresh token.")]
    SessionExpiredNoRefreshToken,

    // --- Token requests ---
    #[error("Token request failed.")]
    TokenRequestFailed { status: u16, body: String },
    #[error("Token refresh failed.")]
    TokenRefreshFailed { status: u16, body: String },
    #[error("Token exchange failed.")]
    TokenExchangeFailed { status: u16, body: String },
    #[error("Failed to parse token response: {0}")]
    TokenParseFailed(String),

    // --- OAuth/PKCE callback ---
    #[error("Failed to accept callback connection: {0}")]
    CallbackAcceptFailed(String),
    #[error("Failed to read callback request: {0}")]
    CallbackReadFailed(String),
    #[error("Empty callback request.")]
    CallbackEmpty,
    #[error("Malformed callback request.")]
    CallbackMalformed,
    #[error("Callback URL missing query parameters.")]
    CallbackMissingQuery,
    #[error("Callback URL missing 'code' parameter.")]
    CallbackMissingCode,
    #[error("Callback URL missing 'state' parameter.")]
    CallbackMissingState,
    #[error("Authorization failed: {error}.")]
    AuthorizationFailed { error: String, description: String },
    #[error("Callback server error: {0}")]
    CallbackServerError(String),
    #[error("State mismatch in OAuth callback.")]
    OAuthStateMismatch,
    #[error("Port {0} is already in use.")]
    PortInUse(u16),

    // --- OAuth callback timeout ---
    #[error("Authentication error.")]
    CallbackTimeout(u64), // timeout_secs

    // --- Storage matrix ---
    #[error("{}: {source}", keychain_operation_message(*resource, *operation))]
    KeychainOperationFailed {
        resource: AuthResource,
        operation: StorageOperation,
        #[source]
        source: keyring::Error,
    },
    #[error("Stored credentials in the OS keychain are corrupted.")]
    KeychainTokenParseFailed(#[source] serde_json::Error),
    #[error("{}: {source}", file_operation_message(*resource, *operation))]
    FileOperationFailed {
        resource: AuthResource,
        operation: StorageOperation,
        #[source]
        source: std::io::Error,
    },
    #[error("Failed to serialize token data: {0}")]
    TokenSerializeFailed(#[source] serde_json::Error),
    #[error("Failed to parse token file: {0}")]
    TokenFileParseFailed(#[source] serde_json::Error),
    #[error("{}", dir_message(*_0))]
    CannotDetermineDir(AuthResource),
}

/// Build a `NotAuthenticated` `RuntimeError` from labelled parts, packing reason/detail/tip into structured `ErrorDetails`.
fn build_auth_runtime_error(
    message: String,
    reason: Option<String>,
    suggestion: Option<String>,
    detail: Option<String>,
    tip: Option<String>,
) -> crate::protocol::error::RuntimeError {
    use crate::protocol::error::{ErrorDetails, RuntimeError, RuntimeErrorKind};

    let has_details = reason.is_some() || detail.is_some() || tip.is_some();
    RuntimeError {
        kind: RuntimeErrorKind::NotAuthenticated,
        message,
        details: if has_details {
            Some(Box::new(ErrorDetails {
                code: None,
                reason,
                detail,
                suggestion_kind: suggestion
                    .as_ref()
                    .map(|_| crate::protocol::error::SuggestionKind::Fix),
                tip,
            }))
        } else {
            None
        },
        hint: suggestion,
        trace: None,
    }
}

impl From<AuthError> for crate::protocol::error::RuntimeError {
    fn from(e: AuthError) -> Self {
        // The user-facing `message` is the variant's Display string (defined
        // by the `#[error(...)]` attributes on `AuthError`). The match below
        // only contributes the structured `details` (reason / suggestion /
        // detail / tip) — the message is computed once here.
        let message = e.to_string();
        match e {
            AuthError::BaseUrlMissing => build_auth_runtime_error(
                message,
                Some("No base_url configured.".into()),
                Some("Run 'ags auth login' or set AGS_BASE_URL.".into()),
                None,
                None,
            ),
            AuthError::InvalidBaseUrl(_) => build_auth_runtime_error(
                message,
                Some("Base URL must be an absolute http(s) URL.".into()),
                Some("Provide a URL like https://development.accelbyte.io.".into()),
                None,
                None,
            ),
            AuthError::ClientIdMissing => build_auth_runtime_error(
                message,
                Some("No client_id configured.".into()),
                Some("Run 'ags auth login' or set AGS_CLIENT_ID.".into()),
                None,
                None,
            ),
            AuthError::ClientSecretMissing => build_auth_runtime_error(
                message,
                Some("No client_secret configured.".into()),
                Some("Run 'ags auth login' or set AGS_CLIENT_SECRET.".into()),
                None,
                None,
            ),
            AuthError::SessionExpiredRefreshFailed(reason) => build_auth_runtime_error(
                message,
                Some(reason),
                Some("Run 'ags auth login' to re-authenticate.".into()),
                None,
                None,
            ),
            AuthError::SessionExpiredRefreshTokenExpired
            | AuthError::SessionExpiredNoRefreshToken => build_auth_runtime_error(
                message,
                None,
                Some("Run 'ags auth login' to re-authenticate.".into()),
                None,
                None,
            ),

            AuthError::TokenRequestFailed { status, body } => build_auth_runtime_error(
                message,
                Some(body),
                None,
                Some(format!("HTTP {status}")),
                None,
            ),
            AuthError::TokenRefreshFailed { status, body } => build_auth_runtime_error(
                message,
                Some(body),
                Some("Run 'ags auth login' to re-authenticate.".into()),
                Some(format!("HTTP {status}")),
                None,
            ),
            AuthError::TokenExchangeFailed { status, body } => build_auth_runtime_error(
                message,
                Some(body),
                None,
                Some(format!("HTTP {status}")),
                None,
            ),
            AuthError::TokenParseFailed(_)
            | AuthError::CallbackAcceptFailed(_)
            | AuthError::CallbackReadFailed(_)
            | AuthError::CallbackEmpty
            | AuthError::CallbackMalformed
            | AuthError::CallbackMissingQuery
            | AuthError::CallbackMissingCode
            | AuthError::CallbackMissingState
            | AuthError::CallbackServerError(_) => {
                build_auth_runtime_error(message, None, None, None, None)
            }
            AuthError::AuthorizationFailed { description, .. } => {
                build_auth_runtime_error(message, Some(description), None, None, None)
            }
            AuthError::OAuthStateMismatch => build_auth_runtime_error(
                message,
                Some("Possible CSRF attack.".into()),
                Some("Try again.".into()),
                None,
                None,
            ),
            AuthError::PortInUse(_) => build_auth_runtime_error(
                message,
                None,
                Some("Use --port to specify a different port, and update the redirect URI in the AccelByte Admin Portal to match.".into()),
                None,
                None,
            ),
            AuthError::CallbackTimeout(timeout_secs) => build_auth_runtime_error(
                message,
                Some(format!(
                    "Timed out waiting for browser callback ({timeout_secs}s)."
                )),
                Some("Run 'ags auth login' again.".into()),
                None,
                Some(
                    "Set AGS_AUTH_TIMEOUT to change the timeout duration (default: 120s).".into(),
                ),
            ),

            AuthError::KeychainOperationFailed { operation, .. } => build_auth_runtime_error(
                message,
                None,
                Some(keychain_runtime_suggestion(operation).to_string()),
                None,
                None,
            ),
            AuthError::KeychainTokenParseFailed(source) => build_auth_runtime_error(
                message,
                Some(format!("Token data could not be parsed: {source}")),
                Some("Run 'ags auth logout' and 'ags auth login' to restore valid credentials.".into()),
                None,
                None,
            ),
            AuthError::FileOperationFailed { .. }
            | AuthError::TokenSerializeFailed(_)
            | AuthError::TokenFileParseFailed(_)
            | AuthError::CannotDetermineDir(_) => {
                build_auth_runtime_error(message, None, None, None, None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AuthError now implements Display via thiserror. Spot-check that
    /// representative variants render the same message strings the
    /// `From<AuthError> for RuntimeError` impl uses for the message field.
    #[test]
    fn test_display_matches_runtime_error_message_for_representative_variants() {
        assert_eq!(AuthError::BaseUrlMissing.to_string(), "Not authenticated.");
        assert_eq!(
            AuthError::PortInUse(8080).to_string(),
            "Port 8080 is already in use."
        );
        assert_eq!(
            AuthError::InvalidBaseUrl("not-a-url".into()).to_string(),
            "Invalid base URL: 'not-a-url'.",
        );
        assert_eq!(
            AuthError::TokenParseFailed("expected token".into()).to_string(),
            "Failed to parse token response: expected token",
        );
    }

    #[test]
    fn test_keychain_operation_message_matrix() {
        use AuthResource::*;
        use StorageOperation::*;
        assert_eq!(
            keychain_operation_message(ClientSecret, Write),
            "Failed to store client secret in OS keychain"
        );
        assert_eq!(
            keychain_operation_message(ClientSecret, Read),
            "Failed to read client secret from OS keychain"
        );
        assert_eq!(
            keychain_operation_message(ClientSecret, Delete),
            "Failed to delete client secret from OS keychain"
        );
        assert_eq!(
            keychain_operation_message(Token, Write),
            "Failed to store tokens in OS keychain"
        );
        assert_eq!(
            keychain_operation_message(Token, Read),
            "Failed to read tokens from OS keychain"
        );
        assert_eq!(
            keychain_operation_message(Token, Delete),
            "Failed to delete tokens from OS keychain"
        );
    }

    #[test]
    fn test_file_operation_message_matrix() {
        use AuthResource::*;
        use StorageOperation::*;
        assert_eq!(
            file_operation_message(ClientSecret, Write),
            "Failed to write secret file"
        );
        assert_eq!(
            file_operation_message(ClientSecret, Read),
            "Failed to read secret file"
        );
        assert_eq!(
            file_operation_message(ClientSecret, Delete),
            "Failed to delete secret file"
        );
        assert_eq!(
            file_operation_message(Token, Write),
            "Failed to write token file"
        );
        assert_eq!(
            file_operation_message(Token, Read),
            "Failed to read token file"
        );
        assert_eq!(
            file_operation_message(Token, Delete),
            "Failed to delete token file"
        );
    }

    #[test]
    fn test_dir_message_matrix() {
        assert_eq!(
            dir_message(AuthResource::ClientSecret),
            "Cannot determine secret file directory."
        );
        assert_eq!(
            dir_message(AuthResource::Token),
            "Cannot determine token file directory."
        );
    }

    #[test]
    fn test_keychain_runtime_suggestion_matrix() {
        use StorageOperation::*;
        assert_eq!(
            keychain_runtime_suggestion(Read),
            "Check OS keychain access or set AGS_NO_KEYCHAIN=1 to use file storage."
        );
        assert_eq!(
            keychain_runtime_suggestion(Write),
            "Check OS keychain access or set AGS_NO_KEYCHAIN=1 to use file storage."
        );
        assert_eq!(
            keychain_runtime_suggestion(Delete),
            "Check OS keychain access and retry 'ags auth logout'."
        );
    }

    #[test]
    fn test_keychain_operation_failed_runtime_message_matrix() {
        use AuthResource::*;
        use StorageOperation::*;
        let cases = [
            (
                ClientSecret,
                Write,
                "Failed to store client secret in OS keychain",
            ),
            (
                ClientSecret,
                Read,
                "Failed to read client secret from OS keychain",
            ),
            (
                ClientSecret,
                Delete,
                "Failed to delete client secret from OS keychain",
            ),
            (Token, Write, "Failed to store tokens in OS keychain"),
            (Token, Read, "Failed to read tokens from OS keychain"),
            (Token, Delete, "Failed to delete tokens from OS keychain"),
        ];
        for (resource, operation, prefix) in cases {
            let auth_err = AuthError::KeychainOperationFailed {
                resource,
                operation,
                source: keyring::Error::NoEntry,
            };
            let runtime_err = crate::protocol::error::RuntimeError::from(auth_err);
            let expected = format!("{prefix}: {}", keyring::Error::NoEntry);
            assert_eq!(
                runtime_err.message, expected,
                "({resource:?}, {operation:?}) message mismatch"
            );
        }
    }

    #[test]
    fn test_file_operation_failed_runtime_message_matrix() {
        use AuthResource::*;
        use StorageOperation::*;
        let io_err = || std::io::Error::other("boom");
        let cases = [
            (ClientSecret, Write, "Failed to write secret file"),
            (ClientSecret, Read, "Failed to read secret file"),
            (ClientSecret, Delete, "Failed to delete secret file"),
            (Token, Write, "Failed to write token file"),
            (Token, Read, "Failed to read token file"),
            (Token, Delete, "Failed to delete token file"),
        ];
        for (resource, operation, prefix) in cases {
            let auth_err = AuthError::FileOperationFailed {
                resource,
                operation,
                source: io_err(),
            };
            let runtime_err = crate::protocol::error::RuntimeError::from(auth_err);
            assert_eq!(
                runtime_err.message,
                format!("{prefix}: boom"),
                "({resource:?}, {operation:?}) message mismatch"
            );
        }
    }

    #[test]
    fn test_cannot_determine_dir_runtime_messages() {
        let secret_err = crate::protocol::error::RuntimeError::from(AuthError::CannotDetermineDir(
            AuthResource::ClientSecret,
        ));
        assert_eq!(
            secret_err.message,
            "Cannot determine secret file directory."
        );

        let token_err = crate::protocol::error::RuntimeError::from(AuthError::CannotDetermineDir(
            AuthResource::Token,
        ));
        assert_eq!(token_err.message, "Cannot determine token file directory.");
    }

    #[test]
    fn test_keychain_token_parse_failed_runtime_message() {
        let serde_err = serde_json::from_str::<u32>("not a number").unwrap_err();
        let auth_err = AuthError::KeychainTokenParseFailed(serde_err);
        let runtime_err = crate::protocol::error::RuntimeError::from(auth_err);
        assert_eq!(
            runtime_err.message,
            "Stored credentials in the OS keychain are corrupted."
        );
    }
}
