use crate::errors::CliError;
use crate::invocation::flags;
use ags_protocol::catalogue::{OperationSchema, ParameterLocation};
use ags_protocol::request::{CommandRequest, OutputFormat};
use ags_runtime::catalogue::ServiceId;
use std::collections::BTreeMap;

/// Maximum byte length accepted for JSON loaded from a file or stdin.
/// Bounds memory use against accidentally-huge inputs (e.g. `/dev/zero`,
/// runaway pipes, mistyped paths pointing at large logs).
const MAX_JSON_INPUT_BYTES: usize = 10 * 1024 * 1024;

/// Source of a `--json` payload, used to shape size and emptiness errors.
/// `File` carries a pre-sanitised path so `validate_json_input` never needs to
/// sanitise again — the input to this enum is the display-safe value.
enum JsonSource {
    File(String),
    Stdin,
}

/// Outcome of resolving a request body from CLI flags alone.
#[derive(Debug, PartialEq)]
pub(super) enum BodyResolution {
    /// User provided a body via --json; here it is.
    Explicit(serde_json::Value),
    /// Operation requires a body and none was provided. The body fields are
    /// left unset; the workflow executor's `gather_workflow_inputs` collects
    /// the missing fields per-input (line-by-line on stdin in human mode).
    NeedsGathering,
    /// Operation does not require a body.
    None,
}

/// Resolve the request body strictly from flags (no interactive prompting).
/// Errors out if a required body is missing and `--no-input` forbids prompting.
pub(super) fn resolve_request_body(
    operation: &ags_protocol::catalogue::OperationSchema,
    arg_matches: &clap::ArgMatches,
    is_no_input: bool,
) -> Result<BodyResolution, CliError> {
    if operation.request_body.is_none() {
        return Ok(BodyResolution::None);
    }
    match arg_matches.get_one::<String>("json") {
        Some(raw) => {
            let resolved = resolve_json_arg(raw, is_no_input)?;
            let value = serde_json::from_str::<serde_json::Value>(&resolved).map_err(|e| {
                CliError::Usage {
                    message: format!("Invalid JSON for --json: {e}"),
                    metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                        "Check the JSON syntax, or run with --skeleton to see the expected shape. \
                         To avoid shell quoting issues, use --json @file.json or --json @- (stdin).",
                    ))),
                }
            })?;
            Ok(BodyResolution::Explicit(value))
        }
        None => {
            if is_no_input {
                Err(CliError::Usage {
                    message: "Missing required request body".to_string(),
                    metadata: Some(Box::new(crate::errors::ErrorMetadata {
                        reason: Some("Interactive input is disabled (--no-input).".to_string()),
                        suggestion: Some(
                            "Pass --json '<value>' (or --json @file.json), or drop --no-input \
                             to provide the body interactively."
                                .to_string(),
                        ),
                        ..Default::default()
                    })),
                })
            } else {
                Ok(BodyResolution::NeedsGathering)
            }
        }
    }
}

/// Translate Clap `ArgMatches` into a protocol-layer `CommandRequest`.
pub(super) fn build_command_request(
    operation: &OperationSchema,
    arg_matches: &clap::ArgMatches,
    flags: &flags::GlobalFlags,
    output_format: OutputFormat,
    service_id: ServiceId,
    body: Option<serde_json::Value>,
) -> Result<CommandRequest, CliError> {
    let mut path_params: BTreeMap<String, String> = BTreeMap::new();
    let mut query_params: BTreeMap<String, String> = BTreeMap::new();
    let mut header_params: BTreeMap<String, String> = BTreeMap::new();

    for parameter in &operation.parameters {
        let value = match arg_matches.get_one::<String>(parameter.name.as_str()) {
            Some(value) => value,
            None => continue,
        };
        match parameter.location {
            ParameterLocation::Path => {
                path_params.insert(parameter.name.clone(), value.clone());
            }
            ParameterLocation::Query => {
                query_params.insert(parameter.name.clone(), value.clone());
            }
            ParameterLocation::Header => {
                header_params.insert(parameter.name.clone(), value.clone());
            }
            ParameterLocation::Body | ParameterLocation::FormData => {
                // Body-parameter metadata is consumed via operation.request_body;
                // formData is treated as body-shaped. Neither appears as a flag.
            }
        }
    }

    let pagination = flags.pagination_hint();

    Ok(CommandRequest {
        service: service_id,
        operation_id: operation.id.clone(),
        namespace: None,
        path_params,
        query_params,
        header_params,
        body,
        output_format,
        pagination,
        verbosity: flags.verbosity,
        output: flags.output.clone(),
    })
}

