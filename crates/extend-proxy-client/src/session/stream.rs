//! One logical tunnel stream: the read-pump task and incoming frame dispatch.
//! Used identically for client-initiated and sidecar-initiated streams — the
//! only variation is which side dialed the underlying connection. Ported
//! from Go's `pkg/tunnel/stream.go`.
//!
//! The local connection's concrete type is erased to a boxed
//! `AsyncRead`/`AsyncWrite` pair at construction (`Stream::new` is generic;
//! `Stream` itself is not), so `Session` can hold a uniform `Stream` type
//! regardless of whether a given stream was dialed as a real `TcpStream`
//! (production) or an in-memory `tokio::io::duplex` pair (tests).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex};

use crate::protocol::{reason, Frame, FrameBody};

/// 32 KiB read buffer, matching Go's `ReadPump` buffer size.
const READ_BUFFER_SIZE: usize = 32 * 1024;

type BoxedRead = Box<dyn AsyncRead + Unpin + Send>;
type BoxedWrite = Box<dyn AsyncWrite + Unpin + Send>;

/// A boxed future, used to erase a dialed connection's concrete type at the
/// `Session`/forwarder boundary (see `crate::session::DialFn`).
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Any local connection a stream can be built on. A blanket impl covers
/// every `AsyncRead + AsyncWrite` type, so `Box<dyn DuplexConn>` is the
/// common erased type shared by real `TcpStream`s and test `DuplexStream`s.
pub trait DuplexConn: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> DuplexConn for T {}

#[derive(Default)]
struct StreamState {
    /// True while waiting for OPEN_ACK/NACK (OPENING state).
    pending: bool,
    /// True once the local side has finished sending (sent FIN).
    local_eof: bool,
    /// True once the remote side has finished sending (received FIN).
    remote_eof: bool,
    /// True once the stream has been fully closed and cleaned up.
    closed: bool,
}

/// Manages one logical tunnel stream. Mirrors Go's `Stream`.
pub struct Stream {
    pub id: u64,
    write_half: AsyncMutex<BoxedWrite>,
    /// Taken exactly once, by `read_pump`, when it starts.
    read_half: AsyncMutex<Option<BoxedRead>>,
    /// The other end of `cancel_tx`; taken exactly once, by `read_pump`.
    cancel_rx: AsyncMutex<Option<oneshot::Receiver<()>>>,
    /// Fired by `close` to unblock a `read_pump` stuck in a blocking read.
    cancel_tx: StdMutex<Option<oneshot::Sender<()>>>,
    /// Shared with the owning `Session`'s writer task.
    outbound: mpsc::Sender<Frame>,
    state: StdMutex<StreamState>,
}

