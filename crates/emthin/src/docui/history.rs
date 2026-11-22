//! Read-only local document history (Kyneta K5).
//!
//! This is not the jump list in `DocUi` and not the Loro undo stack: it is a
//! bounded ring of retained snapshots, each stamped with the share, frontier
//! and owner that produced it. A preview is plain read-only data derived from
//! a retained snapshot; it never touches the live writable model, its save
//! baselines or its undo status. Only an explicit copy export creates a new
//! local document.

use std::collections::VecDeque;

use emthin_collab::wire::ShareId;
use serde::{Deserialize, Serialize};

/// A serializable row. Byte fields are hex strings so the sidecar stays plain
/// JSON that can be inspected and compared without a binary decoder.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryRow {
    share_id: String,
    frontier: String,
    owner: String,
    seq: u64,
    snapshot: String,
    content_digest: String,
}

/// How many snapshots are retained. Old entries fall off the front so a long
/// session cannot grow the history without bound.
pub const HISTORY_LIMIT: usize = 30;

/// Why a history preview is unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryStatus {
    Available,
    /// The requested frontier was never retained or was compacted out.
    Unavailable {
        reason: String,
    },
}

/// A retained snapshot with its explicit ownership metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetainedSnapshot {
    pub share_id: ShareId,
    pub frontier: Vec<u8>,
    pub owner: String,
    /// The committed change sequence this snapshot closed.
    pub seq: u64,
    pub snapshot: Vec<u8>,
    /// Text plus marks, so a marks-only change is distinguishable.
    pub content_digest: String,
}

/// A read-only view of one retained frontier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPreview {
    pub share_id: ShareId,
    pub frontier: Vec<u8>,
    pub owner: String,
    pub seq: u64,
    pub text: String,
    /// Segment marks as `(range, key)`, sorted by range.
    pub marks: Vec<(std::ops::Range<usize>, String)>,
    pub content_digest: String,
    /// The snapshot an explicit copy export materializes.
    snapshot: Vec<u8>,
}

impl HistoryPreview {
    /// Materialize a new local document from this preview. This is the only
    /// path that creates a writable document from history; a preview alone
    /// never changes the live model.
    pub fn into_local_document(self) -> Result<crate::docui::DocModel, String> {
        crate::docui::DocModel::from_snapshot(&self.snapshot)
            .map_err(|error| format!("retained snapshot is no longer readable: {error}"))
    }

    pub fn snapshot_bytes(&self) -> &[u8] {
        &self.snapshot
    }
}

/// A bounded ring of retained snapshots. Snapshot encoding is Loro's COW
/// format, so retaining one per committed change is cheap.
#[derive(Debug, Clone)]
pub struct HistoryStore {
    entries: VecDeque<RetainedSnapshot>,
    limit: usize,
}

impl Default for HistoryStore {
    fn default() -> Self {
        Self::new(HISTORY_LIMIT)
    }
}

