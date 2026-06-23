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

/// Windows Credential Manager's maximum credential-blob size, in bytes
/// (`CRED_MAX_CREDENTIAL_BLOB_SIZE` = 2560). The `keyring` crate's
/// `windows-native` backend stores the password as a UTF-16 blob and rejects
/// (`Error::TooLong`) any value whose UTF-16 byte length exceeds this — see
/// keyring's `windows.rs`: `password.encode_utf16().count() * 2 > LIMIT`.
const WINDOWS_CREDMAN_PASSWORD_LIMIT_BYTES: usize = 2560;

/// Byte length of `s` once UTF-16 encoded — the size Windows Credential Manager
/// actually measures the blob against (2 bytes per BMP code unit). This matches
/// keyring's own length check. Using UTF-8 `str::len()` here would be 2× too
/// lenient for ASCII: a token JSON between ~1281 and 2560 chars would pass the
/// guard, reach the keychain, fail the write, and silently fall back to file
/// storage — leaving a stale keychain entry that shadows the fresh token.
fn credman_blob_bytes(s: &str) -> usize {
    s.encode_utf16().count() * 2
}

/// Return whether `json` is too large to safely store in Windows Credential
/// Manager (so the caller must route it to file storage). Always false on
/// non-Windows hosts, whose keychain backends have no such limit.
fn exceeds_windows_credman_limit(json: &str) -> bool {
    cfg!(windows) && credman_blob_bytes(json) > WINDOWS_CREDMAN_PASSWORD_LIMIT_BYTES
}

/// Build the keychain service name scoped to a profile
fn keyring_service(profile: &str) -> String {
    format!("ags:accelbyte.io:{profile}")
}

/// Returns `true` when an **operation** on an already-initialized keychain entry
/// (`get_password` / `set_password` / `delete_credential`) failed because the
/// backing store became inaccessible (`NoStorageAccess`) — e.g. the keychain was
/// locked, or the secret-service daemon stopped, after `Entry::new` had already
/// succeeded.
///
/// Kept deliberately narrow: every inner operation `match` arm uses this as a
/// race-condition safety net so a backend that disappears mid-operation falls
/// through to file storage, while genuine read/write failures on a working
/// keychain (`PlatformFailure`, `BadEncoding`, …) still surface as errors.
///
/// For the broader *init-time* test, see [`is_keychain_init_unavailable`].
fn is_keychain_unavailable(error: &KeyringError) -> bool {
    matches!(error, KeyringError::NoStorageAccess(_))
}

/// Returns `true` when an error from `keyring::Entry::new` means this platform
/// has no usable keychain backend, so the caller should fall back to file
/// storage instead of failing.
///
/// Broader than [`is_keychain_unavailable`]: at init time we never obtained a
/// handle to operate on, so any *environmental* failure should route to file
/// storage rather than block the command —
/// - `NoStorageAccess` — no secret-service / D-Bus daemon (e.g. WSL2).
/// - `PlatformFailure` — the platform store exists but cannot be initialized,
///   e.g. the Linux keyutils syscall returning `ENOSYS` inside a container
///   (surfaces as `Platform secure storage failure: Unknown(38)`), or a
///   seccomp-blocked syscall surfacing as `Unknown(1)` (EPERM).
///
/// Other variants (`Invalid`, `TooLong`, …) indicate a malformed entry — a
/// programming error rather than a missing backend — and are still surfaced.
///
/// `pub(crate)` so [`crate::runtime::diagnostics::checks`] can classify the
/// keychain-access probe with the same rule.
pub(crate) fn is_keychain_init_unavailable(error: &KeyringError) -> bool {
    matches!(
        error,
        KeyringError::NoStorageAccess(_) | KeyringError::PlatformFailure(_)
    )
}

