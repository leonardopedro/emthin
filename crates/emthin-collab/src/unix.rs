//! Non-blocking Unix-socket transport for a [`Session`].
//!
//! [`Host`] is the server half: it accepts connections, performs each `Hello`
//! handshake without waiting for the peer, and reads every established link
//! until it would block. A connection that has not sent `Hello` yet sits in a
//! small pending queue, so one silent client cannot stall the others.
//! [`UnixLink`] is the client half. Both use [`crate::wire`] for framing.
//!
//! A compositor event loop can call [`Host::poll`] once per tick and then keep
//! drawing; it never waits on a peer. [`Host::accept`] and
//! [`Host::receive_from`] remain available for callers that want to drive one
//! connection at a time, and both are non-blocking too.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use mathed_core::doc::{DocError, MathDoc};

use crate::line::Seq;
use crate::session::{Outbound, ReceiveOutcome, Session, SessionError};
use crate::wire::{self, Decoder, Wire, WireError};

/// How many connections may sit in the pre-`Hello` queue.
///
/// A client that connects and never speaks must not be able to grow the host
/// without bound. Real clients send `Hello` immediately; the cap only matters
/// for a misbehaving or half-dead peer. Once it is reached, newly accepted
/// connections are closed instead of queued.
const MAX_PENDING_HANDSHAKES: usize = 64;

/// One non-blocking read from a link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkRead {
    /// A complete frame.
    Message(Wire),
    /// No complete frame is available yet.
    Pending,
    /// The peer closed cleanly.
    Closed,
}

/// One framed connection.
pub struct UnixLink {
    stream: UnixStream,
    decoder: Decoder,
    /// Bytes queued for writing but not yet accepted by the socket.
    write_buf: Vec<u8>,
    /// How much of `write_buf` has been written.
    write_pos: usize,
}

impl UnixLink {
    /// Wrap a connected stream. The stream keeps its current blocking mode.
    pub fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            decoder: Decoder::new(),
            write_buf: Vec::new(),
            write_pos: 0,
        }
    }

    /// Switch the underlying stream between blocking and non-blocking mode.
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        self.stream.set_nonblocking(nonblocking)
    }

    /// Queue one frame and try to write it immediately.
    ///
    /// On a non-blocking stream this can return with bytes still queued when
    /// the socket is full; [`UnixLink::flush`] finishes the write later. A
    /// real write error (a closed peer) is still returned.
    pub fn send(&mut self, msg: &Wire) -> io::Result<()> {
        self.queue(msg);
        self.flush().map(|_| ())
    }

    /// Queue one frame without writing it.
    pub fn queue(&mut self, msg: &Wire) {
        if self.write_pos > 0 {
            self.write_buf.drain(..self.write_pos);
            self.write_pos = 0;
        }
        self.write_buf.extend_from_slice(&wire::encode(msg));
    }

    /// Write as much of the queued output as the socket accepts.
    ///
    /// Returns `Ok(true)` when the queue is empty and `Ok(false)` when a
    /// non-blocking socket would block with bytes left.
    pub fn flush(&mut self) -> io::Result<bool> {
        loop {
            if self.write_pos == self.write_buf.len() {
                self.write_buf.clear();
                self.write_pos = 0;
                return Ok(true);
            }
            match self.stream.write(&self.write_buf[self.write_pos..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "peer stopped accepting bytes",
                    ));
                }
                Ok(n) => self.write_pos += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(e) => return Err(e),
            }
        }
    }

    /// Read one frame, blocking until it arrives.
    ///
    /// `Ok(None)` means the peer closed cleanly; an EOF in the middle of a
    /// frame is an `UnexpectedEof` error. Do not call this on a non-blocking
    /// stream; use [`UnixLink::try_recv`] there.
    pub fn recv(&mut self) -> io::Result<Option<Wire>> {
        loop {
            match self.decoder.next_frame() {
                Ok(Some(msg)) => return Ok(Some(msg)),
                Ok(None) => {}
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            }
            let mut buf = [0u8; 8192];
            match self.stream.read(&mut buf) {
                Ok(0) if self.decoder.has_partial() => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "peer closed in the middle of a frame",
                    ));
                }
                Ok(0) => return Ok(None),
                Ok(n) => self.decoder.push(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// Read whatever is currently available without waiting.
    ///
    /// A non-blocking stream returns [`LinkRead::Pending`] instead of
    /// `WouldBlock`. A blocking stream works too: it reads one frame normally.
    pub fn try_recv(&mut self) -> io::Result<LinkRead> {
        loop {
            match self.decoder.next_frame() {
                Ok(Some(msg)) => return Ok(LinkRead::Message(msg)),
                Ok(None) => {}
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            }
            let mut buf = [0u8; 8192];
            match self.stream.read(&mut buf) {
                Ok(0) if self.decoder.has_partial() => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "peer closed in the middle of a frame",
                    ));
                }
                Ok(0) => return Ok(LinkRead::Closed),
                Ok(n) => self.decoder.push(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(LinkRead::Pending);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Shut down the connection.
    pub fn shutdown(&self) -> io::Result<()> {
        self.stream.shutdown(std::net::Shutdown::Both)
    }
}

/// A host-side transport failure.
#[derive(Debug)]
pub enum HostError {
    Io(io::Error),
    Wire(WireError),
    Session(SessionError),
    /// The first frame was not a `Hello`.
    BadHandshake,
    /// A frame arrived for a peer that has no link.
    UnknownPeer(String),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::Wire(e) => write!(f, "wire: {e}"),
            Self::Session(e) => write!(f, "session: {e}"),
            Self::BadHandshake => f.write_str("first frame was not a Hello"),
            Self::UnknownPeer(peer) => write!(f, "unknown peer {peer}"),
        }
    }
}

