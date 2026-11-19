//! Document exchange over Loro version vectors.
//!
//! This is the Kyneta `Exchange` idea reduced to the part emthin needs: a
//! peer publishes an opaque version token, the other side returns the
//! operations the peer is missing, and the peer imports them. The token is
//! never parsed by the caller, so the wire protocol is independent of Loro's
//! internal version-vector representation.

use mathed_core::doc::{DocError, MathDoc};

/// A shared document plus the handshake methods for incremental sync.
///
/// Not `Debug`: `MathDoc` owns a whole Loro document and is not `Debug`.
pub struct Exchange {
    doc: MathDoc,
}

impl Exchange {
    /// Wrap an existing document.
    pub fn new(doc: MathDoc) -> Self {
        Self { doc }
    }

    /// The local document.
    pub fn doc(&self) -> &MathDoc {
        &self.doc
    }

    /// The local document, for applying local edits.
    pub fn doc_mut(&mut self) -> &mut MathDoc {
        &mut self.doc
    }

    /// This peer's current version token.
    pub fn version(&self) -> Vec<u8> {
        self.doc.version_vector()
    }

    /// The operations `peer_version` has not seen, encoded for transport.
    ///
    /// An empty token asks for the whole document, which is the right
    /// handshake for a newly connected peer.
    pub fn delta_for(&self, peer_version: &[u8]) -> Result<Vec<u8>, DocError> {
        self.doc.export_delta_since(peer_version)
    }

    /// Merge a delta produced by [`Self::delta_for`].
    pub fn apply(&mut self, delta: &[u8]) -> Result<(), DocError> {
        self.doc.import_delta(delta)
    }

    /// Consume the wrapper and return the document.
    pub fn into_doc(self) -> MathDoc {
        self.doc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::line::{Delivery, Line};

    #[test]
    fn a_new_peer_gets_the_whole_document() {
        let source = Exchange::new(MathDoc::with_text("shared base"));
        let mut late = Exchange::new(MathDoc::new());

        let delta = source.delta_for(&late.version()).expect("initial delta");
        late.apply(&delta).unwrap();

        assert_eq!(late.doc().text(), source.doc().text());
        assert_eq!(late.version(), source.version());
    }

    #[test]
    fn line_buffers_deltas_until_the_missing_prefix_arrives() {
        let base = MathDoc::with_text("base");
        let snapshot = base.snapshot();
        let mut source = Exchange::new(base);
        let mut receiver = Exchange::new(MathDoc::from_snapshot(&snapshot).unwrap());

        let base_version = source.version();
        let end = source.doc().text().len();
        source.doc_mut().insert(end, " + one");
        let first_version = source.version();
        let first = source.delta_for(&base_version).unwrap();

        let end = source.doc().text().len();
        source.doc_mut().insert(end, " + two");
        let second = source.delta_for(&first_version).unwrap();

        let mut line = Line::new();
        let seq_first = line.send(first.clone());
        let seq_second = line.send(second.clone());

        // The first delta is lost; the second is buffered.
        assert_eq!(
            line.receive(seq_second, second.clone()),
            Ok(Delivery::Gap {
                expected: seq_first
            })
        );

        // Replaying the first releases both in order.
        assert_eq!(
            line.receive(seq_first, first.clone()),
            Ok(Delivery::Commands(vec![first.clone(), second.clone()]))
        );
        receiver.apply(&first).unwrap();
        receiver.apply(&second).unwrap();
        assert_eq!(receiver.doc().text(), source.doc().text());

        // A duplicate of either frame is not delivered again.
        assert_eq!(line.receive(seq_first, first), Ok(Delivery::Duplicate));
        assert_eq!(line.receive(seq_second, second), Ok(Delivery::Duplicate));
    }
}
