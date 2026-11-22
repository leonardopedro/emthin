//! The document itself: a Loro-backed [`MathDoc`] plus everything the
//! editor needs to interpret it — the marker scan, the resolved
//! segments, the caret/selection, and the invalidation flag that tells
//! the layout cache when a re-layout is due.
//!
//! `MathDoc` owns the text and the undo/redo history; this wrapper owns
//! everything derived from it. The derived state is *recomputed* rather
//! than incrementally patched, because a single `insert` can change
//! every segment after it (marker ids are first-occurrence-wins) and a
//! correct incremental version is where subtle caret bugs live.

use std::ops::Range;

use mathed_core::doc::MathDoc;
use mathed_core::markers::{resolve_segments, scan, MarkerScan, Segment};
use mathed_core::transform::TransformOptions;
use sha2::{Digest as _, Sha256};

use super::change::{
    source_diagnostics, validate_import, ChangeOrigin, ChangeScope, Diagnostic, DocChange,
};

/// Number actual citation keys, excluding named options such as `bib:`.
/// Mathed's scanner retains literal arguments, including named options, in
/// bibliography references; those options must not consume citation numbers.
pub fn citation_references(scan: &MarkerScan) -> Vec<mathed_core::markers::ReferenceEntry> {
    use mathed_core::markers::ReferenceKind;
    let mut next = 1u64;
    let mut references = mathed_core::markers::scan_references(scan);
    for reference in &mut references {
        let count = match &mut reference.kind {
            ReferenceKind::DocumentRef { .. } => 1,
            ReferenceKind::Bibliography { keys } => {
                keys.retain(|key| {
                    !key.split_once(':')
                        .is_some_and(|(name, _)| matches!(name.trim(), "bib" | "style"))
                });
                keys.len().max(1)
            }
        };
        reference.numbers = (next..next + count as u64).collect();
        next += count as u64;
    }
    references
}

/// Document text + derived state + caret.
///
/// Not `Debug`: `MathDoc` is a whole Loro document and isn't `Debug`
/// either; `text()` is what anyone actually wants to log.
pub struct DocModel {
    doc: MathDoc,
    scan: MarkerScan,
    segments: Vec<Segment>,
    caret: usize,
    /// Selection anchor (the fixed end of a selection). `None` = no
    /// selection.
    anchor: Option<usize>,
    /// Set by any content or forced-layout change; cleared by
    /// [`DocModel::take_dirty`] once the layout has processed it.
    dirty: bool,
    /// Bumped by every content mutation and forced layout invalidation. The
    /// caret/selection does not bump it.
    content_generation: u64,
    /// The origin attributed to the next committed change. Non-local edit
    /// paths (peer delta, undo, redo, restore) set this before mutating; the
    /// default is local.
    change_origin: ChangeOrigin,
    /// The last durable content a change was published for: `(source digest,
    /// content digest)`. A marks-only edit keeps the source digest but changes
    /// the content digest, so it still produces a change. Presentation-only
    /// work (caret, pan, a reading-size reflow) changes neither and produces
    /// nothing.
    last_change_state: Option<(String, String)>,
    /// The content generation last checked by `take_changes`. A fast gate so
    /// an idle tick does not hash the whole document.
    last_checked_generation: u64,
    /// Monotonic sequence assigned to published changes.
    change_seq: u64,
}

/// The part of caret/selection state that can change rendered source. Empty
/// caret ranges are deliberately omitted: moving through prose must not force
/// a recompile just because the document contains a marker elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevealKey {
    spans: Vec<Range<usize>>,
}

impl Default for DocModel {
    fn default() -> Self {
        Self::new("")
    }
}

impl DocModel {
    pub fn new(text: &str) -> Self {
        let doc = MathDoc::with_text(text);
        let scan = scan(doc.text());
        let segments = resolve_segments(&scan);
        let mut model = Self {
            doc,
            scan,
            segments,
            caret: 0,
            anchor: None,
            dirty: true,
            content_generation: 1,
            change_origin: ChangeOrigin::Local,
            last_change_state: None,
            last_checked_generation: 1,
            change_seq: 0,
        };
        model.last_change_state = Some(model.change_state());
        model
    }