impl HistoryStore {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            limit: limit.max(1),
        }
    }

    /// Serialize the retained ring to a JSON sidecar. A failed encode is an
    /// error the caller can report — history is advisory, but silently
    /// dropping the whole ring on save would lose the evidence it exists for.
    pub fn to_json(&self) -> Result<String, String> {
        let rows: Vec<HistoryRow> = self
            .entries
            .iter()
            .map(|entry| HistoryRow {
                share_id: crate::ipc::hex::encode(&entry.share_id),
                frontier: crate::ipc::hex::encode(&entry.frontier),
                owner: entry.owner.clone(),
                seq: entry.seq,
                snapshot: crate::ipc::hex::encode(&entry.snapshot),
                content_digest: entry.content_digest.clone(),
            })
            .collect();
        serde_json::to_string(&rows).map_err(|error| error.to_string())
    }

    /// Restore a ring from a JSON sidecar. A malformed or truncated row is
    /// skipped rather than discarding the rest, so one bad entry cannot erase
    /// the whole history. The configured limit is always enforced on load.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let rows: Vec<HistoryRow> =
            serde_json::from_str(text).map_err(|error| error.to_string())?;
        let mut store = Self::default();
        for row in rows {
            let share_id = crate::ipc::hex::decode(&row.share_id);
            let frontier = crate::ipc::hex::decode(&row.frontier);
            let snapshot = crate::ipc::hex::decode(&row.snapshot);
            if let (Ok(share_id), Ok(frontier), Ok(snapshot)) = (share_id, frontier, snapshot) {
                let share_id: ShareId = match share_id.try_into() {
                    Ok(id) => id,
                    Err(_) => continue,
                };
                store.retain(RetainedSnapshot {
                    share_id,
                    frontier,
                    owner: row.owner,
                    seq: row.seq,
                    snapshot,
                    content_digest: row.content_digest,
                });
            }
        }
        Ok(store)
    }

    /// Retain a snapshot. Consecutive identical frontiers are coalesced so an
    /// idle view tick does not duplicate history.
    pub fn retain(&mut self, entry: RetainedSnapshot) {
        if self.entries.back().is_some_and(|last| {
            last.frontier == entry.frontier
                && last.share_id == entry.share_id
                && last.content_digest == entry.content_digest
        }) {
            return;
        }
        if self.entries.len() == self.limit {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The retained frontiers, oldest first.
    pub fn frontiers(&self) -> Vec<Vec<u8>> {
        self.entries
            .iter()
            .map(|entry| entry.frontier.clone())
            .collect()
    }

    /// A read-only preview of `frontier`, or an explicit unavailable status if
    /// it was compacted out or belongs to another share.
    pub fn preview(
        &self,
        frontier: &[u8],
        share_id: &ShareId,
    ) -> Result<HistoryPreview, HistoryStatus> {
        let entry = self
            .entries
            .iter()
            .rev()
            .find(|entry| entry.frontier == frontier && &entry.share_id == share_id)
            .ok_or_else(|| HistoryStatus::Unavailable {
                reason: "the requested frontier is not retained (compacted or another share)"
                    .to_string(),
            })?;
        let doc = match crate::docui::DocModel::from_snapshot(&entry.snapshot) {
            Ok(doc) => doc,
            Err(error) => {
                return Err(HistoryStatus::Unavailable {
                    reason: format!("retained snapshot could not be decoded: {error}"),
                })
            }
        };
        let mut marks = doc.math_doc().segment_marks();
        marks.sort_by(|a, b| (a.0.start, a.0.end, &a.1).cmp(&(b.0.start, b.0.end, &b.1)));
        Ok(HistoryPreview {
            share_id: entry.share_id,
            frontier: entry.frontier.clone(),
            owner: entry.owner.clone(),
            seq: entry.seq,
            text: doc.text().to_string(),
            marks,
            content_digest: entry.content_digest.clone(),
            snapshot: entry.snapshot.clone(),
        })
    }

    /// Forget every retained snapshot, e.g. after a document replacement.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docui::DocModel;

    fn entry(doc: &DocModel, share: u8, seq: u64) -> RetainedSnapshot {
        RetainedSnapshot {
            share_id: [share; 16],
            frontier: doc.math_doc().version_vector(),
            owner: "owner".into(),
            seq,
            snapshot: doc.snapshot(),
            content_digest: doc.content_digest(),
        }
    }

    #[test]
    fn the_ring_survives_a_json_round_trip_across_a_relaunch() {
        let model = DocModel::new("starter");
        let mut original = HistoryStore::new(4);
        original.retain(entry(&model, 1, 1));
        original.retain(entry(&model, 2, 2));
        let json = original.to_json().expect("serialize");
        let restored = HistoryStore::from_json(&json).expect("deserialize");
        assert_eq!(restored.frontiers(), original.frontiers());
        assert_eq!(
            restored
                .preview(&original.frontiers()[0], &[1; 16])
                .ok()
                .map(|preview| preview.content_digest),
            original
                .preview(&original.frontiers()[0], &[1; 16])
                .ok()
                .map(|preview| preview.content_digest)
        );
    }

    #[test]
    fn a_malformed_sidecar_row_is_skipped_not_fatal() {
        let json = r#"[{"share_id":"zz","frontier":"","owner":"bad","seq":1,"snapshot":"","content_digest":""},{"share_id":"01010101010101010101010101010101","frontier":"00","owner":"ok","seq":2,"snapshot":"00","content_digest":"d"}]"#;
        let restored = HistoryStore::from_json(json).expect("one good row survives");
        assert_eq!(restored.frontiers().len(), 1);
    }

    #[test]
    fn a_preview_is_read_only_and_does_not_change_the_live_model() {
        let mut live = DocModel::new("one");
        let mut store = HistoryStore::new(4);
        store.retain(entry(&live, 1, 1));
        let frontier = live.math_doc().version_vector();

        live.insert_at_caret(" two");
        let before_text = live.text().to_string();
        let before_revision = live.revision();
        let before_undo = live.can_undo();
        let before_digest = live.content_digest();

        let preview = store.preview(&frontier, &[1u8; 16]).unwrap();
        assert_eq!(preview.text, "one");
        assert_eq!(live.text(), before_text);
        assert_eq!(live.revision(), before_revision);
        assert_eq!(live.can_undo(), before_undo);
        assert_eq!(live.content_digest(), before_digest);
    }

    #[test]
    fn an_explicit_export_creates_a_separate_local_document() {
        let original = DocModel::new("old text");
        let mut store = HistoryStore::new(4);
        store.retain(entry(&original, 2, 1));
        let frontier = original.math_doc().version_vector();
        let exported = store
            .preview(&frontier, &[2u8; 16])
            .unwrap()
            .into_local_document()
            .unwrap();
        assert_eq!(exported.text(), "old text");
        assert_eq!(original.text(), "old text");
        // The export is its own document; editing it cannot affect the source.
        // (DocModel is owned, so a separate clone is made by the caller.)
    }

    #[test]
    fn a_compacted_frontier_reports_an_explicit_status() {
        let mut store = HistoryStore::new(2);
        let first = DocModel::new("first");
        let first_frontier = first.math_doc().version_vector();
        store.retain(entry(&first, 3, 1));
        let second = DocModel::new("second");
        store.retain(entry(&second, 3, 2));
        let third = DocModel::new("third");
        store.retain(entry(&third, 3, 3));
        assert_eq!(store.len(), 2);
        assert!(matches!(
            store.preview(&first_frontier, &[3u8; 16]),
            Err(HistoryStatus::Unavailable { .. })
        ));
    }

    #[test]
    fn a_preview_from_another_share_is_unavailable() {
        let doc = DocModel::new("shared");
        let mut store = HistoryStore::new(4);
        store.retain(entry(&doc, 4, 1));
        let frontier = doc.math_doc().version_vector();
        assert!(matches!(
            store.preview(&frontier, &[9u8; 16]),
            Err(HistoryStatus::Unavailable { .. })
        ));
    }

    #[test]
    fn consecutive_identical_frontiers_are_coalesced() {
        let doc = DocModel::new("same");
        let mut store = HistoryStore::new(4);
        store.retain(entry(&doc, 5, 1));
        store.retain(entry(&doc, 5, 2));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn a_marks_only_change_is_visible_in_the_preview_digest() {
        let mut doc = DocModel::new("hello");
        doc.math_doc_mut().mark_segment(0..5, "prop:bold", "v");
        doc.math_doc_mut().commit();
        let digest = doc.content_digest();
        let mut store = HistoryStore::new(4);
        store.retain(entry(&doc, 6, 1));
        let frontier = doc.math_doc().version_vector();
        let preview = store.preview(&frontier, &[6u8; 16]).unwrap();
        assert_eq!(preview.text, "hello");
        assert_eq!(preview.content_digest, digest);
        assert_eq!(preview.marks, vec![(0..5, "prop:bold".to_string())]);
    }
}
