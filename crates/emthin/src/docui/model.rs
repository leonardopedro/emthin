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
    /// Set by any text mutation; cleared by [`DocModel::take_dirty`]
    /// once the layout has been rebuilt.
    dirty: bool,
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
        Self {
            doc,
            scan,
            segments,
            caret: 0,
            anchor: None,
            dirty: true,
        }
    }

    /// Rebuild a model from a Loro snapshot (session restore).
    pub fn from_snapshot(bytes: &[u8]) -> Result<Self, mathed_core::doc::DocError> {
        let doc = MathDoc::from_snapshot(bytes)?;
        let scan = scan(doc.text());
        let segments = resolve_segments(&scan);
        Ok(Self {
            doc,
            scan,
            segments,
            caret: 0,
            anchor: None,
            dirty: true,
        })
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

    /// Move the caret, collapsing any selection.
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
        let Some(_) = self.doc.undo() else {
            return false;
        };
        self.caret = self.caret.min(self.doc.len());
        self.anchor = None;
        self.rescan();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(_) = self.doc.redo() else {
            return false;
        };
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
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Consume the dirty flag (the layout cache calls this after a
    /// successful re-layout).
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
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
            ..Default::default()
        }
    }

    /// A Loro snapshot of the document (session persistence).
    pub fn snapshot(&self) -> Vec<u8> {
        self.doc.snapshot()
    }
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
    use super::*;

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
}
