//! Domain error type for the AMS dedicated-server image upload pipeline.

use std::path::PathBuf;

use ags_protocol::error::{ErrorDetails, RuntimeError, RuntimeErrorKind, SuggestionKind};

/// Everything that can go wrong between `ags ams upload` being invoked and the
/// image being marked complete.
#[derive(Debug, thiserror::Error)]
pub enum AmsUploadError {
    #[error("Image name must be at least {min} characters")]
    ImageNameTooShort { min: usize },
    #[error("Image name must be at most {max} characters")]
    ImageNameTooLong { max: usize },
    #[error("Directory '{0}' does not exist")]
    DirectoryMissing(PathBuf),
    #[error("'{0}' is not a directory")]
    DirectoryNotADirectory(PathBuf),
    #[error("Directory '{0}' is empty")]
    DirectoryEmpty(PathBuf),
    #[error("Cannot find executable at '{0}'")]
    ExecutableMissing(PathBuf),
    #[error("Executable '{0}' is a directory, not a file")]
    ExecutableIsDirectory(PathBuf),
    #[error("Executable '{0}' does not match the on-disk filename case")]
    ExecutableWrongCase(PathBuf),
    #[error("Executable '{0}' resolves outside the upload directory")]
    ExecutableOutsideDirectory(String),
    #[error("Executable must be a 64-bit little-endian ELF binary, or a shell script (.sh)")]
    ExecutableNotAcceptedBinary,
    #[error("Target architecture is required when the entrypoint is a shell script")]
    ShellScriptNeedsTargetArchitecture,
    #[error("'{path}' is not a valid shell script: {reason}")]
    ShellScriptInvalid { path: PathBuf, reason: String },
    #[error(
        "Target architecture '{requested}' does not match the detected architecture '{detected}'"
    )]
    ArchitectureMismatch { requested: String, detected: String },
    #[error("Could not determine the AMS upload host")]
    UploadHostUnresolved { reason: String },
    #[error("AMS upload host '{0}' is not an absolute http(s) URL")]
    UploadHostInvalid(String),
    #[error("{operation} failed: {reason}")]
    ApiCallFailed {
        operation: &'static str,
        reason: String,
        status: Option<u16>,
        /// Request/response detail for the failing call, populated only under
        /// `--verbose` so the frontend can render the same diagnostic block on
        /// this path that a normal service command gets.
        trace: Option<Box<ags_protocol::output_views::ExecutionTrace>>,
    },
    #[error("Upload of part {part_number} failed: {reason}")]
    PartUploadFailed { part_number: usize, reason: String },
    #[error("The storage service did not return an ETag for part {0}")]
    MissingETag(usize),
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
    /// A failure that happened after the image record already existed in AMS.
    ///
    /// Carries the underlying failure unchanged and adds the identity of the
    /// record left behind, so the user is told what to clean up rather than
    /// discovering it later as an incomplete image in the namespace.
    #[error("{source}")]
    OrphanedImage {
        image_name: String,
        image_id: String,
        #[source]
        source: Box<AmsUploadError>,
    },
}

impl AmsUploadError {
    /// Wrap an I/O failure with the operation that was being attempted.
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}

