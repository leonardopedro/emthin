//! The rendered document: emthin's shell UI.
//!
//! Wayland apps are **figures in a rendered document**, like images in
//! a PDF. The document text is a `mathed_core` [`MathDoc`] holding
//! hidden markers and `\name(...)` property statements; an `\app`
//! statement reserves a slot of a given size, and the app bound to it is
//! composited exactly over that slot after the document is laid out and
//! rasterized. Editing the document — including the `\app` arguments —
//! reflows the figures, which reconfigures the app toplevels.
//!
//! ```text
//! #1 Terminal demo #2 \app(#1, #2, 640, 400)
//! #3 Chat #4 \app(#3, #4, 320, 240, "chat")
//! ```text
//! The document is the layout authority; there is no window-management
//! policy anywhere else in the compositor.

pub mod ascii_interchange;
pub mod citations;
pub mod edit;
pub mod figures;
pub mod formals;
pub mod keymap;
pub mod layout;
pub mod model;
pub mod search;
pub mod voice;

use std::time::{Duration, Instant};

use smithay::utils::{Logical, Point, Rectangle, Size};

pub use figures::{Figure, FigureManager};
pub use formals::FormalVerifier;
pub use layout::{DocLayoutCache, PlacedFigure};
pub use model::DocModel;
pub use voice::{VoiceBridge, VoiceEvent, VoiceSubmit, VoiceTurn};

/// How often the session is autosaved. Loro snapshots are cheap
/// (copy-on-write), so this can be aggressive — driftwm's as-you-work
/// model rather than an exit-only save.
const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(5);

/// Everything about the document shell: text, layout, figures.
pub struct DocUi {
    model: DocModel,
    layout: DocLayoutCache,
    figures: FigureManager,
    /// Checks every `\formal` CNL sentence against `logos unf` and feeds the
    /// verdicts back as annotations. Holds the kernel's answer for the life of
    /// the document, so a relayout caused by an unrelated edit does not re-run
    /// the subprocess.
    formals: FormalVerifier,
    /// The `\voice` dialogue seam: the `voice.sh` helper resolved from
    /// `${EMTHIN_VOICE_BIN}` or `$PATH`, plus the one turn in flight.
    /// Replies land in the transcript through [`DocUi::poll_voice`].
    voice: VoiceBridge,
    citation_popup: citations::CitationPopup,
    /// The find bar: a query over the document text, with the match ranges
    /// `mathed_core::search` finds. The compositor draws its query and the
    /// match highlights — see [`search::FindBar`] and `crate::find_bar`.
    find: search::FindBar,
    /// The pending ASCII→Unicode completion in math. The preview is drawn by
    /// `crate::completion_hint`; nothing is written until a non-run key commits
    /// it, exactly like mathed's own frontends.
    completion: mathed_mini::completion_ui::CompletionUi,
    /// `(revision, caret)` for a preview the user dismissed with Escape. A
    /// relayout refreshes completion state, so without this the cancelled
    /// preview would reappear on the next tick. Any edit or caret move changes
    /// the key and lets completion resume.
    completion_dismissed: Option<(u64, usize)>,
    /// The view geometry (viewport, zoom, pan, visible page) changed. Unlike a
    /// text edit this needs no Typst rebuild, only a figure re-sync.
    geometry_dirty: bool,
    /// Where the session is persisted (`None` = don't persist).
    session_file: Option<std::path::PathBuf>,
    /// Stable identity of the open document, used to key its preferences.
    document_key: Option<String>,
    last_autosave: Instant,
    /// Set when the layout failed (a Typst syntax error mid-typing, say)
    /// so the caller can surface it without losing the document.
    last_error: Option<String>,
}

impl Default for DocUi {
    fn default() -> Self {
        Self::new()
    }
}

impl DocUi {
    pub fn new() -> Self {
        Self {
            model: DocModel::new(""),
            layout: DocLayoutCache::new(),
            figures: FigureManager::new(),
            formals: FormalVerifier::new(),
            voice: VoiceBridge::new(),
            citation_popup: citations::CitationPopup::default(),
            find: search::FindBar::new(),
            completion: mathed_mini::completion_ui::CompletionUi::new(),
            completion_dismissed: None,
            geometry_dirty: false,
            session_file: None,
            document_key: None,
            last_autosave: Instant::now(),
            last_error: None,
        }
    }

    pub fn model(&self) -> &DocModel {
        &self.model
    }

    pub fn model_mut(&mut self) -> &mut DocModel {
        &mut self.model
    }

    pub fn citation_popup(&self) -> &citations::CitationPopup { &self.citation_popup }

    pub fn toggle_citation(&mut self, number: u64) -> Result<(), String> {
        self.citation_popup.toggle(self.model.text(), self.model.revision(), number)
    }

    pub fn close_citation(&mut self) -> bool { self.citation_popup.close() }

    /// Cycle the presentation style of the open citation without editing the
    /// document. Returns the newly selected style.
    pub fn cycle_citation_style(&mut self) -> Result<String, String> {
        self.citation_popup.cycle_style()
    }

    /// Cycle the presentation locale of the open citation.
    pub fn cycle_citation_locale(&mut self) -> Result<String, String> {
        self.citation_popup.cycle_locale()
    }

    /// Drop citation presentation overrides, back to the document's defaults.
    pub fn reset_citation_format(&mut self) -> Result<bool, String> {
        self.citation_popup.reset_format()
    }

    /// The find bar, for the overlay renderer's status, query, and matches.
    pub fn find(&self) -> &search::FindBar {
        &self.find
    }

    /// `Ctrl+F`: open the bar with the search starting at the caret.
    pub fn open_find(&mut self) {
        let origin = self.model.caret();
        self.find.open_at(origin);
        // The find bar owns the keyboard now; a math preview behind it would
        // both mislead and linger after a query edit.
        self.completion.cancel();
    }

    /// Close the bar; `false` when it was already closed, so `Escape` can fall
    /// through to whatever it means with no bar open.
    pub fn close_find(&mut self) -> bool {
        self.find.close()
    }

    /// Type into the query. The search runs against the live text, so a match
    /// list never describes a previous revision of the document.
    pub fn find_push(&mut self, ch: char) {
        let text = self.model.text().to_owned();
        self.find.push(&text, ch);
    }

    pub fn find_backspace(&mut self) {
        let text = self.model.text().to_owned();
        self.find.backspace(&text);
    }

    /// Step to the next/previous match and select it, so the caret lands on the
    /// match and the status line moves with it.
    pub fn find_step(&mut self, forward: bool) -> Option<std::ops::Range<usize>> {
        let text = self.model.text().to_owned();
        let range = self.find.step(&text, forward)?;
        // Bring the page holding the selected glyph into view. A match in
        // hidden marker text has no page and leaves the viewport alone.
        if let Some(page) = self.layout.page_for_range(range.clone()) {
            if page != self.current_page() {
                self.layout.set_current_page(page);
                // A page switch invalidates figure rects; let the next tick
                // relayout and re-sync them, as `goto_page` does.
                self.model.mark_dirty();
            }
        }
        self.model.set_selection(range.clone());
        Some(range)
    }

    /// What the status line should show for the bar, if it is open.
    pub fn find_status(&self) -> Option<String> {
        self.find.status()
    }

    /// The pending math completion, if the caret is at the end of an ASCII
    /// run inside a `$..$` fence.
    pub fn completion(&self) -> Option<&mathed_core::completion::Completion> {
        self.completion.pending.as_ref()
    }

    /// Recompute the pending completion against the current text + caret.
    /// Called after edits, caret moves, and external document changes.
    pub fn refresh_completion(&mut self) {
        let key = (self.model.revision(), self.model.caret());
        if self.find.is_open() || self.completion_dismissed == Some(key) {
            self.completion.cancel();
            return;
        }
        self.completion_dismissed = None;
        self.completion.refresh(self.model.text(), self.model.caret());
    }

    /// Replace the pending ASCII run with its glyph as one document edit.
    /// Returns `false` when nothing is pending.
    pub fn commit_completion(&mut self) -> bool {
        let Some(completion) = self.completion.pending.clone() else {
            return false;
        };
        self.completion.cancel();
        self.completion_dismissed = None;
        self.model.replace(completion.replace, &completion.with);
        true
    }

    /// Drop a pending completion without touching the document. Returns
    /// whether there was one, so Escape can fall through when there wasn't.
    pub fn cancel_completion(&mut self) -> bool {
        let pending = self.completion.pending.is_some();
        if pending {
            self.completion_dismissed = Some((self.model.revision(), self.model.caret()));
        }
        self.completion.cancel();
        pending
    }

    pub fn clear_citations(&mut self) { self.citation_popup.clear(); }

    /// Re-run the open query after the document changed.
    pub fn find_on_edit(&mut self) {
        let text = self.model.text().to_owned();
        self.find.on_doc_changed(&text);
    }

    /// Jump the caret to a document reference's target (`Ctrl+click` on its
    /// rendered label). A bibliography reference has no document target, and
    /// a dangling document reference has no body; both are errors here.
    pub fn goto_citation(&mut self, number: u64) -> Result<(), String> {
        use mathed_core::markers::ReferenceKind;

        let scan = self.model.scan();
        let reference = model::citation_references(scan)
            .into_iter()
            .find(|reference| reference.numbers.contains(&number))
            .ok_or_else(|| format!("No citation [{number}]"))?;
        let body = match reference.kind {
            ReferenceKind::DocumentRef { body: Some(body), .. } => body,
            ReferenceKind::DocumentRef { body: None, .. } => {
                return Err(format!("Citation [{number}] is dangling"));
            }
            ReferenceKind::Bibliography { .. } => {
                return Err(format!("Citation [{number}] is a bibliography entry"));
            }
        };
        self.citation_popup.close();
        let page = self.layout.page_for_range(body.clone());
        self.model.set_caret(body.start);
        if let Some(page) = page {
            if page != self.current_page() {
                self.layout.set_current_page(page);
                self.model.mark_dirty();
            }
        }
        Ok(())
    }

