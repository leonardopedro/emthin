//! Anchored, share-scoped collaborator carets (Kyneta K4).
//!
//! A raw byte cursor is wrong the moment a peer inserts text above it. A
//! `CaretAnchor` carries an encoded biased Loro cursor plus the frontier it was
//! captured from, so a receiver can check that its document view is compatible
//! before resolving and hide the caret when the source is missing.

use std::collections::BTreeMap;
use std::fmt;

use mathed_core::doc::MathDoc;

use crate::wire::ShareId;

/// A malformed anchored-presence payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchorError {
    TooShort { len: usize },
    BadPeer,
    BadVersion { len: usize },
    IncompatibleFrontier,
}

impl fmt::Display for AnchorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { len } => write!(f, "anchor payload is {len} bytes, too short"),
            Self::BadPeer => f.write_str("anchor peer id is empty"),
            Self::BadVersion { len } => write!(f, "anchor frontier is {len} bytes, malformed"),
            Self::IncompatibleFrontier => f.write_str("anchor belongs to an incompatible view"),
        }
    }
}

impl std::error::Error for AnchorError {}

/// The cap on an anchor payload. It is a small printable/encoded field, not a
/// document payload, so a hostile peer must not make us allocate freely.
pub const MAX_ANCHOR: usize = 8 * 1024;

/// A collaborator caret anchored to document content, not a byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaretAnchor {
    pub peer: String,
    pub share_id: ShareId,
    /// The connection epoch the anchor was published on.
    pub epoch: u64,
    /// The Loro version vector the anchor was captured against.
    pub frontier: Vec<u8>,
    /// `MathDoc::create_anchor`'s encoded biased cursor.
    pub encoded: Vec<u8>,
    pub side: i32,
    /// UTF-8 byte fallback used only when the anchor cannot resolve.
    pub fallback: usize,
}

impl CaretAnchor {
    pub fn new(
        peer: impl Into<String>,
        share_id: ShareId,
        epoch: u64,
        frontier: Vec<u8>,
        encoded: Vec<u8>,
        side: i32,
        fallback: usize,
    ) -> Self {
        Self {
            peer: peer.into(),
            share_id,
            epoch,
            frontier,
            encoded,
            side,
            fallback,
        }
    }

