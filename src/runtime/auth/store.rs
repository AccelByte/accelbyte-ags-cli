//! Credential storage: OS keychain for secrets, file fallback for tokens.

use keyring::Error as KeyringError;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::protocol::error::{RuntimeError, RuntimeErrorKind};
use crate::runtime::config;

use super::errors::{AuthError, AuthResource, StorageOperation};
use super::locking;

const KEYRING_USER: &str = "client-secret";
const KEYRING_TOKEN_USER: &str = "token-data";

/// Conservative UTF-8 byte-length proxy for the Windows Credential Manager
/// password limit (~2560 UTF-16 chars). For every valid Unicode codepoint,
/// UTF-8 byte count ≥ UTF-16 code unit count, so checking byte length is
/// always at least as strict as the OS limit — we may occasionally route a
/// multi-byte UTF-8 payload to file storage even though Windows would have
/// accepted it, but we never permit a token that would be truncated.
const WINDOWS_CREDMAN_PASSWORD_LIMIT_BYTES: usize = 2560;

/// Return whether `json` is too large to safely store in Windows Credential
/// Manager. See `WINDOWS_CREDMAN_PASSWORD_LIMIT_BYTES` for the byte-vs-UTF-16
/// rationale. Always returns false on non-Windows hosts, since other keychain
/// backends do not have this limit.
fn exceeds_windows_credman_limit(json: &str) -> bool {
    cfg!(windows) && json.len() > WINDOWS_CREDMAN_PASSWORD_LIMIT_BYTES
}

/// Build the keychain service name scoped to a profile
fn keyring_service(profile: &str) -> String {
    format!("ags:accelbyte.io:{profile}")
}

/// Only fall back to file storage when the platform keychain is unavailable.
fn should_fallback_to_file(error: &KeyringError) -> bool {
    matches!(error, KeyringError::NoStorageAccess(_))
}

/// Create a keyring entry scoped to a profile.
fn keychain_entry(profile: &str, user: &str) -> Result<keyring::Entry, RuntimeError> {
    let service = keyring_service(profile);
    keyring::Entry::new(&service, user).map_err(|error| RuntimeError {
        kind: RuntimeErrorKind::Internal,
        message: format!(
            "Failed to initialize OS keychain entry for '{user}' (profile '{profile}'): {error}"
        ),
        details: None,
        hint: None,
        trace: None,
    })
}

/// Stored token data including refresh token.
#[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct TokenData {
    pub access_token: String,
    pub expires_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_expires_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_type: Option<crate::protocol::request::GrantType>,
}

// ── Client secret storage (keychain first, file fallback) ──

/// Build the filesystem path for the file-based secret fallback.
fn secret_file_path(profile: &str) -> Result<std::path::PathBuf, RuntimeError> {
    Ok(config::profile_dir(profile)?.join("credentials.json"))
}

/// Store a client secret. Tries keychain first, falls back to file.
pub fn store_secret(profile: &str, secret: &str) -> Result<(), RuntimeError> {
    if !config::is_keychain_disabled() {
        let entry = keychain_entry(profile, KEYRING_USER)?;
        match entry.set_password(secret) {
            Ok(()) => return Ok(()),
            Err(error) if should_fallback_to_file(&error) => {}
            Err(error) => {
                return Err(RuntimeError::from(AuthError::KeychainOperationFailed {
                    resource: AuthResource::ClientSecret,
                    operation: StorageOperation::Write,
                    source: error,
                }));
            }
        }
    }

    // Fall back to file
    let path = secret_file_path(profile)?;
    crate::support::file_system::create_dir_restricted(path.parent().ok_or_else(|| {
        RuntimeError::from(AuthError::CannotDetermineDir(AuthResource::ClientSecret))
    })?)
    .map_err(|source| {
        RuntimeError::from(AuthError::FileOperationFailed {
            resource: AuthResource::ClientSecret,
            operation: StorageOperation::Write,
            source,
        })
    })?;
    crate::support::file_system::write_file_restricted(&path, secret).map_err(|source| {
        RuntimeError::from(AuthError::FileOperationFailed {
            resource: AuthResource::ClientSecret,
            operation: StorageOperation::Write,
            source,
        })
    })?;
    Ok(())
}