impl From<AmsUploadError> for RuntimeError {
    fn from(error: AmsUploadError) -> Self {
        let message = error.to_string();
        match error {
            // Converted from the inner failure so the original diagnosis — the
            // 403 guidance especially — survives intact, with the orphan noted
            // on its own line rather than competing for reason/detail/tip.
            AmsUploadError::OrphanedImage {
                image_name,
                image_id,
                source,
            } => {
                let mut runtime_error = RuntimeError::from(*source);
                runtime_error.message =
                    format!("{message}\n{}", orphan_notice(&image_name, &image_id));
                runtime_error
            }
            AmsUploadError::ImageNameTooShort { .. } | AmsUploadError::ImageNameTooLong { .. } => {
                validation(message, None, Some("Pass a different --image-name."))
            }
            AmsUploadError::DirectoryMissing(_)
            | AmsUploadError::DirectoryNotADirectory(_)
            | AmsUploadError::DirectoryEmpty(_) => validation(
                message,
                None,
                Some("Point --path at the directory holding the built dedicated server."),
            ),
            AmsUploadError::ExecutableMissing(_) => validation(
                message,
                Some("--executable is resolved relative to --path.".to_string()),
                Some("Check the path and retry."),
            ),
            AmsUploadError::ExecutableOutsideDirectory(_) => validation(
                message,
                Some("The entrypoint must live inside --path so it lands in the archive.".to_string()),
                Some("Pass --executable as a path relative to --path."),
            ),
            AmsUploadError::ExecutableIsDirectory(_) => validation(
                message,
                None,
                Some("Pass the server binary or launch script, not its parent directory."),
            ),
            AmsUploadError::ExecutableWrongCase(_) => validation(
                message,
                Some(
                    "Linux is case-sensitive; a name that resolves on Windows or macOS can fail there."
                        .to_string(),
                ),
                Some("Match --executable to the exact on-disk filename."),
            ),
            AmsUploadError::ExecutableNotAcceptedBinary => validation(
                message,
                Some("AMS accepts ELFCLASS64 little-endian x86-64 or aarch64 binaries.".to_string()),
                Some("Rebuild the server for linux-x86_64 or linux-arm_64."),
            ),
            AmsUploadError::ShellScriptNeedsTargetArchitecture => validation(
                message,
                Some("A script carries no architecture to detect.".to_string()),
                Some("Pass --target-arch linux-x86_64 or --target-arch linux-arm_64."),
            ),
            AmsUploadError::ShellScriptInvalid { .. } => validation(
                message,
                None,
                Some("Fix the script, or pass --skip-script-validation to upload it as-is."),
            ),
            AmsUploadError::ArchitectureMismatch { .. } => validation(
                message,
                None,
                Some("Drop --target-arch to use the detected architecture."),
            ),
            AmsUploadError::UploadHostUnresolved { ref reason } => RuntimeError {
                kind: RuntimeErrorKind::Network,
                message: message.clone(),
                details: Some(Box::new(ErrorDetails {
                    code: None,
                    reason: Some(reason.clone()),
                    detail: Some(
                        "The upload is stopped rather than defaulting to production AMS."
                            .to_string(),
                    ),
                    suggestion_kind: Some(SuggestionKind::Fix),
                    tip: None,
                })),
                hint: Some(
                    "Check the base URL and connectivity, or pass --upload-url to target the host directly."
                        .to_string(),
                ),
                trace: None,
            },
            AmsUploadError::UploadHostInvalid(_) => validation(
                message,
                None,
                Some("Pass --upload-url as an absolute URL, e.g. https://prod.ams.accelbyte.io."),
            ),
            AmsUploadError::ApiCallFailed {
                status, ref trace, ..
            } => RuntimeError {
                kind: match status {
                    Some(401) => RuntimeErrorKind::NotAuthenticated,
                    Some(403) => RuntimeErrorKind::Forbidden,
                    Some(404) => RuntimeErrorKind::NotFound,
                    Some(400) | Some(422) => RuntimeErrorKind::Rejected,
                    Some(status) => RuntimeErrorKind::Upstream {
                        status,
                        code: None,
                    },
                    None => RuntimeErrorKind::Network,
                },
                message,
                details: {
                    let guidance = api_call_guidance(status);
                    guidance.has_context().then(|| {
                        Box::new(ErrorDetails {
                            code: None,
                            reason: guidance.reason.map(str::to_string),
                            detail: guidance.detail.map(str::to_string),
                            suggestion_kind: Some(SuggestionKind::Fix),
                            tip: guidance.tip.map(str::to_string),
                        })
                    })
                },
                hint: api_call_guidance(status).fix.map(str::to_string),
                trace: trace.clone(),
            },
            AmsUploadError::PartUploadFailed { .. } | AmsUploadError::MissingETag(_) => {
                RuntimeError {
                    kind: RuntimeErrorKind::Network,
                    message,
                    details: None,
                    hint: Some("Retry the upload.".to_string()),
                    trace: None,
                }
            }
            AmsUploadError::Io { ref source, .. } => validation(message, None, io_fix(source)),
        }
    }
}