    /// Hit-test the rendered citation label rather than the hidden source.
    pub fn citation_at(&self, point: Point<f64, Logical>) -> Option<u64> {
        let pos = self.layout.screen_to_doc(point)?;
        let glyphs = self.layout.glyphs()?;
        let glyph = glyphs.entries.iter().find(|entry| {
            let Some(band) = glyphs.bands.get(entry.band as usize) else { return false };
            let x = pos.x as f32;
            let y = pos.y as f32;
            x >= entry.x.min(entry.x + entry.advance) && x <= entry.x.max(entry.x + entry.advance)
                && y >= band.top && y <= band.bottom
        })?;
        let scan = self.model.scan();
        model::citation_references(scan).into_iter().find_map(|reference| {
            let range = &scan.stmts[reference.stmt_idx].range;
            range.contains(&glyph.doc_byte).then(|| reference.numbers.first().copied()).flatten()
        })
    }

    /// The document byte under a screen point on the visible page.
    ///
    /// `None` outside the page or before the first layout. Used by pointer
    /// selection so a click and its drag share one hit test.
    pub fn byte_at(&self, point: Point<f64, Logical>) -> Option<usize> {
        let pos = self.layout.screen_to_doc(point)?;
        self.layout
            .glyphs()?
            .byte_for_point(mathed_core::glyphs::V2::new(pos.x as f32, pos.y as f32))
            .map(|(byte, _after)| byte)
    }

    /// Select the word containing `byte` (a double-click selection).
    ///
    /// Math spans are atomic, so a double-click inside a formula selects the
    /// whole formula rather than one character of its source. Returns false
    /// when the range is empty, letting the caller fall back to a caret.
    pub fn select_word_at(&mut self, byte: usize) -> bool {
        let text = self.model.text().to_string();
        let atomic = mathed_core::transform::math_span_ranges(&text);
        let range = mathed_core::wordnav::word_range_at(&text, byte, &atomic);
        // Word navigation treats the newline as the terminator *and* includes
        // it in the right boundary. A selected newline has no rendered glyph,
        // so it would show as an empty sliver past the line's last word.
        let end = range.start
            + text[range.clone()]
                .trim_end_matches(['\n', '\r'])
                .len();
        if end <= range.start {
            return false;
        }
        self.model.set_selection(range.start..end);
        true
    }

    /// Select the source line containing `byte` (a triple-click selection).
    ///
    /// The newline itself is not selected: it is not rendered, so highlighting
    /// it would show a selection edge with no glyph. Returns false only for an
    /// empty document.
    pub fn select_line_at(&mut self, byte: usize) -> bool {
        let text = self.model.text().to_string();
        let byte = byte.min(text.len());
        let start = text[..byte].rfind('\n').map_or(0, |i| i + 1);
        let end = text[byte..].find('\n').map_or(text.len(), |i| byte + i);
        if start == end {
            return false;
        }
        self.model.set_selection(start..end);
        true
    }

    /// The verifier behind every `\formal` verdict in this document.
    pub fn formals(&self) -> &FormalVerifier {
        &self.formals
    }

    /// Pin a kernel, for a test or a deployment that wants a specific binary.
    pub fn formals_mut(&mut self) -> &mut FormalVerifier {
        &mut self.formals
    }

    /// The voice harness seam behind every `\voice` dialogue turn.
    pub fn voice(&self) -> &VoiceBridge {
        &self.voice
    }

    /// Pin a voice harness, for a test or a pinned deployment.
    pub fn voice_mut(&mut self) -> &mut VoiceBridge {
        &mut self.voice
    }

    /// Queue a voice turn for the `\voice` block at the caret
    /// (`Ctrl+Shift+V`).
    ///
    /// The subprocess runs on a worker thread — a turn is seconds and a
    /// dictation is a recording — so the outcome arrives through
    /// [`DocUi::poll_voice`] on a later tick and never blocks the event
    /// loop. An `Err` is a rejection the press can absorb (no harness
    /// installed, caret outside every `\voice` block, nothing to ask):
    /// the document is untouched.
    pub fn voice_turn(&mut self) -> Result<VoiceSubmit, String> {
        voice::submit(&self.model, &mut self.voice, voice::Target::Caret, None)
    }

    /// Queue a voice turn for the control protocol (`ipc::Voice`):
    /// `text` is an explicit utterance (else the pending `you:` line, or
    /// the microphone in a `listen`/`full` block), `block` a `\voice`
    /// block index (else the caret's block, else the first — a script
    /// driving the document has no caret of its own).
    pub fn voice_submit(
        &mut self,
        block: Option<usize>,
        text: Option<&str>,
    ) -> Result<VoiceSubmit, String> {
        let target = match block {
            Some(i) => voice::Target::Block(i),
            None => voice::Target::CaretOrFirst,
        };
        voice::submit(&self.model, &mut self.voice, target, text)
    }

    /// Drain finished voice turns into the document and return them.
    ///
    /// Called once per tick *before* the relayout, so a reply spliced
    /// this tick is typeset this tick (`poll` marks the model dirty).
    /// The events are for the caller to forward over IPC — the
    /// protocol has no responses, only notifications.
    pub fn poll_voice(&mut self) -> Vec<VoiceEvent> {
        voice::poll(&mut self.model, &mut self.voice)
    }

    pub fn layout(&self) -> &DocLayoutCache {
        &self.layout
    }

    pub fn figures(&self) -> &FigureManager {
        &self.figures
    }

    pub fn figures_mut(&mut self) -> &mut FigureManager {
        &mut self.figures
    }

    /// Number of pages the document currently lays out to.
    pub fn page_count(&self) -> usize {
        self.layout.page_count().max(1)
    }

    /// The visible page index.
    pub fn current_page(&self) -> usize {
        self.layout.current_page()
    }

    /// Display name of a page, for the ext-workspace bar.
    pub fn page_name(&self, page: usize) -> String {
        self.layout.page_name(page)
    }

