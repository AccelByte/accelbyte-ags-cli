//! No-op progress sink — JSON mode emits no progress indicators.

use crate::protocol::event::{ProgressEvent, ProgressSink};

#[derive(Default)]
pub struct NoopProgressSink;

impl ProgressSink for NoopProgressSink {
    fn on_event(&mut self, _event: ProgressEvent) {}
}
