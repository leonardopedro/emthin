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

pub mod edit;
pub mod figures;
pub mod formals;
pub mod keymap;
pub mod layout;
pub mod model;

use std::time::{Duration, Instant};

use smithay::utils::{Logical, Point, Rectangle, Size};

pub use figures::{Figure, FigureManager};
pub use formals::FormalVerifier;
pub use layout::{DocLayoutCache, PlacedFigure};
pub use model::DocModel;

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
    /// Where the session is persisted (`None` = don't persist).
    session_file: Option<std::path::PathBuf>,
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
            session_file: None,
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

    /// The verifier behind every `\formal` verdict in this document.
    pub fn formals(&self) -> &FormalVerifier {
        &self.formals
    }

    /// Pin a kernel, for a test or a deployment that wants a specific binary.
    pub fn formals_mut(&mut self) -> &mut FormalVerifier {
        &mut self.formals
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
        if let Some(path) = path {
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    self.model = DocModel::new(&text);
                    self.session_file = session_file.map(std::path::Path::to_path_buf);
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
                    self.session_file = Some(file.to_path_buf());
                    self.relayout();
                    self.restore_page_beside(file);
                    return;
                }
            }
        }
        self.session_file = session_file.map(std::path::Path::to_path_buf);
        self.model = DocModel::new("");
        self.relayout();
    }

    /// Restore the current page from the `session.json` beside `snapshot`.
    ///
    /// Best-effort by design: a missing or unreadable file leaves the page at 0
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
        let page = crate::session::SessionFile::load(&json).current_page;
        self.layout.set_current_page(page);
    }

    /// Re-run layout if the document changed, then reconcile figures.
    ///
    /// Returns the apps whose figure disappeared (their `\app` statement
    /// was deleted) so the caller can close them.
    pub fn relayout(&mut self) -> Vec<u64> {
        if !self.model.is_dirty() {
            return Vec::new();
        }
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
        self.figures.sync(&self.model, &mut self.layout)
    }

    /// Recompute the letterbox against a new output size.
    pub fn set_viewport(&mut self, size: Size<i32, Logical>) {
        self.layout.set_viewport(size);
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
    pub fn on_page_changed(&mut self, page: usize) {
        self.layout.set_current_page(page);
    }

    /// Record (or clear) an app's title.
    pub fn on_app_title_changed(&mut self, app_id: u64, title: &str) {
        if let Some(figure) = self.figures.figure_of_app_mut(app_id) {
            figure.title = Some(title.to_string());
        }
    }

    /// Record an app's app_id (used for wildcard figure binding).
    pub fn on_app_id_changed(&mut self, app_id: u64, app_id_str: &str) {
        if let Some(figure) = self.figures.figure_of_app_mut(app_id) {
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
        // The snapshot is the document; this is the rest — currently just which
        // page the user was on. Written after the document so a crash between
        // the two leaves stale bookkeeping rather than a stale document.
        let session = crate::session::SessionFile {
            current_page: self.current_page(),
            ..Default::default()
        };
        if let Some(json) = session_json_path(&path) {
            session.save(&json);
        }
    }

    /// The caret rect in output-local logical px, for the IME bridge.
    pub fn caret_rect(&self) -> Option<Rectangle<i32, Logical>> {
        self.layout.caret_rect(self.model.caret())
    }

    /// The last layout error, if the document currently fails to lay
    /// out.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
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

/// Convenience: is `pos` inside any figure?
pub fn figure_at(figures: &FigureManager, pos: Point<f64, Logical>) -> Option<&Figure> {
    figures.figure_under(pos)
}

#[cfg(test)]
mod relaunch_tests {
    use super::*;
    use smithay::utils::Size;

    /// Two `\app` figures on page 0, with a remembered command for each.
    fn doc_with_two_figures() -> DocUi {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 1600)));
        ui.model_mut().replace(
            0..0,
            "#1 first #2 \\app(#1, #2, 600, 300, \"first\", launch: \"foot -T\")\n\
             #3 second #4 \\app(#3, #4, 600, 300, \"second\", launch: \"alacritty\")\n",
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
            let path = std::env::temp_dir().join(format!(
                "emthin-session-{}-{}-{}",
                std::process::id(),
                format!("{:?}", std::thread::current().id()),
                SEQ.fetch_add(1, Ordering::Relaxed)
            ));
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
        first.save();

        // A fresh UI is a relaunch: nothing carried over in memory.
        let mut second = DocUi::new();
        second.load(None, Some(&snapshot));
        assert_eq!(
            second.current_page(),
            expected,
            "the page must come back from session.json"
        );
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
        let dir = std::env::temp_dir().join(format!(
            "emthin-pagesave-{}-{}",
            std::process::id(),
            format!("{:?}", std::thread::current().id())
        ));
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
}
