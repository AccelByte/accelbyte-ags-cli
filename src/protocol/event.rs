//! Progress event types — minimal stream for long-running commands.

use serde::{Deserialize, Serialize};

/// A consumer that receives progress events from the runtime.
///
/// Typical implementations render a spinner or a "page N/M" indicator. The
/// runtime is never aware of the implementation — it only calls `on_event`.
pub trait ProgressSink {
    /// Receive a single progress event and react (render a spinner, advance a page counter, etc.).
    fn on_event(&mut self, event: ProgressEvent);
}

/// One progress signal from a long-running command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProgressEvent {
    /// The operation has started; show the initial message.
    Started { message: String },
    /// A page of a paginated result has been fetched.
    Page {
        current: usize,
        total: Option<usize>,
    },
    /// An informational message that should replace any spinner text.
    Message { text: String },
    /// The operation is done; tear down any progress UI.
    Finished,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialise `value` to JSON, parse it back, and assert equality — the contract test for protocol types.
    fn round_trip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let parsed: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, &parsed);
    }

    #[test]
    fn test_progress_event_started_round_trip() {
        round_trip(&ProgressEvent::Started {
            message: "Fetching users…".to_string(),
        });
    }

    #[test]
    fn test_progress_event_page_with_total_round_trip() {
        round_trip(&ProgressEvent::Page {
            current: 3,
            total: Some(12),
        });
    }

    #[test]
    fn test_progress_event_page_unknown_total_round_trip() {
        round_trip(&ProgressEvent::Page {
            current: 7,
            total: None,
        });
    }

    #[test]
    fn test_progress_event_message_round_trip() {
        round_trip(&ProgressEvent::Message {
            text: "Processing page 5".to_string(),
        });
    }

    #[test]
    fn test_progress_event_finished_round_trip() {
        round_trip(&ProgressEvent::Finished);
    }
}
