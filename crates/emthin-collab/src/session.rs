//! Multi-peer collaboration over a host-supplied transport.
//!
//! [`Session`] coordinates one [`Line`] per peer and one [`PresenceStore`].
//! The document is passed in by the caller, so the application keeps
//! ownership of it. The session does not open sockets or serialize frames:
//! every method that produces traffic returns [`Outbound`] values, and the
//! host writes them in whatever framing its transport uses.
//!
//! The session is the **host side** of a star: it applies a remote delta and
//! forwards the relevant operations to the other peers. A client that only
//! talks to the host needs [`Exchange`] and [`Line`] directly; it does not
//! need a second session.

use std::collections::BTreeMap;
use std::fmt;

use mathed_core::doc::{DocError, MathDoc};
use mathed_core::sync::{Presence, PresenceStore};

use crate::line::{Delivery, Line, LineError, Seq};

/// One encoded delta plus the sequence number the transport must carry with
/// it. The host frames `seq` and `delta` for the named peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outbound {
    /// The peer this frame is for.
    pub peer: String,
    /// The reliable-stream sequence number.
    pub seq: Seq,
    /// A Loro delta produced by `Exchange::delta_for`.
    pub delta: Vec<u8>,
}

/// What happened to one inbound delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiveOutcome {
    /// The delta (and any contiguous buffered deltas) was applied. `outbound`
    /// is the traffic the host must send to the other peers.
    Applied {
        operations: usize,
        outbound: Vec<Outbound>,
    },
    /// The sequence number was already delivered or buffered.
    Duplicate,
    /// The delta arrived ahead of a gap and was buffered. `expected` is the
    /// sequence number that must arrive before it can be applied.
    Gap { expected: Seq },
}

/// A session operation that could not be completed.
#[derive(Debug)]
pub enum SessionError {
    /// No peer with this id has joined.
    UnknownPeer(String),
    /// A peer with this id has already joined.
    PeerAlreadyJoined(String),
    /// The document rejected a delta or version token.
    Doc(DocError),
    /// The inbound sequence number was malformed.
    Line(LineError),
    /// The presence payload was malformed.
    Presence(Box<str>),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPeer(peer) => write!(f, "unknown peer {peer}"),
            Self::PeerAlreadyJoined(peer) => write!(f, "peer {peer} already joined"),
            Self::Doc(e) => write!(f, "{e}"),
            Self::Line(e) => write!(f, "{e}"),
            Self::Presence(e) => write!(f, "presence payload: {e}"),
        }
    }
}

impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Doc(e) => Some(e),
            Self::Line(e) => Some(e),
            Self::Presence(_) => None,
            Self::UnknownPeer(_) | Self::PeerAlreadyJoined(_) => None,
        }
    }
}

impl From<DocError> for SessionError {
    fn from(e: DocError) -> Self {
        Self::Doc(e)
    }
}

impl From<LineError> for SessionError {
    fn from(e: LineError) -> Self {
        Self::Line(e)
    }
}

/// One remote peer's reliable stream and the document version we have sent
/// them.
struct PeerState {
    line: Line<Vec<u8>>,
    /// The version the peer has been given. Every delta sent to this peer is
    /// computed from this token, so acknowledged and replayed commands never
    /// have to be recomputed.
    sent_version: Vec<u8>,
}

/// A collaboration host: many peers and ephemeral presence, over a document
/// the caller owns.
pub struct Session {
    presence: PresenceStore,
    peers: BTreeMap<String, PeerState>,
}

impl Session {
    /// Create a session.
    ///
    /// `local_peer` is this host's cosmetic presence id; `local_name` is the
    /// display name shown to collaborators. `timeout_ms` is the presence
    /// inactivity window.
    pub fn new(
        local_peer: impl Into<String>,
        local_name: impl Into<String>,
        timeout_ms: i64,
    ) -> Self {
        Self {
            presence: PresenceStore::new(local_peer, local_name, timeout_ms),
            peers: BTreeMap::new(),
        }
    }

