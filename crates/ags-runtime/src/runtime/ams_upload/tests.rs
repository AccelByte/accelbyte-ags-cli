//! End-to-end pipeline tests against a mocked AMS, covering both the
//! single-shot and the multipart branch.
//!
//! The mock plays three roles at once — the AGS platform answering host
//! discovery, the AMS upload API, and the storage service behind the
//! pre-signed URLs — which is exactly the topology a real upload sees.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use ags_protocol::event::{ProgressEvent, ProgressSink};
use ags_protocol::output::AmsUploadView;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use crate::runtime::execution::ExecutionContext;

use super::{pipeline, TargetArchitecture, UploadRequest};

/// Records progress events so tests can assert the pipeline reports its stages.
#[derive(Default)]
struct RecordingSink {
    messages: Vec<String>,
    is_finished: bool,
}

impl ProgressSink for RecordingSink {
    fn on_event(&mut self, event: ProgressEvent) {
        match event {
            ProgressEvent::Started { message } | ProgressEvent::Message { text: message } => {
                self.messages.push(message)
            }
            ProgressEvent::Finished => self.is_finished = true,
            ProgressEvent::Page { .. } => {}
        }
    }
}

/// One body the storage service received, labelled with the `part` query
/// parameter the mock's pre-signed URLs carry.
type ReceivedPart = (String, Vec<u8>);

/// Captures every part body the storage service received.
#[derive(Clone, Default)]
struct PartRecorder {
    parts: Arc<Mutex<Vec<ReceivedPart>>>,
}

impl Respond for PartRecorder {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let label = request
            .url
            .query_pairs()
            .find(|(key, _)| key == "part")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_else(|| "whole".to_string());
        self.parts
            .lock()
            .unwrap()
            .push((label.clone(), request.body.clone()));
        ResponseTemplate::new(200).insert_header("ETag", format!("\"etag-{label}\"").as_str())
    }
}

impl PartRecorder {
    /// Bodies received, ordered by part number.
    fn sorted_bodies(&self) -> Vec<ReceivedPart> {
        let mut parts = self.parts.lock().unwrap().clone();
        parts.sort_by_key(|(label, _)| label.parse::<usize>().unwrap_or(0));
        parts
    }
}

/// Answers `PUT /multi-part/{uploadId}` with a storage URL tagged by the part
/// number in the request, so no test has to predict how many parts an archive
/// will split into.
struct PartPresigner {
    storage_base: String,
}

impl Respond for PartPresigner {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let part_number = body["partNo"].as_u64().expect("partNo must be sent");
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "url": format!("{}/storage?part={part_number}", self.storage_base),
        }))
    }
}

/// Responds with a failure status on the first call, then succeeds with a 200
/// and an ETag on all subsequent calls. Used by retry and re-sign tests to
/// prove the transport path handles a transient failure.
#[derive(Clone)]
struct FailOnceThenSucceed {
    call_count: Arc<Mutex<usize>>,
    fail_status: u16,
}

impl FailOnceThenSucceed {
    fn new(fail_status: u16) -> Self {
        Self {
            call_count: Arc::new(Mutex::new(0)),
            fail_status,
        }
    }

    fn total_calls(&self) -> usize {
        *self.call_count.lock().unwrap()
    }
}

impl Respond for FailOnceThenSucceed {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let mut count = self.call_count.lock().unwrap();
        *count += 1;
        if *count == 1 {
            ResponseTemplate::new(self.fail_status)
        } else {
            let label = request
                .url
                .query_pairs()
                .find(|(key, _)| key == "part")
                .map(|(_, value)| value.into_owned())
                .unwrap_or_else(|| "whole".to_string());
            ResponseTemplate::new(200).insert_header("ETag", format!("\"etag-{label}\"").as_str())
        }
    }
}