    /// Rebuild a model from a Loro snapshot (session restore).
    ///
    /// The outer envelope — bounds and non-emptiness — is validated before the
    /// Loro decoder runs. A rejection leaves no partially applied state because
    /// no model has been constructed yet.
    pub fn from_snapshot(bytes: &[u8]) -> Result<Self, mathed_core::doc::DocError> {
        validate_import(bytes, None)
            .map_err(|error| mathed_core::doc::DocError::BadSnapshot(error.to_string()))?;
        let doc = MathDoc::from_snapshot(bytes)?;
        let scan = scan(doc.text());
        let segments = resolve_segments(&scan);
        let mut model = Self {
            doc,
            scan,
            segments,
            caret: 0,
            anchor: None,
            dirty: true,
            content_generation: 1,
            change_origin: ChangeOrigin::Restore,
            last_change_state: None,
            last_checked_generation: 1,
            change_seq: 0,
        };
        model.last_change_state = Some(model.change_state());
        Ok(model)
    }

    /// The document's raw source text (with its hidden markers and
    /// `\name(...)` statements).
    pub fn text(&self) -> &str {
        self.doc.text()
    }

    pub fn math_doc(&self) -> &MathDoc {
        &self.doc
    }

    pub fn math_doc_mut(&mut self) -> &mut MathDoc {
        self.dirty = true;
        self.content_generation = self.content_generation.wrapping_add(1);
        &mut self.doc
    }

    /// The Loro version counter — bumped by any committed mutation.
    pub fn revision(&self) -> u64 {
        self.doc.revision()
    }

