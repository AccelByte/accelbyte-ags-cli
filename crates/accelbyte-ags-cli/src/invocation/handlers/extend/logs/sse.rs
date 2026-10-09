//! Pure incremental Server-Sent Events parser. No I/O.

/// Maximum length of a single line in bytes (not counting its line ending).
/// A line of exactly this length is accepted; one more byte is the error.
pub(crate) const MAX_LINE_BYTES: usize = 256 * 1024;

/// A complete SSE event dispatched by the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SseEvent {
    pub(crate) name: Option<String>,
    pub(crate) id: Option<String>,
    pub(crate) data: String,
}

/// Semantic classification of an SSE event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SseEventKind<'a> {
    /// A log line: no `event` field or `event: message`.
    LogLine,
    /// `event: pod-done`
    PodDone,
    /// `event: stream-end`
    StreamEnd,
    /// `event: error`
    Error,
    /// Any other named event.
    Other(&'a str),
}

impl SseEvent {
    /// Classify this event by its name.
    pub(crate) fn kind(&self) -> SseEventKind<'_> {
        match self.name.as_deref() {
            None | Some("message") => SseEventKind::LogLine,
            Some("pod-done") => SseEventKind::PodDone,
            Some("stream-end") => SseEventKind::StreamEnd,
            Some("error") => SseEventKind::Error,
            Some(other) => SseEventKind::Other(other),
        }
    }
}

/// Error from the SSE reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SseError {
    /// A single line exceeded [`MAX_LINE_BYTES`].
    LineOverLimit,
}

impl std::fmt::Display for SseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SseError::LineOverLimit => write!(
                f,
                "A single line exceeded the {}-byte limit.",
                MAX_LINE_BYTES
            ),
        }
    }
}

impl std::error::Error for SseError {}

/// Incremental SSE parser. Feed arbitrary byte chunks and receive complete events.
pub(crate) struct SseReader {
    /// Accumulated bytes for the current line (before the line ending).
    line_buf: Vec<u8>,
    /// Whether the previous byte was CR — if the next byte is LF, it is the
    /// same line ending, not a second one.
    prev_cr: bool,
    /// Accumulated fields for the current event.
    event_name: Option<String>,
    event_id: Option<String>,
    data_lines: Vec<String>,
}

impl SseReader {
    /// Create a new reader with empty state.
    pub(crate) fn new() -> Self {
        Self {
            line_buf: Vec::new(),
            prev_cr: false,
            event_name: None,
            event_id: None,
            data_lines: Vec::new(),
        }
    }

    /// Feed a chunk of bytes. Returns all events completed by this chunk.
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, SseError> {
        let mut events = Vec::new();

        for &byte in chunk {
            // A CR followed by LF is a single line ending — consume the LF
            // without producing a second (empty) line.
            if self.prev_cr && byte == b'\n' {
                self.prev_cr = false;
                continue;
            }
            self.prev_cr = false;

            if byte == b'\n' || byte == b'\r' {
                if byte == b'\r' {
                    self.prev_cr = true;
                }
                // End of line — process it.
                if let Some(event) = self.finish_line()? {
                    events.push(event);
                }
            } else {
                self.line_buf.push(byte);
                if self.line_buf.len() > MAX_LINE_BYTES {
                    return Err(SseError::LineOverLimit);
                }
            }
        }

        Ok(events)
    }

    /// Process a completed line. Returns an event if a blank line finishes one.
    fn finish_line(&mut self) -> Result<Option<SseEvent>, SseError> {
        let raw = std::mem::take(&mut self.line_buf);

        // Blank line: dispatch the accumulated event (if it has data).
        if raw.is_empty() {
            let event = if !self.data_lines.is_empty() {
                Some(SseEvent {
                    name: self.event_name.take(),
                    id: self.event_id.take(),
                    data: self.data_lines.join("\n"),
                })
            } else {
                None
            };
            // Reset all fields after a blank line, whether dispatched or not.
            self.event_name = None;
            self.event_id = None;
            self.data_lines.clear();

            return Ok(event);
        }

        // Decode the line with lossy UTF-8.
        let line = String::from_utf8_lossy(&raw);

        // Comment lines start with ':'.
        if line.starts_with(':') {
            return Ok(None);
        }

        // Parse "field: value" or "field" (no colon means empty value).
        let (field, value) = if let Some(colon_pos) = line.find(':') {
            let field = &line[..colon_pos];
            let rest = &line[colon_pos + 1..];
            // Remove exactly one leading space after the colon, if present.
            let value = rest.strip_prefix(' ').unwrap_or(rest);
            (field, value)
        } else {
            (line.as_ref(), "")
        };

        match field {
            "data" => {
                self.data_lines.push(value.to_string());
            }
            "event" => {
                self.event_name = Some(value.to_string());
            }
            "id" => {
                self.event_id = Some(value.to_string());
            }
            // "retry" and unknown fields are ignored.
            _ => {}
        }

        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: feed the entire input as one chunk and return all events.
    fn feed_all(input: &[u8]) -> Result<Vec<SseEvent>, SseError> {
        let mut reader = SseReader::new();
        reader.feed(input)
    }

    #[test]
    fn reader_dispatches_log_line_with_id_and_data() {
        let input = b"id: pod-a|2026-09-28T09:14:02Z\ndata: hello world\n\n";
        let events = feed_all(input).unwrap();
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.id.as_deref(), Some("pod-a|2026-09-28T09:14:02Z"));
        assert_eq!(e.data, "hello world");
        assert_eq!(e.name, None);
        assert_eq!(e.kind(), SseEventKind::LogLine);
    }