/// Build a small build directory with an x86-64 ELF entrypoint.
fn build_directory(payload_bytes: usize) -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let mut header = vec![0u8; 20];
    header[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    header[4] = 2;
    header[5] = 1;
    header[6] = 1;
    header[18..20].copy_from_slice(&62u16.to_le_bytes());
    std::fs::File::create(temp.path().join("server"))
        .unwrap()
        .write_all(&header)
        .unwrap();
    // Deterministic xorshift bytes: incompressible, so the archive size tracks
    // `payload_bytes` and the injected multipart threshold is reliably crossed.
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let payload: Vec<u8> = (0..payload_bytes)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    std::fs::File::create(temp.path().join("payload.bin"))
        .unwrap()
        .write_all(&payload)
        .unwrap();
    temp
}

/// An upload request pointing at `directory`, with defaults matching the CLI.
fn upload_request(directory: &Path) -> UploadRequest {
    UploadRequest {
        directory: directory.to_path_buf(),
        executable: "server".to_string(),
        image_name: "my-image".to_string(),
        target_architecture: None,
        include_symbol_files: false,
        skip_script_validation: false,
        upload_url_override: None,
        part_concurrency: 4,
        is_verbose: false,
    }
}

/// An execution context pointed at the mock, as the auth prologue would build.
fn context_for(server: &MockServer) -> ExecutionContext {
    ExecutionContext {
        base_url: server.uri(),
        access_token: "test-token".to_string(),
        ..ExecutionContext::default()
    }
}

/// Mount host discovery, image creation, and completion — the calls both
/// upload branches make.
async fn mount_common(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/ams/v1/upload-url"))
        .respond_with(ResponseTemplate::new(200).set_body_string(server.uri()))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload/v1/images"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": "img-1" })),
        )
        .mount(server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/upload/v1/complete"))
        .respond_with(ResponseTemplate::new(200))
        .mount(server)
        .await;
}

/// Run the pipeline against `server` with the multipart threshold set to
/// `chunk_bytes`.
async fn run(
    server: &MockServer,
    request: &UploadRequest,
    chunk_bytes: u64,
    sink: &mut RecordingSink,
) -> Result<AmsUploadView, super::AmsUploadError> {
    let client = reqwest::Client::new();
    pipeline::run_upload_with_chunk_size(
        &client,
        &client,
        &context_for(server),
        request,
        sink,
        chunk_bytes,
    )
    .await
}

#[tokio::test]
async fn test_single_shot_upload_completes() {
    let server = MockServer::start().await;
    mount_common(&server).await;
    let recorder = PartRecorder::default();
    Mock::given(method("POST"))
        .and(path("/upload/v1/pre-sign-url"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "url": format!("{}/storage?part=whole", server.uri()) }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/storage"))
        .respond_with(recorder.clone())
        .mount(&server)
        .await;

    let build = build_directory(1024);
    let mut sink = RecordingSink::default();
    let view = run(
        &server,
        &upload_request(build.path()),
        500 * 1024 * 1024,
        &mut sink,
    )
    .await
    .expect("upload should succeed");

    let AmsUploadView::Uploaded(result) = view else {
        panic!("expected an Uploaded view");
    };
    assert_eq!(result.image_id, "img-1");
    assert_eq!(result.target_architecture, "linux-x86_64");
    assert_eq!(result.command, "./server");
    assert_eq!(result.part_count, 1);
    assert_eq!(result.file_count, 2);
    assert!(sink.is_finished, "the pipeline must close its progress run");

    let bodies = recorder.sorted_bodies();
    assert_eq!(bodies.len(), 1, "one PUT for a single-shot upload");
    assert_eq!(bodies[0].1.len() as u64, result.archive_bytes);
}

#[tokio::test]
async fn test_multipart_upload_sends_ordered_parts_and_finalizes() {
    let server = MockServer::start().await;
    mount_common(&server).await;
    let recorder = PartRecorder::default();

    Mock::given(method("POST"))
        .and(path("/upload/v1/multi-part"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "uploadId": "up-1" })),
        )
        .mount(&server)
        .await;
    // Each pre-sign echoes its requested part number into the storage URL, so
    // the recorder can prove which bytes were sent as which part.
    Mock::given(method("PUT"))
        .and(path("/upload/v1/multi-part/up-1"))
        .respond_with(PartPresigner {
            storage_base: server.uri(),
        })
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/storage"))
        .respond_with(recorder.clone())
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/upload/v1/multi-part/up-1/finalize"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let build = build_directory(64 * 1024);
    let mut sink = RecordingSink::default();
    // A chunk size that splits the archive of ~64 KiB of incompressible bytes
    // into several parts.
    let chunk_bytes = 20 * 1024;
    let view = run(
        &server,
        &upload_request(build.path()),
        chunk_bytes,
        &mut sink,
    )
    .await
    .expect("multipart upload should succeed");

    let AmsUploadView::Uploaded(result) = view else {
        panic!("expected an Uploaded view");
    };
    assert!(
        result.part_count > 1,
        "the fixture must cross the multipart threshold, got {} bytes",
        result.archive_bytes
    );

    let bodies = recorder.sorted_bodies();
    assert_eq!(bodies.len(), result.part_count);
    let reassembled: Vec<u8> = bodies.iter().flat_map(|(_, body)| body.clone()).collect();
    assert_eq!(
        reassembled.len() as u64,
        result.archive_bytes,
        "the parts must cover the archive exactly once"
    );

    // Every part but the last is exactly one chunk; that invariant is what
    // makes an ETag list ordered by part number reassemble correctly.
    for (label, body) in bodies.iter().take(result.part_count - 1) {
        assert_eq!(body.len() as u64, chunk_bytes, "part {label} is short");
    }

    let finalize = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|request| request.url.path().ends_with("/finalize"))
        .expect("finalize must be called");
    let body: serde_json::Value = serde_json::from_slice(&finalize.body).unwrap();
    let parts: Vec<&str> = body["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    let expected: Vec<String> = (1..=result.part_count)
        .map(|part_number| format!("\"etag-{part_number}\""))
        .collect();
    assert_eq!(parts, expected, "ETags must be ordered by part number");
}

