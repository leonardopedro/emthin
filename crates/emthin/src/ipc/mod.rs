mod connection;
pub mod dispatch;
pub mod jsonrpc;
pub mod messages;

use std::os::unix::net::UnixListener;

use connection::IpcConn;
pub use messages::{IncomingMessage, IpcRect, OutgoingMessage, StateFigure};

/// IPC server: listens for a single control-client connection and exchanges
/// JSON messages.
pub struct IpcServer {
    pub socket_path: std::path::PathBuf,
    listener: UnixListener,
    connection: Option<IpcConn>,
    /// Messages queued before a control client connects.
    ///
    /// Bounded: `send` is called for every `figure_bound` / `figure_changed` /
    /// `app_title_changed` / `error`, and a compositor with apps but no control
    /// client queued one message per event for the life of the session.
    pending: Vec<OutgoingMessage>,
}

/// How many messages to hold for a control client that has not connected yet.
///
/// These are the *state* notifications — a client that connects wants the current
/// state, not a replay of every intermediate step — so the oldest are the ones
/// worth dropping. Far more than a real burst.
const MAX_PENDING_MESSAGES: usize = 256;

impl IpcServer {
    /// Create a listening socket at `path` (non-blocking).
    pub fn bind(path: impl Into<std::path::PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        // Remove stale socket file if it exists.
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        tracing::info!("IPC listening on {}", path.display());
        Ok(Self {
            socket_path: path,
            listener,
            connection: None,
            pending: Vec::new(),
        })
    }

    /// Accept a pending connection if available.
    /// Returns `true` if a new client connected.
    pub fn accept(&mut self) -> bool {
        match self.listener.accept() {
            Ok((stream, _)) => {
                match IpcConn::new(stream) {
                    Ok(mut conn) => {
                        tracing::info!("control client connected");
                        // Send handshake + any buffered messages.
                        if let Ok(json) = jsonrpc::serialize_outgoing(OutgoingMessage::Connected {
                            version: "0.1",
                        }) {
                            let _ = conn.enqueue_raw(&json);
                        }
                        for msg in self.pending.drain(..) {
                            if let Ok(json) = jsonrpc::serialize_outgoing(msg) {
                                let _ = conn.enqueue_raw(&json);
                            }
                        }
                        let _ = conn.try_flush();
                        self.connection = Some(conn);
                        true
                    }
                    Err(e) => {
                        tracing::error!("IPC accept error: {e}");
                        false
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => false,
            Err(e) => {
                tracing::error!("IPC listener error: {e}");
                false
            }
        }
    }

    /// Send a message. If no client is connected, the message is buffered.
    ///
    /// Non-blocking: if the socket returns EAGAIN, bytes stay in the write
    /// buffer and are drained by the next `flush()` call from the event
    /// loop tick. This matters because the IPC source is registered with
    /// READ interest only in calloop — WRITE readiness wouldn't fire.
    pub fn send(&mut self, msg: OutgoingMessage) {
        let Some(conn) = &mut self.connection else {
            if self.pending.len() >= MAX_PENDING_MESSAGES {
                // Drop the oldest: the newest is the state that matters.
                let dropped = self.pending.remove(0);
                tracing::debug!(
                    "IPC pending queue full ({MAX_PENDING_MESSAGES}), dropped a stale message"
                );
                drop(dropped);
            }
            self.pending.push(msg);
            return;
        };
        let json = match jsonrpc::serialize_outgoing(msg) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("IPC serialize error: {e}");
                return;
            }
        };
        if let Err(e) = conn.enqueue_raw(&json) {
            tracing::warn!("IPC write error: {e}");
            self.connection = None;
            return;
        }
        if let Err(e) = conn.try_flush() {
            tracing::warn!("IPC write error: {e}");
            self.connection = None;
        }
    }

    /// Read and dispatch all complete incoming messages.
    /// Returns decoded messages (caller decides how to handle them).
    /// Returns `None` on connection close/error (caller should handle disconnect).
    pub fn recv_all(&mut self) -> Option<Vec<IncomingMessage>> {
        let conn = self.connection.as_mut()?;
        match conn.fill_read_buf() {
            Err(e) => {
                tracing::warn!("IPC read error: {e}");
                self.connection = None;
                return None;
            }
            Ok(true) => {
                tracing::info!("control client disconnected");
                self.connection = None;
                return None;
            }
            Ok(false) => {}
        }

        let mut msgs = Vec::new();
        loop {
            match conn.try_recv() {
                Ok(Some(payload)) => match jsonrpc::parse_incoming(&payload) {
                    Ok(msg) => msgs.push(msg),
                    Err(e) => {
                        tracing::warn!(
                            "IPC parse error: {e} — payload: {}",
                            String::from_utf8_lossy(&payload)
                        );
                    }
                },
                Ok(None) => break,
                Err(e) => {
                    tracing::warn!("IPC protocol error: {e}");
                    self.connection = None;
                    return None;
                }
            }
        }
        Some(msgs)
    }

    /// Drain any bytes parked in the write buffer after a previous EAGAIN.
    /// Called every event-loop tick so late messages don't sit indefinitely.
    pub fn flush(&mut self) {
        if let Some(conn) = &mut self.connection {
            if let Err(e) = conn.try_flush() {
                tracing::warn!("IPC flush error: {e}");
                self.connection = None;
            }
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connection.is_some()
    }

    /// Raw fd of the listener socket (for calloop registration).
    pub fn listener_fd(&self) -> std::os::unix::io::RawFd {
        use std::os::unix::io::AsRawFd;
        self.listener.as_raw_fd()
    }

    /// Raw fd of the active connection, if any.
    pub fn connection_fd(&self) -> Option<std::os::unix::io::RawFd> {
        use std::os::unix::io::AsRawFd;
        self.connection.as_ref().map(|c| c.stream.as_raw_fd())
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}
