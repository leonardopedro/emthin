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
    /// Spawn commands remembered per figure id for dormant figures.
    dormant_spawns: std::collections::HashMap<String, (String, Vec<String>)>,
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
            dormant_spawns: std::collections::HashMap::new(),
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
                    return;
                }
            }
        }
        self.session_file = session_file.map(std::path::Path::to_path_buf);
        self.model = DocModel::new("");
        self.relayout();
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
    pub fn on_page_changed(&mut self) {
        // Nothing to do beyond what the state already did; kept as the
        // single seam where page-dependent document state will live.
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

    /// Remember the command that launched an app, so a dormant figure
    /// can relaunch it.
    pub fn remember_spawn(&mut self, figure_key: &str, cmd: String, args: Vec<String>) {
        self.dormant_spawns
            .insert(figure_key.to_string(), (cmd, args));
    }

    /// The saved launch command for a figure, if any.
    pub fn spawn_for(&self, figure_key: &str) -> Option<&(String, Vec<String>)> {
        self.dormant_spawns.get(figure_key)
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

    /// The command to relaunch for `key`, if it is dormant and has one saved.
    ///
    /// Returns the command rather than the key so the caller can spawn without
    /// re-deriving what "dormant with a command" means. A figure that already
    /// has an app is refused: relaunching it would put a second client where
    /// one is already composited.
    pub fn relaunch_target(&self, key: &str) -> Option<&(String, Vec<String>)> {
        let fig = self.figures.get(key)?;
        if !fig.is_dormant() {
            return None;
        }
        self.spawn_for(key)
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
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match std::fs::write(&path, self.model.snapshot()) {
            Ok(()) => tracing::debug!("session saved to {}", path.display()),
            Err(e) => tracing::warn!("session save failed {}: {e}", path.display()),
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

/// The default session path, `$XDG_STATE_HOME/emthin/session.loro`.
pub fn default_session_file() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(|h| std::path::PathBuf::from(h).join(".local/state"))
                .unwrap_or_else(std::env::temp_dir)
        });
    base.join("emthin").join("session.loro")
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
            "#1 first #2 \\app(#1, #2, 600, 300, \"first\")\n\
             #3 second #4 \\app(#3, #4, 600, 300, \"second\")\n",
        );
        ui.relayout();
        ui.remember_spawn("f0", "foot".into(), vec!["-T".into()]);
        ui.remember_spawn("f1", "foot".into(), vec!["-T".into()]);
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

    /// `relaunch_target` needs both halves: dormant *and* a saved command. A
    /// figure with no remembered command must not spawn an empty one.
    #[test]
    fn relaunch_needs_a_saved_command() {
        let mut ui = DocUi::new();
        ui.set_viewport(Size::from((1200, 1600)));
        ui.model_mut()
            .replace(0..0, "#1 a #2 \\app(#1, #2, 600, 300, \"a\")");
        ui.relayout();
        assert_eq!(ui.relaunch_target("f0"), None, "dormant but no command");
        ui.remember_spawn("f0", "foot".into(), vec!["-T".into()]);
        let (cmd, args) = ui.relaunch_target("f0").expect("now launchable");
        assert_eq!(cmd, "foot");
        assert_eq!(*args, vec!["-T".to_string()]);
        // A key that names no figure is not an error, just nothing to do.
        assert_eq!(ui.relaunch_target("f9"), None);
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
        ui.on_page_changed();
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