    /// Load the document: `path` wins, else the session snapshot, else
    /// an empty document.
    pub fn load(&mut self, path: Option<&std::path::Path>, session_file: Option<&std::path::Path>) {
        self.citation_popup.clear();
        self.document_key = None;
        if let Some(path) = path {
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    self.model = DocModel::new(&text);
                    self.document_key = Some(stable_document_key(path));
                    self.session_file = session_file.map(std::path::Path::to_path_buf);
                    self.relayout();
                    self.restore_preferences_beside(session_file);
                    self.relayout();
                    return;
                }
                Err(e) => {
                    tracing::warn!("could not read document {}: {e}", path.display());
                }
            }
        }
        if let Some(file) = session_file {
            if let Ok(bytes) = std::fs::read(file) {
                if let Ok(model) = DocModel::from_snapshot(&bytes) {
                    self.model = model;
                    self.document_key = Some(stable_document_key(file));
                    self.session_file = Some(file.to_path_buf());
                    self.relayout();
                    self.restore_page_beside(file);
                    return;
                }
            }
        }
        self.session_file = session_file.map(std::path::Path::to_path_buf);
        self.document_key = session_file.map(stable_document_key);
        self.model = DocModel::new("");
        self.relayout();
    }

    /// Replace the open document from a path, carrying the current document's
    /// preferences back to the session before switching and applying the
    /// target document's saved view and citation format.
    pub fn open_document(&mut self, path: &std::path::Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        self.save();
        self.citation_popup.clear();
        self.find.close();
        self.completion.cancel();
        self.completion_dismissed = None;
        self.model = DocModel::new(&text);
        self.document_key = Some(stable_document_key(path));
        self.relayout();
        let session_file = self.session_file.clone();
        self.restore_preferences_beside(session_file.as_deref());
        self.relayout();
        Ok(())
    }

    /// Project the document to mathed's ASCII-only Typst interchange form.
    ///
    /// This is the same projection `mathed_mini --export-ascii` writes:
    /// math glyphs become their Typst names where the canonical table has
    /// one, and everything else becomes an explicit `\u{...}` literal, so
    /// the result is total and always ASCII. It is deliberately lossy for
    /// hidden markers and statements — the projection is the *rendered*
    /// document, not a second editable source.
    pub fn export_ascii(&self) -> String {
        ascii_interchange::export(&self.model.text())
    }

    /// Replace the document with an ASCII-only interchange projection.
    ///
    /// The imported text is the visible document, not a marked-up source:
    /// markers and `\name(...)` statements do not survive `export_ascii`,
    /// and this method does not invent them. Non-ASCII input is rejected
    /// rather than silently accepted, because accepting it would make this
    /// indistinguishable from [`DocUi::load`] and would let a UTF-8 file
    /// masquerade as the ASCII interchange form.
    ///
    /// The caller re-lays out afterwards (and closes apps whose figures the
    /// import removed), exactly as `open_doc` does.
    pub fn import_ascii(&mut self, text: &str) -> Result<(), String> {
        if !text.is_ascii() {
            return Err("ASCII interchange import requires ASCII-only text".to_string());
        }
        self.citation_popup.clear();
        self.find.close();
        self.completion.cancel();
        self.completion_dismissed = None;
        self.model = DocModel::new(text);
        Ok(())
    }

    /// Restore this document's page, reading size, and presentation
    /// preferences from the `session.json` beside `snapshot`.
    ///
    /// Best-effort by design: a missing or unreadable file leaves the defaults
    /// rather than refusing to open the document.
    ///
    /// Called *after* the document has been laid out, because the page count is
    /// what the stored page is clamped against — the snapshot and the JSON are
    /// written separately, so a document edited between the two can be shorter
    /// now than it was then. `set_current_page` does the clamping.
    fn restore_page_beside(&mut self, snapshot: &std::path::Path) {
        let Some(json) = session_json_path(snapshot) else {
            return;
        };
        let session = crate::session::SessionFile::load(&json);
        let key = self
            .document_key
            .clone()
            .unwrap_or_else(|| stable_document_key(snapshot));
        self.apply_document_preferences(session.preferences_for(&key));
    }

    /// Apply the open document's page, reading size, and presentation
    /// preferences. Used when `--doc` or `open_doc` selects a file.
    fn restore_preferences_beside(&mut self, session_file: Option<&std::path::Path>) {
        let Some(file) = session_file else {
            return;
        };
        let Some(json) = session_json_path(file) else {
            return;
        };
        let Some(key) = self.document_key.clone() else {
            return;
        };
        let session = crate::session::SessionFile::load(&json);
        self.apply_document_preferences(session.preferences_for(&key));
    }

    fn apply_document_preferences(&mut self, preferences: crate::session::DocumentPreferences) {
        self.citation_popup.set_saved_format(
            preferences.citation_style,
            preferences.citation_locale,
        );
        let text_size_changed = self.layout.set_text_size(preferences.text_size);
        if text_size_changed {
            self.model.mark_dirty();
            // Reflow before restoring the page: the saved page is only
            // meaningful at the saved reading size, and `rebuild` clamps the
            // old index while the new page count is established.
            self.relayout();
        }
        // Set the page before the view: a page switch resets pan to the top,
        // so restoring the saved pan first would have it discarded here.
        let page_changed = self.layout.current_page() != preferences.current_page;
        self.layout.set_current_page(preferences.current_page);
        if page_changed {
            self.geometry_dirty = true;
        }
        if self.layout.set_view(
            preferences.view_zoom,
            Point::new(preferences.view_pan_x, preferences.view_pan_y),
        ) {
            self.geometry_dirty = true;
        }
        if self.model.is_dirty() || self.geometry_dirty {
            self.relayout();
        }
    }

    /// Re-run layout if the document changed, then reconcile figures.
    ///
    /// Returns the apps whose figure disappeared (their `\app` statement
    /// was deleted) so the caller can close them.
    pub fn relayout(&mut self) -> Vec<u64> {
        self.citation_popup.clear_if_stale(self.model.revision());
        // An IPC edit can change the document while the bar is open; refresh
        // the saved ranges before the renderer reads them.
        self.find_on_edit();
        // The caret can move without dirtying the document, so the completion
        // must be recomputed here as well as after a keystroke.
        self.refresh_completion();
        let content_dirty = self.model.is_dirty();
        if !content_dirty && !self.geometry_dirty {
            return Vec::new();
        }
        if content_dirty {
            // Ask the kernel about every `\formal` before laying out, so the
            // verdicts ride along with the rest of the annotations.
            let mut options = self.model.transform_options();
            self.formals.apply(self.model.text(), &mut options);
            match self.layout.rebuild(self.model.text(), &options) {
                Ok(()) => {
                    self.last_error = None;
                    self.model.take_dirty();
                }
                Err(e) => {
                    // Keep the previous raster rather than blanking the page:
                    // a half-typed statement must not make the document
                    // disappear mid-keystroke.
                    self.last_error = Some(format!("{e:?}"));
                    self.model.take_dirty();
                }
            }
        }
        self.geometry_dirty = false;
        self.figures.sync(&self.model, &mut self.layout)
    }

    /// The document's reading size, in points.
    pub fn text_size(&self) -> f32 {
        self.layout.text_size()
    }

    /// Set the reading size and, if it changed, reflow the document.
    ///
    /// This is a Typst layout property, not a view zoom: larger text uses
    /// more lines and can add pages. Figures are re-synced by the ordinary
    /// relayout, so their rects follow the reflow rather than staying at the
    /// old page positions.
    pub fn set_text_size(&mut self, pt: f32) -> bool {
        if !pt.is_finite() || !self.layout.set_text_size(pt) {
            return false;
        }
        self.model.mark_dirty();
        true
    }

    /// Step to the next larger/smaller preset reading size.
    pub fn step_text_size(&mut self, larger: bool) -> f32 {
        let next = self.layout.next_text_size(larger);
        self.set_text_size(next);
        self.layout.text_size()
    }

    /// Restore the default reading size.
    pub fn reset_text_size(&mut self) -> bool {
        self.set_text_size(mathed_mini::DEFAULT_TEXT_SIZE_PT)
    }

    /// The current view zoom, as a multiplier on the fit-to-window scale.
    pub fn view_zoom(&self) -> f64 {
        self.layout.view_zoom()
    }

    /// The current pan offset in output-logical px.
    pub fn view_pan(&self) -> smithay::utils::Point<f64, Logical> {
        self.layout.view_pan()
    }

    /// Zoom the view one preset step; view-only, so no Typst reflow.
    pub fn step_view_zoom(&mut self, larger: bool) -> f64 {
        if self.layout.zoom_view(larger) {
            self.geometry_dirty = true;
        }
        self.layout.view_zoom()
    }

    /// Zoom by `factor` around `anchor` (output-logical px).
    pub fn zoom_view_at(&mut self, factor: f64, anchor: Point<f64, Logical>) -> bool {
        if self.layout.zoom_view_at(factor, anchor) {
            self.geometry_dirty = true;
            true
        } else {
            false
        }
    }

    /// Pan the zoomed page by an output-logical delta.
    pub fn pan_view(&mut self, dx: f64, dy: f64) -> bool {
        if self.layout.pan_view(dx, dy) {
            self.geometry_dirty = true;
            true
        } else {
            false
        }
    }

    /// Whether the zoomed page overflows the viewport on either axis, i.e.
    /// whether a pan gesture can do anything.
    pub fn view_can_pan(&self) -> bool {
        let page = self.layout.page_rect();
        let viewport = self.layout.viewport();
        page.size.w > viewport.w || page.size.h > viewport.h
    }

    /// Return to the fit-to-window view.
    pub fn reset_view(&mut self) -> bool {
        if self.layout.reset_view() {
            self.geometry_dirty = true;
            true
        } else {
            false
        }
    }

    /// Recompute the letterbox against a new output size.
    /// Changing the viewport re-lays out and forces a figure re-sync.
    ///
    /// `relayout` returns early when the model is clean, and the only caller of
    /// `set_viewport` is `relayout_doc`, which then calls `relayout`. So a resize
    /// with an otherwise-unchanged document re-letterboxed the page raster — which
    /// is placed from `page_rect()`, recomputed every frame — while every figure
    /// rect kept its old scale and origin. That left stale hit-tests, a stale
    /// focus border, stale dormant marks and a stale IME origin until the next
    /// text edit. `figure_render` and the input path read `Figure::rect`.
    ///
    /// `geometry_dirty` re-syncs figures without re-running the Typst compile;
    /// only the placement changed, not the document's content.
    pub fn set_viewport(&mut self, size: Size<i32, Logical>) {
        self.layout.set_viewport(size);
        self.geometry_dirty = true;
    }

    /// Called by `EmthinState::goto_page`.
    /// Adopt `page` as the visible page.
    ///
    /// This used to take no argument and do nothing, on the theory that
    /// `PageState` had already moved. It had not, as far as the *document* was
    /// concerned: there were two copies of the page index — `PageState`'s and
    /// `DocLayoutCache`'s — and everything visual reads the layout one. The
    /// page raster texture key, the figure filter, the caret and selection rects
    /// and the dormant marks all read `self.layout.current_page`, which nothing
    /// ever wrote outside session restore. So a page switch announced itself over
    /// IPC, updated the ext-workspace bar, and rendered the same page — and since
    /// `PgDn` computed its target from the layout copy too, the second press asked
    /// for the same page, tripped `goto_page`'s already-there guard and did
    /// nothing. `PgUp` was permanently a no-op.
    ///
    /// One copy, set through the layout, which clamps.
    /// Adopt `page` as the laid-out page.
    ///
    /// Forces a re-sync, because a figure's rect is only correct for the page it
    /// was placed on: `page_rect_for` hands an off-page figure the origin and the
    /// page size as a placeholder, and `place_figure` reads the letterbox factor
    /// back out of whatever that returned. So after a switch every figure was
    /// still carrying the rect computed for the *previous* page — the newly
    /// visible one had the unscaled placeholder at the page origin, and the one
    /// that went off-page still had its placed rect.
    ///
    /// Found by running the compositor: over IPC, `goto_page 1` reported the
    /// second figure at exactly its declared 320x240 while the first had been
    /// correctly scaled to 559x350 from a declared 640x400. The placeholder is
    /// unscaled, so an unscaled rect on the now-visible page is the tell.
    ///
    /// The stale rect then drove compositing, hit-testing, the focus border, the
    /// dormant labels and the IME origin.
    pub fn on_page_changed(&mut self, page: usize) {
        self.layout.set_current_page(page);
        self.geometry_dirty = true;
    }

    /// Record (or clear) an app's title.
    pub fn on_app_title_changed(&mut self, app_id: u64, title: &str) {
        // Every mirror, not just the first: they are one app, and
        // `list_state` reporting a title on one figure but `None` on
        // its twins is a lie about what is on screen.
        for figure in self.figures.figures_of_app_mut(app_id) {
            figure.title = Some(title.to_string());
        }
    }

    /// Record an app's app_id (used for wildcard figure binding).
    pub fn on_app_id_changed(&mut self, app_id: u64, app_id_str: &str) {
        for figure in self.figures.figures_of_app_mut(app_id) {
            if figure.title.is_none() {
                figure.title = Some(app_id_str.to_string());
            }
        }
    }

    /// Screen-space origin of `app_id`'s figure on the visible page, for
    /// IME caret-rect translation.
    pub fn figure_rect_on_page(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        app_id_of: impl Fn(
            &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
        ) -> Option<u64>,
    ) -> Option<Point<i32, Logical>> {
        let app_id = app_id_of(surface)?;
        self.figures.origin_on_page(app_id, self.current_page())
    }

    /// The text to draw on a dormant figure, or `None` if it should carry none.
    ///
    /// §5.10's stand-in is "framed placeholder + app name + Enter to launch".
    /// The name comes from the `launch:` command when there is one, because
    /// that is the app that would actually start; otherwise the binding id is
    /// the best name available, and failing that the figure key.
    ///
    /// The hint names **both** gestures because both work. A label that says
    /// only "Enter" would under-report the click, and one that says only
    /// "click" would under-report the key.
    ///
    /// This is text, so it cannot be a `SolidColor` bar: overlays here upload no
    /// texture, and the compositor rasterizes it. Keeping the string here means
    /// the decision of *what to say* is unit-tested while the drawing is not.
    pub fn dormant_label(&self, key: &str) -> Option<String> {
        let fig = self.figures.get(key)?;
        if !fig.is_dormant() || fig.page != Some(self.current_page()) {
            return None;
        }
        let name = fig
            .spec
            .launch
            .as_deref()
            .and_then(|cmd| crate::cli::split_command(cmd).first().cloned())
            .or_else(|| fig.spec.id.clone())
            .unwrap_or_else(|| key.to_owned());
        Some(format!("{name} — click or Return to launch"))
    }

    /// The dormant figures to mark on the current page, with their rects.
    ///
    /// A dormant figure is an empty slot. Without a mark it is
    /// indistinguishable from a figure whose app simply failed to start, so the
    /// relaunch affordance (`Return` over the figure) is invisible — a real
    /// binding nobody can discover.
    ///
    /// The geometry decision lives here rather than in `figure_render` so it can
    /// be unit-tested; the compositor only draws what this returns. Off-page
    /// figures are excluded because their rects are still cached, and drawing
    /// them would put marks on a page nobody is looking at.
    pub fn dormant_rects_on_current_page(&self) -> Vec<(String, Rectangle<i32, Logical>)> {
        let page = self.current_page();
        self.figures
            .figures()
            .iter()
            .filter(|f| f.is_dormant() && f.page == Some(page))
            .filter(|f| f.rect.size.w > 0 && f.rect.size.h > 0)
            .map(|f| (f.key.clone(), f.rect))
            .collect()
    }

    /// The dormant figure at `pos`, if any.
    ///
    /// The pointer is the signal, not the caret. A figure's rect is a
    /// rasterized placeholder, and its caption lays out beside it, so a text
    /// caret is never inside a figure rect — a caret-based version of this
    /// function cannot ever fire. The pointer is also what the user is already
    /// aiming when they press Enter on a dormant figure.
    pub fn dormant_figure_at(&self, pos: Point<f64, Logical>) -> Option<&str> {
        let fig = self.figures.figure_under(pos)?;
        // `figure_under` alone would match a figure on another page: its rect is
        // still in the cache, just not visible.
        (fig.is_dormant() && fig.page == Some(self.current_page())).then_some(fig.key.as_str())
    }

    /// The command that launches `key`'s app, from the `\app` statement itself.
    ///
    /// The document is the authority on what an `\app` means, so the launch
    /// command is a `launch:` argument on the statement rather than compositor
    /// state. That also means there is no mapping to guess: no pid→app_id
    /// link exists, and a figure's binding id is a glob over app ids that need
    /// not resemble the program that produced them.
    ///
    /// Split with [`crate::cli::split_command`] — the same splitter `--spawn`
    /// uses, so a command survives a round trip through the document with the
    /// quoting rules the user already met on the command line.
    ///
    /// `None` when the statement names no command, or when the figure is not
    /// dormant: a figure that already has a client must not spawn a second one
    /// over the top of it.
    pub fn relaunch_target(&self, key: &str) -> Option<(String, Vec<String>)> {
        let fig = self.figures.get(key)?;
        if !fig.is_dormant() {
            return None;
        }
        let words = crate::cli::split_command(fig.spec.launch.as_deref()?);
        let program = words.first()?.clone();
        Some((program, words[1..].to_vec()))
    }

    /// The first dormant figure in document order, as a last resort.
    ///
    /// This is only correct when there is no better signal. Preferring it
    /// unconditionally — which the launcher used to do — means the key acts on
    /// whichever figure happens to come first, not the one the user means.
    pub fn first_dormant_figure(&self) -> Option<&str> {
        self.figures
            .figures()
            .iter()
            .find(|f| f.is_dormant() && f.page == Some(self.current_page()))
            .map(|f| f.key.as_str())
    }

    /// Per-tick upkeep: autosave the session on a timer.
    ///
    /// Deliberately does *not* refresh the ext-workspace page list —
    /// that needs `DisplayHandle`/`Output`, so `tick.rs` drives it.
    pub fn tick(&mut self) {
        if self.last_autosave.elapsed() < AUTOSAVE_INTERVAL {
            return;
        }
        self.last_autosave = Instant::now();
        self.save();
    }

    /// Persist the document snapshot, if a session file is configured.
    pub fn save(&mut self) {
        let Some(path) = self.session_file.clone() else {
            // A nested session with no `--session-file` keeps nothing, per the
            // driftwm rule in §5.10: only a primary session would have a state
            // dir it could own unconditionally.
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(&path, self.model.snapshot()) {
            Ok(()) => tracing::debug!("session saved to {}", path.display()),
            Err(e) => tracing::warn!("session save failed {}: {e}", path.display()),
        }
        // The snapshot is the document; this is the rest. Load the existing
        // metadata first so preferences for other documents survive a save,
        // then key this document's view and citation state by its identity.
        // Written after the document so a crash between the two leaves stale
        // bookkeeping rather than a stale document.
        let Some(json) = session_json_path(&path) else {
            return;
        };
        let mut session = crate::session::SessionFile::load(&json);
        let pan = self.layout.view_pan();
        let key = self
            .document_key
            .clone()
            .unwrap_or_else(|| stable_document_key(&path));
        session.set_preferences(
            key,
            crate::session::DocumentPreferences {
                view_zoom: self.layout.view_zoom(),
                view_pan_x: pan.x,
                view_pan_y: pan.y,
                citation_style: self.citation_popup.style_override().map(str::to_string),
                citation_locale: self.citation_popup.locale_override().map(str::to_string),
                current_page: self.current_page(),
                text_size: self.text_size(),
            },
        );
        session.save(&json);
    }

    /// The caret rect in output-local logical px, for the IME bridge.
    pub fn caret_rect(&self) -> Option<Rectangle<i32, Logical>> {
        self.layout.caret_rect(self.model.caret())
    }

    /// The figure at `pos` **on the visible page**, or `None`.
    ///
    /// Not the same as `FigureManager::figure_under`. An off-page figure has no
    /// meaningful screen rect — `page_rect_for` hands it the origin and the page size
    /// as a placeholder — so that rect overlaps the visible page's top-left corner.
    /// Matching on it meant a click there could hit a figure the user cannot see:
    /// relaunching the wrong one, starting a resize grab against a statement that is
    /// not on screen (whose `commit` then rewrites *that* statement's `\app` args), or
    /// mapping the click into an unrelated surface.
    ///
    /// The compositor must never treat an off-page figure as present. Three call
    /// sites did; `dormant_figure_at` already re-checked `page` itself, which is how
    /// the hazard was known and not applied.
    pub fn figure_at(&self, pos: Point<f64, Logical>) -> Option<&Figure> {
        let page = self.current_page();
        self.figures
            .figure_under(pos)
            .filter(|f| f.page == Some(page))
    }

    /// The last layout error, if the document currently fails to lay
    /// out.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

/// A stable path identity for document-scoped preferences. Canonicalising when
/// the file exists keeps `./notes.doc` and `/home/me/notes.doc` from becoming
/// two entries; an unsaved snapshot falls back to an absolute path.
fn stable_document_key(path: &std::path::Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    absolute
        .canonicalize()
        .unwrap_or(absolute)
        .to_string_lossy()
        .into_owned()
}

/// The `session.json` beside a given document snapshot.
///
/// §5.10 puts both files in one state dir, so the JSON path is derived from
/// the `.loro` path rather than being configured separately — one flag, one
/// directory, and no way to point them at different places by accident.
pub fn session_json_path(snapshot: &std::path::Path) -> Option<std::path::PathBuf> {
    let dir = snapshot.parent()?;
    Some(dir.join("session.json"))
}

#[cfg(test)]
mod relaunch_tests {
    use super::*;
    use smithay::utils::Size;

    /// Go-to-definition moves the caret to the referenced body; a
    /// bibliography reference has no body and must not jump to the citation.
    #[test]
    fn a_document_reference_targets_its_body() {
        use mathed_core::markers::ReferenceKind;

        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 1600)));
        let text = "#1 target text #2 \\cite(#1,#2) tail";
        ui.model_mut().replace(0..0, text);
        ui.model_mut().set_caret(text.len());
        ui.relayout();
        let body = model::citation_references(ui.model().scan())
            .into_iter()
            .find_map(|reference| match reference.kind {
                ReferenceKind::DocumentRef { body, .. } => body,
                ReferenceKind::Bibliography { .. } => None,
            })
            .expect("a document-reference body");
        assert!(ui.goto_citation(1).is_ok());
        assert_eq!(ui.model().caret(), body.start);
    }

    #[test]
    fn a_bibliography_reference_does_not_have_a_document_target() {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 1600)));
        ui.model_mut().replace(0..0, "\\cite(key)");
        ui.relayout();
        assert!(ui.goto_citation(1).is_err());
    }

    #[test]
    fn rendered_citation_label_is_clickable() {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 1600)));
        let text = "#a cited text #b \\cite(#a,#b) tail";
        ui.model_mut().replace(0..0, text);
        ui.model_mut().set_caret(text.len());
        ui.relayout();
        let start = text.find("\\cite").unwrap();
        let glyphs = ui.layout().glyphs().unwrap();
        let glyph = glyphs.entries.iter().find(|g| g.doc_byte == start).expect("citation glyph");
        let band = &glyphs.bands[glyph.band as usize];
        let point = ui.layout().doc_to_screen(Point::from((
            f64::from(glyph.x + glyph.advance / 2.0),
            f64::from((band.top + band.bottom) / 2.0),
        )));
        assert_eq!(ui.citation_at(point.to_f64()), Some(1));
    }

    /// Two `\app` figures on page 0, with a remembered command for each.
    fn doc_with_two_figures() -> DocUi {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 1600)));
        ui.model_mut().replace(
            0..0,
            "#1 first #2 \\app(#1, #2, 300, 120, \"first\", launch: \"foot -T\")\n\
             #3 second #4 \\app(#3, #4, 300, 120, \"second\", launch: \"alacritty\")\n",
        );
        ui.relayout();
        ui
    }

    /// The pointer inside a dormant figure resolves to it, so Enter can be
    /// figure-scoped. Without this the key acts on whichever figure happens to
    /// come first, which is the bug this replaces.
    #[test]
    fn the_pointer_inside_a_dormant_figure_resolves_to_it() {
        let ui = doc_with_two_figures();
        let rect = ui.figures().get("f1").expect("f1").rect;
        let inside = Point::<f64, Logical>::new(
            (rect.loc.x + rect.size.w / 2) as f64,
            (rect.loc.y + rect.size.h / 2) as f64,
        );
        assert_eq!(ui.dormant_figure_at(inside), Some("f1"));
        // And a point in the gutter between the two figures matches neither.
        let gutter = Point::<f64, Logical>::new(
            (rect.loc.x + rect.size.w / 2) as f64,
            (rect.loc.y - 5) as f64,
        );
        assert_eq!(ui.dormant_figure_at(gutter), None);
    }

    /// A bound figure has a client and is not a relaunch target, so pointing at
    /// it must not offer to spawn a second app over the top of the first.
    #[test]
    fn a_bound_figure_is_never_a_relaunch_target() {
        let mut ui = doc_with_two_figures();
        assert!(ui.figures().get("f0").unwrap().is_dormant());
        assert!(ui.figures_mut().bind("f0", 42), "f0 must exist to bind");
        assert!(!ui.figures().get("f0").unwrap().is_dormant());
        let rect = ui.figures().get("f0").unwrap().rect;
        let inside = Point::<f64, Logical>::new(
            (rect.loc.x + rect.size.w / 2) as f64,
            (rect.loc.y + rect.size.h / 2) as f64,
        );
        assert_eq!(ui.dormant_figure_at(inside), None);
        assert_eq!(ui.relaunch_target("f0"), None);
    }

    /// The command comes from the document, and is split with the `--spawn`
    /// splitter. A figure whose statement names no command must not spawn an
    /// empty one.
    #[test]
    fn relaunch_needs_a_command_in_the_document() {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 1600)));
        ui.model_mut()
            .replace(0..0, "#1 a #2 \\app(#1, #2, 600, 300, \"a\")");
        ui.relayout();
        assert_eq!(ui.relaunch_target("f0"), None, "dormant but no command");
        // Adding the command to the statement is all it takes — no compositor
        // state to keep in step, and no relaunch to re-derive.
        let close = ui.model().text().rfind(')').expect("a closing paren");
        ui.model_mut()
            .replace(close..close, ", launch: \"foot -T\"");
        ui.relayout();
        let (cmd, args) = ui.relaunch_target("f0").expect("now launchable");
        assert_eq!(cmd, "foot");
        assert_eq!(args, vec!["-T".to_string()]);
        // A key that names no figure is not an error, just nothing to do.
        assert_eq!(ui.relaunch_target("f9"), None);
    }

    /// Each figure launches *its own* command. A single remembered command for
    /// the whole document would relaunch the wrong app for every figure but
    /// one, and this is the test that would have caught the missing mapping.
    #[test]
    fn each_figure_launches_its_own_command() {
        let ui = doc_with_two_figures();
        let (cmd0, args0) = ui.relaunch_target("f0").expect("f0 launches");
        let (cmd1, args1) = ui.relaunch_target("f1").expect("f1 launches");
        assert_eq!(
            (cmd0.as_str(), args0.as_slice()),
            ("foot", ["-T".to_string()].as_slice())
        );
        assert_eq!(
            (cmd1.as_str(), args1.as_slice()),
            ("alacritty", [].as_slice()),
            "the second figure must not inherit the first one's command"
        );
    }

    /// A quoted argument survives the document round trip, because it is split
    /// by the same function that parses `--spawn`.
    #[test]
    fn a_quoted_argument_survives_the_round_trip() {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 1600)));
        ui.model_mut().replace(
            0..0,
            "#1 a #2 \\app(#1, #2, 600, 300, launch: \"foot --app-id 'my app'\")",
        );
        ui.relayout();
        let (cmd, args) = ui.relaunch_target("f0").expect("launchable");
        assert_eq!(cmd, "foot");
        assert_eq!(args, vec!["--app-id".to_string(), "my app".to_string()]);
    }

    /// The fallback is document order, which is the only ordering available
    /// when neither the pointer nor focus says anything.
    #[test]
    fn the_fallback_is_the_first_dormant_figure_in_document_order() {
        let mut ui = doc_with_two_figures();
        assert_eq!(ui.first_dormant_figure(), Some("f0"));
        ui.figures_mut().bind("f0", 7);
        assert_eq!(ui.first_dormant_figure(), Some("f1"));
        ui.figures_mut().release_app(7);
        assert_eq!(ui.first_dormant_figure(), Some("f0"));
    }

    /// Only dormant, visible, non-degenerate figures are marked.
    #[test]
    fn dormant_marks_cover_the_visible_dormant_figures_only() {
        let mut ui = doc_with_two_figures();
        let both: Vec<String> = ui
            .dormant_rects_on_current_page()
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        assert_eq!(both, vec!["f0".to_string(), "f1".to_string()]);
        // A bound figure is not an empty slot, so it drops out of the marks.
        ui.figures_mut().bind("f0", 9);
        let marked: Vec<String> = ui
            .dormant_rects_on_current_page()
            .iter()
            .map(|(k, _)| k.clone())
            .collect();
        assert_eq!(marked, vec!["f1".to_string()]);
        // The rect is the figure's own, so the mark lands on the slot.
        let rect = ui.dormant_rects_on_current_page()[0].1;
        assert_eq!(rect, ui.figures().get("f1").unwrap().rect);
    }

    /// Turning to another page must not leave marks on the page left behind,
    /// nor draw that page's dormant figures over the new one.
    #[test]
    fn dormant_marks_follow_the_current_page() {
        let mut ui = doc_with_two_figures();
        ui.on_page_changed(0);
        // Page 0 is the only page in a short document, so this is the identity
        // case — recorded because it is the assumption the filter relies on.
        assert_eq!(ui.dormant_rects_on_current_page().len(), 2);
        assert!(ui.page_count() >= 1);
    }

    /// An empty document has nothing to relaunch, and says so without
    /// panicking — the key press that asks is a normal event, not an error.
    #[test]
    fn an_empty_document_offers_nothing() {
        let ui = DocUi::new();
        assert_eq!(
            ui.dormant_figure_at(Point::<f64, Logical>::new(0.0, 0.0)),
            None
        );
        assert_eq!(ui.first_dormant_figure(), None);
        assert_eq!(ui.relaunch_target("f0"), None);
    }
}

