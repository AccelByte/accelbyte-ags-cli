#[path = "common/mod.rs"]
mod common;

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
#[path = "integration/token_refresh_race.rs"]
mod token_refresh_race;