/// Retrieve client secret. Tries keychain first, falls back to file.
/// Returns `Ok(None)` when no secret is stored (not an error), `Err` for real failures.
///
/// The returned `Zeroizing<String>` ensures the secret buffer is wiped from
/// heap memory when the value is dropped, both for keychain reads and the
/// file fallback path (where the secret would otherwise sit in a plain
/// `String` allocation visible in core dumps or swap).
pub fn get_secret(profile: &str) -> Result<Option<Zeroizing<String>>, RuntimeError> {
    if !config::is_keychain_disabled() {
        let entry = keychain_entry(profile, KEYRING_USER)?;
        match entry.get_password() {
            Ok(secret) => return Ok(Some(Zeroizing::new(secret))),
            Err(KeyringError::NoEntry) => {}
            Err(error) if should_fallback_to_file(&error) => {}
            Err(error) => {
                return Err(RuntimeError::from(AuthError::KeychainOperationFailed {
                    resource: AuthResource::ClientSecret,
                    operation: StorageOperation::Read,
                    source: error,
                }));
            }
        }
    }

    // Fall back to file
    let path = secret_file_path(profile)?;
    if path.exists() {
        return std::fs::read_to_string(&path)
            .map(|s| Some(Zeroizing::new(s)))
            .map_err(|source| {
                RuntimeError::from(AuthError::FileOperationFailed {
                    resource: AuthResource::ClientSecret,
                    operation: StorageOperation::Read,
                    source,
                })
            });
    }

    Ok(None)
}

/// Delete client secret from keychain and file.
pub fn delete_secret(profile: &str) -> Result<(), RuntimeError> {
    if !config::is_keychain_disabled() {
        let entry = keychain_entry(profile, KEYRING_USER)?;
        match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => {}
            Err(error) if should_fallback_to_file(&error) => {}
            Err(error) => {
                return Err(RuntimeError::from(AuthError::KeychainOperationFailed {
                    resource: AuthResource::ClientSecret,
                    operation: StorageOperation::Delete,
                    source: error,
                }));
            }
        }
    }
    let path = secret_file_path(profile)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|source| {
            RuntimeError::from(AuthError::FileOperationFailed {
                resource: AuthResource::ClientSecret,
                operation: StorageOperation::Delete,
                source,
            })
        })?;
    }
    Ok(())
}

// ── Token data storage (keychain first, file fallback) ──

/// Get the token file path scoped to a profile.
fn token_file_path(profile: &str) -> Result<std::path::PathBuf, RuntimeError> {
    Ok(config::profile_dir(profile)?.join("token.json"))
}

/// Result of attempting to persist token data, with an optional warning the
/// caller can surface to the user (e.g. when the keychain rejected the write
/// and we fell back to file storage).
#[derive(Debug, Default, PartialEq)]
pub struct StoreOutcome {
    pub warning: Option<String>,
}

/// Store token data. Tries keychain first, falls back to file with a warning on failure.
///
/// Token blobs can exceed platform keychain limits (notably Windows Credential Manager's
/// 2560-char limit for UTF-16 encoded passwords). Rather than failing login, any keychain
/// write failure falls back to file storage with a visible warning.
pub fn store_token_data(
    profile: &str,
    token_data: &TokenData,
) -> Result<StoreOutcome, RuntimeError> {
    locking::with_token_lock(profile, || store_token_data_unlocked(profile, token_data))
}

/// Store token data assuming the caller already holds the matching token lock.
pub(crate) fn store_token_data_unlocked(
    profile: &str,
    token_data: &TokenData,
) -> Result<StoreOutcome, RuntimeError> {
    let json = serde_json::to_string(token_data)
        .map_err(|e| RuntimeError::from(AuthError::TokenSerializeFailed(e)))?;

    let mut warning: Option<String> = None;

    // On Windows, Credential Manager silently truncates passwords above
    // ~2560 chars. Skip the keychain attempt entirely for over-limit
    // tokens and route to file fallback with an explanatory warning.
    // Best-effort delete any pre-existing keychain entry so a later
    // get_token_data does not return a stale value (e.g. from a token
    // stored before this guard existed).
    if exceeds_windows_credman_limit(&json) {
        if !config::is_keychain_disabled() {
            if let Ok(entry) = keychain_entry(profile, KEYRING_TOKEN_USER) {
                let _ = entry.delete_credential();
            }
        }
        warning = Some(format!(
            "Token exceeds the Windows Credential Manager limit ({WINDOWS_CREDMAN_PASSWORD_LIMIT_BYTES} bytes); \
             storing in file fallback.\n\
             Set AGS_NO_KEYCHAIN=1 to skip keychain and suppress this warning."
        ));
    } else if !config::is_keychain_disabled() {
        let entry = keychain_entry(profile, KEYRING_TOKEN_USER)?;
        match entry.set_password(&json) {
            Ok(()) => return Ok(StoreOutcome { warning: None }),
            Err(error) if should_fallback_to_file(&error) => {}
            Err(error) => {
                warning = Some(format!(
                    "OS keychain write failed, falling back to file storage: {error}\n\
                     Set AGS_NO_KEYCHAIN=1 to skip keychain and suppress this warning."
                ));
            }
        }
    }

    // Fall back to file
    let path = token_file_path(profile)?;
    crate::support::file_system::create_dir_restricted(
        path.parent().ok_or_else(|| {
            RuntimeError::from(AuthError::CannotDetermineDir(AuthResource::Token))
        })?,
    )
    .map_err(|source| {
        RuntimeError::from(AuthError::FileOperationFailed {
            resource: AuthResource::Token,
            operation: StorageOperation::Write,
            source,
        })
    })?;
    crate::support::file_system::write_file_restricted(&path, &json).map_err(|source| {
        RuntimeError::from(AuthError::FileOperationFailed {
            resource: AuthResource::Token,
            operation: StorageOperation::Write,
            source,
        })
    })?;

    Ok(StoreOutcome { warning })
}

