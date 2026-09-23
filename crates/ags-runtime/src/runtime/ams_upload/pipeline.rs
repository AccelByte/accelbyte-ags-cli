//! Orchestration for `ags ams upload`: validate, pack, ship, complete.

use std::sync::Arc;

use ags_protocol::event::{ProgressEvent, ProgressSink};
use ags_protocol::output::{AmsUploadPlan, AmsUploadResult, AmsUploadView};
use reqwest::Client;

use crate::runtime::dispatch::http::{put_file_range, BinaryPutError, FileRange};
use crate::runtime::execution::ExecutionContext;

use super::api::UploadApi;
use super::archive::{self, ArchiveManifest};
use super::discovery;
use super::entrypoint;
use super::errors::AmsUploadError;
use super::UploadRequest;

/// Size above which an archive is uploaded as a multipart upload, and the size
/// of every part but the last. Ported from armada-cli's
/// `MultiPartChunkSizeBytes`.
const MULTIPART_CHUNK_BYTES: u64 = 500 * 1024 * 1024;

/// Validate the request without touching the network or building an archive.
///
/// Backs `--dry-run`: everything that can be decided locally is decided, so a
/// broken entrypoint or an empty build directory is reported before a byte
/// moves.
pub(crate) fn plan_upload(request: &UploadRequest) -> Result<AmsUploadView, AmsUploadError> {
    let upload_base_url = request
        .upload_url_override
        .as_deref()
        .map(discovery::normalise_upload_url)
        .transpose()?;
    let (resolved, manifest) = validate(request)?;
    Ok(AmsUploadView::Planned(AmsUploadPlan {
        image_name: request.image_name.clone(),
        directory: request.directory.display().to_string(),
        executable: request.executable.clone(),
        command: resolved.command,
        target_architecture: resolved.architecture.wire_value().to_string(),
        entrypoint_kind: resolved.kind,
        file_count: manifest.entries.len(),
        total_bytes: manifest.total_bytes,
        include_symbol_files: request.include_symbol_files,
        excluded_symbol_file_count: manifest.excluded_symbol_file_count,
        skipped_directory_symlinks: manifest.skipped_directory_symlinks,
        upload_base_url,
    }))
}

/// Run the full upload pipeline and return the created image.
pub(crate) async fn run_upload(
    api_client: &Client,
    upload_client: &Client,
    context: &ExecutionContext,
    request: &UploadRequest,
    sink: &mut dyn ProgressSink,
) -> Result<AmsUploadView, AmsUploadError> {
    run_upload_with_chunk_size(
        api_client,
        upload_client,
        context,
        request,
        sink,
        MULTIPART_CHUNK_BYTES,
    )
    .await
}

/// The pipeline proper, with the multipart threshold injected so tests can
/// exercise the multipart branch without producing a half-gigabyte archive.
pub(super) async fn run_upload_with_chunk_size(
    api_client: &Client,
    upload_client: &Client,
    context: &ExecutionContext,
    request: &UploadRequest,
    sink: &mut dyn ProgressSink,
    chunk_bytes: u64,
) -> Result<AmsUploadView, AmsUploadError> {
    // A `Message`, not a `Started`: a `Started` lands on the temporary status
    // line, which the next line overwrites and which is dropped entirely when
    // stderr is captured, so the first documented step would never be seen.
    sink.on_event(ProgressEvent::Message {
        text: format!("Validating {}", request.directory.display()),
    });

    let (resolved, manifest) = validate(request)?;

    // Warned before the transfer starts, so a truncated image can be cancelled
    // rather than discovered on the fleet.
    if !manifest.skipped_directory_symlinks.is_empty() {
        sink.on_event(ProgressEvent::Message {
            text: format!(
                "Skipping symlinked {} (not archived): {}",
                if manifest.skipped_directory_symlinks.len() == 1 {
                    "directory"
                } else {
                    "directories"
                },
                manifest.skipped_directory_symlinks.join(", ")
            ),
        });
    }

    sink.on_event(ProgressEvent::Message {
        text: format!(
            "Resolving the AMS upload host for {}",
            discovery::source_environment(&context.base_url)
        ),
    });
    let upload_base_url = discovery::resolve_upload_base_url(
        api_client,
        &context.base_url,
        &context.access_token,
        request.upload_url_override.as_deref(),
    )
    .await?;

    if upload_base_url.starts_with("http://") {
        sink.on_event(ProgressEvent::Message {
            text: format!(
                "The upload host ({upload_base_url}) uses plaintext HTTP. The access \
                 token and archive will travel unencrypted, which is expected only for \
                 a local or internal endpoint."
            ),
        });
    }

    let temp_dir = tempfile::Builder::new()
        .prefix(super::TEMP_DIR_PREFIX)
        .tempdir()
        .map_err(|e| AmsUploadError::io("Failed to create a temporary directory", e))?;
    let archive_name = format!("{}-part-0.tar.gz", archive_stem(&request.image_name));
    let archive_path = temp_dir.path().join(&archive_name);

    sink.on_event(ProgressEvent::Message {
        text: format!(
            "Packing {} files into {archive_name}",
            manifest.entries.len()
        ),
    });
    let archive_bytes = archive::build_archive(&manifest.entries, &archive_path)?;

    let api = Arc::new(UploadApi::new(
        api_client.clone(),
        &upload_base_url,
        context.access_token.clone(),
        discovery::source_environment(&context.base_url),
        request.is_verbose,
    ));

    sink.on_event(ProgressEvent::Message {
        text: format!("Creating image '{}'", request.image_name),
    });
    let image_id = api
        .create_image(&request.image_name, resolved.architecture.wire_value())
        .await?;

    // From here on the image record exists in AMS, so a failure leaves it
    // behind incomplete. Every fallible call below is tagged with it, or the
    // user is never told there is something to clean up.
    let orphan = |error: AmsUploadError| AmsUploadError::OrphanedImage {
        image_name: request.image_name.clone(),
        image_id: image_id.clone(),
        source: Box::new(error),
    };

    let part_count = part_count(archive_bytes, chunk_bytes);
    sink.on_event(ProgressEvent::Message {
        text: upload_message(archive_bytes, part_count),
    });
    if part_count > 1 {
        upload_in_parts(
            &api,
            upload_client,
            &image_id,
            &archive_name,
            &archive_path,
            archive_bytes,
            PartLayout {
                count: part_count,
                chunk_bytes,
            },
            request.part_concurrency,
        )
        .await
        .map_err(&orphan)?;
    } else {
        upload_whole(
            &api,
            upload_client,
            &image_id,
            &archive_name,
            &archive_path,
            archive_bytes,
        )
        .await
        .map_err(&orphan)?;
    }

    sink.on_event(ProgressEvent::Message {
        text: "Marking the image as complete".to_string(),
    });
    api.complete(&image_id, archive_bytes, &resolved.command)
        .await
        .map_err(&orphan)?;
    sink.on_event(ProgressEvent::Finished);

    Ok(AmsUploadView::Uploaded(AmsUploadResult {
        image_id,
        image_name: request.image_name.clone(),
        target_architecture: resolved.architecture.wire_value().to_string(),
        command: resolved.command,
        file_count: manifest.entries.len(),
        archive_bytes,
        part_count,
        upload_base_url,
        skipped_directory_symlinks: manifest.skipped_directory_symlinks,
    }))
}

