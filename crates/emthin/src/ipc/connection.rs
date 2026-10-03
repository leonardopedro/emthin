use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

/// Maximum allowed IPC message payload size (1 MiB).
const MAX_MSG_SIZE: usize = 1024 * 1024;

/// Maximum size of a message header (8 KiB).
///
/// A well-formed header is `Content-Length: <n>` — under 32 bytes. Nothing
/// legitimate approaches 8 KiB, so anything larger is a peer that is not
/// speaking this protocol.
const MAX_HEADER_SIZE: usize = 8 * 1024;

/// Ceiling on buffered incoming bytes.
///
/// `MAX_MSG_SIZE` bounds a *declared* length, which is why it looked like the
/// buffer was bounded. It is not, in two ways:
///
/// - a peer that never sends `\r\n\r\n` never reaches the length check at
///   all, and `fill_read_buf` appends until the socket drains — so a client
///   writing plain bytes grows the buffer without limit;
/// - a peer that pipelines several maximum-size messages faster than we drain
///   them accumulates all of them first, because `fill_read_buf` reads to
///   `WouldBlock` before any parsing happens.
///
/// Both are reachable from an unprivileged local client that connects to the
/// control socket, so the buffer gets its own ceiling.
///
/// Sized for *two* maximum-size messages plus a header, not one: `fill_read_buf`
/// reads to `WouldBlock`, so one call can absorb whatever the socket holds — a
/// client's default receive buffer is ~208 KiB. A reader part-way through a 1 MiB
/// message that then receives the tail plus a second large message would cross a
/// one-message ceiling and be disconnected for sending data the protocol allows.
/// Two messages is far more than any real client pipelines and still bounded.
const MAX_READ_BUF: usize = 2 * MAX_MSG_SIZE + MAX_HEADER_SIZE + 64 * 1024;

// Checked when the crate is built, not when the test runs: lowering the ceiling
// below what the protocol permits would silently reject legal input.
const _: () = assert!(MAX_MSG_SIZE + 64 < MAX_READ_BUF);
const _: () = assert!(
    MAX_READ_BUF < 16 * MAX_MSG_SIZE,
    "the ceiling must stay a ceiling"
);

/// Ceiling on queued outbound bytes.
///
/// `3ace423` bounded only the inbound side, but the outbound buffer had the same
/// shape of problem: `enqueue_raw` appends without a limit, and `send` calls it
/// for every `figure_bound` / `figure_changed` / `app_title_changed` / `error`
/// — including on every `set_title`. A client that connects and then stops reading
/// grows it for the life of the session.
///
/// Overflow disconnects rather than dropping the backlog silently: a control plane
/// that has stopped reading is no longer receiving a coherent stream anyway, and
/// every message in the buffer would be stale by the time it caught up. Generous
/// enough that a slow reader riding out a burst is fine.
const MAX_WRITE_BUF: usize = 4 * MAX_MSG_SIZE;

const _: () = assert!(
    MAX_WRITE_BUF < 64 * MAX_MSG_SIZE,
    "the ceiling must stay a ceiling"
);

/// A single active IPC connection (one control client).
pub struct IpcConn {
    pub(super) stream: UnixStream,
    /// Incomplete incoming bytes waiting to form a full message.
    read_buf: Vec<u8>,
    /// Serialized bytes queued for writing.
    write_buf: VecDeque<u8>,
}

