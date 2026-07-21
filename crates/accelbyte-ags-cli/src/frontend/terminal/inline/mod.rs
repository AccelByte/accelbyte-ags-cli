//! Inline terminal frontend: drives a single command through up to four interactive
//! phases (body, confirm, progress, result) and emits the same plain-text
//! scrollback artifact as the human frontend on exit.

pub mod chrome;
pub mod form;
pub mod form_builder;
pub mod frontend;
pub mod interaction;
pub mod json_editor;
pub mod lifecycle;
pub mod phases;
pub mod progress_state;
pub mod session;