impl Stream {
    /// Creates a new `Stream` over `conn`, initially in the OPENING (pending) state.
    pub fn new<C>(id: u64, conn: C, outbound: mpsc::Sender<Frame>) -> Arc<Self>
    where
        C: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (read_half, write_half) = tokio::io::split(conn);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        Arc::new(Self {
            id,
            write_half: AsyncMutex::new(Box::new(write_half)),
            read_half: AsyncMutex::new(Some(Box::new(read_half))),
            cancel_rx: AsyncMutex::new(Some(cancel_rx)),
            cancel_tx: StdMutex::new(Some(cancel_tx)),
            outbound,
            state: StdMutex::new(StreamState {
                pending: true,
                ..Default::default()
            }),
        })
    }

    /// Reads from the local connection and emits DATA/FIN/RST frames to the
    /// peer. Must be spawned as a dedicated task exactly once; returns when
    /// the connection closes, an unrecoverable error occurs, or `close` is
    /// called concurrently. Mirrors `Stream.ReadPump`.
    pub async fn read_pump(self: Arc<Self>) {
        let (Some(mut read_half), Some(mut cancel_rx)) = (
            self.read_half.lock().await.take(),
            self.cancel_rx.lock().await.take(),
        ) else {
            return;
        };

        let mut buf = vec![0u8; READ_BUFFER_SIZE];
        loop {
            let read_result = tokio::select! {
                _ = &mut cancel_rx => return,
                result = read_half_read(&mut read_half, &mut buf) => result,
            };
            match read_result {
                Ok(0) => {
                    tracing::debug!(stream_id = self.id, "stream: local read got EOF");
                    let _ = self.send(Frame::new_fin(self.id)).await;
                    if self.mark_local_eof() {
                        self.close().await;
                    }
                    return;
                }
                Ok(n) => {
                    // Awaits when the owning session's outbound channel is
                    // full, so a stalled peer pauses this loop's next local
                    // `read()` instead of buffering unboundedly.
                    if self
                        .send(Frame::new_data(self.id, buf[..n].to_vec()))
                        .await
                        .is_err()
                    {
                        tracing::warn!(stream_id = self.id, "stream: ws write error in read pump");
                        self.close().await;
                        return;
                    }
                    tracing::debug!(stream_id = self.id, bytes = n, "stream: DATA sent");
                }
                Err(error) => {
                    tracing::warn!(stream_id = self.id, %error, "stream: local read error");
                    let _ = self
                        .send(Frame::new_rst(self.id, reason::LOCAL_READ_ERROR))
                        .await;
                    self.close().await;
                    return;
                }
            }
        }
    }

    /// Handles an incoming DATA, FIN, or RST frame addressed to this stream.
    /// Called inline from the session's read loop, same as Go's `Dispatch` —
    /// per-stream frame order is preserved by the caller processing frames
    /// one at a time. Mirrors `Stream.Dispatch`.
    pub async fn dispatch(self: &Arc<Self>, frame: Frame) {
        match frame.body {
            FrameBody::Data(data) => {
                tracing::debug!(
                    stream_id = self.id,
                    bytes = data.len(),
                    "stream: DATA received"
                );
                let write_err = {
                    let mut w = self.write_half.lock().await;
                    tokio::io::AsyncWriteExt::write_all(&mut *w, &data).await
                };
                if let Err(error) = write_err {
                    tracing::warn!(stream_id = self.id, %error, "stream: local write error");
                    let _ = self
                        .send(Frame::new_rst(self.id, reason::LOCAL_WRITE_ERROR))
                        .await;
                    self.close().await;
                }
            }
            FrameBody::Fin => {
                {
                    let mut w = self.write_half.lock().await;
                    let _ = w.shutdown().await;
                }
                if self.mark_remote_eof() {
                    self.close().await;
                }
            }
            FrameBody::Rst(reason) => {
                tracing::info!(stream_id = self.id, %reason, "stream: RST received");
                self.close().await;
            }
            _ => {}
        }
    }

    /// Forcibly closes the underlying connection. Safe to call multiple times.
    pub async fn close(&self) {
        let should_close = {
            let mut state = self.state.lock().unwrap();
            if state.closed {
                false
            } else {
                state.closed = true;
                true
            }
        };
        if !should_close {
            return;
        }
        if let Some(cancel_tx) = self.cancel_tx.lock().unwrap().take() {
            let _ = cancel_tx.send(());
        }
        let mut w = self.write_half.lock().await;
        let _ = w.shutdown().await;
    }

    /// Reports whether the stream is in the OPENING state (waiting for OPEN_ACK/NACK).
    pub fn is_pending(&self) -> bool {
        self.state.lock().unwrap().pending
    }

    /// Updates the stream's OPENING state.
    pub fn set_pending(&self, value: bool) {
        self.state.lock().unwrap().pending = value;
    }

    /// Reports whether the stream's underlying connection has been closed.
    pub fn is_closed(&self) -> bool {
        self.state.lock().unwrap().closed
    }

    /// Records that the local side has finished sending. Returns true when
    /// both sides are done and the stream should be closed.
    fn mark_local_eof(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        state.local_eof = true;
        state.local_eof && state.remote_eof
    }

    /// Records that the remote side has finished sending. Returns true when
    /// both sides are done and the stream should be closed.
    fn mark_remote_eof(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        state.remote_eof = true;
        state.local_eof && state.remote_eof
    }

    /// Hands a frame back to the owning session's writer task. Bounded (see
    /// [`crate::protocol::OUTBOUND_CHANNEL_CAPACITY`]), so this awaits when
    /// the writer is stalled — the backpressure point that pauses this
    /// stream's local reads instead of buffering unboundedly.
    async fn send(&self, frame: Frame) -> Result<(), mpsc::error::SendError<Frame>> {
        self.outbound.send(frame).await
    }
}