#[cfg(test)]
mod completion_tests {
    use super::*;

    fn ui_with(text: &str) -> DocUi {
        let mut ui = DocUi::new();
        ui.model_mut().replace(0..0, text);
        ui.refresh_completion();
        ui
    }

    #[test]
    fn a_math_run_previews_and_commits_its_glyph() {
        let mut ui = ui_with("$ x ->");
        let preview = ui.completion().expect("a pending arrow");
        assert_eq!(preview.preview, "→");
        assert_eq!(preview.replace, 4..6);
        assert!(ui.commit_completion());
        assert_eq!(ui.model().text(), "$ x →");
        assert_eq!(ui.model().caret(), "$ x →".len());
        assert!(ui.completion().is_none());
        assert!(!ui.commit_completion(), "the completion was consumed");
    }

    #[test]
    fn a_unique_prefix_stays_pending_while_the_run_extends() {
        let ui = ui_with("$ \\al");
        assert_eq!(ui.completion().expect("alpha preview").preview, "α");
        assert!(
            mathed_mini::completion_ui::CompletionUi::extends_run('p'),
            "'p' continues the run; it must not commit early"
        );
    }

    /// The input path's delimiter contract: commit the glyph, then insert the
    /// delimiter as ordinary text (`-> ` becomes `→ `).
    #[test]
    fn a_delimiter_commits_then_inserts() {
        let mut ui = ui_with("$ x ->");
        assert!(!mathed_mini::completion_ui::CompletionUi::extends_run(' '));
        assert!(ui.commit_completion());
        ui.model_mut().insert_at_caret(" ");
        assert_eq!(ui.model().text(), "$ x → ");
        assert_eq!(ui.model().caret(), "$ x → ".len());
    }