    pub fn scan(&self) -> &MarkerScan {
        &self.scan
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn len(&self) -> usize {
        self.doc.len()
    }

    pub fn is_empty(&self) -> bool {
        self.doc.is_empty()
    }

    // ----- caret / selection ----------------------------------------

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// Move the caret, collapsing any selection. This does not dirty the
    /// content; [`DocModel::reveal_key`] tells the layout whether the move
    /// actually changes revealed source.
    pub fn set_caret(&mut self, pos: usize) {
        self.caret = pos.min(self.doc.len());
        self.anchor = None;
    }

    /// Extend the selection to `pos` from the existing anchor (or from
    /// the caret when there is no anchor yet).
    pub fn extend_selection_to(&mut self, pos: usize) {
        let pos = pos.min(self.doc.len());
        let anchor = self.anchor.unwrap_or(self.caret);
        self.anchor = Some(anchor);
        self.caret = pos;
    }

    /// Select `range`, putting the caret at its end. A reversed range
    /// (`11..6`) is normalised: the anchor becomes the range's end, the
    /// caret its start — so drag-selection that goes backwards reads
    /// the same as the forward one.
    pub fn set_selection(&mut self, range: Range<usize>) {
        let end = range.end.min(self.len());
        let start = range.start.min(self.len());
        if start <= end {
            self.anchor = Some(start);
            self.caret = end;
        } else {
            self.anchor = Some(end);
            self.caret = start;
        }
    }

    /// The fixed and active ends of a non-empty selection. Unlike
    /// [`DocModel::selection`] this keeps the direction, so a backward drag
    /// restores with the same active end.
    pub fn selection_ends(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        (anchor != self.caret).then_some((anchor, self.caret))
    }

    /// Restore explicit fixed/active selection ends without normalising them.
    pub fn set_selection_ends(&mut self, anchor: usize, caret: usize) {
        let anchor = anchor.min(self.len());
        let caret = caret.min(self.len());
        if anchor == caret {
            self.anchor = None;
        } else {
            self.anchor = Some(anchor);
        }
        self.caret = caret;
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    pub fn has_selection(&self) -> bool {
        self.anchor.is_some_and(|a| a != self.caret)
    }

    /// The selected range, normalised so `start <= end`.
    pub fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        if anchor == self.caret {
            return None;
        }
        Some(if anchor < self.caret {
            anchor..self.caret
        } else {
            self.caret..anchor
        })
    }

    /// Selected text, or the empty string.
    pub fn selected_text(&self) -> String {
        self.selection()
            .map(|r| self.doc.text()[r].to_string())
            .unwrap_or_default()
    }

    /// Delete the selection (if any) and return it, collapsing the
    /// caret to the deletion point.
    pub fn take_selection(&mut self) -> Option<String> {
        let range = self.selection()?;
        let text = self.doc.text()[range.clone()].to_string();
        self.doc.delete(range.clone());
        self.doc.commit();
        self.caret = range.start;
        self.anchor = None;
        self.rescan();
        Some(text)
    }

    // ----- mutation --------------------------------------------------

    /// Insert `text` at `pos`, moving the caret past it.
    pub fn insert(&mut self, pos: usize, text: &str) {
        let pos = pos.min(self.doc.len());
        self.doc.insert(pos, text);
        self.doc.commit();
        self.caret = pos + text.len();
        self.anchor = None;
        self.rescan();
    }

    /// Insert at the caret.
    pub fn insert_at_caret(&mut self, text: &str) {
        let caret = self.caret;
        self.insert(caret, text);
    }

    /// Delete the selection if there is one, else the grapheme before
    /// the caret. Returns the deleted text.
    pub fn backspace(&mut self) -> Option<String> {
        if let Some(text) = self.take_selection() {
            return Some(text);
        }
        if self.caret == 0 {
            return None;
        }
        let start = previous_grapheme_boundary(self.doc.text(), self.caret);
        let text = self.doc.text()[start..self.caret].to_string();
        self.doc.delete(start..self.caret);
        self.doc.commit();
        self.caret = start;
        self.rescan();
        Some(text)
    }

    /// Delete the selection if there is one, else the grapheme after
    /// the caret.
    pub fn delete_forward(&mut self) -> Option<String> {
        if let Some(text) = self.take_selection() {
            return Some(text);
        }
        let len = self.doc.len();
        if self.caret >= len {
            return None;
        }
        let end = next_grapheme_boundary(self.doc.text(), self.caret);
        let text = self.doc.text()[self.caret..end].to_string();
        self.doc.delete(self.caret..end);
        self.doc.commit();
        self.rescan();
        Some(text)
    }

    /// Replace `range` with `text`, committing as one undo step.
    pub fn replace(&mut self, range: Range<usize>, text: &str) {
        let end = range.end.min(self.doc.len());
        let start = range.start.min(end);
        self.doc.replace(start..end, text);
        self.doc.commit();
        self.caret = start + text.len();
        self.anchor = None;
        self.rescan();
    }

    /// Apply a pre-checked batch as one undo step, then rescan once for
    /// the final text. Used by literal Find/Replace plans.
    pub fn replace_many(&mut self, ops: Vec<mathed_core::doc::ReplaceOp>) {
        self.doc.replace_many(ops);
        self.rescan();
    }

    /// Replace `range` with `text` **without** moving the caret — used
    /// by geometry writes (a figure resize rewriting its `\app` args),
    /// where the caret is about something else entirely.
    pub fn replace_keeping_caret(&mut self, range: Range<usize>, text: &str) {
        let end = range.end.min(self.doc.len());
        let start = range.start.min(end);
        self.doc.replace(start..end, text);
        self.doc.commit();
        self.rescan();
    }

    pub fn undo(&mut self) -> bool {
        let revision = self.doc.revision();
        self.doc.undo();
        if self.doc.revision() == revision {
            return false;
        }
        self.change_origin = ChangeOrigin::Undo;
        self.caret = self.caret.min(self.doc.len());
        self.anchor = None;
        self.rescan();
        true
    }

    pub fn redo(&mut self) -> bool {
        let revision = self.doc.revision();
        self.doc.redo();
        if self.doc.revision() == revision {
            return false;
        }
        self.change_origin = ChangeOrigin::Redo;
        self.caret = self.caret.min(self.doc.len());
        self.anchor = None;
        self.rescan();
        true
    }

    pub fn can_undo(&self) -> bool {
        self.doc.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.doc.can_redo()
    }

    // ----- invalidation ----------------------------------------------

    /// Recompute the scan + segments from the current text.
    pub fn rescan(&mut self) {
        self.scan = scan(self.doc.text());
        self.segments = resolve_segments(&self.scan);
        self.dirty = true;
        self.content_generation = self.content_generation.wrapping_add(1);
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Monotonic content/layout generation. Caret and selection changes do
    /// not affect it.
    pub fn content_generation(&self) -> u64 {
        self.content_generation
    }

    /// Force the next layout to do work without a text change.
    ///
    /// For changes that invalidate the layout but not the text — today, a
    /// reading-size change, which reflows every page.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.content_generation = self.content_generation.wrapping_add(1);
    }

    /// Consume the dirty flag (the layout cache calls this after a
    /// successful re-layout).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// The non-empty reveal ranges for the current caret/selection. Sorted and
    /// deduplicated so ordinary caret movement through prose compares equal.
    pub fn reveal_key(&self) -> RevealKey {
        let mut spans: Vec<Range<usize>> = self
            .reveal_spans()
            .into_iter()
            .filter(|range| range.start < range.end)
            .collect();
        if let Some(span) =
            mathed_mini::render::reveal_span_in(&self.scan, &self.segments, self.caret)
        {
            spans.push(span);
        }
        if let Some(marker) = self
            .scan
            .markers
            .iter()
            .find(|marker| marker.range.start <= self.caret && self.caret <= marker.range.end)
        {
            spans.push(marker.range.clone());
        }
        spans.sort_by_key(|range| (range.start, range.end));
        spans.dedup();
        RevealKey { spans }
    }

    /// The reveal ranges the transform needs for the current caret /
    /// selection: any special-rendered statement the caret touches shows
    /// its raw source instead of its computed rendering.
    pub fn reveal_spans(&self) -> Vec<Range<usize>> {
        let mut spans = Vec::new();
        if let Some(sel) = self.selection() {
            spans.push(sel);
        }
        spans.push(self.caret..self.caret);
        mathed_core::markers::references_for_cursor_segments(
            self.doc.text(),
            &self.segments,
            self.caret,
        )
        .into_iter()
        .for_each(|entry| spans.push(entry.segment_range));
        spans
    }

    /// The [`TransformOptions`] for a layout pass at the current caret.
    pub fn transform_options(&self) -> TransformOptions {
        TransformOptions {
            reveal: self.reveal_spans(),
            references: citation_references(&self.scan),
            ..Default::default()
        }
    }

    /// A Loro snapshot of the document (session persistence).
    pub fn snapshot(&self) -> Vec<u8> {
        self.doc.snapshot()
    }

    /// SHA-256 of the source representation. A source export omits marks, so
    /// this deliberately covers only the text.
    pub fn source_digest(&self) -> String {
        sha256_hex(self.doc.text().as_bytes())
    }

    /// SHA-256 of the logical rich-text content a snapshot file represents:
    /// text plus segment marks, independent of snapshot encoding details.
    pub fn content_digest(&self) -> String {
        let mut hash = Sha256::new();
        let text = self.doc.text();
        hash.update((text.len() as u64).to_le_bytes());
        hash.update(text.as_bytes());
        let mut marks = self.doc.segment_marks();
        marks.sort_by(|a, b| (a.0.start, a.0.end, &a.1).cmp(&(b.0.start, b.0.end, &b.1)));
        hash.update((marks.len() as u64).to_le_bytes());
        for (range, mark) in marks {
            hash.update((range.start as u64).to_le_bytes());
            hash.update((range.end as u64).to_le_bytes());
            hash.update((mark.len() as u64).to_le_bytes());
            hash.update(mark.as_bytes());
        }
        format!("{:x}", hash.finalize())
    }

    // ----- typed committed changes (K1) --------------------------------

    /// Attribute the next committed change to `origin`. Callers on a non-local
    /// edit path (a peer delta, undo/redo, a restore) set this before mutating;
    /// ordinary editing leaves it at [`ChangeOrigin::Local`].
    pub fn set_change_origin(&mut self, origin: ChangeOrigin) {
        self.change_origin = origin;
    }

    fn change_state(&self) -> (String, String) {
        (self.source_digest(), self.content_digest())
    }

    /// Drain the committed changes since the last call, coalescing every edit
    /// in the interval into one summary per origin. The document instance is
    /// supplied by the owner (`DocUi`) because the model does not know which
    /// open document it belongs to.
    ///
    /// A presentation-only change — a caret move, a pan, a status update —
    /// leaves all three state components untouched and produces no change, so
    /// it can never advance a derived durable version.
    pub fn take_changes(&mut self, document: u64) -> Vec<DocChange> {
        if self.last_checked_generation == self.content_generation {
            return Vec::new();
        }
        self.last_checked_generation = self.content_generation;
        let current = self.change_state();
        if self.last_change_state.as_ref() == Some(&current) {
            return Vec::new();
        }
        self.last_change_state = Some(current.clone());
        let origin = self.change_origin;
        // The next mutation on this model is local unless a caller says
        // otherwise. Consuming the origin here prevents a one-shot peer/undo
        // tag from leaking onto a later local keystroke.
        self.change_origin = ChangeOrigin::Local;
        self.change_seq = self.change_seq.wrapping_add(1);
        let seq = self.change_seq;
        let version = hex_encode(&self.doc.version_vector());
        vec![DocChange {
            seq,
            document,
            content_generation: self.content_generation,
            version,
            origin,
            scope: ChangeScope::Content,
            ranges: Vec::new(),
            source_digest: current.0,
            content_digest: current.1,
        }]
    }

    /// The golden set of source diagnostics for the current text, derived from
    /// the shared scanner and semantic index. Compiled verbatim by the pad.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        source_diagnostics(self.text(), &self.scan, &self.segments)
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Byte offset of the grapheme boundary before `pos`.
///
/// Walks back to a *char* boundary first: slicing at an arbitrary byte
/// inside a multi-byte char (the middle of a 4-byte emoji, say) would
/// panic, and a single `graphemes().next_back()` on the un-backed-up
/// prefix would silently split a composed character.
fn previous_grapheme_boundary(text: &str, pos: usize) -> usize {
    let mut start = pos.min(text.len());
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    if start == 0 {
        return 0;
    }
    text[..start]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(offset, _)| offset)
}

/// Byte offset of the grapheme boundary after `pos`.
fn next_grapheme_boundary(text: &str, pos: usize) -> usize {
    let mut start = pos.min(text.len());
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    if start >= text.len() {
        return text.len();
    }
    match text[start..].grapheme_indices(true).next() {
        Some((offset, g)) => start + offset + g.len(),
        None => text.len(),
    }
}

use unicode_segmentation::UnicodeSegmentation;

#[cfg(test)]
mod tests {
    use super::super::change::MAX_IMPORT_BYTES;
    use super::*;