    /// The local presence id.
    pub fn local_peer(&self) -> &str {
        self.presence.peer()
    }

    /// Add a peer and return the catch-up delta it must apply.
    ///
    /// An empty `version` asks for the whole document, which is the right
    /// handshake for a newly connected peer.
    pub fn join(
        &mut self,
        doc: &MathDoc,
        peer: impl Into<String>,
        version: &[u8],
    ) -> Result<Outbound, SessionError> {
        let peer = peer.into();
        if self.peers.contains_key(&peer) {
            return Err(SessionError::PeerAlreadyJoined(peer));
        }
        let delta = doc.export_delta_since(version)?;
        let mut line = Line::new();
        let seq = line.send(delta.clone());
        self.peers.insert(
            peer.clone(),
            PeerState {
                line,
                sent_version: doc.version_vector(),
            },
        );
        Ok(Outbound { peer, seq, delta })
    }

    /// Reconnect an existing peer and return the catch-up delta it must
    /// apply.
    ///
    /// The peer supplies the version token it last saw. The reliable stream
    /// is replaced with a fresh one, so the peer must also start a fresh
    /// [`Line`]; the version token — not the old sequence cursor — is what
    /// makes the reconnect lossless. Use [`Session::join`] for a peer that
    /// was never connected.
    pub fn reconnect(
        &mut self,
        doc: &MathDoc,
        peer: &str,
        version: &[u8],
    ) -> Result<Outbound, SessionError> {
        if !self.peers.contains_key(peer) {
            return Err(SessionError::UnknownPeer(peer.to_string()));
        }
        let delta = doc.export_delta_since(version)?;
        let mut line = Line::new();
        let seq = line.send(delta.clone());
        self.peers.insert(
            peer.to_string(),
            PeerState {
                line,
                sent_version: doc.version_vector(),
            },
        );
        Ok(Outbound {
            peer: peer.to_string(),
            seq,
            delta,
        })
    }

    /// Whether `peer` has joined.
    pub fn has_peer(&self, peer: &str) -> bool {
        self.peers.contains_key(peer)
    }

    /// Remove a peer. Returns whether it was present.
    pub fn leave(&mut self, peer: &str) -> bool {
        self.peers.remove(peer).is_some()
    }

    /// Drop every peer, for when the underlying document is replaced rather
    /// than edited. Their version tokens belong to the old document.
    pub fn clear_peers(&mut self) {
        self.peers.clear();
    }

    /// Publish the document's changes since each peer was last sent a delta.
    ///
    /// Call this after editing the passed-in document. A peer with no
    /// unseen operations still receives the small Loro envelope; an empty
    /// peer list produces no traffic.
    pub fn local_edit(&mut self, doc: &MathDoc) -> Result<Vec<Outbound>, SessionError> {
        let version = doc.version_vector();
        let mut sent = Vec::with_capacity(self.peers.len());
        for (peer, state) in &self.peers {
            sent.push((peer.clone(), doc.export_delta_since(&state.sent_version)?));
        }
        let mut out = Vec::with_capacity(sent.len());
        for (peer, delta) in sent {
            let state = self
                .peers
                .get_mut(&peer)
                .expect("peer collected from the same map");
            let seq = state.line.send(delta.clone());
            state.sent_version = version.clone();
            out.push(Outbound { peer, seq, delta });
        }
        Ok(out)
    }