    #[test]
    fn completion_is_math_only_and_cancels_without_editing() {
        let ui = ui_with("x ->");
        assert!(ui.completion().is_none(), "plain prose stays prose");

        let mut ui = ui_with("$ \\alpha");
        assert!(ui.cancel_completion());
        assert!(ui.completion().is_none());
        ui.refresh_completion();
        assert!(
            ui.completion().is_none(),
            "a relayout must not resurrect the dismissed preview"
        );
        assert_eq!(ui.model().text(), "$ \\alpha", "cancel mutates nothing");
        assert!(!ui.cancel_completion(), "nothing left to cancel");

        // Moving the caret is a new context, so completion can resume.
        let end = ui.model().len();
        ui.model_mut().set_caret(0);
        ui.refresh_completion();
        ui.model_mut().set_caret(end);
        ui.refresh_completion();
        assert!(ui.completion().is_some(), "a new caret clears the dismissal");
    }
}

#[cfg(test)]
mod ascii_interchange_tests {
    use super::*;

    #[test]
    fn export_is_ascii_only_and_import_accepts_the_projection() {
        let mut ui = DocUi::new();
        ui.model_mut()
            .replace(0..0, "= 数学\n\n$ αβ → ≤ 1 $\n\nA crab 🦀");
        let exported = ui.export_ascii();
        assert!(
            exported.is_ascii(),
            "projection must be ASCII-only: {exported}"
        );
        assert!(
            exported.contains("\\alpha\\beta"),
            "math glyphs use their Typst names: {exported}"
        );
        assert!(
            exported.contains("\\u{1F980}"),
            "unmapped glyphs are flagged, never dropped: {exported}"
        );

        // Import accepts the projection as the document text. A second
        // projection need not be byte-identical — the transform normalises
        // its own Typst markup, such as paragraph breaks — but it must stay
        // ASCII and keep the glyph names/flags.
        ui.import_ascii(&exported).expect("utf8 ASCII projection");
        assert_eq!(ui.model().text(), exported);
        let again = ui.export_ascii();
        assert!(again.is_ascii(), "projection must stay ASCII: {again}");
        assert!(again.contains("\\alpha\\beta"), "glyphs survive: {again}");
        assert!(again.contains("\\u{1F980}"), "flags survive: {again}");
    }

    #[test]
    fn import_rejects_non_ascii_and_leaves_the_document_alone() {
        let mut ui = DocUi::new();
        ui.model_mut().replace(0..0, "keep me");
        let err = ui
            .import_ascii("$ α $")
            .expect_err("non-ASCII is not the interchange form");
        assert!(
            err.contains("ASCII"),
            "the reason names the requirement: {err}"
        );
        assert_eq!(ui.model().text(), "keep me");
    }