    #[test]
    fn layout_options_include_numbered_citations() {
        let mut doc = DocModel::new("#a referenced #b \\cite(#a,#b) text");
        doc.set_caret(doc.len());
        let options = doc.transform_options();
        assert_eq!(options.references.len(), 1);
        assert_eq!(
            mathed_core::markers::cite_label_text(&options.references[0]),
            "[1]"
        );
    }

    #[test]
    fn insert_moves_the_caret_and_dirties() {
        let mut m = DocModel::new("");
        m.take_dirty();
        m.insert_at_caret("hello");
        assert_eq!(m.text(), "hello");
        assert_eq!(m.caret(), 5);
        assert!(m.is_dirty());
    }

    #[test]
    fn caret_is_clamped_to_the_document() {
        let mut m = DocModel::new("ab");
        m.set_caret(99);
        assert_eq!(m.caret(), 2);
    }

    #[test]
    fn selection_is_normalised_and_readable() {
        let mut m = DocModel::new("hello world");
        m.set_selection(6..11);
        assert!(m.has_selection());
        assert_eq!(m.selection(), Some(6..11));
        assert_eq!(m.selected_text(), "world");
        // A reversed range normalises: the caret always ends up at the
        // *later* byte, whichever end was passed first. Built from two
        // locals so clippy doesn't flag the deliberately-reversed
        // literal.
        let (hi, lo) = (11usize, 6usize);
        m.set_selection(hi..lo);
        assert_eq!(m.selection(), Some(6..11));
        assert_eq!(m.selected_text(), "world");
        assert_eq!(m.caret(), 11);
    }