/// The four user-facing lines the CLI can show for a failed AMS upload API call.
///
/// Every field is optional so a status with nothing useful to add stays quiet
/// rather than padding the error with filler.
struct ApiCallGuidance {
    /// Why the call was refused.
    reason: Option<&'static str>,
    /// The non-obvious specifics needed to act — exact permission strings and
    /// the prefix rules that are easy to get wrong.
    detail: Option<&'static str>,
    /// The corrective action.
    fix: Option<&'static str>,
    /// A follow-on gotcha that bites after the fix is applied.
    tip: Option<&'static str>,
}

impl ApiCallGuidance {
    /// Whether anything beyond the bare message is worth rendering.
    fn has_context(&self) -> bool {
        self.reason.is_some() || self.detail.is_some() || self.tip.is_some()
    }
}

/// Explain a failed AMS upload API call in enough detail that a reader — human
/// or agent — can fix it without consulting the source or the AMS team.
///
/// The 403 gets the most, because it is both the common first-run failure and a
/// genuinely surprising one: uploading is guarded by `AMS:UPLOAD`, a different
/// resource from the `AMS:IMAGE` behind `ags ams images`, and the two differ in
/// whether they take a namespace prefix. Both actions are required — creation
/// and URL signing take CREATE while finalize and complete take UPDATE
/// (`armada-core-api` `service/routes.go`) — so a CREATE-only identity fails
/// only after every byte has been transferred.
fn api_call_guidance(status: Option<u16>) -> ApiCallGuidance {
    match status {
        Some(401) => ApiCallGuidance {
            reason: Some("The access token was rejected by AMS."),
            detail: None,
            fix: Some("Run 'ags auth login' and retry."),
            tip: None,
        },
        Some(403) => ApiCallGuidance {
            reason: Some(
                "The identity you authenticated as does not carry the AMS:UPLOAD permission.",
            ),
            detail: Some(
                "Enter it exactly as 'AMS:UPLOAD' with no ADMIN:NAMESPACE:... prefix, and \
                 grant both the Create and Update actions. It is a different permission from \
                 the namespaced ADMIN:NAMESPACE:{namespace}:AMS:IMAGE behind 'ags ams images', \
                 so being able to list images does not allow uploading one.",
            ),
            fix: Some(
                "Grant AMS:UPLOAD (Create, Update) to the IAM client you authenticate as — it \
                 must be a confidential client, used via AGS_CLIENT_ID / AGS_CLIENT_SECRET — or \
                 to your user's roles if you sign in with 'ags auth login'.",
            ),
            tip: Some(
                "A permission change needs a new token: 'ags auth refresh' after a \
                 client-credentials login, or a full 'ags auth login' after a browser login \
                 (refresh does not pick up new role grants).",
            ),
        },
        Some(404) => ApiCallGuidance {
            reason: Some("AMS has no such route, or no AMS account for this namespace."),
            detail: Some(
                "The destination namespace comes from your token, not from --namespace, so an \
                 image can only be uploaded into the namespace its client belongs to — and that \
                 namespace must have AMS enabled.",
            ),
            fix: Some(
                "Authenticate with a client in a namespace that has an AMS account, or check \
                 --upload-url if you set one.",
            ),
            tip: None,
        },
        _ => ApiCallGuidance {
            reason: None,
            detail: None,
            fix: None,
            tip: None,
        },
    }
}