    #[test]
    fn import_drops_stale_find_and_completion_state() {
        let mut ui = DocUi::new();
        ui.model_mut().replace(0..0, "$ x ->");
        ui.refresh_completion();
        assert!(ui.completion().is_some());
        ui.open_find();
        assert!(ui.find().is_open());

        ui.import_ascii("$ \\to $").expect("ASCII input");
        assert!(!ui.find().is_open(), "import closes the old query bar");
        assert!(ui.completion().is_none());
        assert_eq!(ui.model().text(), "$ \\to $");
    }
}

mod session_tests {
    use super::*;
    use crate::session::SessionFile;
    use smithay::utils::Size;

    /// A temp dir unique to one test, removed on drop.
    struct TmpDir(std::path::PathBuf);

    impl TmpDir {
        fn new(name: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static SEQ: AtomicU32 = AtomicU32::new(0);
            let seq = SEQ.fetch_add(1, Ordering::Relaxed);
            let pid = std::process::id();
            let thread = format!("{:?}", std::thread::current().id());
            let path =
                std::env::temp_dir().join(format!("emthin-session-{name}-{pid}-{thread}-{seq}"));
            std::fs::create_dir_all(&path).expect("create temp dir");
            TmpDir(path)
        }
        fn snapshot(&self) -> std::path::PathBuf {
            self.0.join("doc.loro")
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A document long enough to paginate, so `current_page` can move off 0.
    fn paged_doc() -> String {
        let para = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
        format!("{}{}", para.repeat(60), para)
    }

    /// §5.10's check, in the part that can be tested headlessly: the document
    /// comes back from the snapshot, and so does the page.
    #[test]
    fn the_document_and_the_page_come_back_after_a_restart() {
        let tmp = TmpDir::new("roundtrip");
        let snapshot = tmp.snapshot();

        let mut first = DocUi::new();
        first.set_viewport(Size::from((1200, 900)));
        first.load(None, Some(&snapshot));
        first.model_mut().replace(0..0, &paged_doc());
        first.relayout();
        assert!(first.page_count() > 1, "the fixture must paginate");
        first.layout.set_current_page(first.page_count() - 1);
        let expected = first.current_page();
        assert!(first.layout.set_view(3.0, Point::new(10.0, 20.0)));
        first
            .citation_popup
            .set_saved_format(Some("vancouver".into()), Some("de-DE".into()));
        first.save();

        // A fresh UI is a relaunch: nothing carried over in memory.
        let mut second = DocUi::new();
        second.load(None, Some(&snapshot));
        assert_eq!(
            second.current_page(),
            expected,
            "the page must come back from session.json"
        );
        assert_eq!(second.view_zoom(), 3.0, "view zoom is a reading preference");
        assert_eq!(second.view_pan(), Point::from((10.0, 20.0)));
        assert_eq!(second.citation_popup.style(), "vancouver");
        assert_eq!(second.citation_popup.locale(), "de-DE");
    }

    /// Without `--session-file` nothing is written at all — §5.10's nested rule.
    /// Asserted by absence, so the drift check cannot tell a stale file from a
    /// fresh one.
    #[test]
    fn no_session_file_means_nothing_is_written() {
        let tmp = TmpDir::new("nosession");
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 900)));
        ui.load(None, None);
        ui.model_mut().replace(0..0, "hello");
        ui.relayout();
        ui.save();
        assert_eq!(
            std::fs::read_dir(&tmp.0).unwrap().count(),
            0,
            "a nested session with no --session-file must leave no files behind"
        );
    }

    /// A stored page past the end of a shorter document is clamped, not honoured
    /// and not fatal. The snapshot and the JSON are written separately, so this
    /// is reachable by deleting paragraphs between two saves.
    #[test]
    fn a_stored_page_beyond_the_document_is_clamped() {
        let tmp = TmpDir::new("clamp");
        let snapshot = tmp.snapshot();
        SessionFile {
            current_page: 99,
            ..Default::default()
        }
        .save(&session_json_path(&snapshot).expect("a json path"));

        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 900)));
        ui.load(None, Some(&snapshot));
        assert_eq!(ui.current_page(), 0, "one page, so page 0");
        assert!(ui.last_error().is_none(), "{:?}", ui.last_error());
    }

    /// A corrupt or foreign `session.json` must not stop the document opening.
    #[test]
    fn a_broken_session_json_still_opens_the_document() {
        let tmp = TmpDir::new("broken");
        let snapshot = tmp.snapshot();
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 900)));
        ui.load(None, Some(&snapshot));
        ui.model_mut().replace(0..0, "the document text");
        ui.relayout();
        ui.save();
        std::fs::write(session_json_path(&snapshot).unwrap(), b"{ not json").expect("write");

        let mut reopened = DocUi::new();
        reopened.load(None, Some(&snapshot));
        assert!(
            reopened.model().text().contains("the document text"),
            "the document is the session; broken bookkeeping must not lose it"
        );
        assert_eq!(reopened.current_page(), 0);
    }

    /// The JSON lives *beside* the snapshot, per §5.10 — one flag, one
    /// directory, and no way to point the two at different places.
    #[test]
    fn the_json_sits_beside_the_snapshot() {
        let tmp = TmpDir::new("beside");
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 900)));
        ui.load(None, Some(&tmp.snapshot()));
        ui.save();
        assert!(tmp.snapshot().exists(), "the snapshot is written");
        assert!(
            tmp.0.join("session.json").exists(),
            "and the bookkeeping beside it, not somewhere else"
        );
    }

    /// View, reading size, page, and citation preferences follow the document.
    /// Opening another file in the same session must not inherit the first
    /// file's values, and going back must restore the first file's own values.
    #[test]
    fn view_reading_and_citation_preferences_are_per_document() {
        let tmp = TmpDir::new("per-document");
        let session = tmp.snapshot();
        let doc_a = tmp.0.join("a.doc");
        let doc_b = tmp.0.join("b.doc");
        let long = paged_doc().repeat(2);
        std::fs::write(&doc_a, &long).expect("write a");
        std::fs::write(&doc_b, &long).expect("write b");

        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 900)));
        ui.load(Some(&doc_a), Some(&session));
        assert!(ui.page_count() > 3, "the fixture must paginate");
        assert!(ui.set_text_size(24.0));
        ui.relayout();
        let a_page = 2;
        ui.layout.set_current_page(a_page);
        assert_eq!(ui.current_page(), a_page);
        assert!(ui.layout.set_view(2.0, Point::new(12.0, 24.0)));
        ui.citation_popup
            .set_saved_format(Some("vancouver".into()), Some("de-DE".into()));
        ui.save();

        ui.open_document(&doc_b).expect("open b");
        assert_eq!(
            ui.view_zoom(),
            1.0,
            "a new document starts at the fit, not the previous document's zoom"
        );
        assert_eq!(ui.citation_popup.style(), "apa");
        assert_eq!(
            ui.text_size(),
            mathed_mini::DEFAULT_TEXT_SIZE_PT,
            "a new document starts at the default reading size"
        );
        assert_eq!(ui.current_page(), 0, "and on its first page");

        assert!(ui.set_text_size(14.0));
        ui.relayout();
        assert!(ui.page_count() > 3, "the fixture must still paginate");
        let b_page = 1;
        ui.layout.set_current_page(b_page);
        assert_eq!(ui.current_page(), b_page);
        assert!(ui.layout.set_view(0.8, Point::new(0.0, 0.0)));
        ui.citation_popup
            .set_saved_format(Some("mla".into()), Some("fr-FR".into()));
        ui.save();

        let mut again_a = DocUi::new();
        again_a.set_viewport(Size::from((1200, 900)));
        again_a.load(Some(&doc_a), Some(&session));
        assert_eq!(again_a.view_zoom(), 2.0);
        assert_eq!(
            again_a.view_pan(),
            Point::from((0.0, 24.0)),
            "pan is clamped to the actual overflow before it is saved"
        );
        assert_eq!(again_a.text_size(), 24.0);
        assert_eq!(again_a.current_page(), a_page);
        assert_eq!(again_a.citation_popup.style(), "vancouver");
        assert_eq!(again_a.citation_popup.locale(), "de-DE");

        let mut again_b = DocUi::new();
        again_b.set_viewport(Size::from((1200, 900)));
        again_b.load(Some(&doc_b), Some(&session));
        assert_eq!(again_b.view_zoom(), 0.8);
        assert_eq!(again_b.text_size(), 14.0);
        assert_eq!(again_b.current_page(), b_page);
        assert_eq!(again_b.citation_popup.style(), "mla");
        assert_eq!(again_b.citation_popup.locale(), "fr-FR");
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use smithay::utils::Size;

    fn ui() -> DocUi {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 4000)));
        ui.model_mut().replace(0..0, "hello world\nsecond line");
        ui.relayout();
        ui
    }

    #[test]
    fn a_word_selection_stops_at_whitespace() {
        let mut ui = ui();
        assert!(ui.select_word_at(8));
        assert_eq!(ui.model().selected_text(), "world");
    }

    #[test]
    fn a_line_selection_stays_on_its_line() {
        let mut ui = ui();
        assert!(ui.select_line_at(8));
        assert_eq!(ui.model().selected_text(), "hello world");
    }

    #[test]
    fn an_empty_line_has_no_selection_to_offer() {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 4000)));
        ui.model_mut().replace(0..0, "a\n\nb");
        ui.relayout();
        assert!(!ui.select_line_at(2));
    }
}

#[cfg(test)]
mod label_tests {
    use super::*;
    use smithay::utils::Size;