    #[test]
    fn caret_and_selection_changes_update_the_reveal_key_without_dirting_content() {
        let mut model = DocModel::new("#a hello #b");
        model.take_dirty();
        model.set_caret(4);
        let prose = model.reveal_key();
        model.set_caret(5);
        assert_eq!(
            model.reveal_key(),
            prose,
            "moving within prose does not change the reveal key"
        );
        model.set_caret(1);
        assert!(!model.is_dirty(), "caret motion is not a content edit");
        let in_marker = model.reveal_key();
        assert_ne!(in_marker, prose, "entering a marker changes the key");
        model.extend_selection_to(5);
        assert!(!model.is_dirty());
        assert_ne!(model.reveal_key(), in_marker, "selection reveals source");
        model.clear_selection();
        assert!(!model.is_dirty());
        assert_eq!(
            model.reveal_key(),
            prose,
            "clearing the selection leaves the caret in prose"
        );
        model.set_caret(1);
        assert_eq!(model.reveal_key(), in_marker);
    }

    #[test]
    fn a_stable_anchor_tracks_an_edit_above_it() {
        let mut model = DocModel::new("alpha target");
        let target = model.text().find("target").expect("target");
        let anchor = model.math_doc().create_anchor(target, -1).expect("anchor");
        model.insert(0, "prefix ");
        let resolved = model
            .math_doc()
            .resolve_anchor(&anchor.encoded, anchor.fallback);
        assert_eq!(
            &model.text()[resolved..resolved + "target".len()],
            "target",
            "the logical caret follows the text it was attached to"
        );
    }