impl std::error::Error for HostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Wire(e) => Some(e),
            Self::Session(e) => Some(e),
            Self::BadHandshake | Self::UnknownPeer(_) => None,
        }
    }
}

impl From<io::Error> for HostError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<WireError> for HostError {
    fn from(e: WireError) -> Self {
        Self::Wire(e)
    }
}

impl From<SessionError> for HostError {
    fn from(e: SessionError) -> Self {
        Self::Session(e)
    }
}

impl From<DocError> for HostError {
    fn from(e: DocError) -> Self {
        Self::Session(SessionError::Doc(e))
    }
}

/// One thing that happened while polling the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    /// A completed `Hello` added or reconnected a peer.
    Joined(String),
    /// A peer's connection closed and its session state was dropped.
    Left(String),
}

/// The server half of the Unix-socket transport.
pub struct Host {
    socket_path: PathBuf,
    listener: UnixListener,
    session: Session,
    doc: MathDoc,
    links: BTreeMap<String, UnixLink>,
    /// Accepted connections whose `Hello` has not fully arrived.
    handshaking: Vec<UnixLink>,
}

impl Host {
    /// Bind `path` and serve `doc`.
    ///
    /// `local_peer` / `local_name` label this host's presence; `timeout_ms`
    /// is the presence inactivity window.
    pub fn bind(
        path: impl Into<PathBuf>,
        doc: MathDoc,
        local_peer: impl Into<String>,
        local_name: impl Into<String>,
        timeout_ms: i64,
    ) -> io::Result<Self> {
        let path = path.into();
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            socket_path: path,
            listener,
            session: Session::new(local_peer, local_name, timeout_ms),
            doc,
            links: BTreeMap::new(),
            handshaking: Vec::new(),
        })
    }

    /// The socket path.
    pub fn path(&self) -> &Path {
        &self.socket_path
    }

    /// The document.
    pub fn doc(&self) -> &MathDoc {
        &self.doc
    }

    /// The document, for local edits. Call [`Host::local_edit`] afterwards.
    pub fn doc_mut(&mut self) -> &mut MathDoc {
        &mut self.doc
    }

    /// The coordination state.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Established peers.
    pub fn peers(&self) -> impl Iterator<Item = &str> {
        self.links.keys().map(String::as_str)
    }

    /// Whether `peer` has an established link.
    pub fn has_peer(&self, peer: &str) -> bool {
        self.links.contains_key(peer)
    }

    /// Connections waiting for a complete `Hello`.
    pub fn pending_handshakes(&self) -> usize {
        self.handshaking.len()
    }

    /// Service the listener, every pending handshake, and every peer.
    ///
    /// This never waits for a peer. It accepts everything the listener has,
    /// progresses every `Hello`, reads each link until `WouldBlock`, and
    /// flushes queued output. The returned events describe peers that joined
    /// or left. A bad `Hello` is removed and returned as an error; other links
    /// remain usable on the next poll.
    pub fn poll(&mut self) -> Result<Vec<HostEvent>, HostError> {
        let mut events = Vec::new();
        while self.accept_one()? {}
        while let Some(peer) = self.progress_one_handshake()? {
            events.push(HostEvent::Joined(peer));
        }

        let peers: Vec<String> = self.links.keys().cloned().collect();
        for peer in peers {
            if !self.links.contains_key(&peer) {
                continue;
            }
            match self.receive_from(&peer) {
                Ok(false) => events.push(HostEvent::Left(peer)),
                Ok(true) => {}
                Err(e) => {
                    self.drop_link(&peer);
                    return Err(e);
                }
            }
        }
        self.flush_links()?;
        Ok(events)
    }

    /// Accept at most one pending connection and perform its handshake if it
    /// is already complete.
    ///
    /// Returns `Ok(None)` when no peer joined this call. A connection that has
    /// not finished its `Hello` is kept for the next call; it does not block.
    /// A peer that already has a link is treated as a reconnect: its old link
    /// is replaced and it receives the delta for the version in its `Hello`.
    pub fn accept(&mut self) -> Result<Option<String>, HostError> {
        if let Some(peer) = self.progress_one_handshake()? {
            return Ok(Some(peer));
        }
        if !self.accept_one()? {
            return Ok(None);
        }
        self.progress_one_handshake()
    }

    /// Read and handle every currently available frame from `peer`.
    ///
    /// Returns `Ok(false)` when the peer closed; its link and session state are
    /// removed. Returns `Ok(true)` when frames were handled or no frame was
    /// ready yet.
    pub fn receive_from(&mut self, peer: &str) -> Result<bool, HostError> {
        if !self.links.contains_key(peer) {
            return Err(HostError::UnknownPeer(peer.to_string()));
        }
        loop {
            let outcome = {
                let link = self
                    .links
                    .get_mut(peer)
                    .expect("the link was checked above");
                link.try_recv()?
            };
            match outcome {
                LinkRead::Message(msg) => self.handle(peer, msg)?,
                LinkRead::Pending => {
                    self.flush_peer(peer)?;
                    return Ok(true);
                }
                LinkRead::Closed => {
                    self.drop_link(peer);
                    return Ok(false);
                }
            }
        }
    }

    /// Publish local document changes to every peer.
    pub fn local_edit(&mut self) -> Result<Vec<Outbound>, HostError> {
        let out = self.session.local_edit(&self.doc)?;
        self.route(&out)?;
        self.flush_links()?;
        Ok(out)
    }

    /// Acknowledge `peer`'s reliable stream.
    pub fn ack(&mut self, peer: &str, seq: Seq) -> Result<(), HostError> {
        self.session.ack(peer, seq)?;
        Ok(())
    }

    /// Send this host's current presence to every peer.
    pub fn broadcast_presence(&mut self) -> Result<(), HostError> {
        let msg = Wire::Presence {
            blob: self.session.presence_blob(),
        };
        for link in self.links.values_mut() {
            link.send(&msg)?;
        }
        self.flush_links()
    }

    /// Remove a peer and close its link.
    pub fn leave(&mut self, peer: &str) {
        self.drop_link(peer);
    }

    /// Accept one connection without waiting, queueing it for its handshake.
    ///
    /// Returns `false` when the listener has no pending connection.
    fn accept_one(&mut self) -> Result<bool, HostError> {
        let (stream, _) = match self.listener.accept() {
            Ok(pair) => pair,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        if self.handshaking.len() >= MAX_PENDING_HANDSHAKES {
            // Refuse instead of growing the queue. The listener is still
            // drained, so this cannot spin on the same connection.
            drop(stream);
            return Ok(true);
        }
        stream.set_nonblocking(true)?;
        self.handshaking.push(UnixLink::new(stream));
        Ok(true)
    }

    /// Progress pending handshakes until one completes or all are still
    /// waiting. Closed or malformed handshakes are removed as they are found.
    fn progress_one_handshake(&mut self) -> Result<Option<String>, HostError> {
        let mut index = 0;
        while index < self.handshaking.len() {
            let outcome = self.handshaking[index].try_recv();
            match outcome {
                Ok(LinkRead::Message(Wire::Hello { peer, version })) => {
                    let link = self.handshaking.remove(index);
                    self.establish(peer.clone(), &version, link)?;
                    return Ok(Some(peer));
                }
                Ok(LinkRead::Message(Wire::HelloV2 {
                    peer,
                    share_id,
                    protocol_version,
                    envelope_version,
                    epoch: _,
                    version,
                })) => {
                    let mut link = self.handshaking.remove(index);
                    let handshake = if self.links.contains_key(&peer) {
                        self.session.reconnect_v2(
                            &self.doc,
                            &peer,
                            &version,
                            &share_id,
                            protocol_version,
                            envelope_version,
                            None,
                        )?
                    } else {
                        self.session.join_v2(
                            &self.doc,
                            peer.clone(),
                            &version,
                            &share_id,
                            protocol_version,
                            envelope_version,
                            None,
                        )?
                    };
                    // Echo the accepted share id and the assigned epoch so the
                    // client can fence every later frame by it.
                    link.queue(&Wire::HelloV2 {
                        peer: self.session.local_peer().to_string(),
                        share_id: *self.session.share_id(),
                        protocol_version,
                        envelope_version,
                        epoch: handshake.epoch,
                        version: self.doc.version_vector(),
                    });
                    link.queue(&Wire::Delta {
                        seq: handshake.outbound.seq,
                        delta: handshake.outbound.delta,
                    });
                    self.links.insert(peer.clone(), link);
                    if let Err(e) = self.flush_peer(&peer) {
                        self.drop_link(&peer);
                        return Err(e);
                    }
                    return Ok(Some(peer));
                }
                Ok(LinkRead::Message(_)) => {
                    self.handshaking.remove(index);
                    return Err(HostError::BadHandshake);
                }
                Ok(LinkRead::Pending) => index += 1,
                Ok(LinkRead::Closed) => {
                    self.handshaking.remove(index);
                }
                Err(e) => {
                    self.handshaking.remove(index);
                    return Err(e.into());
                }
            }
        }
        Ok(None)
    }

    /// Attach a handshaken link to the session and deliver its catch-up delta.
    fn establish(
        &mut self,
        peer: String,
        version: &[u8],
        mut link: UnixLink,
    ) -> Result<(), HostError> {
        let out = if self.links.contains_key(&peer) {
            self.session.reconnect(&self.doc, &peer, version)?
        } else {
            self.session.join(&self.doc, &peer, version)?
        };
        link.queue(&Wire::Delta {
            seq: out.seq,
            delta: out.delta,
        });
        self.links.insert(peer.clone(), link);
        if let Err(e) = self.flush_peer(&peer) {
            self.drop_link(&peer);
            return Err(e);
        }
        Ok(())
    }

    /// Handle one decoded frame.
    fn handle(&mut self, peer: &str, msg: Wire) -> Result<(), HostError> {
        match msg {
            Wire::Delta { seq, delta } => {
                let outcome = self.session.receive(&mut self.doc, peer, seq, &delta)?;
                if let ReceiveOutcome::Applied { outbound, .. } = outcome {
                    self.route(&outbound)?;
                }
            }
            Wire::Ack { seq } => self.session.ack(peer, seq)?,
            Wire::Presence { blob } => {
                self.session
                    .apply_presence(&blob)
                    .map_err(HostError::Session)?;
            }
            Wire::Hello { .. } | Wire::HelloV2 { .. } => return Err(HostError::BadHandshake),
            // A Resync is a host→client instruction. A client sending one to
            // the host is a protocol error, not document traffic.
            Wire::Resync { .. } => return Err(HostError::BadHandshake),
        }
        Ok(())
    }

    fn route(&mut self, outbound: &[Outbound]) -> Result<(), HostError> {
        for out in outbound {
            if let Some(link) = self.links.get_mut(&out.peer) {
                link.send(&Wire::Delta {
                    seq: out.seq,
                    delta: out.delta.clone(),
                })?;
            }
        }
        Ok(())
    }

    /// Flush one peer's queued output, dropping it if the socket is broken.
    fn flush_peer(&mut self, peer: &str) -> Result<(), HostError> {
        let result = self.links.get_mut(peer).map(UnixLink::flush);
        match result {
            Some(Ok(_)) => Ok(()),
            Some(Err(e)) => {
                self.drop_link(peer);
                Err(e.into())
            }
            None => Err(HostError::UnknownPeer(peer.to_string())),
        }
    }

    /// Flush every link; a broken peer is removed rather than stopping the
    /// others from receiving their queued output.
    fn flush_links(&mut self) -> Result<(), HostError> {
        let peers: Vec<String> = self.links.keys().cloned().collect();
        let mut failure = None;
        for peer in peers {
            let result = self.links.get_mut(&peer).map(UnixLink::flush);
            match result {
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    self.drop_link(&peer);
                    failure.get_or_insert(e);
                }
                None => {}
            }
        }
        match failure {
            Some(e) => Err(e.into()),
            None => Ok(()),
        }
    }

    fn drop_link(&mut self, peer: &str) {
        self.links.remove(peer);
        self.session.leave(peer);
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn socket_path() -> PathBuf {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("emthin-collab-{}-{n}.sock", std::process::id()))
    }

    fn hello(peer: &str, version: Vec<u8>) -> Wire {
        Wire::Hello {
            peer: peer.into(),
            version,
        }
    }

    fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(value) = f() {
                return value;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn two_instances_converge_through_the_unix_host() {
        // Two disposable instances start from the same share identity and edit
        // independently, then exchange deltas through their Unix hosts (K6).
        let base = MathDoc::with_text("shared");
        let snapshot = base.snapshot();
        let path_a = socket_path();
        let path_b = socket_path();
        let mut a = Host::bind(
            &path_a,
            MathDoc::from_snapshot(&snapshot).unwrap(),
            "instance-a",
            "A",
            60_000,
        )
        .expect("bind a");
        let mut b = Host::bind(
            &path_b,
            MathDoc::from_snapshot(&snapshot).unwrap(),
            "instance-b",
            "B",
            60_000,
        )
        .expect("bind b");

        // Each instance connects to the other and handshakes as a client.
        let mut a_to_b = UnixLink::new(UnixStream::connect(&path_b).expect("a connects to b"));
        a_to_b.send(&hello("instance-a", Vec::new())).unwrap();
        let mut b_to_a = UnixLink::new(UnixStream::connect(&path_a).expect("b connects to a"));
        b_to_a.send(&hello("instance-b", Vec::new())).unwrap();
        assert_eq!(b.accept().unwrap().as_deref(), Some("instance-a"));
        assert_eq!(a.accept().unwrap().as_deref(), Some("instance-b"));

        // Apply each side's catch-up delta to its own document.
        if let Some(Wire::Delta { delta, .. }) = a_to_b.recv().unwrap() {
            a.doc_mut().import_delta(&delta).unwrap();
        }
        if let Some(Wire::Delta { delta, .. }) = b_to_a.recv().unwrap() {
            b.doc_mut().import_delta(&delta).unwrap();
        }
        assert_eq!(a.doc().text(), b.doc().text());

        // Offline-style independent edits, then exchange and converge.
        a.doc_mut().insert(0, "A");
        a.local_edit().unwrap();
        // A's host routes the edit to B's client link; apply it there.
        if let Some(Wire::Delta { delta, .. }) = b_to_a.recv().unwrap() {
            b.doc_mut().import_delta(&delta).unwrap();
        }

        let b_len = b.doc().text().len();
        b.doc_mut().insert(b_len, "B");
        b.local_edit().unwrap();
        if let Some(Wire::Delta { delta, .. }) = a_to_b.recv().unwrap() {
            a.doc_mut().import_delta(&delta).unwrap();
        }

        assert_eq!(a.doc().text(), b.doc().text());
        assert_eq!(a.doc().version_vector(), b.doc().version_vector());
    }

    #[test]
    fn hello_delta_ack_and_presence_cross_a_socket() {
        let path = socket_path();
        let mut host =
            Host::bind(&path, MathDoc::with_text("base"), "host", "Host", 60_000).expect("bind");

        let mut client = UnixLink::new(UnixStream::connect(&path).expect("connect"));
        client.send(&hello("alice", Vec::new())).unwrap();

        assert_eq!(host.accept().unwrap().as_deref(), Some("alice"));
        let Some(Wire::Delta { seq, delta }) = client.recv().unwrap() else {
            panic!("expected the catch-up delta");
        };
        let mut alice = MathDoc::new();
        alice.import_delta(&delta).unwrap();
        assert_eq!(alice.text(), "base");

        client.send(&Wire::Ack { seq }).unwrap();
        assert!(host.receive_from("alice").unwrap());
        assert!(
            host.session().resend("alice", 0).unwrap().is_empty(),
            "the ack must release the catch-up frame"
        );

        // Host edit -> client frame.
        let end = host.doc().text().len();
        host.doc_mut().insert(end, " + host");
        let out = host.local_edit().unwrap();
        assert_eq!(out.len(), 1);
        let Some(Wire::Delta { seq, delta }) = client.recv().unwrap() else {
            panic!("expected the local edit");
        };
        alice.import_delta(&delta).unwrap();
        assert_eq!(alice.text(), host.doc().text());
        client.send(&Wire::Ack { seq }).unwrap();
        assert!(host.receive_from("alice").unwrap());

        // Client edit -> host document.
        let before = alice.version_vector();
        alice.insert(alice.text().len(), " + client");
        let from_client = alice.export_delta_since(&before).unwrap();
        client
            .send(&Wire::Delta {
                seq: 1,
                delta: from_client,
            })
            .unwrap();
        assert!(host.receive_from("alice").unwrap());
        assert!(host.doc().text().contains("+ client"));

        // Presence is carried but does not touch the document.
        host.session().set_cursor(Some(0));
        host.broadcast_presence().unwrap();
        let Some(Wire::Presence { blob }) = client.recv().unwrap() else {
            panic!("expected presence");
        };
        let observer = Session::new("observer", "Observer", 60_000);
        observer.apply_presence(&blob).unwrap();
        assert_eq!(observer.present().len(), 1);
    }

    #[test]
    fn reconnecting_replaces_the_old_link() {
        let path = socket_path();
        let mut host =
            Host::bind(&path, MathDoc::with_text("base"), "host", "Host", 60_000).expect("bind");

        let mut first = UnixLink::new(UnixStream::connect(&path).expect("connect"));
        first.send(&hello("alice", Vec::new())).unwrap();
        assert_eq!(host.accept().unwrap().as_deref(), Some("alice"));
        let _ = first.recv().unwrap();

        let mut second = UnixLink::new(UnixStream::connect(&path).expect("connect"));
        second.send(&hello("alice", Vec::new())).unwrap();
        assert_eq!(host.accept().unwrap().as_deref(), Some("alice"));
        let Some(Wire::Delta { seq, .. }) = second.recv().unwrap() else {
            panic!("expected the reconnect delta");
        };
        assert_eq!(seq, 1, "the reconnect starts a fresh reliable stream");
    }

    #[test]
    fn a_bad_handshake_is_refused() {
        let path = socket_path();
        let mut host = Host::bind(&path, MathDoc::new(), "host", "Host", 60_000).unwrap();
        let mut client = UnixLink::new(UnixStream::connect(&path).unwrap());
        client.send(&Wire::Ack { seq: 1 }).unwrap();
        assert!(matches!(host.accept(), Err(HostError::BadHandshake)));
    }

    #[test]
    fn a_silent_handshake_does_not_block_another_peer() {
        let path = socket_path();
        let mut host = Host::bind(&path, MathDoc::new(), "host", "Host", 60_000).unwrap();

        // Connect and send nothing. `accept` must return without waiting.
        let _silent = UnixStream::connect(&path).unwrap();
        assert_eq!(host.accept().unwrap(), None);
        assert_eq!(host.pending_handshakes(), 1);

        let mut alice = UnixLink::new(UnixStream::connect(&path).unwrap());
        alice.send(&hello("alice", Vec::new())).unwrap();
        assert_eq!(host.accept().unwrap().as_deref(), Some("alice"));
        assert_eq!(host.pending_handshakes(), 1, "the silent peer stays queued");
        let _ = alice.recv().unwrap();
    }

    #[test]
    fn poll_services_two_peers_and_relays_between_them() {
        let path = socket_path();
        let mut host =
            Host::bind(&path, MathDoc::with_text("base"), "host", "Host", 60_000).unwrap();

        let mut alice = UnixLink::new(UnixStream::connect(&path).unwrap());
        let mut bob = UnixLink::new(UnixStream::connect(&path).unwrap());
        alice.send(&hello("alice", Vec::new())).unwrap();
        bob.send(&hello("bob", Vec::new())).unwrap();

        let joined = wait_for("both handshakes", || {
            let events = host.poll().unwrap();
            let joined: Vec<String> = events
                .into_iter()
                .filter_map(|event| match event {
                    HostEvent::Joined(peer) => Some(peer),
                    HostEvent::Left(_) => None,
                })
                .collect();
            (joined.len() == 2).then_some(joined)
        });
        assert_eq!(joined, ["alice", "bob"]);
        assert_eq!(host.peers().collect::<Vec<_>>(), ["alice", "bob"]);

        let mut alice_doc = MathDoc::new();
        let mut bob_doc = MathDoc::new();
        let Some(Wire::Delta { seq, delta }) = alice.recv().unwrap() else {
            panic!("alice catch-up");
        };
        alice_doc.import_delta(&delta).unwrap();
        alice.send(&Wire::Ack { seq }).unwrap();
        let Some(Wire::Delta { seq, delta }) = bob.recv().unwrap() else {
            panic!("bob catch-up");
        };
        bob_doc.import_delta(&delta).unwrap();
        bob.send(&Wire::Ack { seq }).unwrap();

        // Alice edits; one poll applies it and forwards it to Bob only.
        let before = alice_doc.version_vector();
        alice_doc.insert(alice_doc.text().len(), " + alice");
        let from_alice = alice_doc.export_delta_since(&before).unwrap();
        alice
            .send(&Wire::Delta {
                seq: 1,
                delta: from_alice,
            })
            .unwrap();

        wait_for("alice's delta to reach the host", || {
            host.poll().unwrap();
            host.doc().text().contains("+ alice").then_some(())
        });
        let Some(Wire::Delta { seq, delta }) = bob.recv().unwrap() else {
            panic!("bob forwarded delta");
        };
        bob_doc.import_delta(&delta).unwrap();
        assert_eq!(bob_doc.text(), host.doc().text());
        bob.send(&Wire::Ack { seq }).unwrap();
        host.poll().unwrap();

        // A host edit fans out to both peers in the same poll.
        let end = host.doc().text().len();
        host.doc_mut().insert(end, " + host");
        host.local_edit().unwrap();
        let Some(Wire::Delta { seq, delta }) = alice.recv().unwrap() else {
            panic!("alice host edit");
        };
        alice_doc.import_delta(&delta).unwrap();
        alice.send(&Wire::Ack { seq }).unwrap();
        let Some(Wire::Delta { seq, delta }) = bob.recv().unwrap() else {
            panic!("bob host edit");
        };
        bob_doc.import_delta(&delta).unwrap();
        bob.send(&Wire::Ack { seq }).unwrap();
        host.poll().unwrap();

        assert_eq!(alice_doc.text(), host.doc().text());
        assert_eq!(bob_doc.text(), host.doc().text());
    }

    #[test]
    fn a_closing_peer_is_reported_and_removed() {
        let path = socket_path();
        let mut host = Host::bind(&path, MathDoc::new(), "host", "Host", 60_000).unwrap();
        let mut alice = UnixLink::new(UnixStream::connect(&path).unwrap());
        alice.send(&hello("alice", Vec::new())).unwrap();
        assert_eq!(host.accept().unwrap().as_deref(), Some("alice"));
        let _ = alice.recv().unwrap();

        drop(alice);
        let events = wait_for("the close event", || {
            let events = host.poll().unwrap();
            events
                .iter()
                .any(|event| *event == HostEvent::Left("alice".into()))
                .then_some(events)
        });
        assert_eq!(events, [HostEvent::Left("alice".into())]);
        assert!(!host.has_peer("alice"));
        assert!(!host.session().has_peer("alice"));
    }
}
