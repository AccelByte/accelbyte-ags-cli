//! Manages TCP listeners for forwarded service entries. Each
//! `ServiceListener` holds one `TcpListener` per port, all bound to the same
//! local IP address — mirroring kubefwd's per-port task model. Ported from
//! Go's `pkg/simpleforwarder/listener.go`.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex as StdMutex};

use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::session::{BoxFuture, Session};

/// Can open a tunneled stream for an accepted TCP connection. Implemented by
/// [`Session`]. Mirrors Go's `StreamOpener` interface. Named distinctly from
/// `Session::open_remote_stream` (rather than reusing that name for this
/// trait method) to avoid inherent-vs-trait method resolution ambiguity.
pub trait StreamOpener: Send + Sync {
    fn open_stream(
        self: Arc<Self>,
        conn: TcpStream,
        target_host: String,
        target_port: i32,
    ) -> BoxFuture<()>;
}

impl StreamOpener for Session {
    fn open_stream(
        self: Arc<Self>,
        conn: TcpStream,
        target_host: String,
        target_port: i32,
    ) -> BoxFuture<()> {
        Box::pin(async move {
            let remote_addr = conn.peer_addr().map(|a| a.to_string()).unwrap_or_default();
            Session::open_remote_stream(&self, conn, target_host, target_port, remote_addr).await;
        })
    }
}

/// Returns the currently active tunnel session as a [`StreamOpener`], or
/// `None` if none is established. Mirrors Go's `GetStreamOpener func() StreamOpener`.
pub type GetStreamOpener = Arc<dyn Fn() -> Option<Arc<dyn StreamOpener>> + Send + Sync>;

/// All TCP listeners for a single forwarded service. Mirrors Go's `ServiceListener`.
pub struct ServiceListener {
    pub name: String,
    pub remote_host: String,
    /// The loopback IP allocated for this service.
    pub local_ip: IpAddr,
    /// The TCP ports to listen on.
    pub ports: Vec<u16>,
    pub get_stream_opener: GetStreamOpener,
    accept_tasks: StdMutex<Vec<JoinHandle<()>>>,
}

impl ServiceListener {
    /// Creates a `ServiceListener`; call [`ServiceListener::start`] to begin accepting connections.
    pub fn new(
        name: impl Into<String>,
        remote_host: impl Into<String>,
        local_ip: IpAddr,
        ports: Vec<u16>,
        get_stream_opener: GetStreamOpener,
    ) -> Self {
        Self {
            name: name.into(),
            remote_host: remote_host.into(),
            local_ip,
            ports,
            get_stream_opener,
            accept_tasks: StdMutex::new(Vec::new()),
        }
    }

    /// Opens one TCP listener per port on the service's local IP and spawns
    /// an accept-loop task for each. Mirrors `ServiceListener.Start`.
    pub async fn start(&self) -> std::io::Result<()> {
        let mut tasks = Vec::with_capacity(self.ports.len());
        for &port in &self.ports {
            let listener = match TcpListener::bind(SocketAddr::new(self.local_ip, port)).await {
                Ok(l) => l,
                Err(e) => {
                    for task in tasks {
                        let task: JoinHandle<()> = task;
                        task.abort();
                    }
                    return Err(e);
                }
            };
            tracing::info!(service = %self.name, local_ip = %self.local_ip, port, "listening");
            tasks.push(tokio::spawn(accept_loop(
                listener,
                self.name.clone(),
                port,
                self.remote_host.clone(),
                self.get_stream_opener.clone(),
            )));
        }
        // nosemgrep -- mutex lock: only fails on poisoning by a panicking thread; propagating is not actionable
        *self.accept_tasks.lock().unwrap() = tasks;
        Ok(())
    }

    /// Aborts every accept-loop task for this service. Mirrors `ServiceListener.Stop`.
    pub fn stop(&self) {
        for task in self.accept_tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }
}

