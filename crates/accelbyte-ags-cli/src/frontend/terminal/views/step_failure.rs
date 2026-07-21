//! Shared step-failure gate card text, so the inline and fullscreen surfaces
//! compose the failure card identically instead of each hand-rolling it.

use ags_protocol::error::RuntimeError;

/// The `(title, message)` for a step-failure gate card. The title carries the
/// error headline; the message is the server reason when present, else a prompt
/// naming the available actions. Shared so the two card surfaces never drift on
/// wording (and neither reintroduces the em-dash join the fullscreen card once
/// used).
pub(crate) fn step_failure_card_text(error: &RuntimeError) -> (String, String) {
    let title = format!("Step failed: {}", error.message);
    let message = error
        .reason_detail()
        .map(str::to_string)
        .unwrap_or_else(|| "Retry the step, skip it, or cancel the run.".to_string());
    (title, message)
}
