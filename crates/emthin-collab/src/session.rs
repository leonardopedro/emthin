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
//! talks to the host needs [`crate::Exchange`] and [`crate::Line`] directly; it does not
//! need a second session.

use std::collections::BTreeMap;
use std::fmt;

use mathed_core::doc::{DocError, MathDoc};
use mathed_core::sync::{Presence, PresenceStore};

use crate::anchor::{AnchorStore, CaretAnchor};
use crate::grants::{Grant, GrantTable, Principal};
use crate::line::{Delivery, Line, LineError, Seq};
use crate::wire::{ShareId, ENVELOPE_VERSION, MAX_FRAME, PROTOCOL_VERSION_V2};

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
    /// The recipient's document-scope generation, when it negotiated one.
    /// Legacy peers get `None` and see exactly the old wire shape.
    pub scope: Option<u64>,
}

/// The result of accepting a v2 handshake. The client echoes `share_id` and
/// `epoch` on every later frame; a mismatch is rejected before any document
/// state is touched (K3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    pub outbound: Outbound,
    pub share_id: ShareId,
    pub epoch: u64,
    pub protocol_version: u16,
    pub envelope_version: u16,
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
    /// The frame belongs to a replaced document scope. It is ignored.
    Stale,
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
    /// The handshake named a different share.
    ShareMismatch,
    /// The handshake declared an unsupported protocol version.
    UnsupportedProtocol(u16),
    /// The handshake declared an unsupported envelope version.
    UnsupportedEnvelope(u16),
    /// The frame came from a replaced connection epoch.
    StaleEpoch,
    /// A peer's retained queue exceeded its bound; it must be isolated and
    /// rejoin from a frontier.
    QueueOverflow,
    /// The caller has no grant for this operation. The same error is returned
    /// for a denied principal and an unknown share.
    Denied,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPeer(peer) => write!(f, "unknown peer {peer}"),
            Self::PeerAlreadyJoined(peer) => write!(f, "peer {peer} already joined"),
            Self::Doc(e) => write!(f, "{e}"),
            Self::Line(e) => write!(f, "{e}"),
            Self::Presence(e) => write!(f, "presence payload: {e}"),
            Self::ShareMismatch => f.write_str("handshake named a different share"),
            Self::UnsupportedProtocol(version) => {
                write!(f, "unsupported protocol version {version}")
            }
            Self::UnsupportedEnvelope(version) => {
                write!(f, "unsupported envelope version {version}")
            }
            Self::StaleEpoch => f.write_str("frame came from a replaced connection epoch"),
            Self::QueueOverflow => f.write_str("peer queue exceeded its bound"),
            Self::Denied => f.write_str("no grant for this operation on this share"),
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
            Self::ShareMismatch
            | Self::UnsupportedProtocol(_)
            | Self::UnsupportedEnvelope(_)
            | Self::StaleEpoch
            | Self::QueueOverflow
            | Self::Denied => None,
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
    /// The peer's document scope, learned at join. Frames for this peer carry
    /// that scope so a delayed frame from the previous document is rejected.
    peer_scope: Option<u64>,
    /// The connection epoch assigned when this peer was accepted. A new
    /// connection gets a fresh epoch so a delayed frame from the old one is
    /// ignored even if the document scope is unchanged (K3).
    epoch: u64,
}

/// Generate a stable random share id without a new dependency. Two independent
/// hash draws over process/time/counter state are enough to make paths and Loro
/// peer ids irrelevant to share identity.
fn new_share_id() -> ShareId {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let hasher = RandomState::new();
    let first = hasher.hash_one((
        std::time::SystemTime::now(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
    ));
    let second = hasher.hash_one((first, COUNTER.fetch_add(1, Ordering::Relaxed)));
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&first.to_be_bytes());
    id[8..].copy_from_slice(&second.to_be_bytes());
    id
}

