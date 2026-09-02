//! Uploading a dedicated-server image to AMS.
//!
//! A port of the `armada-cli` `ams upload` pipeline: validate the build
//! directory and its entrypoint, pack it into a `tar.gz`, and ship it to the
//! environment's AMS upload host through pre-signed URLs.
//!
//! It sits beside the workflow engine rather than inside it. Every
//! `StepDefinition` references a catalogued API operation, and archiving a
//! directory is not one — so this follows the `auth` precedent instead: a
//! bespoke runtime entry point that still reports through `ProgressSink` and
//! returns a `CommandOutput`.

pub mod archive;
pub mod entrypoint;

mod api;
mod discovery;
mod elf;
mod errors;
mod pipeline;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use ags_protocol::event::ProgressSink;
use ags_protocol::output::AmsUploadView;

pub use errors::AmsUploadError;

/// Version reported to AMS as `ams-cli-version`.
pub(crate) const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Prefix of the staging directory the archive is built in. Shared with the
/// startup sweep in `runtime::cleanup`, which reclaims directories left behind
/// when a run is killed by a signal.
pub(crate) const TEMP_DIR_PREFIX: &str = "ags-ams-upload-";

/// Default number of multipart parts uploaded at once.
pub const DEFAULT_PART_CONCURRENCY: usize = 4;

/// Architectures AMS can run a dedicated server on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetArchitecture {
    LinuxX86_64,
    LinuxArm64,
}

impl TargetArchitecture {
    /// The value AMS expects on the wire.
    pub const fn wire_value(self) -> &'static str {
        match self {
            TargetArchitecture::LinuxX86_64 => "linux-x86_64",
            TargetArchitecture::LinuxArm64 => "linux-arm_64",
        }
    }

    /// Every accepted value, for CLI help text and flag validation.
    pub const fn all() -> [&'static str; 2] {
        ["linux-x86_64", "linux-arm_64"]
    }

    /// Parse a wire value back into a target architecture.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "linux-x86_64" => Some(TargetArchitecture::LinuxX86_64),
            "linux-arm_64" => Some(TargetArchitecture::LinuxArm64),
            _ => None,
        }
    }
}

/// Everything `ags ams upload` needs, resolved from flags.
///
/// Credentials and the platform host are deliberately absent: they come from
/// the runtime's `ExecutionContext`, the same as every other command.
#[derive(Debug, Clone)]
pub struct UploadRequest {
    /// Directory whose contents become the image.
    pub directory: PathBuf,
    /// Entrypoint, relative to `directory`.
    pub executable: String,
    pub image_name: String,
    /// Required for a `.sh` entrypoint; cross-checked against the detected
    /// architecture for an ELF one.
    pub target_architecture: Option<TargetArchitecture>,
    pub include_symbol_files: bool,
    pub skip_script_validation: bool,
    /// Skips AMS host discovery when set.
    pub upload_url_override: Option<String>,
    pub part_concurrency: usize,
    /// Attach request/response detail to failures, so `--verbose` reports the
    /// failing call rather than staying silent.
    pub is_verbose: bool,
}

impl crate::runtime::Runtime {
    /// Upload a dedicated-server image to AMS, reporting progress through `sink`.
    pub async fn ams_upload(
        &self,
        request: &UploadRequest,
        sink: &mut dyn ProgressSink,
    ) -> Result<AmsUploadView, ags_protocol::error::RuntimeError> {
        let upload_client = crate::runtime::dispatch::http::build_upload_client()?;
        pipeline::run_upload(
            &self.reqwest_client,
            &upload_client,
            &self.context,
            request,
            sink,
        )
        .await
        .map_err(Into::into)
    }

    /// Validate an upload request without archiving anything or calling AMS.
    pub fn ams_upload_dry_run(
        &self,
        request: &UploadRequest,
    ) -> Result<AmsUploadView, ags_protocol::error::RuntimeError> {
        pipeline::plan_upload(request).map_err(Into::into)
    }
}

#[cfg(test)]
mod target_architecture_tests {
    use super::*;

    #[test]
    fn test_target_architecture_round_trips_its_wire_values() {
        for value in TargetArchitecture::all() {
            let parsed = TargetArchitecture::parse(value).expect("all() values must parse");
            assert_eq!(parsed.wire_value(), value);
        }
        assert!(TargetArchitecture::parse("linux-x86").is_none());
    }
}