/// Return a `--json @<path>` example using the host's native path separator.
fn json_file_example() -> &'static str {
    if cfg!(windows) {
        "--json @path\\to\\file.json"
    } else {
        "--json @path/to/file.json"
    }
}

/// Resolve the raw `--json` argument value to a JSON string.
///
/// - `@-`          -> read all of stdin (rejected when `is_no_input` is true)
/// - `@<path>`     -> read the file at `<path>` (capped at `MAX_JSON_INPUT_BYTES`)
/// - anything else -> returned unchanged
fn resolve_json_arg(raw: &str, is_no_input: bool) -> Result<String, CliError> {
    if raw == "@-" {
        if is_no_input {
            return Err(CliError::Usage {
                message: "--json @- reads from stdin, which is not available with --no-input"
                    .to_string(),
                metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                    format!(
                        "Use {} or pass JSON inline with --json '{{...}}'",
                        json_file_example()
                    ),
                ))),
            });
        }
        return read_stdin_json();
    }
    if let Some(path) = raw.strip_prefix('@') {
        if path.is_empty() {
            return Err(CliError::Usage {
                message: format!(
                    "Missing path after '@': use {} or --json @- for stdin",
                    json_file_example()
                ),
                metadata: None,
            });
        }
        return read_json_file(path);
    }
    Ok(raw.to_string())
}

/// Open a `--json @<path>` file and run it through the size/UTF-8/JSON validators.
fn read_json_file(path: &str) -> Result<String, CliError> {
    // `safe_path` is for display only — never pass it to the filesystem.
    // The OS is authoritative for path interpretation; stripping control
    // characters for display avoids terminal injection in error output.
    let safe_path = ags_runtime::support::strings::strip_terminal_control_sequences(path);
    let file = std::fs::File::open(path).map_err(|e| CliError::Usage {
        message: format!("Failed to read --json file '{safe_path}': {e}"),
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            "Check the path is correct and readable, or pass JSON inline with --json '{...}'",
        ))),
    })?;
    read_and_validate(file, JsonSource::File(safe_path))
}

/// Read JSON from stdin for `--json @-`, refusing to block when stdin is a TTY.
fn read_stdin_json() -> Result<String, CliError> {
    let stdin = std::io::stdin();
    let locked = stdin.lock();
    if ags_runtime::support::is_stdin_tty() {
        return Err(CliError::Usage {
            message: "--json @- is waiting on stdin but no data is being piped in".to_string(),
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                "Pipe a JSON document into the command, or use --json @path or --json '{...}'",
            ))),
        });
    }
    read_and_validate(locked, JsonSource::Stdin)
}

/// Read a JSON payload from any `Read` source and run the size/emptiness guards.
///
/// Reads into a `Vec<u8>` (capped at `MAX_JSON_INPUT_BYTES + 1`) so we can
/// distinguish three distinct failures with the right message:
/// 1. I/O error during read
/// 2. Size exceeded (checked before UTF-8 decode, so a payload that trips the
///    cap mid-codepoint reports as "too large", not "non-UTF-8")
/// 3. UTF-8 decode error
fn read_and_validate<R: std::io::Read>(reader: R, source: JsonSource) -> Result<String, CliError> {
    use std::io::Read;
    let mut bytes = Vec::new();
    reader
        .take(MAX_JSON_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| read_error(&source, e))?;
    if bytes.len() > MAX_JSON_INPUT_BYTES {
        return Err(size_exceeded_error(&source));
    }
    let raw_json = String::from_utf8(bytes).map_err(|_| {
        read_error(
            &source,
            std::io::Error::new(std::io::ErrorKind::InvalidData, "non-UTF-8 bytes"),
        )
    })?;
    validate_json_input(raw_json, source)
}