    #[test]
    fn reader_dispatches_named_lifecycle_events() {
        let input = b"event: pod-done\ndata: {\"pod\":\"a\"}\n\n\
                       event: stream-end\ndata: {\"reason\":\"completed\"}\n\n\
                       event: error\ndata: {\"message\":\"oops\"}\n\n";
        let events = feed_all(input).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].kind(), SseEventKind::PodDone);
        assert_eq!(events[0].data, "{\"pod\":\"a\"}");
        assert_eq!(events[1].kind(), SseEventKind::StreamEnd);
        assert_eq!(events[1].data, "{\"reason\":\"completed\"}");
        assert_eq!(events[2].kind(), SseEventKind::Error);
        assert_eq!(events[2].data, "{\"message\":\"oops\"}");
    }

    #[test]
    fn reader_ignores_keepalive_comments() {
        let input = b": keepalive\n\nid: a|1\ndata: line\n\n";
        let events = feed_all(input).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "line");
    }

    #[test]
    fn reader_joins_multiple_data_lines_with_newline() {
        let input = b"data: first\ndata: second\n\n";
        let events = feed_all(input).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "first\nsecond");
    }

    #[test]
    fn reader_accepts_field_without_space_after_colon() {
        let input = b"data:x\n\n";
        let events = feed_all(input).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "x");
    }

    #[test]
    fn reader_accepts_crlf_and_cr_line_endings() {
        // LF
        let lf = b"id: a\ndata: v\n\n";
        let lf_events = feed_all(lf).unwrap();

        // CRLF
        let crlf = b"id: a\r\ndata: v\r\n\r\n";
        let crlf_events = feed_all(crlf).unwrap();

        // CR
        let cr = b"id: a\rdata: v\r\r";
        let cr_events = feed_all(cr).unwrap();

        assert_eq!(lf_events, crlf_events);
        assert_eq!(lf_events, cr_events);
        assert_eq!(lf_events.len(), 1);
    }

    #[test]
    fn reader_reassembles_events_fed_one_byte_at_a_time() {
        // This test also covers a CR at the end of one chunk followed by LF
        // at the start of the next (must be ONE line ending, not two), and a
        // multi-byte UTF-8 character split across chunks.
        let snowman = "\u{2603}"; // ☃, 3 bytes: E2 98 83
        let input_str = format!("id: a\r\ndata: hello {snowman}\r\n\r\n");
        let input = input_str.as_bytes();

        // Feed all at once for reference.
        let reference = feed_all(input).unwrap();
        assert_eq!(reference.len(), 1);
        assert_eq!(reference[0].data, format!("hello {snowman}"));

        // Feed one byte at a time.
        let mut reader = SseReader::new();
        let mut all_events = Vec::new();
        for &b in input {
            let events = reader.feed(&[b]).unwrap();
            all_events.extend(events);
        }
        assert_eq!(all_events, reference);
    }

    #[test]
    fn reader_skips_event_without_data() {
        let input = b"event: x\n\n";
        let events = feed_all(input).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn reader_discards_incomplete_event_at_end_of_stream() {
        let input = b"id: a\ndata: hello";
        let events = feed_all(input).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn reader_replaces_invalid_utf8() {
        // 0xFF is not valid UTF-8; it should become U+FFFD.
        let mut input = Vec::new();
        input.extend_from_slice(b"data: hi\xFF");
        input.extend_from_slice(b"lo\n\ndata: next\n\n");
        let events = feed_all(&input).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events[0].data.contains('\u{FFFD}'));
        assert!(events[0].data.contains("hi"));
        assert!(events[0].data.contains("lo"));
        assert_eq!(events[1].data, "next");
    }

    #[test]
    fn reader_accepts_line_at_limit() {
        // A data line whose total length is exactly MAX_LINE_BYTES is
        // accepted. Together with reader_rejects_line_over_limit this
        // pins the boundary from both sides.
        let prefix = b"data: ";
        let payload_len = MAX_LINE_BYTES - prefix.len();
        let mut input = Vec::new();
        input.extend_from_slice(prefix);
        input.extend(std::iter::repeat_n(b'x', payload_len));
        input.push(b'\n');
        // Blank line to dispatch the event.
        input.push(b'\n');
        let events = feed_all(&input).unwrap();
        assert_eq!(
            events.len(),
            1,
            "exactly-at-limit line should produce one event"
        );
        assert_eq!(events[0].data.len(), payload_len);
    }

    #[test]
    fn reader_rejects_line_over_limit() {
        let mut input = Vec::new();
        input.extend_from_slice(b"data: ");
        // MAX_LINE_BYTES is 256 * 1024. The "data: " prefix is 6 bytes.
        // Total line length = 6 + payload. We need total > MAX_LINE_BYTES.
        let payload_len = MAX_LINE_BYTES - 6 + 1; // one byte over
        input.extend(std::iter::repeat_n(b'x', payload_len));
        input.push(b'\n');
        let result = feed_all(&input);
        assert_eq!(result, Err(SseError::LineOverLimit));
    }

    #[test]
    fn reader_treats_message_event_as_log_line() {
        let input = b"event: message\ndata: hello\n\n";
        let events = feed_all(input).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind(), SseEventKind::LogLine);
        assert_eq!(events[0].name.as_deref(), Some("message"));
    }

    #[test]
    fn reader_ignores_retry_and_unknown_fields() {
        let input = b"retry: 5000\nfoo: bar\ndata: keep\n\n";
        let events = feed_all(input).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "keep");
        assert_eq!(events[0].name, None);
        assert_eq!(events[0].id, None);
    }
}