    /// Encode the anchor for transport: `[u16 peer][peer][16 share][u64 epoch]
    /// [u16 frontier][frontier][u16 encoded][encoded][u8 side][u64 fallback]`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            2 + self.peer.len() + 16 + 8 + 2 + self.frontier.len() + 2 + self.encoded.len() + 9,
        );
        out.extend_from_slice(&(self.peer.len() as u16).to_be_bytes());
        out.extend_from_slice(self.peer.as_bytes());
        out.extend_from_slice(&self.share_id);
        out.extend_from_slice(&self.epoch.to_be_bytes());
        out.extend_from_slice(&(self.frontier.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.frontier);
        out.extend_from_slice(&(self.encoded.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.encoded);
        out.push(self.side as u8);
        out.extend_from_slice(&(self.fallback as u64).to_be_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, AnchorError> {
        // 2 peer len + 16 share + 8 epoch + 2 frontier len + 2 encoded len
        // + 1 side + 8 fallback.
        const MIN: usize = 2 + 16 + 8 + 2 + 2 + 1 + 8;
        if bytes.len() < MIN || bytes.len() > MAX_ANCHOR {
            return Err(AnchorError::TooShort { len: bytes.len() });
        }
        let peer_len = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
        if peer_len == 0 {
            return Err(AnchorError::BadPeer);
        }
        let mut at = 2usize.checked_add(peer_len).ok_or(AnchorError::BadPeer)?;
        if at + 16 + 8 + 2 > bytes.len() {
            return Err(AnchorError::TooShort { len: bytes.len() });
        }
        let peer = String::from_utf8_lossy(&bytes[2..at]).into_owned();
        let mut share_id = [0u8; 16];
        share_id.copy_from_slice(&bytes[at..at + 16]);
        at += 16;
        let epoch = u64::from_be_bytes(bytes[at..at + 8].try_into().unwrap());
        at += 8;
        let frontier_len = u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize;
        at += 2;
        let frontier_end = at
            .checked_add(frontier_len)
            .ok_or(AnchorError::BadVersion { len: frontier_len })?;
        if frontier_end + 2 > bytes.len() {
            return Err(AnchorError::BadVersion { len: frontier_len });
        }
        let frontier = bytes[at..frontier_end].to_vec();
        at = frontier_end;
        let encoded_len = u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize;
        at += 2;
        let encoded_end = at
            .checked_add(encoded_len)
            .filter(|end| *end + 1 + 8 <= bytes.len())
            .ok_or(AnchorError::TooShort { len: bytes.len() })?;
        let encoded = bytes[at..encoded_end].to_vec();
        at = encoded_end;
        let side = bytes[at] as i8 as i32;
        at += 1;
        let fallback = u64::from_be_bytes(bytes[at..at + 8].try_into().unwrap()) as usize;
        Ok(Self {
            peer,
            share_id,
            epoch,
            frontier,
            encoded,
            side,
            fallback,
        })
    }

    /// Resolve this anchor in `doc`, but only if the document's current view
    /// still contains the captured frontier. A compatible descendant keeps the
    /// caret attached to the text it was on; an incompatible or deleted source
    /// returns `None` so the caller hides the caret rather than guessing.
    pub fn resolve(&self, doc: &MathDoc) -> Option<usize> {
        if doc.export_delta_since(&self.frontier).is_err() {
            return None;
        }
        let resolved = doc.resolve_anchor(&self.encoded, self.fallback);
        // A fallback result means the anchor no longer resolves in this view.
        // Distinguish that from a genuine resolution at the fallback offset by
        // checking that re-anchoring the resolved byte round-trips.
        if resolved == self.fallback {
            let roundtrip = doc.create_anchor(self.fallback, self.side)?;
            if roundtrip.encoded != self.encoded {
                return None;
            }
        }
        Some(resolved)
    }
}

/// Ephemeral store of remote caret anchors. Nothing here enters document
/// history, undo or persistence.
#[derive(Debug, Default)]
pub struct AnchorStore {
    anchors: BTreeMap<String, CaretAnchor>,
}

impl AnchorStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge an encoded anchor blob. A malformed blob changes nothing.
    pub fn apply(&mut self, blob: &[u8]) -> Result<(), AnchorError> {
        let anchor = CaretAnchor::decode(blob)?;
        self.anchors.insert(anchor.peer.clone(), anchor);
        Ok(())
    }

    pub fn set(&mut self, anchor: CaretAnchor) {
        self.anchors.insert(anchor.peer.clone(), anchor);
    }

    pub fn remove(&mut self, peer: &str) -> Option<CaretAnchor> {
        self.anchors.remove(peer)
    }

    pub fn clear(&mut self) {
        self.anchors.clear();
    }

    pub fn get(&self, peer: &str) -> Option<&CaretAnchor> {
        self.anchors.get(peer)
    }

    pub fn anchors(&self) -> impl Iterator<Item = &CaretAnchor> {
        self.anchors.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_anchor_roundtrips_through_the_codec() {
        let anchor = CaretAnchor::new("ada", [3u8; 16], 2, vec![1, 2, 3], vec![9, 8, 7], -1, 42);
        let decoded = CaretAnchor::decode(&anchor.encode()).unwrap();
        assert_eq!(decoded, anchor);
    }

    #[test]
    fn a_malformed_anchor_is_refused() {
        assert!(matches!(
            CaretAnchor::decode(&[0u8; 3]),
            Err(AnchorError::TooShort { .. })
        ));
        let mut blob = CaretAnchor::new("a", [0; 16], 1, vec![], vec![], 1, 0).encode();
        // Corrupt the peer length to zero.
        blob[0] = 0;
        blob[1] = 0;
        assert!(matches!(
            CaretAnchor::decode(&blob),
            Err(AnchorError::BadPeer)
        ));
    }

    #[test]
    fn an_insertion_above_the_caret_moves_the_anchor() {
        // One shared document: a peer captures an anchor, then a collaborator
        // inserts above it. The anchor must follow the text, not the offset.
        let mut doc = MathDoc::with_text("hello world");
        let target = doc.text().find("world").unwrap();
        let anchored = doc.create_anchor(target, -1).unwrap();
        let anchor = CaretAnchor::new(
            "ada",
            [1u8; 16],
            1,
            doc.version_vector(),
            anchored.encoded,
            anchored.side,
            anchored.fallback,
        );

        doc.insert(0, "prefix ");
        doc.commit();
        let resolved = anchor.resolve(&doc).unwrap();
        assert_eq!(
            &doc.text()[resolved..resolved + "world".len()],
            "world",
            "the collaborator caret follows the text it was attached to"
        );
    }

    #[test]
    fn an_anchor_from_an_incompatible_view_is_hidden() {
        let doc = MathDoc::with_text("hello");
        let anchored = doc.create_anchor(0, -1).unwrap();
        let anchor = CaretAnchor::new(
            "ada",
            [1u8; 16],
            1,
            doc.version_vector(),
            anchored.encoded,
            anchored.side,
            anchored.fallback,
        );
        // A different document with the same text does not share the frontier.
        let foreign = MathDoc::with_text("hello");
        assert!(anchor.resolve(&foreign).is_none());
    }

    #[test]
    fn a_store_merges_and_clears_anchors_without_touching_the_document() {
        let doc = MathDoc::with_text("abc");
        let before = doc.text().to_string();
        let mut store = AnchorStore::new();
        let anchor = CaretAnchor::new("a", [0; 16], 1, doc.version_vector(), vec![], 1, 1);
        let blob = anchor.encode();
        store.apply(&blob).unwrap();
        assert_eq!(store.get("a"), Some(&anchor));
        store.clear();
        assert!(store.anchors().next().is_none());
        assert_eq!(doc.text(), before);
    }
}