    #[test]
    fn set_caret_collapses_a_selection() {
        let mut m = DocModel::new("hello");
        m.set_selection(1..4);
        m.set_caret(2);
        assert!(!m.has_selection());
        assert!(m.selection().is_none());
    }

    #[test]
    fn take_selection_deletes_and_collapses() {
        let mut m = DocModel::new("hello world");
        m.set_selection(0..6);
        assert_eq!(m.take_selection().as_deref(), Some("hello "));
        assert_eq!(m.text(), "world");
        assert_eq!(m.caret(), 0);
        assert!(!m.has_selection());
    }

    #[test]
    fn backspace_deletes_a_grapheme_at_a_time() {
        let mut m = DocModel::new("a👋b");
        m.set_caret("a👋".len());
        assert_eq!(m.backspace().as_deref(), Some("👋"));
        assert_eq!(m.text(), "ab");
        assert_eq!(m.backspace().as_deref(), Some("a"));
        assert_eq!(m.backspace(), None, "nothing before the start");
    }

    #[test]
    fn delete_forward_deletes_a_grapheme_at_a_time() {
        let mut m = DocModel::new("a👋b");
        m.set_caret(1);
        assert_eq!(m.delete_forward().as_deref(), Some("👋"));
        assert_eq!(m.text(), "ab");
        // Walk forward to the end: each step eats exactly one grapheme.
        m.set_caret(0);
        assert_eq!(m.delete_forward().as_deref(), Some("a"));
        assert_eq!(m.delete_forward().as_deref(), Some("b"));
        assert_eq!(m.text(), "");
        assert_eq!(m.delete_forward(), None, "nothing after the end");
    }