    /// Three figures on page 0, one per naming case.
    fn ui() -> DocUi {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 4000)));
        ui.model_mut().replace(
            0..0,
            "#1 a #2 \\app(#1, #2, 600, 300, \"foot\", launch: \"foot -T\")\n\
             #3 b #4 \\app(#3, #4, 600, 300, \"alacritty\")\n\
             #5 c #6 \\app(#5, #6, 600, 300)\n",
        );
        ui.relayout();
        ui
    }

    /// The label names the app that would actually start — the `launch:`
    /// command's program — not the binding id, which is a glob and may not look
    /// like anything the user can run.
    #[test]
    fn the_label_names_the_program_that_would_launch() {
        let ui = ui();
        let label = ui.dormant_label("f0").expect("a label");
        assert!(label.starts_with("foot —"), "{label}");
        // Only the program, not its arguments: "foot -T — click…" would read as
        // one long program name.
        assert!(!label.contains("-T"), "{label}");
    }

    /// With no `launch:` the binding id is the best name available.
    #[test]
    fn without_a_launch_command_the_binding_id_names_the_app() {
        let ui = ui();
        let label = ui.dormant_label("f1").expect("a label");
        assert!(label.starts_with("alacritty —"), "{label}");
    }

    /// The hint names both gestures, because both work.
    #[test]
    fn the_label_names_both_gestures() {
        let ui = ui();
        let label = ui.dormant_label("f0").expect("a label");
        assert!(label.contains("click"), "{label}");
        assert!(label.contains("Return"), "{label}");
    }

    /// A figure with a client is not dormant and carries no label — a live app
    /// does not need telling how to start.
    #[test]
    fn a_bound_figure_carries_no_label() {
        let mut ui = ui();
        assert!(ui.dormant_label("f0").is_some());
        ui.figures_mut().bind("f0", 3);
        assert_eq!(ui.dormant_label("f0"), None);
    }

    /// Only the visible page is labelled. A label on an off-page figure would
    /// name a slot the user is not looking at.
    #[test]
    fn only_the_visible_page_is_labelled() {
        let mut ui = ui();
        assert!(ui.page_count() > 1, "the fixture must paginate");
        let other = ui
            .figures()
            .figures()
            .iter()
            .find(|f| f.page != Some(0))
            .expect("a figure off page 0")
            .key
            .clone();
        assert_eq!(ui.dormant_label(&other), None, "off page 0");
        ui.layout.set_current_page(1);
        // f2 has neither a `launch:` command nor a binding id, so this is also
        // the only coverage of the key fallback: the figure key names the slot
        // rather than the label going blank.
        assert_eq!(other, "f2", "the fixture's id-less figure");
        let label = ui.dormant_label(&other).expect("labelled on its own page");
        assert!(label.starts_with("f2 —"), "{label}");
    }

    /// A key naming no figure is not an error.
    #[test]
    fn an_unknown_key_has_no_label() {
        assert_eq!(ui().dormant_label("f9"), None);
    }
}

#[cfg(test)]
mod page_tests {
    use super::*;
    use smithay::utils::Size;