/// Runs the accept loop for one listener. Mirrors `ServiceListener.acceptLoop`.
async fn accept_loop(
    listener: TcpListener,
    name: String,
    port: u16,
    remote_host: String,
    get_stream_opener: GetStreamOpener,
) {
    loop {
        let (conn, _peer_addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(_) => {
                // listener closed/aborted — normal shutdown path
                tracing::debug!(service = %name, port, "listener closed");
                return;
            }
        };
        tokio::spawn(handle_conn(
            conn,
            name.clone(),
            port as i32,
            remote_host.clone(),
            get_stream_opener.clone(),
        ));
    }
}

/// Opens a tunnel stream for the accepted TCP connection. Conn ownership is
/// transferred to the stream opener. Mirrors `ServiceListener.handleConn`.
async fn handle_conn(
    conn: TcpStream,
    name: String,
    port: i32,
    remote_host: String,
    get_stream_opener: GetStreamOpener,
) {
    let Some(opener) = get_stream_opener() else {
        tracing::debug!(service = %name, port, "no active tunnel session, closing connection");
        drop(conn);
        return;
    };
    opener.open_stream(conn, remote_host, port).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    struct RecordingOpener {
        last_port: AtomicI32,
        last_host: StdMutex<String>,
    }

    impl StreamOpener for RecordingOpener {
        fn open_stream(
            self: Arc<Self>,
            mut conn: TcpStream,
            target_host: String,
            target_port: i32,
        ) -> BoxFuture<()> {
            self.last_port.store(target_port, Ordering::SeqCst);
            *self.last_host.lock().unwrap() = target_host;
            Box::pin(async move {
                let _ = conn.shutdown().await;
            })
        }
    }

    /// Reserves an ephemeral port by binding then immediately releasing it —
    /// a brief TOCTOU race, acceptable for test purposes.
    async fn free_port() -> u16 {
        let probe = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .unwrap();
        probe.local_addr().unwrap().port()
    }

    /// A `ServiceListener` forwards an accepted connection to the current
    /// stream opener with the configured remote host and the accepted port.
    #[tokio::test]
    async fn test_service_listener_forwards_to_stream_opener() {
        let opener = Arc::new(RecordingOpener {
            last_port: AtomicI32::new(0),
            last_host: StdMutex::new(String::new()),
        });
        let opener_for_closure = opener.clone();
        let get_stream_opener: GetStreamOpener =
            Arc::new(move || Some(opener_for_closure.clone() as Arc<dyn StreamOpener>));

        let port = free_port().await;
        let listener = ServiceListener::new(
            "svc",
            "remote.internal",
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            vec![port],
            get_stream_opener,
        );
        listener.start().await.expect("start");

        let mut stream = TcpStream::connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
            .await
            .expect("connect to forwarded port");
        // Wait for the peer to close its end (the recording opener shuts
        // down the connection), confirming handle_conn actually ran.
        let mut buf = [0u8; 1];
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            tokio::io::AsyncReadExt::read(&mut stream, &mut buf),
        )
        .await
        .expect("timed out waiting for opener to handle the connection");

        assert_eq!(opener.last_port.load(Ordering::SeqCst), port as i32);
        assert_eq!(*opener.last_host.lock().unwrap(), "remote.internal");

        listener.stop();
    }

    /// When no stream opener is active, an accepted connection is dropped
    /// (the peer observes the connection close) rather than panicking.
    #[tokio::test]
    async fn test_service_listener_drops_connection_when_no_opener() {
        let get_stream_opener: GetStreamOpener = Arc::new(|| None);
        let port = free_port().await;
        let listener = ServiceListener::new(
            "svc",
            "remote.internal",
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            vec![port],
            get_stream_opener,
        );
        listener.start().await.expect("start");

        let mut stream = TcpStream::connect(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
            .await
            .expect("connect to forwarded port");
        let mut buf = [0u8; 1];
        let n = tokio::time::timeout(
            Duration::from_secs(1),
            tokio::io::AsyncReadExt::read(&mut stream, &mut buf),
        )
        .await
        .expect("timed out waiting for connection to close")
        .expect("read error");
        assert_eq!(n, 0, "expected EOF when no stream opener is active");

        listener.stop();
    }
}