    #[test]
    fn backspace_deletes_the_selection_first() {
        let mut m = DocModel::new("hello world");
        m.set_selection(0..6);
        assert_eq!(m.backspace().as_deref(), Some("hello "));
        assert_eq!(m.text(), "world");
    }

    #[test]
    fn replace_keeps_or_moves_the_caret() {
        let mut m = DocModel::new("hello");
        m.replace(1..3, "EL");
        assert_eq!(m.text(), "hELlo");
        assert_eq!(m.caret(), 3, "caret lands after the replacement");
    }

    #[test]
    fn replace_keeping_caret_leaves_the_caret_alone() {
        let mut m = DocModel::new("#1 x #2 \\app(#1, #2, 100, 50)");
        m.set_caret(2);
        m.replace_keeping_caret(36..39, "640");
        assert!(m.text().contains("640"));
        assert_eq!(m.caret(), 2, "caret untouched by a geometry write");
    }

    #[test]
    fn undo_and_redo_round_trip_an_insert() {
        let mut m = DocModel::new("");
        m.insert_at_caret("abc");
        assert!(m.can_undo());
        assert!(m.undo());
        assert_eq!(m.text(), "");
        assert!(m.can_redo());
        assert!(m.redo());
        assert_eq!(m.text(), "abc");
        assert!(!m.redo(), "nothing left to redo");
    }

    #[test]
    fn a_source_digest_ignores_marks_but_the_snapshot_content_digest_does_not() {
        let mut m = DocModel::new("hello");
        let source = m.source_digest();
        let content = m.content_digest();
        m.math_doc_mut().mark_segment(0..5, "prop:bold", "test");
        m.math_doc_mut().commit();
        assert_eq!(m.source_digest(), source, "a source export omits marks");
        assert_ne!(
            m.content_digest(),
            content,
            "a snapshot represents the mark change"
        );
    }

    #[test]
    fn undo_and_redo_invalidate_after_mark_only_changes() {
        let mut m = DocModel::new("hello");
        m.math_doc_mut().mark_segment(0..5, "prop:bold", "test");
        m.math_doc_mut().commit();
        m.take_dirty();
        assert!(m.undo());
        assert!(m.math_doc().segment_marks().is_empty());
        assert!(m.take_dirty());
        assert!(m.redo());
        assert_eq!(
            m.math_doc().segment_marks(),
            vec![(0..5, "prop:bold".into())]
        );
        assert!(m.take_dirty());
    }

    #[test]
    fn rescan_picks_up_new_segments() {
        let mut m = DocModel::new("");
        assert!(m.segments().is_empty());
        m.insert_at_caret("#1 cap #2 \\app(#1, #2, 10, 20)");
        assert_eq!(m.segments().len(), 1);
        assert_eq!(
            m.segments()[0].span,
            Some(2..7),
            "the caption between #1 and #2"
        );
    }

    #[test]
    fn grapheme_boundaries_survive_multibyte_text() {
        let text = "aé👋b";
        assert_eq!(next_grapheme_boundary(text, 1), 1 + "é".len());
        assert_eq!(previous_grapheme_boundary(text, 1 + "é".len()), 1);
        let zwj = "👩‍👩‍👧";
        assert_eq!(next_grapheme_boundary(zwj, 0), zwj.len());
    }