/// Run every local pre-flight check and enumerate what would be archived.
fn validate(
    request: &UploadRequest,
) -> Result<(entrypoint::ResolvedEntrypoint, ArchiveManifest), AmsUploadError> {
    entrypoint::validate_image_name(&request.image_name)?;
    entrypoint::validate_directory(&request.directory)?;
    let resolved = entrypoint::resolve_entrypoint(
        &request.directory,
        &request.executable,
        request.target_architecture,
        request.skip_script_validation,
    )?;
    let manifest = archive::collect_entries(&request.directory, request.include_symbol_files)?;
    if manifest.entries.is_empty() {
        return Err(AmsUploadError::DirectoryEmpty(request.directory.clone()));
    }
    Ok((resolved, manifest))
}

/// Upload the archive with a single pre-signed PUT.
async fn upload_whole(
    api: &UploadApi,
    upload_client: &Client,
    image_id: &str,
    archive_name: &str,
    archive_path: &std::path::Path,
    archive_bytes: u64,
) -> Result<(), AmsUploadError> {
    let url = api.presign_url(image_id, archive_name).await?;
    let range = FileRange::whole(archive_path, archive_bytes);
    match put_file_range(upload_client, &url, &range).await {
        Ok(_) => Ok(()),
        // A single-shot PUT can also outlive its signature on a slow link, so
        // it gets the same one re-sign as a multipart part.
        Err(BinaryPutError::Unauthorized { .. }) => {
            let url = api.presign_url(image_id, archive_name).await?;
            put_file_range(upload_client, &url, &range)
                .await
                .map(|_| ())
                .map_err(|error| AmsUploadError::PartUploadFailed {
                    part_number: 1,
                    reason: error.into_runtime_error().message,
                })
        }
        Err(error) => Err(AmsUploadError::PartUploadFailed {
            part_number: 1,
            reason: error.into_runtime_error().message,
        }),
    }
}

/// How an archive is divided into multipart parts.
#[derive(Debug, Clone, Copy)]
struct PartLayout {
    count: usize,
    chunk_bytes: u64,
}