/// Build the user-facing error returned when a `--json` payload exceeds the size cap.
fn size_exceeded_error(source: &JsonSource) -> CliError {
    let limit_mib = MAX_JSON_INPUT_BYTES / (1024 * 1024);
    let message = match source {
        JsonSource::File(safe_path) => {
            format!("--json file '{safe_path}' exceeds the {limit_mib} MiB size limit")
        }
        JsonSource::Stdin => format!("JSON from stdin exceeds the {limit_mib} MiB size limit"),
    };
    CliError::Usage {
        message,
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            "Split the request into smaller batches, or send the body inline",
        ))),
    }
}

/// Translate an I/O or UTF-8 error reading a `--json` payload into a `CliError` with a fix suggestion.
fn read_error(source: &JsonSource, e: std::io::Error) -> CliError {
    let is_utf8 = e.kind() == std::io::ErrorKind::InvalidData;
    let (message, suggestion) = match source {
        JsonSource::File(safe_path) => {
            let message = if is_utf8 {
                format!(
                    "--json file '{safe_path}' contains non-UTF-8 bytes; JSON must be UTF-8 encoded"
                )
            } else {
                format!("Failed to read --json file '{safe_path}': {e}")
            };
            (
                message,
                "Check the path is correct and readable, or pass JSON inline with --json '{...}'",
            )
        }
        JsonSource::Stdin => {
            let message = if is_utf8 {
                "--json from stdin contains non-UTF-8 bytes; JSON must be UTF-8 encoded".to_string()
            } else {
                format!("Failed to read --json from stdin: {e}")
            };
            (
                message,
                "Pipe valid JSON into the command, or use --json @path or --json '{...}'",
            )
        }
    };
    CliError::Usage {
        message,
        metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
            suggestion,
        ))),
    }
}

/// Apply size and emptiness checks to a JSON payload after it has been read.
/// `read_and_validate` already enforces the size cap before calling this, but
/// the check is repeated here so direct callers (primarily unit tests) get
/// the same guard.
fn validate_json_input(raw_json: String, source: JsonSource) -> Result<String, CliError> {
    if raw_json.len() > MAX_JSON_INPUT_BYTES {
        return Err(size_exceeded_error(&source));
    }
    if raw_json.trim().is_empty() {
        let (message, suggestion) = match &source {
            JsonSource::File(safe_path) => (
                format!("--json file '{safe_path}' is empty or contains only whitespace"),
                "Ensure the file contains a valid JSON document, or use --skeleton to see the expected shape",
            ),
            JsonSource::Stdin => (
                "Expected JSON on stdin but got empty input".to_string(),
                "Pipe a JSON document into the command, or use --json @path or --json '{...}'",
            ),
        };
        return Err(CliError::Usage {
            message,
            metadata: Some(Box::new(crate::errors::ErrorMetadata::with_suggestion(
                suggestion,
            ))),
        });
    }
    Ok(raw_json)
}

#[cfg(test)]
mod tests {
    use super::{resolve_json_arg, validate_json_input, JsonSource, MAX_JSON_INPUT_BYTES};
    use std::io::Write;

