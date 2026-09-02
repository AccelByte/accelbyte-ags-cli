//! Local TCP-to-tunnel-stream forwarder. Ported from Go's `pkg/simpleforwarder`.

mod listener;
mod runner;

pub use listener::{GetStreamOpener, ServiceListener, StreamOpener};
pub use runner::{run, Config, ServiceSpec};