    #[test]
    fn reveal_spans_cover_the_caret() {
        let m = DocModel::new("plain text");
        let spans = m.reveal_spans();
        assert!(
            spans.iter().any(|s| s.start == 0),
            "the caret is always a reveal span: {spans:?}"
        );
    }

    #[test]
    fn one_grouped_edit_publishes_one_change_and_one_undo_step() {
        let mut m = DocModel::new("");
        assert!(
            m.take_changes(7).is_empty(),
            "a fresh model is not a change"
        );
        m.insert_at_caret("hello world");
        let changes = m.take_changes(7);
        assert_eq!(changes.len(), 1, "one grouped edit produces one change");
        let change = &changes[0];
        assert_eq!(change.document, 7);
        assert_eq!(change.seq, 1);
        assert_eq!(change.origin, ChangeOrigin::Local);
        assert_eq!(change.scope, ChangeScope::Content);
        assert!(!change.version.is_empty(), "the version token is captured");
        assert!(m.take_changes(7).is_empty(), "draining is idempotent");
        assert!(m.can_undo(), "the same edit is one undo step");
        assert!(m.undo());
        assert_eq!(m.text(), "");
    }

    #[test]
    fn a_marks_only_edit_publishes_a_change_with_an_identical_source_digest() {
        let mut m = DocModel::new("hello");
        m.take_changes(1);
        m.math_doc_mut().mark_segment(0..5, "prop:bold", "test");
        m.math_doc_mut().commit();
        let changes = m.take_changes(1);
        assert_eq!(changes.len(), 1, "a marks-only edit is still a change");
        let change = &changes[0];
        assert_eq!(change.source_digest, sha256_hex(b"hello"));
        assert_ne!(
            change.content_digest, change.source_digest,
            "the content digest covers marks"
        );
        assert_eq!(change.origin, ChangeOrigin::Local);
    }

    #[test]
    fn caret_and_view_changes_publish_no_change() {
        let mut m = DocModel::new("some prose #a hidden #b");
        m.take_changes(1);
        m.set_caret(3);
        m.set_selection(2..8);
        m.clear_selection();
        m.set_caret(m.len());
        m.mark_dirty();
        assert!(
            m.take_changes(1).is_empty(),
            "presentation-only activity never produces a durable change"
        );
        assert_eq!(m.change_origin, ChangeOrigin::Local);
    }

    #[test]
    fn undo_and_redo_are_attributed_to_their_origin() {
        let mut m = DocModel::new("");
        m.take_changes(1);
        m.insert_at_caret("x");
        m.take_changes(1);
        assert!(m.undo());
        let undo = m.take_changes(1);
        assert_eq!(undo.len(), 1);
        assert_eq!(undo[0].origin, ChangeOrigin::Undo);
        assert!(m.redo());
        let redo = m.take_changes(1);
        assert_eq!(redo.len(), 1);
        assert_eq!(redo[0].origin, ChangeOrigin::Redo);
        assert_eq!(redo[0].seq, undo[0].seq + 1);
    }

    #[test]
    fn an_invalid_snapshot_leaves_no_partial_document() {
        assert!(DocModel::from_snapshot(&[]).is_err());
        assert!(DocModel::from_snapshot(b"not a loro snapshot").is_err());
        let huge = vec![0u8; MAX_IMPORT_BYTES + 1];
        assert!(
            DocModel::from_snapshot(&huge).is_err(),
            "an over-bound envelope is rejected before the decoder runs"
        );
    }

    #[test]
    fn a_peer_origin_is_consumed_by_one_change() {
        let mut m = DocModel::new("");
        m.take_changes(1);
        m.set_change_origin(ChangeOrigin::Peer);
        m.insert_at_caret("remote");
        assert_eq!(m.take_changes(1)[0].origin, ChangeOrigin::Peer);
        m.insert_at_caret(" local");
        assert_eq!(
            m.take_changes(1)[0].origin,
            ChangeOrigin::Local,
            "the peer tag does not leak onto the next local keystroke"
        );
    }
}