/// Upload the archive as a multipart upload with bounded part concurrency.
///
/// ETags are placed by part number rather than completion order — AMS
/// reassembles the object from the order of the `parts` array, so a
/// completion-ordered list would corrupt the image.
#[allow(clippy::too_many_arguments)]
async fn upload_in_parts(
    api: &Arc<UploadApi>,
    upload_client: &Client,
    image_id: &str,
    archive_name: &str,
    archive_path: &std::path::Path,
    archive_bytes: u64,
    layout: PartLayout,
    concurrency: usize,
) -> Result<(), AmsUploadError> {
    let upload_id = api.initiate_multipart(image_id, archive_name).await?;
    let permits = Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let mut tasks = tokio::task::JoinSet::new();

    for index in 0..layout.count {
        let part_number = index + 1;
        let offset = index as u64 * layout.chunk_bytes;
        let range = FileRange {
            path: archive_path.to_path_buf(),
            offset,
            length: layout.chunk_bytes.min(archive_bytes - offset),
        };
        let api = Arc::clone(api);
        let permits = Arc::clone(&permits);
        let upload_client = upload_client.clone();
        let upload_id = upload_id.clone();
        let image_id = image_id.to_string();
        let archive_name = archive_name.to_string();

        tasks.spawn(async move {
            let _permit = permits.acquire().await;
            let etag = upload_part(
                &api,
                &upload_client,
                &upload_id,
                &image_id,
                &archive_name,
                part_number,
                &range,
            )
            .await?;
            Ok::<(usize, String), AmsUploadError>((part_number, etag))
        });
    }

    let mut etags: Vec<Option<String>> = vec![None; layout.count];
    while let Some(joined) = tasks.join_next().await {
        let (part_number, etag) = joined.map_err(|error| AmsUploadError::PartUploadFailed {
            part_number: 0,
            reason: error.to_string(),
        })??;
        etags[part_number - 1] = Some(etag);
    }

    let ordered = etags
        .into_iter()
        .enumerate()
        .map(|(index, etag)| etag.ok_or(AmsUploadError::MissingETag(index + 1)))
        .collect::<Result<Vec<_>, _>>()?;

    api.finalize_multipart(&upload_id, image_id, archive_name, &ordered)
        .await
}

/// Upload one part, re-signing once if the signature has expired mid-upload.
async fn upload_part(
    api: &UploadApi,
    upload_client: &Client,
    upload_id: &str,
    image_id: &str,
    archive_name: &str,
    part_number: usize,
    range: &FileRange,
) -> Result<String, AmsUploadError> {
    let url = api
        .presign_part(upload_id, image_id, archive_name, part_number)
        .await?;
    let outcome = match put_file_range(upload_client, &url, range).await {
        Ok(outcome) => outcome,
        // Signed-URL lifetime is set server-side by AMS and is not visible to
        // the client, so a large build can outlive it. One re-sign turns that
        // from a failed multi-gigabyte upload into a retried part.
        Err(BinaryPutError::Unauthorized { .. }) => {
            let url = api
                .presign_part(upload_id, image_id, archive_name, part_number)
                .await?;
            put_file_range(upload_client, &url, range)
                .await
                .map_err(|error| AmsUploadError::PartUploadFailed {
                    part_number,
                    reason: error.into_runtime_error().message,
                })?
        }
        Err(error) => {
            return Err(AmsUploadError::PartUploadFailed {
                part_number,
                reason: error.into_runtime_error().message,
            })
        }
    };
    outcome.etag.ok_or(AmsUploadError::MissingETag(part_number))
}

/// How many multipart parts an archive of `archive_bytes` splits into.
/// A value of 1 means a single-shot pre-signed PUT.
fn part_count(archive_bytes: u64, chunk_bytes: u64) -> usize {
    if archive_bytes <= chunk_bytes {
        return 1;
    }
    archive_bytes.div_ceil(chunk_bytes) as usize
}

/// Progress line describing the transfer that is about to start.
fn upload_message(archive_bytes: u64, part_count: usize) -> String {
    let mebibytes = archive_bytes as f64 / (1024.0 * 1024.0);
    if part_count > 1 {
        format!("Uploading {mebibytes:.1} MiB in {part_count} parts")
    } else {
        format!("Uploading {mebibytes:.1} MiB")
    }
}

/// Filesystem-safe stem for the temporary archive, derived from the image name.
fn archive_stem(image_name: &str) -> String {
    let stem: String = image_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect();
    if stem.starts_with('.') {
        format!("image{stem}")
    } else {
        stem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_part_count_uses_the_multipart_threshold() {
        assert_eq!(part_count(1, MULTIPART_CHUNK_BYTES), 1);
        assert_eq!(part_count(MULTIPART_CHUNK_BYTES, MULTIPART_CHUNK_BYTES), 1);
        assert_eq!(
            part_count(MULTIPART_CHUNK_BYTES + 1, MULTIPART_CHUNK_BYTES),
            2
        );
        assert_eq!(
            part_count(MULTIPART_CHUNK_BYTES * 8, MULTIPART_CHUNK_BYTES),
            8
        );
        assert_eq!(
            part_count(MULTIPART_CHUNK_BYTES * 8 + 1, MULTIPART_CHUNK_BYTES),
            9
        );
    }

    #[test]
    fn test_archive_stem_is_filesystem_safe() {
        assert_eq!(archive_stem("my-image"), "my-image");
        assert_eq!(archive_stem("my image/v2"), "my-image-v2");
        assert_eq!(archive_stem(".hidden"), "image.hidden");
    }

    #[test]
    fn test_upload_message_mentions_parts_only_when_multipart() {
        assert_eq!(upload_message(1024 * 1024, 1), "Uploading 1.0 MiB");
        assert_eq!(
            upload_message(MULTIPART_CHUNK_BYTES * 2, 2),
            "Uploading 1000.0 MiB in 2 parts"
        );
    }
}