    /// Apply one inbound delta from `peer`, then broadcast the resulting
    /// document state to the other peers.
    ///
    /// The sender is not echoed its own delta. If the sequence number fills a
    /// gap, every command released by the gap is applied in order.
    pub fn receive(
        &mut self,
        doc: &mut MathDoc,
        peer: &str,
        seq: Seq,
        delta: &[u8],
    ) -> Result<ReceiveOutcome, SessionError> {
        let state = self
            .peers
            .get_mut(peer)
            .ok_or_else(|| SessionError::UnknownPeer(peer.to_string()))?;
        let delivery = state.line.receive(seq, delta.to_vec())?;
        let commands = match delivery {
            Delivery::Commands(commands) => commands,
            Delivery::Duplicate => return Ok(ReceiveOutcome::Duplicate),
            Delivery::Gap { expected } => return Ok(ReceiveOutcome::Gap { expected }),
        };
        let operations = commands.len();
        for delta in commands {
            doc.import_delta(&delta)?;
        }

        let version = doc.version_vector();
        let mut sent = Vec::new();
        for (other, state) in &self.peers {
            if other == peer {
                continue;
            }
            sent.push((other.clone(), doc.export_delta_since(&state.sent_version)?));
        }
        let mut outbound = Vec::with_capacity(sent.len());
        for (other, delta) in sent {
            let state = self
                .peers
                .get_mut(&other)
                .expect("peer collected from the same map");
            let seq = state.line.send(delta.clone());
            state.sent_version = version.clone();
            outbound.push(Outbound {
                peer: other,
                seq,
                delta,
            });
        }
        Ok(ReceiveOutcome::Applied {
            operations,
            outbound,
        })
    }

    /// Acknowledge every command through `seq` for `peer`.
    pub fn ack(&mut self, peer: &str, seq: Seq) -> Result<(), SessionError> {
        self.peer_mut(peer)?.line.ack(seq);
        Ok(())
    }

    /// Replay unacknowledged commands from `from` for `peer`.
    pub fn resend(&self, peer: &str, from: Seq) -> Result<Vec<Outbound>, SessionError> {
        let state = self
            .peers
            .get(peer)
            .ok_or_else(|| SessionError::UnknownPeer(peer.to_string()))?;
        Ok(state
            .line
            .resend_from(from)
            .map(|(seq, delta)| Outbound {
                peer: peer.to_string(),
                seq,
                delta: delta.clone(),
            })
            .collect())
    }

    /// The next sequence number expected from `peer`.
    pub fn expected(&self, peer: &str) -> Result<Seq, SessionError> {
        Ok(self.peer(peer)?.line.expected())
    }

    /// The highest contiguous sequence number received from `peer`.
    pub fn ack_cursor(&self, peer: &str) -> Result<Seq, SessionError> {
        Ok(self.peer(peer)?.line.ack_cursor())
    }

    /// Set this host's display name and publish it in the next presence blob.
    pub fn set_name(&self, name: impl Into<String>) {
        self.presence.set_name(name);
    }

    /// Set this host's caret, or `None` when it leaves the document.
    pub fn set_cursor(&self, cursor: Option<usize>) {
        self.presence.set_cursor(cursor);
    }

    /// The local presence payload to broadcast.
    pub fn presence_blob(&self) -> Vec<u8> {
        self.presence.encode()
    }

    /// Merge a remote presence payload. Presence never touches the document.
    pub fn apply_presence(&self, blob: &[u8]) -> Result<(), SessionError> {
        self.presence.apply(blob).map_err(SessionError::Presence)
    }

    /// Live collaborators, self excluded.
    pub fn present(&self) -> Vec<Presence> {
        self.presence.peers()
    }

    /// Prune peers whose presence heartbeat lapsed.
    pub fn remove_outdated_presence(&self) {
        self.presence.remove_outdated();
    }

    fn peer(&self, peer: &str) -> Result<&PeerState, SessionError> {
        self.peers
            .get(peer)
            .ok_or_else(|| SessionError::UnknownPeer(peer.to_string()))
    }