#[tokio::test]
async fn test_host_discovery_failure_is_fatal() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ams/v1/upload-url"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let build = build_directory(64);
    let mut sink = RecordingSink::default();
    let error = run(
        &server,
        &upload_request(build.path()),
        500 * 1024 * 1024,
        &mut sink,
    )
    .await
    .expect_err("a failed discovery must not fall back to production");
    assert!(matches!(
        error,
        super::AmsUploadError::UploadHostUnresolved { .. }
    ));
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "nothing is created once discovery fails"
    );
}

#[tokio::test]
async fn test_upload_url_override_skips_discovery() {
    let server = MockServer::start().await;
    mount_common(&server).await;
    Mock::given(method("POST"))
        .and(path("/upload/v1/pre-sign-url"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "url": format!("{}/storage?part=whole", server.uri()) }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/storage"))
        .respond_with(PartRecorder::default())
        .mount(&server)
        .await;

    let build = build_directory(64);
    let mut request = upload_request(build.path());
    request.upload_url_override = Some(server.uri());
    // Point the platform base URL somewhere unroutable: if discovery ran, the
    // upload would fail rather than succeed.
    let mut sink = RecordingSink::default();
    let client = reqwest::Client::new();
    let context = ExecutionContext {
        base_url: "https://platform.invalid".to_string(),
        access_token: "test-token".to_string(),
        ..ExecutionContext::default()
    };
    let view = pipeline::run_upload_with_chunk_size(
        &client,
        &client,
        &context,
        &request,
        &mut sink,
        500 * 1024 * 1024,
    )
    .await
    .expect("an explicit --upload-url must not need discovery");
    assert!(matches!(view, AmsUploadView::Uploaded(_)));
}

#[tokio::test]
async fn test_image_creation_failure_surfaces_the_ams_error_message() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ams/v1/upload-url"))
        .respond_with(ResponseTemplate::new(200).set_body_string(server.uri()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload/v1/images"))
        .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
            "errorCode": 12345,
            "errorMessage": "image name already in use",
        })))
        .mount(&server)
        .await;

    let build = build_directory(64);
    let mut sink = RecordingSink::default();
    let error = run(
        &server,
        &upload_request(build.path()),
        500 * 1024 * 1024,
        &mut sink,
    )
    .await
    .expect_err("a 409 must fail the upload");
    let message = error.to_string();
    assert!(
        message.contains("image name already in use"),
        "unexpected message: {message}"
    );
}