/// Name the image record left behind by a failure, and how to remove it.
///
/// The removal command is guarded by `ADMIN:NAMESPACE:{namespace}:AMS:IMAGE`
/// (Delete), which an upload-only identity does not carry — hence the Admin
/// Portal alternative, since a CI client is the case most likely to orphan one
/// and least likely to be able to delete it.
fn orphan_notice(image_name: &str, image_id: &str) -> String {
    format!(
        "Image '{image_name}' (id {image_id}) was created in AMS before this failure and is \
         incomplete. Remove it with 'ags ams images mark-for-deletion --image-id {image_id} \
         --namespace <namespace>' (needs AMS:IMAGE Delete) or from the Admin Portal."
    )
}

/// Suggest a fix for the local failure modes the pipeline actually hits.
///
/// A `NotFound` here is not a mistyped `--path` — that is caught up front by
/// `DirectoryMissing` — it means a name the directory listing returned could not
/// then be opened, which in practice is a dangling symlink.
fn io_fix(source: &std::io::Error) -> Option<&'static str> {
    match source.kind() {
        std::io::ErrorKind::PermissionDenied => {
            Some("Check the file permissions on --path and on the temporary directory.")
        }
        std::io::ErrorKind::NotFound => {
            Some("A dangling symlink is the usual cause; remove it or fix its target.")
        }
        std::io::ErrorKind::StorageFull => {
            Some("Free space on the filesystem behind TMPDIR — the archive is staged there.")
        }
        _ => None,
    }
}