    fn peer_mut(&mut self, peer: &str) -> Result<&mut PeerState, SessionError> {
        self.peers
            .get_mut(peer)
            .ok_or_else(|| SessionError::UnknownPeer(peer.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Session {
        Session::new("host", "Host", 60_000)
    }

    fn doc(text: &str) -> MathDoc {
        MathDoc::with_text(text)
    }

    /// Apply a host outbound frame to a client-side document.
    fn apply_client(doc: &mut MathDoc, out: &Outbound) {
        doc.import_delta(&out.delta).unwrap();
    }

    #[test]
    fn join_sends_the_whole_document_to_a_new_peer() {
        let source = doc("shared");
        let mut host = host();
        let out = host.join(&source, "alice", &[]).unwrap();
        let mut alice = MathDoc::new();
        apply_client(&mut alice, &out);
        assert_eq!(alice.text(), "shared");
    }

    #[test]
    fn local_edit_sends_only_operations_each_peer_has_not_seen() {
        let mut source = doc("base");
        let mut host = host();
        let alice_join = host.join(&source, "alice", &[]).unwrap();
        let bob_join = host.join(&source, "bob", &[]).unwrap();
        let mut alice = MathDoc::new();
        let mut bob = MathDoc::new();
        apply_client(&mut alice, &alice_join);
        apply_client(&mut bob, &bob_join);

        let end = source.text().len();
        source.insert(end, " + edit");
        let out = host.local_edit(&source).unwrap();
        assert_eq!(out.len(), 2);

        let alice_frame = out.iter().find(|o| o.peer == "alice").unwrap();
        let bob_frame = out.iter().find(|o| o.peer == "bob").unwrap();
        apply_client(&mut alice, alice_frame);
        apply_client(&mut bob, bob_frame);
        assert_eq!(alice.text(), source.text());
        assert_eq!(bob.text(), source.text());

        // A second local edit is incremental again, not a full replay.
        let end = source.text().len();
        source.insert(end, " again");
        let out = host.local_edit(&source).unwrap();
        assert_eq!(out.len(), 2);
        for frame in &out {
            apply_client(
                if frame.peer == "alice" {
                    &mut alice
                } else {
                    &mut bob
                },
                frame,
            );
        }
        assert_eq!(alice.text(), source.text());
        assert_eq!(bob.text(), source.text());
    }

    #[test]
    fn an_inbound_delta_is_applied_and_broadcast_to_other_peers() {
        let mut source = doc("base");
        let mut host = host();
        let alice_join = host.join(&source, "alice", &[]).unwrap();
        let bob_join = host.join(&source, "bob", &[]).unwrap();
        let mut alice = MathDoc::new();
        let mut bob = MathDoc::new();
        apply_client(&mut alice, &alice_join);
        apply_client(&mut bob, &bob_join);

        // Alice edits and sends the delta to the host.
        let before = alice.version_vector();
        alice.insert(alice.text().len(), " + alice");
        let from_alice = alice.export_delta_since(&before).unwrap();
        let outcome = host.receive(&mut source, "alice", 1, &from_alice).unwrap();

        let ReceiveOutcome::Applied {
            operations,
            outbound,
        } = outcome
        else {
            panic!("the first delta must apply, got {outcome:?}");
        };
        assert_eq!(operations, 1);
        assert_eq!(outbound.len(), 1, "only Bob should be forwarded to");
        assert_eq!(outbound[0].peer, "bob");
        apply_client(&mut bob, &outbound[0]);

        assert!(source.text().contains("+ alice"));
        assert_eq!(bob.text(), source.text());
    }

    #[test]
    fn a_gap_buffers_and_a_duplicate_is_not_delivered_twice() {
        let mut source = doc("base");
        let mut host = host();
        host.join(&source, "alice", &[]).unwrap();

        let mut alice = MathDoc::new();
        let join = host.resend("alice", 1).unwrap();
        apply_client(&mut alice, &join[0]);

        let before = alice.version_vector();
        alice.insert(0, "one ");
        let first = alice.export_delta_since(&before).unwrap();
        let mid = alice.version_vector();
        alice.insert(alice.text().len(), " two");
        let second = alice.export_delta_since(&mid).unwrap();

        let gap = host.receive(&mut source, "alice", 2, &second).unwrap();
        assert_eq!(gap, ReceiveOutcome::Gap { expected: 1 });
        let applied = host.receive(&mut source, "alice", 1, &first).unwrap();
        assert!(matches!(
            applied,
            ReceiveOutcome::Applied { operations: 2, .. }
        ));
        assert!(source.text().contains("one "));
        assert!(source.text().contains(" two"));

        assert_eq!(
            host.receive(&mut source, "alice", 1, &first).unwrap(),
            ReceiveOutcome::Duplicate
        );
    }

    #[test]
    fn a_reconnecting_peer_gets_only_operations_it_missed() {
        let mut source = doc("base");
        let mut host = host();
        let join = host.join(&source, "alice", &[]).unwrap();
        let mut alice = MathDoc::new();
        apply_client(&mut alice, &join);

        // Alice is connected for this edit.
        let end = source.text().len();
        source.insert(end, " + one");
        let first = host.local_edit(&source).unwrap().remove(0);
        apply_client(&mut alice, &first);

        // Alice disconnects; the host keeps editing.
        let end = source.text().len();
        source.insert(end, " + two");
        let _missed = host.local_edit(&source).unwrap();

        let rejoin = host
            .reconnect(&source, "alice", &alice.version_vector())
            .unwrap();
        apply_client(&mut alice, &rejoin);
        assert_eq!(alice.text(), source.text());
        assert!(alice.text().contains("+ two"));

        // The fresh line restarts at sequence 1, so a frame from the old
        // line's cursor is not mistaken for a duplicate.
        assert_eq!(rejoin.seq, 1);
        assert!(matches!(
            host.receive(&mut source, "alice", 1, &rejoin.delta)
                .unwrap(),
            ReceiveOutcome::Applied { .. }
        ));
        assert_eq!(alice.text(), source.text());
    }

    #[test]
    fn reconnecting_an_unknown_peer_is_rejected() {
        let source = doc("base");
        let mut host = host();
        assert!(matches!(
            host.reconnect(&source, "nobody", &[]),
            Err(SessionError::UnknownPeer(_))
        ));
    }

    #[test]
    fn acked_frames_are_not_replayed_on_reconnect() {
        let mut source = doc("base");
        let mut host = host();
        let first = host.join(&source, "alice", &[]).unwrap();
        let end = source.text().len();
        source.insert(end, " + one");
        let second = host.local_edit(&source).unwrap().remove(0);
        host.ack("alice", second.seq).unwrap();

        let replay = host.resend("alice", 0).unwrap();
        assert_eq!(replay, Vec::<Outbound>::new());
        assert_eq!(first.seq, 1);
    }

    #[test]
    fn presence_roundtrips_without_touching_the_document() {
        let host = host();
        let client = Session::new("alice", "Alice", 60_000);

        host.set_cursor(Some(3));
        client.apply_presence(&host.presence_blob()).unwrap();

        let present = client.present();
        assert_eq!(present.len(), 1);
        assert_eq!(present[0].peer, "host");
        assert_eq!(present[0].cursor, Some(3));
    }

    #[test]
    fn duplicate_join_and_unknown_peer_are_rejected() {
        let source = doc("base");
        let mut host = host();
        host.join(&source, "alice", &[]).unwrap();
        assert!(matches!(
            host.join(&source, "alice", &[]),
            Err(SessionError::PeerAlreadyJoined(_))
        ));
        let mut source = source;
        assert!(matches!(
            host.receive(&mut source, "nobody", 1, &[]),
            Err(SessionError::UnknownPeer(_))
        ));
        assert!(matches!(
            host.ack("nobody", 1),
            Err(SessionError::UnknownPeer(_))
        ));
    }

    #[test]
    fn a_bad_version_token_does_not_create_a_peer() {
        let source = doc("base");
        let mut host = host();
        assert!(matches!(
            host.join(&source, "alice", b"not a version"),
            Err(SessionError::Doc(DocError::BadVersion(_)))
        ));
        assert!(host.local_edit(&source).unwrap().is_empty());
    }
}