/// Once `create_image` has succeeded the record exists in the namespace, so a
/// later failure has to name it. Without this the image is only discoverable by
/// listing the namespace and noticing something incomplete.
#[tokio::test]
async fn test_failure_after_image_creation_names_the_orphaned_image() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ams/v1/upload-url"))
        .respond_with(ResponseTemplate::new(200).set_body_string(server.uri()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload/v1/images"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": "img-1" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload/v1/pre-sign-url"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let build = build_directory(64);
    let mut sink = RecordingSink::default();
    let error = run(
        &server,
        &upload_request(build.path()),
        500 * 1024 * 1024,
        &mut sink,
    )
    .await
    .expect_err("a 500 on pre-sign must fail the upload");

    let message = ags_protocol::error::RuntimeError::from(error).message;
    assert!(message.contains("img-1"), "{message}");
    assert!(message.contains("my-image"), "{message}");
    assert!(message.contains("mark-for-deletion"), "{message}");
}

/// A shell-script entrypoint must reach AMS with the architecture the user
/// declared, since there is nothing in the file to detect.
#[tokio::test]
async fn test_shell_script_entrypoint_sends_the_declared_architecture() {
    let server = MockServer::start().await;
    mount_common(&server).await;
    Mock::given(method("POST"))
        .and(path("/upload/v1/pre-sign-url"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "url": format!("{}/storage?part=whole", server.uri()) }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/storage"))
        .respond_with(PartRecorder::default())
        .mount(&server)
        .await;

    let build = build_directory(64);
    std::fs::write(
        build.path().join("start.sh"),
        b"#!/bin/bash\nexec ./server\n",
    )
    .unwrap();
    let mut request = upload_request(build.path());
    request.executable = "start.sh".to_string();
    request.target_architecture = Some(TargetArchitecture::LinuxArm64);

    let mut sink = RecordingSink::default();
    let view = run(&server, &request, 500 * 1024 * 1024, &mut sink)
        .await
        .expect("upload should succeed");
    let AmsUploadView::Uploaded(result) = view else {
        panic!("expected an Uploaded view");
    };
    assert_eq!(result.command, "./start.sh");

    let create = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|request| request.url.path() == "/upload/v1/images")
        .expect("the image must be created");
    let body: serde_json::Value = serde_json::from_slice(&create.body).unwrap();
    assert_eq!(body["targetArchitecture"], "linux-arm_64");
    assert_eq!(body["format"], "tgz");
    assert_eq!(body["tags"], serde_json::json!([]));

    let expected_source = url::Url::parse(&server.uri())
        .unwrap()
        .host_str()
        .unwrap()
        .to_string();
    assert_eq!(
        create
            .headers
            .get("ams-source-environment")
            .and_then(|value| value.to_str().ok()),
        Some(expected_source.as_str()),
        "AMS must be told which platform the image came from"
    );
}

/// `--verbose` must report the failing call. Without this the flag is silent on
/// exactly the runs where a user most needs to know what was attempted.
#[tokio::test]
async fn test_verbose_attaches_the_failing_request_to_the_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ams/v1/upload-url"))
        .respond_with(ResponseTemplate::new(200).set_body_string(server.uri()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload/v1/images"))
        .respond_with(ResponseTemplate::new(403).set_body_string("nope"))
        .mount(&server)
        .await;

    let build = build_directory(64);
    let mut request = upload_request(build.path());
    request.is_verbose = true;

    let mut sink = RecordingSink::default();
    let error = run(&server, &request, 500 * 1024 * 1024, &mut sink)
        .await
        .expect_err("a 403 must fail the upload");
    let runtime_error: ags_protocol::error::RuntimeError = error.into();

    let trace = runtime_error.trace.expect("verbose must attach a trace");
    assert_eq!(trace.request.http_method, "POST");
    assert!(
        trace.request.url.ends_with("/upload/v1/images"),
        "{}",
        trace.request.url
    );
    assert!(trace.request.has_auth_header);
    assert_eq!(trace.response.expect("response trace").status, 403);
}

/// Without `--verbose` no trace is attached, so ordinary failures stay concise.
#[tokio::test]
async fn test_non_verbose_run_attaches_no_trace() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ams/v1/upload-url"))
        .respond_with(ResponseTemplate::new(200).set_body_string(server.uri()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/upload/v1/images"))
        .respond_with(ResponseTemplate::new(403).set_body_string("nope"))
        .mount(&server)
        .await;

    let build = build_directory(64);
    let mut sink = RecordingSink::default();
    let error = run(
        &server,
        &upload_request(build.path()),
        500 * 1024 * 1024,
        &mut sink,
    )
    .await
    .expect_err("a 403 must fail the upload");
    let runtime_error: ags_protocol::error::RuntimeError = error.into();
    assert!(runtime_error.trace.is_none());
}

/// A transient 500 from the storage service is retried by `put_file_range`,
/// and the upload succeeds once the storage recovers.
///
/// This test exercises a real backoff sleep: `UPLOAD_RETRY_BASE_DELAY` is 500 ms,
/// so the single transient failure adds ~500 ms of wall time. One failure is
/// enough to prove the retry path; avoid adding more retry-path tests without
/// accounting for the cumulative delay.
#[tokio::test]
async fn test_retry_on_server_error_succeeds_after_transient_failure() {
    let server = MockServer::start().await;
    mount_common(&server).await;

    Mock::given(method("POST"))
        .and(path("/upload/v1/pre-sign-url"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "url": format!("{}/storage?part=whole", server.uri()) }),
        ))
        .mount(&server)
        .await;

    let storage = FailOnceThenSucceed::new(500);
    Mock::given(method("PUT"))
        .and(path("/storage"))
        .respond_with(storage.clone())
        .mount(&server)
        .await;

    let build = build_directory(1024);
    let mut sink = RecordingSink::default();
    let view = run(
        &server,
        &upload_request(build.path()),
        500 * 1024 * 1024,
        &mut sink,
    )
    .await
    .expect("upload must succeed after retrying the transient 500");

    assert!(matches!(view, AmsUploadView::Uploaded(_)));
    assert_eq!(
        storage.total_calls(),
        2,
        "storage must receive exactly 2 PUTs (one 500 + one 200), proving exactly one retry"
    );
}

/// A 401 from the storage service triggers one re-sign of the pre-signed URL
/// in `upload_whole`, and the upload succeeds on the second attempt.
#[tokio::test]
async fn test_resign_on_expiry_single_shot() {
    let server = MockServer::start().await;
    mount_common(&server).await;

    Mock::given(method("POST"))
        .and(path("/upload/v1/pre-sign-url"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "url": format!("{}/storage?part=whole", server.uri()) }),
        ))
        .mount(&server)
        .await;

    let storage = FailOnceThenSucceed::new(401);
    Mock::given(method("PUT"))
        .and(path("/storage"))
        .respond_with(storage.clone())
        .mount(&server)
        .await;

    let build = build_directory(1024);
    let mut sink = RecordingSink::default();
    let view = run(
        &server,
        &upload_request(build.path()),
        500 * 1024 * 1024,
        &mut sink,
    )
    .await
    .expect("upload must succeed after re-signing the expired URL");

    assert!(matches!(view, AmsUploadView::Uploaded(_)));

    let presign_calls = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/upload/v1/pre-sign-url")
        .count();
    assert_eq!(
        presign_calls, 2,
        "presign_url must be called exactly twice (initial + re-sign)"
    );
}

