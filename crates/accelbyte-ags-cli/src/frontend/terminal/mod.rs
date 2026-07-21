//! Terminal interaction surfaces: plain (line-oriented), inline (TUI),
//! and fullscreen (alt-screen workflow surface).

pub(crate) mod backend;
pub(crate) mod date_field;
pub(crate) mod dynamic_enums;
pub mod form_runner;
pub mod fullscreen;
pub mod inline;
pub mod machine_json;
pub(crate) mod no_color_backend;
pub(crate) mod picker_list;
pub mod plain;
pub(crate) mod views;