/// Build a pre-flight validation error with an optional reason and fix line.
fn validation(message: String, reason: Option<String>, suggestion: Option<&str>) -> RuntimeError {
    RuntimeError {
        kind: RuntimeErrorKind::Validation,
        message,
        details: reason.map(|reason| {
            Box::new(ErrorDetails {
                code: None,
                reason: Some(reason),
                detail: None,
                suggestion_kind: Some(SuggestionKind::Fix),
                tip: None,
            })
        }),
        hint: suggestion.map(str::to_string),
        trace: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 403 must be self-contained: a reader who has never seen this code —
    /// human or agent — should be able to fix it from the error text alone,
    /// without consulting the source, the AMS team, or the AccelByte docs.
    ///
    /// Each assertion below stands for one thing that actually went wrong during
    /// the first real integration, so weakening any of them re-opens a trap.
    #[test]
    fn test_forbidden_upload_is_self_contained() {
        let error: RuntimeError = AmsUploadError::ApiCallFailed {
            operation: "Creating the image",
            reason: "HTTP 403 token is missing required permissions".to_string(),
            status: Some(403),
            trace: None,
        }
        .into();
        assert_eq!(error.kind, RuntimeErrorKind::Forbidden);

        let details = error.details.expect("a 403 must explain itself");
        let reason = details.reason.expect("reason line");
        let detail = details.detail.expect("detail line");
        let tip = details.tip.expect("tip line");
        let fix = error.hint.expect("fix line");

        // Names the permission, not just "forbidden".
        assert!(reason.contains("AMS:UPLOAD"), "{reason}");
        // Both actions — a Create-only grant fails after transferring everything.
        assert!(fix.contains("Create") && fix.contains("Update"), "{fix}");
        // The prefix rule: AMS:UPLOAD is bare, AMS:IMAGE is namespaced. Getting
        // this backwards was the actual mistake made during integration.
        assert!(detail.contains("no ADMIN:NAMESPACE"), "{detail}");
        assert!(detail.contains("AMS:IMAGE"), "{detail}");
        // Client-credentials needs a confidential client; a public one cannot.
        assert!(fix.contains("confidential"), "{fix}");
        assert!(fix.contains("AGS_CLIENT_ID"), "{fix}");
        // A permission change is invisible until the token is re-minted.
        assert!(tip.contains("ags auth login"), "{tip}");
    }

    /// The destination namespace comes from the token, so a 404 must not send
    /// the reader hunting for a wrong `--namespace`.
    #[test]
    fn test_not_found_explains_the_namespace_comes_from_the_token() {
        let error: RuntimeError = AmsUploadError::ApiCallFailed {
            operation: "Creating the image",
            reason: "HTTP 404 no account associated with namespace foo".to_string(),
            status: Some(404),
            trace: None,
        }
        .into();
        let detail = error
            .details
            .expect("a 404 must explain itself")
            .detail
            .expect("detail line");
        assert!(detail.contains("--namespace"), "{detail}");
        assert!(detail.contains("token"), "{detail}");
    }

    #[test]
    fn test_unauthorized_upload_points_at_login() {
        let error: RuntimeError = AmsUploadError::ApiCallFailed {
            operation: "Creating the image",
            reason: "HTTP 401".to_string(),
            status: Some(401),
            trace: None,
        }
        .into();
        assert_eq!(error.kind, RuntimeErrorKind::NotAuthenticated);
        assert!(error.hint.unwrap().contains("ags auth login"));
    }

    /// Local I/O failures are environment conditions, not CLI bugs. Routing them
    /// to Internal spends exit code 5 — "you hit a bug" — on a read-only build
    /// directory, and drops the hint that would have fixed it.
    #[test]
    fn test_io_failures_are_reported_as_user_fixable() {
        let error: RuntimeError = AmsUploadError::io(
            "Failed to read /build",
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        )
        .into();
        assert_eq!(error.kind, RuntimeErrorKind::Validation);
        assert!(error.hint.unwrap().contains("permissions"));
    }

    /// A failure after `create_image` leaves a record in the namespace. The user
    /// has to be told which one and how to remove it, or a retrying CI job
    /// silently accumulates incomplete images.
    #[test]
    fn test_orphaned_image_is_named_with_its_id() {
        let error: RuntimeError = AmsUploadError::OrphanedImage {
            image_name: "my-server".to_string(),
            image_id: "img_abc123".to_string(),
            source: Box::new(AmsUploadError::PartUploadFailed {
                part_number: 3,
                reason: "connection reset".to_string(),
            }),
        }
        .into();

        assert!(error.message.contains("img_abc123"), "{}", error.message);
        assert!(error.message.contains("my-server"), "{}", error.message);
        assert!(
            error.message.contains("mark-for-deletion"),
            "{}",
            error.message
        );
        // The original failure still leads; the orphan is the second line.
        let (headline, notice) = error.message.split_once('\n').expect("two lines");
        assert!(headline.contains("part 3"), "{headline}");
        assert!(notice.contains("incomplete"), "{notice}");
    }

    /// Wrapping must not cost the underlying diagnosis — a 403 that orphans an
    /// image still has to explain AMS:UPLOAD, or the wrapper has made the error
    /// worse than the one it replaced.
    #[test]
    fn test_orphan_wrapper_preserves_the_inner_guidance() {
        let inner = AmsUploadError::ApiCallFailed {
            operation: "Marking the image as complete",
            reason: "HTTP 403 token is missing required permissions".to_string(),
            status: Some(403),
            trace: None,
        };
        let error: RuntimeError = AmsUploadError::OrphanedImage {
            image_name: "my-server".to_string(),
            image_id: "img_abc123".to_string(),
            source: Box::new(inner),
        }
        .into();

        assert_eq!(error.kind, RuntimeErrorKind::Forbidden);
        let details = error
            .details
            .expect("the 403 guidance must survive wrapping");
        assert!(details.reason.unwrap().contains("AMS:UPLOAD"));
        assert!(error.hint.unwrap().contains("Create"));
    }

    /// A 5xx is the service's problem; inventing a user-facing fix would be noise.
    #[test]
    fn test_server_error_carries_no_hint() {
        let error: RuntimeError = AmsUploadError::ApiCallFailed {
            operation: "Creating the image",
            reason: "HTTP 500".to_string(),
            status: Some(500),
            trace: None,
        }
        .into();
        assert!(error.hint.is_none());
    }
}