/// A 401 on a multipart part triggers one re-sign via `presign_part`, and the
/// affected part succeeds on the second attempt.
#[tokio::test]
async fn test_resign_on_expiry_multipart_part() {
    let server = MockServer::start().await;
    mount_common(&server).await;

    Mock::given(method("POST"))
        .and(path("/upload/v1/multi-part"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({ "uploadId": "up-1" })),
        )
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/upload/v1/multi-part/up-1"))
        .respond_with(PartPresigner {
            storage_base: server.uri(),
        })
        .mount(&server)
        .await;

    let storage = FailOnceThenSucceed::new(401);
    Mock::given(method("PUT"))
        .and(path("/storage"))
        .respond_with(storage.clone())
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/upload/v1/multi-part/up-1/finalize"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let build = build_directory(64 * 1024);
    let mut request = upload_request(build.path());
    request.part_concurrency = 1;
    let chunk_bytes = 20 * 1024;
    let mut sink = RecordingSink::default();
    let view = run(&server, &request, chunk_bytes, &mut sink)
        .await
        .expect("multipart upload must succeed after re-signing the expired part");

    let AmsUploadView::Uploaded(result) = view else {
        panic!("expected an Uploaded view");
    };
    assert!(
        result.part_count > 1,
        "fixture must produce multiple parts, got {} bytes",
        result.archive_bytes
    );

    // Count presign_part calls: one initial per part, plus one re-sign for the
    // affected part.
    let presign_part_calls = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/upload/v1/multi-part/up-1")
        .count();
    assert_eq!(
        presign_part_calls,
        result.part_count + 1,
        "presign_part must be called part_count + 1 times (one extra re-sign \
         for the affected part)"
    );
}