    #[test]
    fn test_inline_value_passes_through() {
        let out = resolve_json_arg(r#"{"a":1}"#, false).unwrap();
        assert_eq!(out, r#"{"a":1}"#);
    }

    #[test]
    fn test_at_prefix_reads_file() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, r#"{{"b":2}}"#).unwrap();
        let arg = format!("@{}", f.path().display());
        let out = resolve_json_arg(&arg, false).unwrap();
        assert!(out.trim() == r#"{"b":2}"#);
    }

    #[test]
    fn test_missing_file_returns_usage_error() {
        let err = resolve_json_arg("@/definitely/not/here.json", false).unwrap_err();
        match err {
            crate::errors::CliError::Usage { message, .. } => {
                assert!(
                    message.contains("Failed to read --json file"),
                    "unexpected message: {message}"
                );
                assert!(message.contains("/definitely/not/here.json"));
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn test_empty_at_prefix_returns_guidance_error() {
        let err = resolve_json_arg("@", false).unwrap_err();
        match err {
            crate::errors::CliError::Usage { message, .. } => {
                assert!(
                    message.contains("Missing path after '@'"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn test_buffer_at_exact_limit_passes_size_guard() {
        let raw_json = "x".repeat(MAX_JSON_INPUT_BYTES);
        validate_json_input(raw_json, JsonSource::Stdin)
            .expect("size guard must accept exact-limit");
    }

    #[test]
    fn test_stdin_rejected_when_no_input_set() {
        let err = resolve_json_arg("@-", true).unwrap_err();
        match err {
            crate::errors::CliError::Usage { message, .. } => {
                assert!(
                    message.contains("not available with --no-input"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn test_empty_file_returns_usage_error() {
        let f = tempfile::NamedTempFile::new().unwrap();
        let arg = format!("@{}", f.path().display());
        let err = resolve_json_arg(&arg, false).unwrap_err();
        match err {
            crate::errors::CliError::Usage { message, .. } => {
                assert!(
                    message.contains("is empty"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn test_empty_stdin_input_returns_usage_error() {
        let err = validate_json_input(String::new(), JsonSource::Stdin).unwrap_err();
        match err {
            crate::errors::CliError::Usage { message, .. } => {
                assert!(
                    message.contains("Expected JSON on stdin but got empty input"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn test_oversized_stdin_input_returns_usage_error() {
        let raw_json = "x".repeat(MAX_JSON_INPUT_BYTES + 1);
        let err = validate_json_input(raw_json, JsonSource::Stdin).unwrap_err();
        match err {
            crate::errors::CliError::Usage { message, .. } => {
                assert!(
                    message.contains("JSON from stdin exceeds"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn test_oversized_file_rejected() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        let oversized = vec![b'a'; MAX_JSON_INPUT_BYTES + 1];
        f.write_all(&oversized).unwrap();
        let arg = format!("@{}", f.path().display());
        let err = resolve_json_arg(&arg, false).unwrap_err();
        match err {
            crate::errors::CliError::Usage { message, .. } => {
                assert!(
                    message.contains("exceeds the") && message.contains("MiB size limit"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    /// `@-` with `--no-input` is rejected with a suggestion that includes
    /// the host-native path example. Pinning the format prevents a future
    /// refactor from silently dropping the example.
    #[test]
    fn test_at_dash_with_no_input_suggests_path_example() {
        let err = resolve_json_arg("@-", true).unwrap_err();
        let rendered = format!("{err:?}");
        assert!(rendered.contains("@path"), "rendered: {rendered}");
        assert!(rendered.contains("file.json"), "rendered: {rendered}");
    }

    /// Empty path after `@` produces a Usage error mentioning the path
    /// example. Same regression-pinning rationale as above.
    #[test]
    fn test_at_with_empty_path_mentions_path_example() {
        let err = resolve_json_arg("@", false).unwrap_err();
        let rendered = format!("{err:?}");
        assert!(rendered.contains("@path"), "rendered: {rendered}");
        assert!(rendered.contains("file.json"), "rendered: {rendered}");
    }
}

#[cfg(test)]
mod resolve_tests {
    use super::*;
    use ags_protocol::catalogue::{
        ApiVersion, BodyField, BodyFieldType, BodySchema, HttpMethod, MutationClass, OperationId,
        OperationSchema,
    };

    /// Build a sample operation schema that declares a required request body.
    fn op_with_body() -> OperationSchema {
        OperationSchema {
            id: OperationId::new("test-create"),
            name: "create".to_string(),
            summary: "Create something".to_string(),
            description: None,
            mutation_class: MutationClass::Mutating,
            http_method: HttpMethod::Post,
            path_template: "/test/v1/items".to_string(),
            parameters: vec![],
            request_body: Some(BodySchema {
                item_type: None,
                is_array: false,
                definition_name: "ModelTestRequest".to_string(),
                fields: vec![BodyField {
                    name: "name".to_string(),
                    field_type: BodyFieldType::String,
                    required: true,
                    description: None,
                    children: vec![],
                    default: None,
                }],
            }),
            response: None,
            permissions: vec![],
            scope: "admin".to_string(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
            has_file_upload: false,
        }
    }

    /// Build a sample operation schema with no request body.
    fn op_without_body() -> OperationSchema {
        OperationSchema {
            id: OperationId::new("test-get"),
            name: "get".to_string(),
            summary: "Get something".to_string(),
            description: None,
            mutation_class: MutationClass::ReadOnly,
            http_method: HttpMethod::Get,
            path_template: "/test/v1/items/{id}".to_string(),
            parameters: vec![],
            request_body: None,
            response: None,
            permissions: vec![],
            scope: "admin".to_string(),
            api_version: ApiVersion(1),
            deprecated: false,
            response_content_type: None,
            has_file_upload: false,
        }
    }

    /// Build `ArgMatches` with `--json` set to the given value.
    fn matches_with_json(value: &str) -> clap::ArgMatches {
        clap::Command::new("x")
            .arg(clap::Arg::new("json").long("json").num_args(1))
            .try_get_matches_from(vec!["x", "--json", value])
            .unwrap()
    }

    /// Build `ArgMatches` with no `--json` flag set.
    fn matches_without_json() -> clap::ArgMatches {
        clap::Command::new("x")
            .arg(clap::Arg::new("json").long("json").num_args(1))
            .try_get_matches_from(vec!["x"])
            .unwrap()
    }

    #[test]
    fn test_resolve_body_explicit_json_returns_value() {
        let out = resolve_request_body(&op_with_body(), &matches_with_json(r#"{"foo":1}"#), false)
            .unwrap();
        assert_eq!(out, BodyResolution::Explicit(serde_json::json!({"foo": 1})));
    }

    #[test]
    fn test_resolve_body_missing_returns_needs_gathering() {
        let out = resolve_request_body(&op_with_body(), &matches_without_json(), false).unwrap();
        assert_eq!(out, BodyResolution::NeedsGathering);
    }

    #[test]
    fn test_resolve_body_missing_with_no_input_errors() {
        let err = resolve_request_body(&op_with_body(), &matches_without_json(), true)
            .expect_err("expected Usage error");
        match err {
            crate::errors::CliError::Usage { message, metadata } => {
                assert_eq!(message, "Missing required request body");
                let suggestion = metadata
                    .and_then(|m| m.suggestion.clone())
                    .expect("expected a suggestion directing the user to --json");
                assert!(
                    suggestion.contains("--json"),
                    "unexpected suggestion: {suggestion}"
                );
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn test_resolve_body_when_operation_has_no_body_returns_none() {
        let out = resolve_request_body(&op_without_body(), &matches_with_json(r#"{"x":1}"#), false)
            .unwrap();
        // Operation has no body → explicit input is ignored.
        assert_eq!(out, BodyResolution::None);
    }

    /// `build_command_request` stores the supplied protocol output format
    /// verbatim. The inline-surface→plain final-render resolution itself is
    /// covered in `context.rs`.
    #[test]
    fn test_build_command_request_uses_supplied_human_format() {
        let request = build_command_request(
            &op_without_body(),
            &matches_without_json(),
            &flags::GlobalFlags::default(),
            OutputFormat::Human,
            ServiceId::new("iam"),
            None,
        )
        .unwrap();
        assert_eq!(request.output_format, OutputFormat::Human);
    }

    /// JSON mode threads `OutputFormat::Json` straight into the request.
    #[test]
    fn test_build_command_request_uses_supplied_json_format() {
        let request = build_command_request(
            &op_without_body(),
            &matches_without_json(),
            &flags::GlobalFlags::default(),
            OutputFormat::Json,
            ServiceId::new("iam"),
            None,
        )
        .unwrap();
        assert_eq!(request.output_format, OutputFormat::Json);
    }
}
