//! Runs a simple TCP forwarder: forwards ports on 127.0.0.1 for each
//! configured service. Ported from Go's `pkg/simpleforwarder/runner.go`.

use std::net::{IpAddr, Ipv4Addr};

use tokio_util::sync::CancellationToken;

use super::listener::{GetStreamOpener, ServiceListener};

/// One forwarded service: a name, its remote (in-cluster) host, and the
/// local ports to listen on. Mirrors Go's `ServiceSpec`.
pub struct ServiceSpec {
    pub name: String,
    pub remote_host: String,
    pub ports: Vec<u16>,
}

/// Forwarder configuration. Mirrors Go's `simpleforwarder.Config`.
pub struct Config {
    pub services: Vec<ServiceSpec>,
    pub get_stream_opener: GetStreamOpener,
}

/// Starts one [`ServiceListener`] per service and blocks until `cancel`
/// fires, then stops every listener. All services must use distinct port
/// numbers — two services sharing a port causes a bind error. Distinct
/// loopback IPs per service are not yet supported (`ServiceListener.local_ip`
/// is reserved for that future feature). Mirrors `simpleforwarder.Run`.
pub async fn run(cfg: Config, cancel: CancellationToken) -> std::io::Result<()> {
    let mut listeners = Vec::with_capacity(cfg.services.len());

    for spec in cfg.services {
        let local_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let listener = ServiceListener::new(
            spec.name.clone(),
            spec.remote_host,
            local_ip,
            spec.ports,
            cfg.get_stream_opener.clone(),
        );
        if let Err(e) = listener.start().await {
            // Deliberate departure from Go: `simpleforwarder.Run` returns
            // immediately here without stopping already-started sibling
            // listeners, leaking their accept-loop goroutines and listening
            // sockets for the process's remaining lifetime. Stopping them
            // is a straightforward correctness improvement, not a protocol
            // behavior this crate needs to reproduce.
            for started in &listeners {
                let started: &ServiceListener = started;
                started.stop();
            }
            return Err(std::io::Error::other(format!(
                "failed to start listener for service {}: {e}",
                spec.name
            )));
        }
        listeners.push(listener);
    }

    if listeners.is_empty() {
        return Err(std::io::Error::other(
            "no services were successfully started",
        ));
    }

    cancel.cancelled().await;

    for listener in &listeners {
        listener.stop();
    }
    tracing::info!("service listeners stopped");

    Ok(())
}