/// An `http://` upload host emits a plaintext warning through the progress
/// sink so the user knows the access token and archive travel unencrypted.
#[tokio::test]
async fn test_http_upload_host_emits_plaintext_warning() {
    let server = MockServer::start().await;
    mount_common(&server).await;

    Mock::given(method("POST"))
        .and(path("/upload/v1/pre-sign-url"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({ "url": format!("{}/storage?part=whole", server.uri()) }),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/storage"))
        .respond_with(PartRecorder::default())
        .mount(&server)
        .await;

    let build = build_directory(1024);
    let mut sink = RecordingSink::default();
    run(
        &server,
        &upload_request(build.path()),
        500 * 1024 * 1024,
        &mut sink,
    )
    .await
    .expect("upload should succeed");

    assert!(
        sink.messages.iter().any(|m| m.contains("unencrypted")),
        "a plaintext upload host must trigger a warning; got messages: {:?}",
        sink.messages
    );
}

/// An `https://` upload host does not emit the plaintext warning.
#[tokio::test]
async fn test_https_upload_host_does_not_emit_plaintext_warning() {
    let build = build_directory(1024);
    let mut request = upload_request(build.path());
    request.upload_url_override = Some("https://127.0.0.1:1".to_string());

    let client = reqwest::Client::new();
    let context = ExecutionContext {
        base_url: "https://platform.invalid".to_string(),
        access_token: "test-token".to_string(),
        ..ExecutionContext::default()
    };
    let mut sink = RecordingSink::default();
    // The pipeline will fail at image creation (no server at 127.0.0.1:1),
    // but the warning check happens earlier — its absence is what we verify.
    let _ = pipeline::run_upload_with_chunk_size(
        &client,
        &client,
        &context,
        &request,
        &mut sink,
        500 * 1024 * 1024,
    )
    .await;

    assert!(
        !sink.messages.iter().any(|m| m.contains("unencrypted")),
        "an https upload host must not trigger a plaintext warning; got: {:?}",
        sink.messages
    );
}

#[test]
fn test_plan_upload_rejects_an_invalid_upload_url_override() {
    let build = build_directory(64);
    let mut request = upload_request(build.path());
    request.upload_url_override = Some("not-a-url".to_string());

    let error = pipeline::plan_upload(&request).unwrap_err();

    assert!(
        matches!(error, super::AmsUploadError::UploadHostInvalid(ref value) if value == "not-a-url"),
        "expected UploadHostInvalid(\"not-a-url\"), got: {error:?}"
    );
}

#[test]
fn test_plan_upload_normalises_a_trailing_slash_in_the_upload_url_override() {
    let build = build_directory(64);
    let mut request = upload_request(build.path());
    request.upload_url_override = Some("https://prod.ams.accelbyte.io/".to_string());

    let view = pipeline::plan_upload(&request).unwrap();

    let AmsUploadView::Planned(plan) = view else {
        panic!("expected a Planned view, got: {view:?}");
    };
    assert_eq!(
        plan.upload_base_url.as_deref(),
        Some("https://prod.ams.accelbyte.io")
    );
}