    /// A document long enough to paginate in the fixture viewport.
    fn paged() -> DocUi {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 900)));
        let para = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
        ui.model_mut().replace(0..0, &para.repeat(300));
        ui.relayout();
        assert!(ui.page_count() > 1, "the fixture must paginate");
        ui
    }

    /// Switching pages must change the page the layout reports, because
    /// everything visual reads it: `doc_render`'s texture key, `figure_render`'s
    /// figure filter, the caret and selection rects, and the dormant marks.
    ///
    /// There were **two** copies of the page index — `PageState::current_page`
    /// and `DocLayoutCache::current_page` — and the page-switch path wrote only
    /// the first, while all of those read the second. So `PgDn` announced
    /// `page_changed{page:1}` over IPC, marked the ext-workspace bar, and
    /// rendered the same page. Worse, `PgDn` computed its target from the layout
    /// copy, so the second press asked for page 1 again, hit the "already there"
    /// guard in `goto_page` and did nothing: the key was permanently stuck, and
    /// `PgUp` was always a no-op.
    #[test]
    fn switching_pages_changes_the_page_the_layout_reports() {
        let mut ui = paged();
        assert_eq!(ui.current_page(), 0);
        let first = ui
            .layout()
            .page()
            .expect("page 0 is laid out")
            .glyphs
            .entries
            .len();

        ui.on_page_changed(1);

        assert_eq!(ui.current_page(), 1, "the layout must follow");
        let second = ui
            .layout()
            .page()
            .expect("page 1 is laid out")
            .glyphs
            .entries
            .len();
        assert_ne!(
            first, second,
            "the two pages must differ, or the switch did nothing"
        );

        // And back again, since `PgUp` computed from the same copy.
        ui.on_page_changed(0);
        assert_eq!(ui.current_page(), 0);
    }

    /// Page navigation is relative, so the caller must be able to ask "what is
    /// next?" and get an answer that is not permanently stuck on the first page.
    #[test]
    fn page_navigation_terms_move() {
        let mut ui = paged();
        let start = ui.current_page();
        ui.on_page_changed(start + 1);
        assert_eq!(ui.current_page(), start + 1);
        ui.on_page_changed(ui.current_page().saturating_sub(1));
        assert_eq!(ui.current_page(), start);
    }

    /// An out-of-range page clamps rather than pointing past the end, matching
    /// `set_current_page`'s contract.
    #[test]
    fn an_out_of_range_page_clamps() {
        let mut ui = paged();
        let last = ui.page_count() - 1;
        ui.on_page_changed(9999);
        assert_eq!(ui.current_page(), last);
    }

    /// The page is persisted, so what `save` writes must be the page the layout
    /// is actually showing — `session.json` recorded page 0 unconditionally
    /// before, because it read the copy nobody updated.
    #[test]
    fn the_saved_page_is_the_visible_page() {
        let pid = std::process::id();
        let thread = format!("{:?}", std::thread::current().id());
        let dir = std::env::temp_dir().join(format!("emthin-pagesave-{pid}-{thread}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let snapshot = dir.join("doc.loro");

        let mut ui = paged();
        ui.load(None, Some(&snapshot));
        ui.on_page_changed(1);
        ui.save();

        let json = dir.join("session.json");
        let text = std::fs::read_to_string(&json).expect("session.json written");
        assert!(
            text.contains(&format!("\"current_page\": {}", ui.current_page())),
            "session.json must record the visible page, got {text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A hit test must never return a figure from another page.
    ///
    /// `page_rect_for` hands an off-page figure the origin and the page size as a
    /// placeholder rect, so that rect sits on top of the visible page's top-left
    /// corner. `FigureManager::figure_under` matched on it, which meant a click in
    /// that corner could relaunch a figure the user cannot see, start a resize
    /// grab against a statement that is not on screen — whose `commit` then
    /// rewrites *that* statement's `\app` args — or map the click into an
    /// unrelated surface.
    #[test]
    fn a_hit_test_never_returns_a_figure_from_another_page() {
        /// Centre of a screen rect, in the `f64` space `figure_under` wants.
        fn centre(r: Rectangle<i32, Logical>) -> Point<f64, Logical> {
            Point::new(
                f64::from(r.loc.x) + f64::from(r.size.w) / 2.0,
                f64::from(r.loc.y) + f64::from(r.size.h) / 2.0,
            )
        }

        let mut ui = paged();
        assert!(ui.page_count() > 1, "the fixture must paginate");

        // A figure far past the end of the document, so it lands on a later page.
        let end = ui.model().text().len();
        ui.model_mut().replace(
            end..end,
            "\n\n#1 late #2 \\app(#1, #2, 200, 150, \"late\")\n",
        );
        ui.relayout();

        let off_page = ui
            .figures()
            .figures()
            .iter()
            .find(|f| f.page != Some(0))
            .expect("a figure on a page other than 0")
            .clone();
        let at = centre(off_page.rect);

        // The raw manager *does* match it — the placeholder rect is real.
        assert!(
            ui.figures().figure_under(at).is_some(),
            "FigureManager::figure_under matches the placeholder rect, \
             which is the hazard this test pins"
        );
        // The page-aware lookup must not.
        assert!(
            ui.figure_at(at).is_none(),
            "figure {} from page {:?} must not be hit-testable on page 0",
            off_page.key,
            off_page.page
        );
    }

    /// The page-aware hit test still finds a figure that *is* on the page.
    #[test]
    fn the_page_aware_hit_test_still_finds_a_visible_figure() {
        fn centre(r: Rectangle<i32, Logical>) -> Point<f64, Logical> {
            Point::new(
                f64::from(r.loc.x) + f64::from(r.size.w) / 2.0,
                f64::from(r.loc.y) + f64::from(r.size.h) / 2.0,
            )
        }

        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 900)));
        ui.model_mut()
            .replace(0..0, "#1 a #2 \\app(#1, #2, 200, 150, \"a\")");
        ui.relayout();

        let visible = ui
            .figures()
            .figures()
            .iter()
            .find(|f| f.page == Some(ui.current_page()) && f.rect.size.w > 0)
            .expect("a figure on the visible page")
            .clone();
        assert_eq!(
            ui.figure_at(centre(visible.rect)).map(|f| f.key.as_str()),
            Some(visible.key.as_str()),
            "a figure on the visible page must remain hittable"
        );
    }

    /// An app bound to a figure must survive an edit *above* that figure.
    ///
    /// A figure's layout key is `f<stmt-index>`, so inserting an `\app` above a
    /// running app's figure renumbers it. `sync` used to carry bindings over by
    /// that key: the carry read whichever figure had taken the old index, and the
    /// displaced app was reported as gone and released. So typing one new
    /// statement above a live terminal unmapped the terminal and could hand its
    /// surface to an unrelated figure.
    ///
    /// The binding is remembered against the statement's marker pair instead,
    /// which is the document's own identity and does not renumber.
    #[test]
    fn an_app_binding_survives_an_insertion_above_its_figure() {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 900)));
        ui.model_mut()
            .replace(0..0, "#1 a #2 \\app(#1, #2, 200, 150, \"a\")\n");
        ui.relayout();

        let key = ui
            .figures()
            .figures()
            .first()
            .expect("one figure")
            .key
            .clone();
        assert_eq!(key, "f0", "the fixture starts with a single figure");
        ui.figures_mut().get_mut("f0").expect("f0").app_id = Some(7);

        // Insert a whole new figure *above* it: the old statement becomes f1.
        ui.model_mut()
            .replace(0..0, "#3 new #4 \\app(#3, #4, 100, 100, \"new\")\n");
        let released = ui.relayout();

        assert!(
            released.is_empty(),
            "no app should be released by an insertion above it, got {released:?}"
        );
        assert_eq!(
            ui.figures().get("f1").and_then(|f| f.app_id),
            Some(7),
            "the displaced figure lost its binding"
        );
        assert_eq!(
            ui.figures().get("f0").and_then(|f| f.app_id),
            None,
            "the newly inserted figure must not inherit app 7"
        );
        // And the binding belongs to the right statement, not to the index.
        assert_eq!(ui.figures().get("f1").unwrap().stable_id, "1>2");
        assert_eq!(ui.figures().get("f0").unwrap().stable_id, "3>4");
    }

    /// Renumbering must not cross-assign between two bound figures.
    #[test]
    fn renumbering_does_not_swap_two_apps_bindings() {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 900)));
        ui.model_mut().replace(
            0..0,
            "#1 a #2 \\app(#1, #2, 200, 150, \"a\")\n#3 b #4 \\app(#3, #4, 200, 150, \"b\")\n",
        );
        ui.relayout();
        ui.figures_mut().get_mut("f0").expect("f0").app_id = Some(11);
        ui.figures_mut().get_mut("f1").expect("f1").app_id = Some(22);

        // Push a new figure to the very top: f0 -> f1, f1 -> f2.
        ui.model_mut()
            .replace(0..0, "#5 c #6 \\app(#5, #6, 100, 100, \"c\")\n");
        let released = ui.relayout();

        assert!(
            released.is_empty(),
            "nothing should be released: {released:?}"
        );
        let ids: Vec<_> = ui
            .figures()
            .figures()
            .iter()
            .map(|f| (f.key.as_str(), f.stable_id.as_str(), f.app_id))
            .collect();
        assert_eq!(
            ids,
            vec![
                ("f0", "5>6", None),
                ("f1", "1>2", Some(11)),
                ("f2", "3>4", Some(22)),
            ],
            "each app must stay with its own statement across a renumbering"
        );
    }

    /// Deleting a figure must still release its app — matching on the marker pair
    /// must not make bindings immortal.
    #[test]
    fn deleting_a_figure_still_releases_its_app() {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 900)));
        ui.model_mut().replace(
            0..0,
            "#1 a #2 \\app(#1, #2, 200, 150, \"a\")\n#3 b #4 \\app(#3, #4, 200, 150, \"b\")\n",
        );
        ui.relayout();
        ui.figures_mut().get_mut("f1").expect("f1").app_id = Some(22);

        // Delete the second figure outright.
        let text = ui.model().text().to_string();
        let cut = text.find("#3 b").expect("the second figure's caption");
        let end = text[cut..]
            .find('\n')
            .map(|n| cut + n + 1)
            .unwrap_or(text.len());
        ui.model_mut().replace(cut..end, "");
        let released = ui.relayout();

        assert_eq!(
            released,
            vec![22],
            "the deleted figure's app must be released"
        );
        assert_eq!(ui.figures().figures().len(), 1);
        assert_eq!(ui.figures().get("f0").and_then(|f| f.app_id), None);
    }

    /// An edit must reach the layout, not just the model.
    ///
    /// `DocUi::tick` autosaves the model, so the text was being persisted while
    /// the page raster, the glyph index and every figure rect stayed as they were
    /// — nothing in the input path called `relayout`, so a typed character only
    /// appeared when something unrelated (a resize, `PgDn`, an IPC message)
    /// happened to rebuild the layout. The caret is measured against the glyph
    /// index, so it did not move either.
    ///
    /// This is the lower half of that bug: the model must be dirty after an edit,
    /// and `relayout` must incorporate it. The upper half — that the keystroke
    /// handler actually calls `relayout` — needs a `Seat` and a live keyboard, so
    /// it is covered by the wrapper's construction rather than here.
    #[test]
    fn an_edit_reaches_the_layout_only_after_a_relayout() {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 900)));
        ui.model_mut().replace(0..0, "hi");
        ui.relayout();
        assert!(
            ui.model().caret() >= 2,
            "the caret sits after the inserted text"
        );

        let caret_before = ui.caret_rect().expect("a caret for the typed text").loc.x;

        ui.model_mut().insert_at_caret("x");
        assert!(
            ui.model().is_dirty(),
            "an edit must mark the model dirty or no relayout can be triggered"
        );

        // The glyph index still describes "hi", so the caret is measured against
        // the old page and cannot advance past the new glyph.
        let caret_stale = ui.caret_rect().expect("a caret").loc.x;
        assert_eq!(
            caret_stale, caret_before,
            "before a re-layout the caret is still measured against the old glyphs"
        );

        ui.relayout();
        assert!(!ui.model().is_dirty(), "relayout must clear the dirty flag");
        let caret_after = ui.caret_rect().expect("a caret").loc.x;
        assert!(
            caret_after > caret_before,
            "the re-laid-out page must place the caret past the new glyph \
             ({caret_before} -> {caret_after})"
        );
    }

    /// A caret-only edit does not dirty the model, so the keystroke wrapper's
    /// `is_dirty` gate lets it skip the re-layout.
    #[test]
    fn caret_motion_does_not_dirty_the_model() {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 900)));
        ui.model_mut().replace(0..0, "hello world");
        ui.relayout();
        let len = ui.model().len();
        ui.model_mut().set_caret(0);
        assert!(
            !ui.model().is_dirty(),
            "moving the caret must not force a re-layout"
        );
        ui.model_mut().set_caret(len);
        assert!(!ui.model().is_dirty());
    }

    /// Two statements sharing a marker pair must not both claim one app.
    ///
    /// Pasting a bound figure's line duplicates its `\app(#3, #4, …)` verbatim,
    /// and `stable_id` is that pair. The carry matched with `find` over the
    /// previous list without removing what it found, so both copies read the same
    /// binding: the app was composited twice, and `release_app` returned both
    /// keys.
    #[test]
    fn a_duplicated_statement_does_not_claim_one_app_twice() {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 900)));
        ui.model_mut()
            .replace(0..0, "#3 t #4 \\app(#3, #4, 200, 150, \"t\")\n");
        ui.relayout();
        ui.figures_mut().get_mut("f0").expect("f0").app_id = Some(7);

        // Paste the very same line again: identical markers, identical id.
        let text = ui.model().text().to_string();
        let end = ui.model().text().len();
        ui.model_mut().replace(end..end, &text);
        ui.relayout();

        let bound: Vec<_> = ui
            .figures()
            .figures()
            .iter()
            .filter(|f| f.app_id == Some(7))
            .map(|f| f.key.clone())
            .collect();
        assert_eq!(
            bound.len(),
            1,
            "app 7 must be claimed by exactly one figure, not {:?}",
            bound
        );
    }

    /// A viewport change must re-place the figure rects.
    ///
    /// `relayout` returns early when the model is clean, so resizing the window
    /// re-letterboxed the page raster — placed from `page_rect()`, recomputed
    /// every frame — while every figure rect kept its old scale and origin. The
    /// stale rect then drove hit-testing, the focus border, dormant marks and the
    /// IME origin.
    #[test]
    fn a_viewport_change_re_places_the_figure_rects() {
        let mut ui = DocUi::new();
        // Small viewport first: the A4 page is downscaled into it.
        ui.set_viewport(Size::from((700, 700)));
        // A figure that fits the page, so containment is a meaningful assertion.
        ui.model_mut()
            .replace(0..0, "#1 a #2 \\app(#1, #2, 300, 200, \"a\")\n");
        ui.relayout();
        let before = ui.figures().figures().first().expect("a figure").rect;
        assert!(
            before.size.w < 300,
            "the fixture must start downscaled, got {before:?}"
        );

        // Grow the window past the page size: the scale goes to 1.0.
        ui.set_viewport(Size::from((4000, 4000)));
        assert!(
            ui.geometry_dirty,
            "a viewport change must force the next relayout to re-sync geometry"
        );
        ui.relayout();
        assert!(
            !ui.geometry_dirty,
            "a geometry relayout must clear the re-sync flag"
        );

        let after = ui.figures().figures().first().expect("a figure").rect;
        assert_ne!(
            before.size, after.size,
            "the figure rect must follow the letterbox, not keep its old scale"
        );
        assert_eq!(
            (after.size.w, after.size.h),
            (300, 200),
            "a page smaller than the viewport is mapped 1:1, so the figure is \
             back to its declared size: {after:?}"
        );
        let page = ui.layout().page_rect();
        assert!(
            after.loc.x >= page.loc.x && after.loc.x + after.size.w <= page.loc.x + page.size.w,
            "the re-placed figure {after:?} must sit inside the new page {page:?}"
        );
    }

    /// Switching pages must re-place the figure rects.
    ///
    /// A figure's rect is only correct for the page it was placed on: an off-page
    /// figure gets the origin and the page size as a placeholder, and
    /// `place_figure` derives the letterbox factor from whatever `page_rect_for`
    /// returned — so the placeholder yields an *unscaled* rect. After a switch
    /// every figure still carried the rect computed for the previous page.
    ///
    /// Found by running the compositor for the first time: over IPC, `goto_page 1`
    /// reported the second figure at exactly its declared 320x240, while the first
    /// had been correctly scaled from a declared 640x400 to 559x350. The stale
    /// rect then drove compositing, hit-testing, the focus border, the dormant
    /// labels and the IME origin.
    #[test]
    fn switching_pages_re_places_the_figure_rects() {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1272, 736)));
        ui.model_mut().replace(
            0..0,
            "#1 notes #2 \\app(#1, #2, 640, 400, \"foot\")\n\
             prose that pushes the next figure onto a second page\n\
             #3 chat #4 \\app(#3, #4, 320, 240, \"foot\")\n",
        );
        ui.relayout();
        assert!(ui.page_count() > 1, "the fixture must paginate");

        // On page 0: the first figure is placed and scaled, the second is not.
        let first = ui.figures().get("f0").expect("f0").rect;
        assert!(
            first.size.w < 640,
            "the visible figure must be letterboxed, got {first:?}"
        );
        assert!(
            ui.figures().get("f1").expect("f1").page != Some(0),
            "the second figure must be off page 0"
        );

        ui.on_page_changed(1);
        assert!(
            ui.geometry_dirty,
            "a page change must force the figure re-sync"
        );
        ui.relayout();

        // Now page 1 is visible: the second figure must be placed *and scaled*,
        // and the first must no longer be the placed one.
        let second = ui.figures().get("f1").expect("f1").rect;
        assert_eq!(ui.figures().get("f1").expect("f1").page, Some(1));
        assert!(
            second.size.w < 320,
            "the newly visible figure must be letterboxed too, got {second:?} \
             (320x240 is the unscaled placeholder)"
        );
        let scale = second.size.w as f64 / 320.0;
        assert!(
            (scale - 0.87).abs() < 0.05,
            "both figures must share the page's letterbox factor, got {scale}"
        );
    }
}