/// Create a keyring entry, or return `None` when the platform has no usable
/// keychain backend (see [`is_keychain_init_unavailable`]). Any other
/// initialization failure (a malformed service/user) is an `Err`.
///
/// Preferred for operations with a file fallback: a missing backend at init time
/// routes straight to file storage, exactly as [`is_keychain_unavailable`]
/// routes a `NoStorageAccess` error from a later `get_password` / `set_password`.
fn keychain_entry_opt(profile: &str, user: &str) -> Result<Option<keyring::Entry>, RuntimeError> {
    let service = keyring_service(profile);
    match keyring::Entry::new(&service, user) {
        Ok(entry) => Ok(Some(entry)),
        Err(error) if is_keychain_init_unavailable(&error) => Ok(None),
        Err(error) => Err(RuntimeError {
            kind: RuntimeErrorKind::Internal,
            message: format!(
                "Failed to initialize OS keychain entry for '{user}' (profile '{profile}'): {error}"
            ),
            details: None,
            hint: None,
            trace: None,
        }),
    }
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
        if let Some(entry) = keychain_entry_opt(profile, KEYRING_USER)? {
            match entry.set_password(secret) {
                Ok(()) => return Ok(()),
                // Race: backend disappeared between Entry::new and this call.
                Err(error) if is_keychain_unavailable(&error) => {}
                Err(error) => {
                    return Err(RuntimeError::from(AuthError::KeychainOperationFailed {
                        resource: AuthResource::ClientSecret,
                        operation: StorageOperation::Write,
                        source: error,
                    }));
                }
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
        if let Some(entry) = keychain_entry_opt(profile, KEYRING_USER)? {
            match entry.get_password() {
                Ok(secret) => return Ok(Some(Zeroizing::new(secret))),
                Err(KeyringError::NoEntry) => {}
                // Race: backend disappeared between Entry::new and this call.
                Err(error) if is_keychain_unavailable(&error) => {}
                Err(error) => {
                    return Err(RuntimeError::from(AuthError::KeychainOperationFailed {
                        resource: AuthResource::ClientSecret,
                        operation: StorageOperation::Read,
                        source: error,
                    }));
                }
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
        if let Some(entry) = keychain_entry_opt(profile, KEYRING_USER)? {
            match entry.delete_credential() {
                Ok(()) | Err(KeyringError::NoEntry) => {}
                // Race: backend disappeared between Entry::new and this call.
                Err(error) if is_keychain_unavailable(&error) => {}
                Err(error) => {
                    return Err(RuntimeError::from(AuthError::KeychainOperationFailed {
                        resource: AuthResource::ClientSecret,
                        operation: StorageOperation::Delete,
                        source: error,
                    }));
                }
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

/// Best-effort delete of the profile's keychain token entry. Called on every
/// file-storage fallback so a stale keychain entry can never shadow the fresh
/// file token on a later keychain-first [`get_token_data`] read. No-op when the
/// keychain is disabled; ignores `NoEntry` and any delete error.
fn clear_keychain_token_entry(profile: &str) {
    if config::is_keychain_disabled() {
        return;
    }
    if let Ok(Some(entry)) = keychain_entry_opt(profile, KEYRING_TOKEN_USER) {
        let _ = entry.delete_credential();
    }
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
/// 2560-byte limit — ~1280 chars — for UTF-16 encoded passwords). Rather than failing
/// login, any keychain write failure falls back to file storage with a visible warning,
/// after clearing any stale keychain entry so it can't shadow the file token on read.
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

    // On Windows, Credential Manager rejects passwords whose UTF-16 blob
    // exceeds ~2560 bytes. Skip the keychain attempt entirely for over-limit
    // tokens and route to file fallback with an explanatory warning.
    if exceeds_windows_credman_limit(&json) {
        warning = Some(format!(
            "Token exceeds the Windows Credential Manager limit ({WINDOWS_CREDMAN_PASSWORD_LIMIT_BYTES} bytes); \
             storing in file fallback.\n\
             Set AGS_NO_KEYCHAIN=1 to skip keychain and suppress this warning."
        ));
    } else if !config::is_keychain_disabled() {
        match keychain_entry_opt(profile, KEYRING_TOKEN_USER)? {
            Some(entry) => match entry.set_password(&json) {
                Ok(()) => return Ok(StoreOutcome { warning: None }),
                // Race: backend disappeared between Entry::new and this call.
                Err(error) if is_keychain_unavailable(&error) => {}
                Err(error) => {
                    warning = Some(format!(
                        "OS keychain write failed, falling back to file storage: {error}\n\
                         Set AGS_NO_KEYCHAIN=1 to skip keychain and suppress this warning."
                    ));
                }
            },
            // No usable keychain backend at init (none present, or it failed to
            // initialize — e.g. keyutils ENOSYS in a container). Fall back to
            // file with the same one-time, suppressible hint the other
            // keychain-skip paths emit, so the user knows the token is not in
            // the keychain and can opt out of the probe entirely.
            None => {
                warning = Some(
                    "OS keychain unavailable, storing token in file fallback.\n\
                     Set AGS_NO_KEYCHAIN=1 to skip keychain and suppress this warning."
                        .to_string(),
                );
            }
        }
    }

    // Reaching here means the token is NOT being stored in the keychain (it was
    // over-limit, the keychain is disabled, or the keychain write failed). Clear
    // any pre-existing keychain entry first, so a keychain-first `get_token_data`
    // cannot return a STALE token (old access AND old refresh token) that shadows
    // the fresh value we are about to write to the file. Without this, a rotated
    // refresh token from the stale entry is sent on the next refresh and the
    // server rejects it ("Session expired and token refresh failed").
    clear_keychain_token_entry(profile);

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
        if let Some(entry) = keychain_entry_opt(profile, KEYRING_TOKEN_USER)? {
            match entry.get_password() {
                Ok(json) => {
                    let token_data = serde_json::from_str::<TokenData>(&json)
                        .map_err(|e| RuntimeError::from(AuthError::KeychainTokenParseFailed(e)))?;
                    return Ok(Some(token_data));
                }
                Err(KeyringError::NoEntry) => {}
                // Race: backend disappeared between Entry::new and this call.
                Err(error) if is_keychain_unavailable(&error) => {}
                Err(error) => {
                    return Err(RuntimeError::from(AuthError::KeychainOperationFailed {
                        resource: AuthResource::Token,
                        operation: StorageOperation::Read,
                        source: error,
                    }));
                }
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
        if let Some(entry) = keychain_entry_opt(profile, KEYRING_TOKEN_USER)? {
            match entry.delete_credential() {
                Ok(()) | Err(KeyringError::NoEntry) => {}
                // Race: backend disappeared between Entry::new and this call.
                Err(error) if is_keychain_unavailable(&error) => {}
                Err(error) => {
                    return Err(RuntimeError::from(AuthError::KeychainOperationFailed {
                        resource: AuthResource::Token,
                        operation: StorageOperation::Delete,
                        source: error,
                    }));
                }
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

    /// Operation-time fallback is narrow: only `NoStorageAccess` (the backend
    /// went away after `Entry::new` succeeded) routes to file. A
    /// `PlatformFailure` on a working keychain is a genuine error and must surface.
    #[test]
    fn test_is_keychain_unavailable_only_for_no_storage_access() {
        let unavailable = KeyringError::NoStorageAccess(Box::new(std::io::Error::other("locked")));
        let other_failure = KeyringError::PlatformFailure(Box::new(std::io::Error::other("boom")));

        assert!(is_keychain_unavailable(&unavailable));
        assert!(!is_keychain_unavailable(&other_failure));
        assert!(!is_keychain_unavailable(&KeyringError::NoEntry));
    }

    /// Init-time fallback is broader: any *environmental* `Entry::new` failure
    /// routes to file storage. Covers the regression where `ags auth login`
    /// hard-failed in a container instead of falling back to file.
    ///
    /// Both container failure modes reach us as `PlatformFailure`: the Linux
    /// keyutils backend maps the failing `keyctl` errno through
    /// `linux_keyutils::from_errno`, which names only a handful of errnos
    /// (notably `EACCES` → `AccessDenied`) and sends everything else to
    /// `Unknown(errno)` → keyring `PlatformFailure`. So:
    /// - **ENOSYS (38)** — syscall absent (customer's stripped environment).
    /// - **EPERM (1)** — syscall blocked by the default Docker seccomp profile.
    ///
    /// Both must fall back; malformed-entry variants are still surfaced.
    #[test]
    fn test_is_keychain_init_unavailable_covers_storage_and_platform_failures() {
        let no_storage = KeyringError::NoStorageAccess(Box::new(std::io::Error::other("no d-bus")));
        let enosys = KeyringError::PlatformFailure(Box::new(std::io::Error::other("Unknown(38)")));
        // Default Docker seccomp blocks keyctl/add_key, surfacing as EPERM.
        let eperm = KeyringError::PlatformFailure(Box::new(std::io::Error::other("Unknown(1)")));

        // Every environmental failure routes to file fallback at init time.
        assert!(is_keychain_init_unavailable(&no_storage));
        assert!(is_keychain_init_unavailable(&enosys));
        assert!(is_keychain_init_unavailable(&eperm));

        // Malformed entries are a programming error, not a missing backend.
        assert!(!is_keychain_init_unavailable(&KeyringError::Invalid(
            "user".into(),
            "empty".into()
        )));
        assert!(!is_keychain_init_unavailable(&KeyringError::TooLong(
            "password".into(),
            2560
        )));
        assert!(!is_keychain_init_unavailable(&KeyringError::NoEntry));
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

    /// The credman blob size is the UTF-16 byte length (2 bytes per ASCII char) —
    /// the size keyring/Windows actually measures against the 2560-byte limit.
    #[test]
    fn test_credman_blob_bytes_is_twice_ascii_length() {
        assert_eq!(super::credman_blob_bytes(""), 0);
        assert_eq!(super::credman_blob_bytes("abcd"), 8);
    }

    /// Regression guard for the reported Windows refresh failure: a 1300-char
    /// ASCII token JSON is 1300 UTF-8 bytes — the old `str::len()` guard saw
    /// `1300 <= 2560` and kept it in the keychain — but its UTF-16 blob is 2600
    /// bytes, which keyring's `windows-native` backend rejects. The corrected
    /// size measure must report it as over-limit so it routes to file storage.
    #[test]
    fn test_token_in_utf16_dead_zone_exceeds_real_credman_limit() {
        let json = "a".repeat(1300);
        assert!(
            json.len() <= super::WINDOWS_CREDMAN_PASSWORD_LIMIT_BYTES,
            "precondition: the old UTF-8-byte guard would have passed this token"
        );
        assert!(
            super::credman_blob_bytes(&json) > super::WINDOWS_CREDMAN_PASSWORD_LIMIT_BYTES,
            "the real UTF-16 blob exceeds the credman limit"
        );
    }

    /// On Windows, a token in the UTF-16 dead zone (UTF-8 len under the limit,
    /// UTF-16 blob over it) must now trip the guard so it routes to file rather
    /// than failing the keychain write and silently leaving a stale entry.
    #[test]
    #[cfg(windows)]
    fn test_exceeds_windows_credman_limit_true_in_utf16_dead_zone() {
        let json = "a".repeat(1300);
        assert!(super::exceeds_windows_credman_limit(&json));
    }

    /// The file fallback round-trips the refresh token intact. Directly guards
    /// the reported failure mode (a refresh token must survive storage and read
    /// back unchanged). Exercises the unified fallback path that now clears any
    /// stale keychain entry before writing the file.
    #[test]
    #[serial_test::serial]
    fn test_store_then_get_round_trips_refresh_token_via_file_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var(crate::runtime::config::ENV_HOME, tmp.path());
        std::env::set_var(crate::runtime::config::ENV_NO_KEYCHAIN, "1");

        let token = TokenData {
            access_token: "access-xyz".to_string(),
            expires_at: 9_999_999_999,
            refresh_token: Some("refresh-abc".to_string()),
            refresh_expires_at: Some(9_999_999_999),
            grant_type: Some(crate::protocol::request::GrantType::AuthorizationCode),
        };
        store_token_data("default", &token).unwrap();

        let read = get_token_data("default").unwrap().expect("token stored");
        assert_eq!(read.refresh_token.as_deref(), Some("refresh-abc"));
        assert_eq!(read.access_token, "access-xyz");

        std::env::remove_var(crate::runtime::config::ENV_HOME);
        std::env::remove_var(crate::runtime::config::ENV_NO_KEYCHAIN);
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