/// Retrieve token data. Tries keychain first, falls back to file.
pub fn get_token_data(profile: &str) -> Result<Option<TokenData>, RuntimeError> {
    // Try keychain first
    if !config::is_keychain_disabled() {
        let entry = keychain_entry(profile, KEYRING_TOKEN_USER)?;
        match entry.get_password() {
            Ok(json) => {
                let token_data = serde_json::from_str::<TokenData>(&json)
                    .map_err(|e| RuntimeError::from(AuthError::KeychainTokenParseFailed(e)))?;
                return Ok(Some(token_data));
            }
            Err(KeyringError::NoEntry) => {}
            Err(error) if should_fallback_to_file(&error) => {}
            Err(error) => {
                return Err(RuntimeError::from(AuthError::KeychainOperationFailed {
                    resource: AuthResource::Token,
                    operation: StorageOperation::Read,
                    source: error,
                }));
            }
        }
    }

    // Fall back to file
    let path = token_file_path(profile)?;
    if !path.exists() {
        return Ok(None);
    }
    let json = std::fs::read_to_string(&path).map_err(|source| {
        RuntimeError::from(AuthError::FileOperationFailed {
            resource: AuthResource::Token,
            operation: StorageOperation::Read,
            source,
        })
    })?;
    let token_data: TokenData = serde_json::from_str(&json)
        .map_err(|e| RuntimeError::from(AuthError::TokenFileParseFailed(e)))?;
    Ok(Some(token_data))
}

/// Delete token data from both keychain and file.
pub fn delete_token_data(profile: &str) -> Result<(), RuntimeError> {
    locking::with_token_lock(profile, || delete_token_data_unlocked(profile))
}

/// Delete token data assuming the caller already holds the matching token lock.
pub(crate) fn delete_token_data_unlocked(profile: &str) -> Result<(), RuntimeError> {
    // Try keychain
    if !config::is_keychain_disabled() {
        let entry = keychain_entry(profile, KEYRING_TOKEN_USER)?;
        match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => {}
            Err(error) if should_fallback_to_file(&error) => {}
            Err(error) => {
                return Err(RuntimeError::from(AuthError::KeychainOperationFailed {
                    resource: AuthResource::Token,
                    operation: StorageOperation::Delete,
                    source: error,
                }));
            }
        }
    }

    // Delete file
    let path = token_file_path(profile)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|source| {
            RuntimeError::from(AuthError::FileOperationFailed {
                resource: AuthResource::Token,
                operation: StorageOperation::Delete,
                source,
            })
        })?;
    }

    Ok(())
}

// ── Async wrappers ──

/// Async-friendly wrapper for [`get_token_data`] — runs the blocking
/// keychain/file read on a Tokio blocking thread so async callers don't
/// park runtime workers.
pub async fn get_token_data_async(profile: &str) -> Result<Option<TokenData>, RuntimeError> {
    let profile = profile.to_string();
    tokio::task::spawn_blocking(move || get_token_data(&profile))
        .await
        .map_err(|e| {
            crate::runtime::config::internal_error(format!("Failed to join token-read task: {e}"))
        })?
}

/// Async-friendly wrapper for [`store_token_data`].
pub async fn store_token_data_async(
    profile: &str,
    token_data: TokenData,
) -> Result<StoreOutcome, RuntimeError> {
    let profile = profile.to_string();
    tokio::task::spawn_blocking(move || store_token_data(&profile, &token_data))
        .await
        .map_err(|e| {
            crate::runtime::config::internal_error(format!("Failed to join token-write task: {e}"))
        })?
}