impl IpcConn {
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            read_buf: Vec::new(),
            write_buf: VecDeque::new(),
        })
    }

    /// Drain available bytes from the stream into `read_buf`.
    /// Returns `true` if the peer closed the connection.
    ///
    /// Errors once `read_buf` would exceed `MAX_READ_BUF`; the caller drops the
    /// connection on any error, so an abusive peer is disconnected rather than
    /// allowed to keep allocating. The check is on the length *after* each read,
    /// so the buffer overshoots by at most one 4 KiB chunk.
    pub fn fill_read_buf(&mut self) -> io::Result<bool> {
        let mut tmp = [0u8; 4096];
        loop {
            match self.stream.read(&mut tmp) {
                Ok(0) => return Ok(true), // EOF — peer closed
                Ok(n) => {
                    self.read_buf.extend_from_slice(&tmp[..n]);
                    if self.read_buf.len() > MAX_READ_BUF {
                        self.read_buf.clear();
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "inbound buffer exceeded {MAX_READ_BUF} bytes \
                                 without a complete message"
                            ),
                        ));
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(e) => return Err(e),
            }
        }
    }

    /// Attempt to decode and return the next complete message, if available.
    /// Returns `Err` if the framed length exceeds `MAX_MSG_SIZE`.
    pub fn try_recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        // Bounding the header search is what makes the length check reachable:
        // without it a peer that never terminates its header never gets parsed,
        // so `len` is never consulted and `MAX_MSG_SIZE` never applies.
        let searchable = self.read_buf.len().min(MAX_HEADER_SIZE);
        let header_end = self.read_buf[..searchable]
            .windows(4)
            .position(|w| w == b"\r\n\r\n");
        let Some(header_end) = header_end else {
            if self.read_buf.len() >= MAX_HEADER_SIZE {
                self.read_buf.clear();
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("header exceeded {MAX_HEADER_SIZE} bytes without a blank line"),
                ));
            }
            return Ok(None);
        };
        let header = &self.read_buf[..header_end];
        let header_str = std::str::from_utf8(header)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-utf8 header"))?;
        let len = header_str
            .strip_prefix("Content-Length:")
            .and_then(|s| s.trim().parse::<usize>().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
        if len > MAX_MSG_SIZE {
            self.read_buf.clear();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Content-Length {len} exceeds maximum {MAX_MSG_SIZE}"),
            ));
        }
        let body_start = header_end + 4;
        let body_end = body_start + len;
        if self.read_buf.len() < body_end {
            return Ok(None);
        }
        let payload = self.read_buf[body_start..body_end].to_vec();
        self.read_buf.drain(..body_end);
        Ok(Some(payload))
    }

    /// Enqueue a raw byte payload with a `Content-Length` header.
    ///
    /// Errors once the queue would exceed `MAX_WRITE_BUF`; the caller drops the
    /// connection, which `IpcServer::send` already does for any write error.
    pub fn enqueue_raw(&mut self, data: &[u8]) -> io::Result<()> {
        let header = format!("Content-Length: {}\r\n\r\n", data.len());
        let added = header.len() + data.len();
        if self.write_buf.len() + added > MAX_WRITE_BUF {
            self.write_buf.clear();
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!(
                    "outbound queue exceeded {MAX_WRITE_BUF} bytes: \
                     the control client is not reading"
                ),
            ));
        }
        self.write_buf.extend(header.as_bytes());
        self.write_buf.extend(data);
        Ok(())
    }

    /// Bytes still queued for writing.
    #[cfg(test)]
    pub fn write_buf_len(&self) -> usize {
        self.write_buf.len()
    }

    /// Flush as many bytes as possible from `write_buf` without blocking.
    /// Returns `true` if there is still data remaining to write.
    pub fn try_flush(&mut self) -> io::Result<bool> {
        while !self.write_buf.is_empty() {
            // Collect contiguous bytes for a single write call.
            let (front, back) = self.write_buf.as_slices();
            let slice = if !front.is_empty() { front } else { back };
            match self.stream.write(slice) {
                Ok(0) => return Ok(!self.write_buf.is_empty()),
                Ok(n) => {
                    self.write_buf.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(true);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(false)
    }

    #[cfg(test)]
    pub fn has_pending_writes(&self) -> bool {
        !self.write_buf.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    fn make_pair() -> (IpcConn, UnixStream) {
        let (a, b) = UnixStream::pair().unwrap();
        (IpcConn::new(a).unwrap(), b)
    }

    fn write_content_length(stream: &mut UnixStream, payload: &[u8]) {
        let header = format!("Content-Length: {}\r\n\r\n", payload.len());
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(payload).unwrap();
    }

    #[test]
    fn try_recv_returns_none_on_empty_buffer() {
        let (mut conn, _peer) = make_pair();
        assert!(conn.try_recv().unwrap().is_none());
    }

    #[test]
    fn try_recv_returns_none_on_incomplete_header() {
        let (mut conn, mut peer) = make_pair();
        peer.write_all(b"Content-L").unwrap();
        conn.fill_read_buf().ok();
        assert!(conn.try_recv().unwrap().is_none());
    }

    #[test]
    fn try_recv_decodes_single_message() {
        let (mut conn, mut peer) = make_pair();
        let payload = b"hello world";
        write_content_length(&mut peer, payload);
        conn.fill_read_buf().ok();
        let msg = conn.try_recv().unwrap().unwrap();
        assert_eq!(msg, payload);
    }

    #[test]
    fn try_recv_returns_none_on_incomplete_payload() {
        let (mut conn, mut peer) = make_pair();
        let header = b"Content-Length: 10\r\n\r\n";
        peer.write_all(header).unwrap();
        peer.write_all(b"hello").unwrap();
        conn.fill_read_buf().ok();
        assert!(conn.try_recv().unwrap().is_none());
    }

    #[test]
    fn try_recv_handles_multiple_messages_in_one_read() {
        let (mut conn, mut peer) = make_pair();
        write_content_length(&mut peer, b"msg1");
        write_content_length(&mut peer, b"msg2");
        write_content_length(&mut peer, b"msg3");
        conn.fill_read_buf().ok();

        assert_eq!(conn.try_recv().unwrap().unwrap(), b"msg1");
        assert_eq!(conn.try_recv().unwrap().unwrap(), b"msg2");
        assert_eq!(conn.try_recv().unwrap().unwrap(), b"msg3");
        assert!(conn.try_recv().unwrap().is_none());
    }

    #[test]
    fn try_recv_handles_empty_payload() {
        let (mut conn, mut peer) = make_pair();
        write_content_length(&mut peer, b"");
        conn.fill_read_buf().ok();
        let msg = conn.try_recv().unwrap().unwrap();
        assert!(msg.is_empty());
    }

    #[test]
    fn try_recv_rejects_oversized_message() {
        let (mut conn, mut peer) = make_pair();
        let header = b"Content-Length: 2097152\r\n\r\n";
        peer.write_all(header).unwrap();
        conn.fill_read_buf().ok();
        let result = conn.try_recv();
        assert!(result.is_err());
    }

    #[test]
    fn enqueue_and_flush_roundtrip() {
        let (mut conn, mut peer) = make_pair();
        peer.set_nonblocking(true).unwrap();

        let payload = b"{\"jsonrpc\":\"2.0\",\"method\":\"test\"}";
        conn.enqueue_raw(payload).expect("enqueue");
        assert!(conn.has_pending_writes());

        conn.try_flush().unwrap();
        assert!(!conn.has_pending_writes());

        // Read the Content-Length framed message from the peer side.
        peer.set_nonblocking(false).unwrap();

        // Read until we see \r\n\r\n
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1];
        loop {
            peer.read_exact(&mut tmp).unwrap();
            buf.push(tmp[0]);
            if buf.len() >= 4 && buf[buf.len() - 4..] == *b"\r\n\r\n" {
                break;
            }
        }
        let header_str = std::str::from_utf8(&buf[..buf.len() - 4]).unwrap();
        let len: usize = header_str
            .strip_prefix("Content-Length: ")
            .and_then(|s| s.trim().parse().ok())
            .unwrap();
        let mut body = vec![0u8; len];
        peer.read_exact(&mut body).unwrap();
        assert_eq!(body, payload);
    }

    #[test]
    fn fill_read_buf_detects_eof() {
        let (mut conn, peer) = make_pair();
        drop(peer); // Close the peer end.
        let eof = conn.fill_read_buf().unwrap();
        assert!(eof);
    }

    /// A peer that never terminates its header must not grow the buffer without
    /// limit.
    ///
    /// `MAX_MSG_SIZE` bounds a *declared* length, so the buffer looked bounded.
    /// It is not: a peer writing plain bytes never reaches the length check at
    /// all, and `fill_read_buf` appends until the socket drains. Any process that
    /// can connect to the control socket could do this.
    #[test]
    fn a_peer_that_never_finishes_its_header_is_cut_off() {
        let (a, mut b) = UnixStream::pair().expect("pair");
        let mut conn = IpcConn::new(a).expect("conn");

        // A peer writing as fast as it can. The socket is non-blocking, so the
        // writer has to retry on EAGAIN like any real client would; the first
        // version of this test used `write_all` and silently wrote only as much
        // as the socket buffer held, which fit under the ceiling and passed
        // without testing anything.
        let writer = std::thread::spawn(move || {
            let chunk = vec![b'x'; 64 * 1024];
            let mut sent = 0usize;
            while sent < 4 * 1024 * 1024 {
                match b.write(&chunk) {
                    Ok(0) => break,
                    Ok(n) => sent += n,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Err(_) => break,
                }
            }
            sent
        });

        let mut refused = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            let before = conn.read_buf.len();
            match conn.fill_read_buf() {
                Ok(_) => assert!(
                    conn.read_buf.len() <= MAX_READ_BUF,
                    "the buffer grew to {} without being refused",
                    conn.read_buf.len()
                ),
                Err(e) => {
                    refused = Some(e);
                    break;
                }
            }
            // Yield rather than spin: a tight loop burns its whole budget
            // before the writer thread is scheduled, which made this test fail
            // under load and pass alone.
            if conn.read_buf.len() == before {
                if writer.is_finished() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }

        let err = refused.expect("an unbounded peer must be refused, not buffered");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{err}");
        assert!(
            conn.read_buf.is_empty(),
            "the buffer must be released on refusal, got {} bytes",
            conn.read_buf.len()
        );
    }

    /// The same peer, but this time a `Content-Length` header that is
    /// syntactically fine and merely enormous — the check that already existed.
    #[test]
    fn an_oversized_declared_length_is_still_refused() {
        let (a, _b) = UnixStream::pair().expect("pair");
        let mut conn = IpcConn::new(a).expect("conn");
        conn.read_buf
            .extend_from_slice(b"Content-Length: 99999999\r\n\r\n");
        let err = conn.try_recv().expect_err("must refuse");
        assert!(err.to_string().contains("exceeds maximum"), "{err}");
    }

    /// A header padded out to something no real client would send.
    #[test]
    fn an_oversized_header_is_refused() {
        let (a, _b) = UnixStream::pair().expect("pair");
        let mut conn = IpcConn::new(a).expect("conn");
        conn.read_buf
            .extend_from_slice(&vec![b'C'; MAX_HEADER_SIZE + 16]);
        let err = conn.try_recv().expect_err("must refuse");
        assert!(err.to_string().contains("header exceeded"), "{err}");
    }

    /// A short header is still parsed — the bound must not reject real messages.
    #[test]
    fn an_ordinary_message_still_round_trips() {
        let (a, _b) = UnixStream::pair().expect("pair");
        let mut conn = IpcConn::new(a).expect("conn");
        conn.enqueue_raw(b"{\"jsonrpc\":\"2.0\"}").expect("enqueue");
        // Reuse the writer's framing to build a well-formed inbound message.
        let framed: Vec<u8> = conn.write_buf.iter().copied().collect();
        conn.write_buf.clear();
        conn.read_buf.extend_from_slice(&framed);
        assert_eq!(
            conn.try_recv().expect("parse"),
            Some(b"{\"jsonrpc\":\"2.0\"}".to_vec())
        );
    }

    /// A maximum-size message is within the new ceiling — the bound must not
    /// reject the largest message the protocol allows.
    #[test]
    fn a_maximum_size_message_is_within_the_buffer_ceiling() {
        let (a, _b) = UnixStream::pair().expect("pair");
        let mut conn = IpcConn::new(a).expect("conn");
        let body = vec![b'.'; MAX_MSG_SIZE];
        conn.enqueue_raw(&body).expect("enqueue");
        let framed: Vec<u8> = conn.write_buf.iter().copied().collect();
        conn.write_buf.clear();
        conn.read_buf.extend_from_slice(&framed);
        assert_eq!(
            conn.try_recv().expect("parse").map(|v| v.len()),
            Some(MAX_MSG_SIZE)
        );
    }

    /// A client that stops reading must not grow the outbound queue for the life
    /// of the session.
    ///
    /// `3ace423` bounded only the inbound buffer. This one had the same shape of
    /// hole: `enqueue_raw` appended without a limit, and `send` calls it for every
    /// `figure_bound` / `figure_changed` / `app_title_changed` / `error` —
    /// including on every `set_title`.
    #[test]
    fn a_client_that_stops_reading_is_disconnected() {
        let (a, _b) = UnixStream::pair().expect("pair");
        let mut conn = IpcConn::new(a).expect("conn");

        // The peer never reads, so nothing is ever flushed to it.
        let chunk = vec![b'x'; 64 * 1024];
        let mut queued = 0usize;
        let mut refused = None;
        for _ in 0..1024 {
            match conn.enqueue_raw(&chunk) {
                Ok(()) => queued += chunk.len(),
                Err(e) => {
                    refused = Some(e);
                    break;
                }
            }
        }

        let err = refused.expect("a client that never reads must be cut off");
        assert_eq!(err.kind(), io::ErrorKind::WriteZero, "{err}");
        assert!(
            queued <= MAX_WRITE_BUF,
            "the queue must never exceed its ceiling, queued {queued} of {MAX_WRITE_BUF}"
        );
        assert_eq!(
            conn.write_buf_len(),
            0,
            "the backlog must be released, not left for a client that never reads"
        );
    }

    /// An ordinary client is unaffected — the ceiling is far above any burst.
    #[test]
    fn an_ordinary_client_is_unaffected_by_the_write_ceiling() {
        let (a, _b) = UnixStream::pair().expect("pair");
        let mut conn = IpcConn::new(a).expect("conn");
        for _ in 0..1024 {
            conn.enqueue_raw(b"{\"jsonrpc\":\"2.0\",\"method\":\"state\"}")
                .expect("a normal burst must be accepted");
        }
        assert!(conn.write_buf_len() < MAX_WRITE_BUF);
    }
}
