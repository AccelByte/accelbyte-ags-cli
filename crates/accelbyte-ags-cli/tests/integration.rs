#[path = "common/mod.rs"]
mod common;

#[path = "integration/ams_upload.rs"]
mod ams_upload;
#[path = "integration/auth.rs"]
mod auth;
#[path = "integration/binary_response.rs"]
mod binary_response;
#[path = "integration/builder.rs"]
mod builder;
#[path = "integration/completions.rs"]
mod completions;
#[path = "integration/config.rs"]
mod config;
#[path = "integration/error_pipeline.rs"]
mod error_pipeline;
#[path = "integration/format_precedence.rs"]
mod format_precedence;
#[path = "integration/namespace.rs"]
mod namespace;
#[path = "integration/output_flag.rs"]
mod output_flag;
#[path = "integration/parser.rs"]
mod parser;
#[path = "integration/profile.rs"]
mod profile;
#[path = "integration/renderer.rs"]
mod renderer;
#[path = "integration/service_naming.rs"]
mod service_naming;
#[path = "integration/stream_ownership.rs"]
mod stream_ownership;
#[path = "integration/token_refresh_race.rs"]
mod token_refresh_race;
#[path = "integration/tty_topology.rs"]
mod tty_topology;
#[path = "integration/tui_e2e.rs"]
mod tui_e2e;
#[path = "integration/tui_format.rs"]
mod tui_format;
#[path = "integration/ui_flag.rs"]
mod ui_flag;
