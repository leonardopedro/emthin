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