/// Async-friendly wrapper for [`store_token_data_unlocked`].
pub(crate) async fn store_token_data_unlocked_async(
    profile: &str,
    token_data: TokenData,
) -> Result<StoreOutcome, RuntimeError> {
    let profile = profile.to_string();
    tokio::task::spawn_blocking(move || store_token_data_unlocked(&profile, &token_data))
        .await
        .map_err(|e| {
            crate::runtime::config::internal_error(format!("Failed to join token-write task: {e}"))
        })?
}

/// Async-friendly wrapper for [`delete_token_data`].
pub async fn delete_token_data_async(profile: &str) -> Result<(), RuntimeError> {
    let profile = profile.to_string();
    tokio::task::spawn_blocking(move || delete_token_data(&profile))
        .await
        .map_err(|e| {
            crate::runtime::config::internal_error(format!("Failed to join token-delete task: {e}"))
        })?
}

/// Async-friendly wrapper for [`get_secret`].
pub async fn get_secret_async(
    profile: &str,
) -> Result<Option<zeroize::Zeroizing<String>>, RuntimeError> {
    let profile = profile.to_string();
    tokio::task::spawn_blocking(move || get_secret(&profile))
        .await
        .map_err(|e| {
            crate::runtime::config::internal_error(format!("Failed to join secret-read task: {e}"))
        })?
}

/// Async-friendly wrapper for [`store_secret`].
pub async fn store_secret_async(profile: &str, secret: String) -> Result<(), RuntimeError> {
    let profile = profile.to_string();
    tokio::task::spawn_blocking(move || store_secret(&profile, &secret))
        .await
        .map_err(|e| {
            crate::runtime::config::internal_error(format!("Failed to join secret-write task: {e}"))
        })?
}

/// Async-friendly wrapper for [`delete_secret`].
pub async fn delete_secret_async(profile: &str) -> Result<(), RuntimeError> {
    let profile = profile.to_string();
    tokio::task::spawn_blocking(move || delete_secret(&profile))
        .await
        .map_err(|e| {
            crate::runtime::config::internal_error(format!(
                "Failed to join secret-delete task: {e}"
            ))
        })?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// File fallback is only allowed when the platform keychain is unavailable.
    #[test]
    fn test_should_fallback_to_file_only_for_no_storage_access() {
        let unavailable = KeyringError::NoStorageAccess(Box::new(std::io::Error::other("locked")));
        let other_failure = KeyringError::PlatformFailure(Box::new(std::io::Error::other("boom")));

        assert!(should_fallback_to_file(&unavailable));
        assert!(!should_fallback_to_file(&other_failure));
        assert!(!should_fallback_to_file(&KeyringError::NoEntry));
    }

    /// Different profiles produce different keychain service names.
    #[test]
    fn test_keyring_service_includes_profile() {
        assert_eq!(keyring_service("default"), "ags:accelbyte.io:default");
        assert_eq!(keyring_service("staging"), "ags:accelbyte.io:staging");
    }

    /// File-fallback storage creates the profile directory with 0700 permissions to keep tokens private
    #[cfg(unix)]
    #[test]
    #[serial_test::serial]
    fn test_store_token_data_file_fallback_creates_profile_dir_with_0700() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, tmp.path());
        std::env::set_var(crate::runtime::config::ENV_NO_KEYCHAIN, "1");

        let token = TokenData {
            access_token: "tok".to_string(),
            expires_at: 9_999_999_999,
            refresh_token: None,
            refresh_expires_at: None,
            grant_type: None,
        };
        store_token_data("default", &token).unwrap();

        let profile_dir = tmp.path().join("profiles").join("default");
        let mode = std::fs::metadata(&profile_dir)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700, "profile dir should be 0700");

        std::env::remove_var(crate::runtime::config::ENV_HOME);
        std::env::remove_var(crate::runtime::config::ENV_NO_KEYCHAIN);
    }

    /// Tokens shorter than the Windows Credential Manager byte limit are accepted on every platform
    #[test]
    fn test_exceeds_windows_credman_limit_returns_false_below_threshold() {
        let small = "a".repeat(100);
        assert!(!super::exceeds_windows_credman_limit(&small));
    }

    /// On Windows, oversized tokens trip the limit so callers fall back to the file store
    #[test]
    #[cfg(windows)]
    fn test_exceeds_windows_credman_limit_returns_true_above_threshold_on_windows() {
        let large = "a".repeat(3000);
        assert!(super::exceeds_windows_credman_limit(&large));
    }

    /// On non-Windows platforms, the credman size check is a no-op regardless of token length
    #[test]
    #[cfg(not(windows))]
    fn test_exceeds_windows_credman_limit_returns_false_above_threshold_on_unix() {
        let large = "a".repeat(3000);
        assert!(!super::exceeds_windows_credman_limit(&large));
    }
}