/// Free function (rather than a method) so it can be named directly inside
/// `tokio::select!` without borrow-checker issues over `self`.
async fn read_half_read(read_half: &mut BoxedRead, buf: &mut [u8]) -> std::io::Result<usize> {
    tokio::io::AsyncReadExt::read(read_half, buf).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture_frames() -> (mpsc::Sender<Frame>, mpsc::Receiver<Frame>) {
        mpsc::channel(crate::protocol::OUTBOUND_CHANNEL_CAPACITY)
    }

    /// A stalled/slow outbound channel must pause `read_pump`'s next local
    /// read instead of buffering unboundedly — the backpressure behavior the
    /// bounded outbound channel exists to provide.
    #[tokio::test]
    async fn test_read_pump_blocks_on_full_outbound_channel() {
        let (local, mut remote) = tokio::io::duplex(1024);
        let (tx, mut rx) = mpsc::channel(1);
        let st = Stream::new(1, local, tx);

        let handle = tokio::spawn(st.read_pump());

        tokio::io::AsyncWriteExt::write_all(&mut remote, b"first")
            .await
            .unwrap();
        // Give read_pump time to read "first" and fill the capacity-1 channel.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        tokio::io::AsyncWriteExt::write_all(&mut remote, b"second")
            .await
            .unwrap();
        // read_pump has read "second" but the channel is still full — it
        // must be blocked on `send`, not exited or dropping the frame.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !handle.is_finished(),
            "read_pump should be applying backpressure, not exited"
        );

        let first = rx.recv().await.expect("first frame");
        assert!(matches!(first.body, FrameBody::Data(ref d) if d == b"first"));
        let second = rx.recv().await.expect("second frame");
        assert!(matches!(second.body, FrameBody::Data(ref d) if d == b"second"));

        drop(remote);
        while let Ok(Some(f)) =
            tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
        {
            if matches!(f.body, FrameBody::Fin) {
                break;
            }
        }

        tokio::time::timeout(std::time::Duration::from_millis(300), handle)
            .await
            .expect("read_pump did not finish after channel drained")
            .expect("read_pump task panicked");
    }

    /// Translated from `stream_test.go`'s `TestReadPumpSendsData`.
    #[tokio::test]
    async fn test_read_pump_sends_data() {
        let (local, mut remote) = tokio::io::duplex(64);
        let (tx, mut rx) = capture_frames();
        let st = Stream::new(1, local, tx);

        tokio::spawn(st.clone().read_pump());

        let payload = b"hello tunnel";
        tokio::io::AsyncWriteExt::write_all(&mut remote, payload)
            .await
            .unwrap();
        drop(remote); // triggers EOF on the local side

        let first = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("first frame")
            .expect("channel open");
        assert!(matches!(first.body, FrameBody::Data(ref d) if d == payload));

        let mut last = first;
        while let Ok(Some(f)) =
            tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
        {
            last = f;
        }
        assert!(matches!(last.body, FrameBody::Fin));
    }

    /// Translated from `stream_test.go`'s `TestReadPumpSendsRSTOnError` — just
    /// asserts no panic/deadlock when the local side is closed abruptly.
    #[tokio::test]
    async fn test_read_pump_sends_rst_on_error() {
        let (local, remote) = tokio::io::duplex(64);
        let (tx, _rx) = capture_frames();
        let st = Stream::new(1, local, tx);

        let handle = tokio::spawn(st.read_pump());
        drop(remote);

        tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("read_pump did not finish")
            .expect("read_pump task panicked");
    }

    /// Translated from `stream_test.go`'s `TestDispatchWritesData`.
    #[tokio::test]
    async fn test_dispatch_writes_data() {
        let (local, mut remote) = tokio::io::duplex(64);
        let (tx, _rx) = capture_frames();
        let st = Stream::new(1, local, tx);

        let read_task = tokio::spawn(async move {
            let mut buf = [0u8; 64];
            let n = tokio::io::AsyncReadExt::read(&mut remote, &mut buf)
                .await
                .unwrap();
            buf[..n].to_vec()
        });

        st.dispatch(Frame::new_data(1, b"dispatched".to_vec()))
            .await;

        let got = tokio::time::timeout(std::time::Duration::from_secs(1), read_task)
            .await
            .expect("timeout waiting for dispatched data")
            .expect("read task panicked");
        assert_eq!(got, b"dispatched");
    }

    /// Translated from `stream_test.go`'s `TestDispatchRSTClosesStream`.
    #[tokio::test]
    async fn test_dispatch_rst_closes_stream() {
        let (local, mut remote) = tokio::io::duplex(64);
        let (tx, _rx) = capture_frames();
        let st = Stream::new(1, local, tx);

        st.dispatch(Frame::new_rst(1, "forced")).await;

        let mut buf = [0u8; 1];
        let result = tokio::io::AsyncReadExt::read(&mut remote, &mut buf).await;
        // After RST, the local conn is closed — the peer sees EOF (Ok(0)) or an error.
        assert!(matches!(result, Ok(0) | Err(_)));
    }

    /// Translated from `stream_test.go`'s `TestBothEOFClosesStream`.
    #[tokio::test]
    async fn test_both_eof_closes_stream() {
        let (local, remote) = tokio::io::duplex(64);
        let (tx, mut rx) = capture_frames();
        let st = Stream::new(1, local, tx);

        tokio::spawn(st.clone().read_pump());

        // Signal remote EOF (local side gets FIN from remote).
        st.dispatch(Frame::new_fin(1)).await;
        // Close the remote end so the local read pump gets EOF too.
        drop(remote);

        let mut has_fin = false;
        while let Ok(Some(f)) =
            tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
        {
            if matches!(f.body, FrameBody::Fin) {
                has_fin = true;
            }
        }
        assert!(has_fin, "expected FIN frame from read pump after local EOF");
    }
}
