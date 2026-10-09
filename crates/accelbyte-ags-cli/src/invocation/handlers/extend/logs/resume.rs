//! Duplicate suppression for SSE log streaming and the pod-from-id helper.

use std::collections::{HashSet, VecDeque};

/// Maximum number of event ids remembered for duplicate suppression.
pub(crate) const RESUME_WINDOW_IDS: usize = 2048;

/// Sliding window of recently printed event ids, used to suppress duplicates
/// after a reconnect. Lookups are O(1) via a `HashSet` alongside a `VecDeque`.
pub(crate) struct ResumeWindow {
    ids: VecDeque<String>,
    set: HashSet<String>,
    resume_point: Option<String>,
}

impl ResumeWindow {
    /// Create an empty window.
    pub(crate) fn new() -> Self {
        Self {
            ids: VecDeque::with_capacity(RESUME_WINDOW_IDS),
            set: HashSet::with_capacity(RESUME_WINDOW_IDS),
            resume_point: None,
        }
    }

    /// Decide whether a line should be printed.
    ///
    /// - `None` id: always printed, does not change the window or resume point.
    /// - Known id: not printed, does not change anything.
    /// - New id: printed, remembered, becomes the resume point.
    pub(crate) fn admit(&mut self, id: Option<&str>) -> bool {
        let id = match id {
            None => return true,
            Some(id) => id,
        };

        if self.set.contains(id) {
            return false;
        }

        // Evict the oldest if at capacity.
        if self.ids.len() == RESUME_WINDOW_IDS {
            if let Some(evicted) = self.ids.pop_front() {
                self.set.remove(&evicted);
            }
        }

        let owned = id.to_string();
        self.ids.push_back(owned.clone());
        self.set.insert(owned.clone());
        self.resume_point = Some(owned);
        true
    }

    /// The id of the most recent printed line, or `None` if no line with an id
    /// has been printed.
    pub(crate) fn resume_point(&self) -> Option<&str> {
        self.resume_point.as_deref()
    }
}

/// Extract the pod name from an SSE event id.
///
/// The id format is `{podName}|{RFC3339Nano}`. Returns the text before the
/// first `|`. `None` when the id is empty, has no `|`, or the text before the
/// first `|` is empty.
pub(crate) fn pod_from_event_id(id: &str) -> Option<&str> {
    let pos = id.find('|')?;
    let pod = &id[..pos];
    if pod.is_empty() {
        None
    } else {
        Some(pod)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_window_skips_repeated_id() {
        let mut w = ResumeWindow::new();
        assert!(w.admit(Some("a")));
        assert!(!w.admit(Some("a")));
    }

    #[test]
    fn resume_window_always_emits_line_without_id() {
        let mut w = ResumeWindow::new();
        assert!(w.admit(Some("a")));
        assert!(w.admit(None));
        assert!(w.admit(None));
        // Resume point is unchanged by None ids.
        assert_eq!(w.resume_point(), Some("a"));
    }

    #[test]
    fn resume_window_forgets_oldest_after_2048_ids() {
        let mut w = ResumeWindow::new();

        // Admit 2,049 ids: id-1 through id-2049.
        for i in 1..=2049 {
            let id = format!("id-{i}");
            assert!(w.admit(Some(&id)), "id-{i} should be admitted");
        }

        // id-2 is still known (admit returns false, which changes nothing).
        assert!(!w.admit(Some("id-2")), "id-2 should still be in the window");

        // id-1 was evicted (admit returns true — the eviction happened
        // when id-2049 was admitted, pushing id-1 out).
        assert!(
            w.admit(Some("id-1")),
            "id-1 should be forgotten after 2049 admissions"
        );
    }

    #[test]
    fn resume_point_is_most_recent_emitted_id() {
        let mut w = ResumeWindow::new();
        assert_eq!(w.resume_point(), None);
        assert!(w.admit(Some("a")));
        assert_eq!(w.resume_point(), Some("a"));
        assert!(w.admit(Some("b")));
        assert_eq!(w.resume_point(), Some("b"));
        // A skipped duplicate does not move the resume point.
        assert!(!w.admit(Some("a")));
        assert_eq!(w.resume_point(), Some("b"));
    }

    #[test]
    fn pod_from_event_id_takes_text_before_first_separator() {
        assert_eq!(pod_from_event_id("a-1|2026-09-28T09:14:02Z"), Some("a-1"));
        // Multiple separators: only the text before the first.
        assert_eq!(pod_from_event_id("a|b|c"), Some("a"));
    }

    #[test]
    fn pod_from_event_id_is_none_without_separator_or_id() {
        assert_eq!(pod_from_event_id(""), None);
        assert_eq!(pod_from_event_id("no-separator"), None);
        assert_eq!(pod_from_event_id("|trailing"), None);
    }
}
