//! Thread-safe holder for the currently active tunnel session. Ported from
//! Go's `pkg/client/session_holder.go`.

use std::sync::{Arc, RwLock};

use crate::session::Session;

/// Stores the currently active tunnel session in a thread-safe way, updated
/// by `Agent` whenever a session is established or torn down. Mirrors Go's `SessionHolder`.
#[derive(Default)]
pub struct SessionHolder {
    current: RwLock<Option<Arc<Session>>>,
}

impl SessionHolder {
    /// Creates an empty holder (no active session).
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the active session, or `None` when no session is established.
    pub fn current(&self) -> Option<Arc<Session>> {
        self.current.read().unwrap().clone()
    }

    /// Records the newly established session as active.
    pub(crate) fn set(&self, session: Arc<Session>) {
        *self.current.write().unwrap() = Some(session);
    }

    /// Clears the active session, e.g. once it has ended.
    pub(crate) fn clear(&self) {
        *self.current.write().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_holder_has_no_current_session() {
        let holder = SessionHolder::new();
        assert!(holder.current().is_none());
    }
}