/// A collaboration host: many peers and ephemeral presence, over a document
/// the caller owns.
pub struct Session {
    presence: PresenceStore,
    peers: BTreeMap<String, PeerState>,
    /// Changes whenever the document is replaced. Scope-aware peers echo the
    /// host's value on frames they send here.
    scope: u64,
    /// Stable identity of the share this host serves. Changing the document
    /// changes the share id, so a delayed frame from another share cannot be
    /// mistaken for one belonging to the current document.
    share_id: ShareId,
    /// Epoch assigned to the next accepted connection.
    next_epoch: u64,
    /// Hard ceiling on a peer's retained/buffered queue bytes. The wire cap is
    /// the default so no peer can hold more than one frame's worth of history.
    queue_limit: usize,
    /// The owner's grant table. A local Unix peer credential establishes the
    /// local principal; remote principals must be independently verified.
    grants: GrantTable,
    /// Share-scoped caret anchors, resolved only in compatible document views.
    anchors: AnchorStore,
    local_peer: String,
    local_name: String,
    timeout_ms: i64,
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
        Self::with_queue_limit(local_peer, local_name, timeout_ms, MAX_FRAME)
    }

    /// Create a session with an explicit per-peer queue byte ceiling. A smaller
    /// cap makes stalled-peer isolation testable without huge payloads.
    pub fn with_queue_limit(
        local_peer: impl Into<String>,
        local_name: impl Into<String>,
        timeout_ms: i64,
        queue_limit: usize,
    ) -> Self {
        let local_peer = local_peer.into();
        let local_name = local_name.into();
        Self {
            presence: PresenceStore::new(local_peer.clone(), local_name.clone(), timeout_ms),
            peers: BTreeMap::new(),
            scope: 1,
            share_id: new_share_id(),
            next_epoch: 1,
            queue_limit,
            grants: GrantTable::new(Principal::verified(local_peer.clone())),
            anchors: AnchorStore::new(),
            local_peer,
            local_name,
            timeout_ms,
        }
    }

    /// The local presence id.
    pub fn local_peer(&self) -> &str {
        &self.local_peer
    }

    /// The active document-scope generation.
    pub fn scope(&self) -> u64 {
        self.scope
    }

    /// The stable identity of this share. It is independent of the document
    /// path, display names and Loro peer ids.
    pub fn share_id(&self) -> &ShareId {
        &self.share_id
    }

    /// The current connection epoch counter.
    pub fn epoch(&self) -> u64 {
        self.next_epoch
    }

    /// Forget every peer and every remote presence entry because the document
    /// was replaced. The local identity and display name survive; the scope
    /// advances so delayed frames from the old document are rejected.
    pub fn reset_document_scope(&mut self) {
        self.peers.clear();
        self.scope = self.scope.wrapping_add(1).max(1);
        self.share_id = new_share_id();
        self.next_epoch = 1;
        self.anchors.clear();
        self.presence = PresenceStore::new(
            self.local_peer.clone(),
            self.local_name.clone(),
            self.timeout_ms,
        );
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
        self.join_scoped(doc, peer, version, None)
    }

    /// Join a peer that supplied a document-scope generation. The response
    /// carries this host's scope so the peer can tag its next frame.
    pub fn join_scoped(
        &mut self,
        doc: &MathDoc,
        peer: impl Into<String>,
        version: &[u8],
        peer_scope: Option<u64>,
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
                peer_scope,
                epoch: 0,
            },
        );
        Ok(Outbound {
            peer,
            seq,
            delta,
            scope: peer_scope.map(|_| self.scope),
        })
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
        self.reconnect_scoped(doc, peer, version, None)
    }

    /// Reconnect with an explicit peer scope, used by scope-aware clients.
    pub fn reconnect_scoped(
        &mut self,
        doc: &MathDoc,
        peer: &str,
        version: &[u8],
        peer_scope: Option<u64>,
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
                peer_scope,
                epoch: 0,
            },
        );
        Ok(Outbound {
            peer: peer.to_string(),
            seq,
            delta,
            scope: peer_scope.map(|_| self.scope),
        })
    }

    /// Accept a v2 handshake. The share id, protocol version and envelope
    /// version are validated before a peer or any delta is created, so a
    /// mismatched or replaced scope cannot apply traffic. A fresh epoch is
    /// assigned to this connection.
    #[allow(clippy::too_many_arguments)]
    pub fn join_v2(
        &mut self,
        doc: &MathDoc,
        peer: impl Into<String>,
        version: &[u8],
        share_id: &ShareId,
        protocol_version: u16,
        envelope_version: u16,
        peer_scope: Option<u64>,
    ) -> Result<Handshake, SessionError> {
        self.validate_hello_v2(share_id, protocol_version, envelope_version)?;
        let peer = peer.into();
        if self.peers.contains_key(&peer) {
            return Err(SessionError::PeerAlreadyJoined(peer));
        }
        let epoch = self.next_epoch;
        self.next_epoch = self.next_epoch.wrapping_add(1).max(1);
        let outbound = self.insert_peer(doc, peer, version, peer_scope, epoch)?;
        Ok(Handshake {
            outbound,
            share_id: self.share_id,
            epoch,
            protocol_version,
            envelope_version,
        })
    }

    /// Reconnect a v2 peer. A new epoch is assigned even though the peer id is
    /// unchanged, so frames from the old connection are stale.
    #[allow(clippy::too_many_arguments)]
    pub fn reconnect_v2(
        &mut self,
        doc: &MathDoc,
        peer: &str,
        version: &[u8],
        share_id: &ShareId,
        protocol_version: u16,
        envelope_version: u16,
        peer_scope: Option<u64>,
    ) -> Result<Handshake, SessionError> {
        self.validate_hello_v2(share_id, protocol_version, envelope_version)?;
        if !self.peers.contains_key(peer) {
            return Err(SessionError::UnknownPeer(peer.to_string()));
        }
        let epoch = self.next_epoch;
        self.next_epoch = self.next_epoch.wrapping_add(1).max(1);
        let outbound = self.insert_peer(doc, peer.to_string(), version, peer_scope, epoch)?;
        Ok(Handshake {
            outbound,
            share_id: self.share_id,
            epoch,
            protocol_version,
            envelope_version,
        })
    }

    fn validate_hello_v2(
        &self,
        share_id: &ShareId,
        protocol_version: u16,
        envelope_version: u16,
    ) -> Result<(), SessionError> {
        if share_id != &self.share_id {
            return Err(SessionError::ShareMismatch);
        }
        if protocol_version != PROTOCOL_VERSION_V2 {
            return Err(SessionError::UnsupportedProtocol(protocol_version));
        }
        if envelope_version != ENVELOPE_VERSION {
            return Err(SessionError::UnsupportedEnvelope(envelope_version));
        }
        Ok(())
    }

    fn insert_peer(
        &mut self,
        doc: &MathDoc,
        peer: String,
        version: &[u8],
        peer_scope: Option<u64>,
        epoch: u64,
    ) -> Result<Outbound, SessionError> {
        let delta = doc.export_delta_since(version)?;
        let mut line = Line::with_byte_limit(self.queue_limit);
        let seq = line
            .send_checked(delta.clone(), delta.len())
            .map_err(|_| SessionError::QueueOverflow)?;
        self.peers.insert(
            peer.clone(),
            PeerState {
                line,
                sent_version: doc.version_vector(),
                peer_scope,
                epoch,
            },
        );
        Ok(Outbound {
            peer,
            seq,
            delta,
            scope: peer_scope.map(|_| self.scope),
        })
    }

    /// The epoch currently assigned to `peer`, if it has a live link.
    ///
    /// A client learns this from its join reply and stamps it on later
    /// frames, so a reconnect fences everything the old connection sends.
    pub fn peer_epoch(&self, peer: &str) -> Option<u64> {
        self.peers.get(peer).map(|state| state.epoch)
    }

    /// Whether `epoch` is the epoch currently assigned to `peer`.
    pub fn is_peer_epoch_current(&self, peer: &str, epoch: u64) -> bool {
        self.peers
            .get(peer)
            .is_some_and(|state| state.epoch == epoch)
    }

    /// Apply a v2 delta, rejecting a stale scope or epoch before touching the
    /// document or the reliable line.
    pub fn receive_v2(
        &mut self,
        doc: &mut MathDoc,
        peer: &str,
        seq: Seq,
        delta: &[u8],
        scope: Option<u64>,
        epoch: Option<u64>,
    ) -> Result<ReceiveOutcome, SessionError> {
        if let Some(epoch) = epoch {
            if !self.is_peer_epoch_current(peer, epoch) {
                return Ok(ReceiveOutcome::Stale);
            }
        }
        self.receive_scoped(doc, peer, seq, delta, scope)
    }

    /// Acknowledge a v2 stream only from the current connection epoch.
    pub fn ack_v2(&mut self, peer: &str, seq: Seq, epoch: Option<u64>) -> Result<(), SessionError> {
        if let Some(epoch) = epoch {
            if !self.is_peer_epoch_current(peer, epoch) {
                return Err(SessionError::StaleEpoch);
            }
        }
        self.ack(peer, seq)
    }

    /// Apply v2 presence only from the current connection epoch and scope.
    pub fn apply_presence_v2(
        &self,
        blob: &[u8],
        scope: Option<u64>,
        epoch: Option<u64>,
    ) -> Result<bool, SessionError> {
        if let Some(epoch) = epoch {
            // Presence has no peer id; the epoch must still match the active
            // connection counter's predecessor, i.e. one of the live epochs.
            if !self.peers.values().any(|state| state.epoch == epoch) {
                return Ok(false);
            }
        }
        self.apply_presence_scoped(blob, scope)
    }

    // ----- grants and anchored presence (K4) ---------------------------

    pub fn grant_table(&self) -> &GrantTable {
        &self.grants
    }

    pub fn grant_table_mut(&mut self) -> &mut GrantTable {
        &mut self.grants
    }

    /// Whether this share has explicit grants. An unconfigured share keeps the
    /// legacy open path so existing unscoped clients are not locked out.
    pub fn has_explicit_grants(&self) -> bool {
        self.grants.has_explicit_grants()
    }

    /// The local principal, established from the transport's verified peer
    /// credentials (not from a display name).
    pub fn local_principal(&self) -> Principal {
        Principal::verified(self.local_peer.clone())
    }

    /// Whether `principal` may perform `grant` on this share. A denied and a
    /// missing share are the same outward result (`false`).
    pub fn permitted(&self, principal: &Principal, grant: Grant) -> bool {
        use crate::grants::ShareLookup;
        match self.grants.lookup(&self.share_id, principal) {
            ShareLookup::Access { grants, .. } => grants.contains(&grant),
            ShareLookup::Unavailable => false,
        }
    }

    /// Apply a delta only from a verified editor whose connection epoch was
    /// not invalidated by a revoke. Display names and unverified ids carry no
    /// authority.
    #[allow(clippy::too_many_arguments)]
    pub fn receive_granted(
        &mut self,
        doc: &mut MathDoc,
        principal: &Principal,
        peer: &str,
        seq: Seq,
        delta: &[u8],
        scope: Option<u64>,
        epoch: Option<u64>,
    ) -> Result<ReceiveOutcome, SessionError> {
        if !self.permitted(principal, Grant::Edit) {
            return Err(SessionError::Denied);
        }
        if let Some(epoch) = epoch {
            if self.grants.is_epoch_revoked(peer, epoch) {
                return Err(SessionError::Denied);
            }
        }
        self.receive_v2(doc, peer, seq, delta, scope, epoch)
    }

    /// Relay an outbound frame to a peer that still has a read grant. Revoked
    /// and ungranted peers receive nothing.
    pub fn relay_granted(&self, outbound: Vec<Outbound>) -> Vec<Outbound> {
        outbound
            .into_iter()
            .filter(|out| self.permitted(&Principal::verified(out.peer.clone()), Grant::Read))
            .collect()
    }

    /// Publish a caret anchor with this session's share id and epoch.
    pub fn publish_anchor(&mut self, mut anchor: CaretAnchor) {
        anchor.share_id = self.share_id;
        anchor.epoch = self.epoch().saturating_sub(1).max(1);
        self.anchors.set(anchor);
    }

    pub fn anchor_blob(&self, peer: &str) -> Vec<u8> {
        self.anchors
            .get(peer)
            .map(CaretAnchor::encode)
            .unwrap_or_default()
    }

    /// Merge a remote anchor blob from a granted reader of the current share
    /// and connection epoch. A stale or ungranted blob changes nothing.
    pub fn apply_anchor_blob(
        &mut self,
        blob: &[u8],
        principal: &Principal,
        epoch: Option<u64>,
    ) -> Result<bool, SessionError> {
        if !self.permitted(principal, Grant::Read) {
            return Ok(false);
        }
        let anchor = match CaretAnchor::decode(blob) {
            Ok(anchor) => anchor,
            Err(_) => return Ok(false),
        };
        if anchor.share_id != self.share_id {
            return Ok(false);
        }
        if let Some(epoch) = epoch {
            if anchor.epoch != epoch {
                return Ok(false);
            }
        }
        self.anchors.set(anchor);
        Ok(true)
    }

    /// Resolve a peer's anchor in the current document view. Unresolvable or
    /// share-mismatched anchors return `None` so the caller hides the caret.
    pub fn resolved_caret(&self, peer: &str, doc: &MathDoc) -> Option<usize> {
        let anchor = self.anchors.get(peer)?;
        if anchor.share_id != self.share_id {
            return None;
        }
        if self.grants.is_epoch_revoked(peer, anchor.epoch) {
            return None;
        }
        anchor.resolve(doc)
    }

    pub fn remove_outdated_anchors(&mut self, peer: &str) {
        self.anchors.remove(peer);
    }

    /// The frontier a peer must restart from after a gap beyond retained
    /// history, a queue overflow or a stalled connection. Local edits are
    /// still in the document and are included in this frontier.
    pub fn resync_frontier(&self, doc: &MathDoc) -> Vec<u8> {
        doc.version_vector()
    }

    /// Isolate a stalled peer. Its retained queue is dropped; the other peers
    /// are unaffected.
    pub fn isolate_peer(&mut self, peer: &str) -> bool {
        self.peers.remove(peer).is_some()
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
        let peers: Vec<String> = self.peers.keys().cloned().collect();
        let mut out = Vec::with_capacity(peers.len());
        for peer in peers {
            let delta = {
                let state = self.peers.get(&peer).expect("peer from the same map");
                doc.export_delta_since(&state.sent_version)?
            };
            let scope = {
                let state = self.peers.get_mut(&peer).expect("peer from the same map");
                match state.line.send_checked(delta.clone(), delta.len()) {
                    Ok(seq) => {
                        state.sent_version = version.clone();
                        (state.peer_scope, seq)
                    }
                    Err(LineError::QueueOverflow { .. }) => {
                        // A stalled peer's queue cannot grow without bound.
                        // Isolate it; healthy peers keep their outbounds.
                        self.peers.remove(&peer);
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            out.push(Outbound {
                peer,
                seq: scope.1,
                delta,
                scope: scope.0,
            });
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
        self.receive_scoped(doc, peer, seq, delta, None)
    }

    /// Apply one inbound delta with the document scope it was tagged with.
    /// A mismatched scope is stale traffic from a replaced document and is
    /// ignored; an untagged frame is only accepted from a legacy join.
    pub fn receive_scoped(
        &mut self,
        doc: &mut MathDoc,
        peer: &str,
        seq: Seq,
        delta: &[u8],
        scope: Option<u64>,
    ) -> Result<ReceiveOutcome, SessionError> {
        let state = self
            .peers
            .get_mut(peer)
            .ok_or_else(|| SessionError::UnknownPeer(peer.to_string()))?;
        match scope {
            Some(scope) if scope != self.scope => return Ok(ReceiveOutcome::Stale),
            None if state.peer_scope.is_some() => return Ok(ReceiveOutcome::Stale),
            _ => {}
        }
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
        let others: Vec<String> = self
            .peers
            .keys()
            .filter(|other| *other != peer)
            .cloned()
            .collect();
        let mut outbound = Vec::with_capacity(others.len());
        for other in others {
            let delta = {
                let state = self.peers.get(&other).expect("peer from the same map");
                doc.export_delta_since(&state.sent_version)?
            };
            let sent_state = {
                let state = self.peers.get_mut(&other).expect("peer from the same map");
                match state.line.send_checked(delta.clone(), delta.len()) {
                    Ok(seq) => {
                        state.sent_version = version.clone();
                        (state.peer_scope, seq)
                    }
                    Err(LineError::QueueOverflow { .. }) => {
                        self.peers.remove(&other);
                        continue;
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            outbound.push(Outbound {
                peer: other,
                seq: sent_state.1,
                delta,
                scope: sent_state.0,
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
                scope: state.peer_scope,
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
    pub fn set_name(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.local_name = name.clone();
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

    /// Merge a scoped presence payload. A stale generation is ignored rather
    /// than merged into the new document's peer list.
    pub fn apply_presence_scoped(
        &self,
        blob: &[u8],
        scope: Option<u64>,
    ) -> Result<bool, SessionError> {
        if scope.is_some_and(|scope| scope != self.scope) {
            return Ok(false);
        }
        self.presence.apply(blob).map_err(SessionError::Presence)?;
        Ok(true)
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
        let mut host = host();
        host.set_name("Host");
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
    fn a_scoped_peer_rejects_frames_from_the_old_document() {
        let mut source = doc("base");
        let mut host = host();
        let join = host.join_scoped(&source, "alice", &[], Some(7)).unwrap();
        assert_eq!(join.scope, Some(host.scope()));
        let mut alice = MathDoc::new();
        apply_client(&mut alice, &join);

        let before = alice.version_vector();
        alice.insert(alice.text().len(), " old");
        let old_delta = alice.export_delta_since(&before).unwrap();

        // Replace the document, then rejoin the same peer id.
        host.reset_document_scope();
        let old_scope = host.scope() - 1;
        source = doc("replacement");
        let join = host.join_scoped(&source, "alice", &[], Some(8)).unwrap();
        assert_eq!(join.scope, Some(host.scope()));

        assert_eq!(
            host.receive_scoped(&mut source, "alice", 1, &old_delta, Some(old_scope))
                .unwrap(),
            ReceiveOutcome::Stale
        );
        assert_eq!(source.text(), "replacement");
    }

    #[test]
    fn a_legacy_peer_keeps_the_unscoped_wire_path() {
        let mut source = doc("base");
        let mut host = host();
        let join = host.join(&source, "alice", &[]).unwrap();
        assert_eq!(join.scope, None);
        let mut alice = MathDoc::new();
        apply_client(&mut alice, &join);
        let before = alice.version_vector();
        alice.insert(alice.text().len(), " + legacy");
        let delta = alice.export_delta_since(&before).unwrap();
        assert!(matches!(
            host.receive(&mut source, "alice", 1, &delta).unwrap(),
            ReceiveOutcome::Applied { .. }
        ));
        assert!(source.text().contains("+ legacy"));
    }

    #[test]
    fn stale_presence_is_ignored_without_touching_the_document() {
        let host = host();
        let before = host.presence_blob();
        let client = Session::new("alice", "Alice", 60_000);
        client.set_cursor(Some(3));
        let blob = client.presence_blob();
        assert!(!host.apply_presence_scoped(&blob, Some(999)).unwrap());
        assert!(host.present().is_empty());
        assert_eq!(host.presence_blob(), before);
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

    #[test]
    fn a_v2_handshake_validates_share_and_versions_and_assigns_a_fresh_epoch() {
        let source = doc("base");
        let mut host = host();
        let share_id = *host.share_id();
        assert_eq!(host.epoch(), 1);
        let first = host
            .join_v2(
                &source,
                "alice",
                &[],
                &share_id,
                PROTOCOL_VERSION_V2,
                ENVELOPE_VERSION,
                None,
            )
            .expect("valid v2 handshake");
        assert_eq!(first.epoch, 1);
        assert_eq!(&first.share_id, &share_id);
        assert!(host.is_peer_epoch_current("alice", 1));

        // Another accepted connection gets a distinct epoch.
        let second = host
            .join_v2(
                &source,
                "bob",
                &[],
                &share_id,
                PROTOCOL_VERSION_V2,
                ENVELOPE_VERSION,
                None,
            )
            .expect("second v2 handshake");
        assert_eq!(second.epoch, 2);
        assert!(host.is_peer_epoch_current("bob", 2));
    }

    #[test]
    fn a_v2_handshake_rejects_a_different_share_or_bad_versions() {
        let source = doc("base");
        let mut host = host();
        let share_id = *host.share_id();
        let mut wrong_share = share_id;
        wrong_share[0] ^= 0xff;
        assert!(matches!(
            host.join_v2(
                &source,
                "alice",
                &[],
                &wrong_share,
                PROTOCOL_VERSION_V2,
                ENVELOPE_VERSION,
                None
            ),
            Err(SessionError::ShareMismatch)
        ));
        assert!(matches!(
            host.join_v2(&source, "alice", &[], &share_id, 1, ENVELOPE_VERSION, None),
            Err(SessionError::UnsupportedProtocol(1))
        ));
        assert!(matches!(
            host.join_v2(
                &source,
                "alice",
                &[],
                &share_id,
                PROTOCOL_VERSION_V2,
                0,
                None
            ),
            Err(SessionError::UnsupportedEnvelope(0))
        ));
        assert!(
            !host.has_peer("alice"),
            "no peer was created by a bad handshake"
        );
    }

    #[test]
    fn a_stale_epoch_is_rejected_for_delta_ack_and_presence() {
        let mut source = doc("base");
        let mut host = host();
        let share_id = *host.share_id();
        host.join_v2(
            &source,
            "alice",
            &[],
            &share_id,
            PROTOCOL_VERSION_V2,
            ENVELOPE_VERSION,
            None,
        )
        .expect("handshake");
        let catchup = host.local_edit(&source).unwrap();
        let delta = catchup[0].delta.clone();

        // Reconnect assigns a new epoch; the old one is now stale.
        host.reconnect_v2(
            &source,
            "alice",
            &[],
            &share_id,
            PROTOCOL_VERSION_V2,
            ENVELOPE_VERSION,
            None,
        )
        .expect("reconnect");
        assert!(!host.is_peer_epoch_current("alice", 1));
        assert!(host.is_peer_epoch_current("alice", 2));

        let outcome = host
            .receive_v2(&mut source, "alice", 1, &delta, None, Some(1))
            .expect("stale frame is answered, not applied");
        assert_eq!(outcome, ReceiveOutcome::Stale);
        assert!(matches!(
            host.ack_v2("alice", 1, Some(1)),
            Err(SessionError::StaleEpoch)
        ));
        assert!(!host.apply_presence_v2(&[1, 2, 3], None, Some(1)).unwrap());
    }

    #[test]
    fn a_share_switch_changes_the_share_id_and_discards_old_runtime_state() {
        let source = doc("base");
        let mut host = host();
        let first_share = *host.share_id();
        host.join(&source, "alice", &[]).expect("legacy join");
        host.reset_document_scope();
        assert_ne!(*host.share_id(), first_share);
        assert!(!host.has_peer("alice"));
        assert!(host
            .join_v2(
                &source,
                "alice",
                &[],
                &first_share,
                PROTOCOL_VERSION_V2,
                ENVELOPE_VERSION,
                None
            )
            .is_err());
    }

    #[test]
    fn a_stalled_peer_is_isolated_without_affecting_healthy_peers() {
        let source = doc("base");
        let mut host = host();
        host.join(&source, "slow", &[]).expect("join");
        host.join(&source, "fast", &[]).expect("join");
        // A stalled peer is dropped; its retained queue goes with it and the
        // healthy peer keeps receiving edits (K3).
        assert!(host.isolate_peer("slow"));
        assert!(!host.has_peer("slow"));
        assert!(host.has_peer("fast"));
        let out = host.local_edit(&source).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].peer, "fast");
    }

    #[test]
    fn two_writers_reconnect_and_converge_from_a_frontier() {
        let mut alice_doc = doc("shared");
        // Bob starts from the same Loro document identity, as a late joiner
        // restoring the same share would.
        let mut bob_doc = MathDoc::from_snapshot(&alice_doc.snapshot()).unwrap();
        let mut alice = host();
        let share = *alice.share_id();

        // Alice edits before anyone is connected; the operation is durable in
        // her document, not lost because there was no live line.
        alice_doc.insert(0, "x");
        alice.local_edit(&alice_doc).unwrap();

        // Bob joins with an empty frontier: the catch-up delta is a fresh
        // handshake from the beginning of the share (K3).
        let handshake = alice
            .join_v2(
                &alice_doc,
                "bob",
                &[],
                &share,
                PROTOCOL_VERSION_V2,
                ENVELOPE_VERSION,
                None,
            )
            .expect("handshake");
        bob_doc.import_delta(&handshake.outbound.delta).unwrap();
        assert_eq!(bob_doc.text(), alice_doc.text());

        // Bob edits while offline; Alice accepts his reliable delta on the
        // v2 line; both converge.
        bob_doc.insert(bob_doc.len(), "y");
        let delta = bob_doc
            .export_delta_since(&alice_doc.version_vector())
            .expect("delta since the shared frontier");
        alice
            .receive_v2(&mut alice_doc, "bob", 1, &delta, None, Some(1))
            .expect("v2 frame from bob");
        assert_eq!(alice_doc.text(), bob_doc.text());
    }

    #[test]
    fn a_read_only_peer_cannot_commit_but_can_still_read() {
        let mut shared = doc("base");
        let mut host = host();
        let owner = host.local_principal();
        host.grant_table_mut()
            .grant(&owner, "bob", Grant::Read)
            .unwrap();
        host.join(&shared, "bob", &[]).unwrap();
        let catchup = host.local_edit(&shared).unwrap();
        let delta = catchup[0].delta.clone();

        let before = shared.text().to_string();
        let outcome = host.receive_granted(
            &mut shared,
            &Principal::verified("bob"),
            "bob",
            1,
            &delta,
            None,
            None,
        );
        assert!(matches!(outcome, Err(SessionError::Denied)));
        assert_eq!(shared.text(), before, "a denied commit changes nothing");
        assert!(host.permitted(&Principal::verified("bob"), Grant::Read));
        assert!(!host.permitted(&Principal::verified("bob"), Grant::Edit));
    }

    #[test]
    fn revoke_stops_exchange_and_invalidates_pending_work() {
        let mut shared = doc("base");
        let mut host = host();
        let owner = host.local_principal();
        host.grant_table_mut()
            .grant(&owner, "bob", Grant::Read)
            .unwrap();
        host.grant_table_mut()
            .grant(&owner, "bob", Grant::Edit)
            .unwrap();
        let share = *host.share_id();
        host.join_v2(
            &shared,
            "bob",
            &[],
            &share,
            PROTOCOL_VERSION_V2,
            ENVELOPE_VERSION,
            None,
        )
        .unwrap();
        let catchup = host.local_edit(&shared).unwrap();
        let delta = catchup[0].delta.clone();
        host.grant_table_mut().revoke(&owner, "bob", 1).unwrap();

        assert!(!host.permitted(&Principal::verified("bob"), Grant::Read));
        assert!(matches!(
            host.receive_granted(
                &mut shared,
                &Principal::verified("bob"),
                "bob",
                1,
                &delta,
                None,
                Some(1),
            ),
            Err(SessionError::Denied)
        ));
        assert!(host.relay_granted(catchup).is_empty());
    }

    #[test]
    fn an_anchored_caret_follows_an_insertion_and_a_stale_one_is_ignored() {
        let mut shared = doc("alpha target");
        let mut host = host();
        let owner = host.local_principal();
        host.grant_table_mut()
            .grant(&owner, "bob", Grant::Read)
            .unwrap();

        let byte = shared.text().find("target").unwrap();
        let anchor_data = shared.create_anchor(byte, -1).unwrap();
        let anchor = CaretAnchor::new(
            "bob",
            *host.share_id(),
            1,
            shared.version_vector(),
            anchor_data.encoded,
            anchor_data.side,
            anchor_data.fallback,
        );
        let blob = anchor.encode();
        assert!(host
            .apply_anchor_blob(&blob, &Principal::verified("bob"), Some(1))
            .unwrap());
        let resolved = host.resolved_caret("bob", &shared).unwrap();
        assert_eq!(&shared.text()[resolved..resolved + 6], "target");

        shared.insert(0, "prefix ");
        shared.commit();
        let moved = host.resolved_caret("bob", &shared).unwrap();
        assert_eq!(&shared.text()[moved..moved + 6], "target");

        // A stale epoch cannot publish a caret that changes the document view.
        assert!(!host
            .apply_anchor_blob(&blob, &Principal::verified("bob"), Some(99))
            .unwrap());
        let before = shared.text().to_string();
        assert_eq!(host.resolved_caret("bob", &shared), Some(moved));
        assert_eq!(shared.text(), before);
    }

    #[test]
    fn a_forged_display_name_carries_no_authority() {
        let mut shared = doc("base");
        let mut host = host();
        let before = shared.text().to_string();
        // The presence display name says "owner" but the transport identity is
        // unverified, so neither reading grants nor committing is possible.
        let spoof = Principal::unverified("owner");
        assert!(!host.permitted(&spoof, Grant::Edit));
        assert!(matches!(
            host.receive_granted(&mut shared, &spoof, "mallory", 1, b"delta", None, None),
            Err(SessionError::Denied)
        ));
        assert_eq!(shared.text(), before);
    }
}
