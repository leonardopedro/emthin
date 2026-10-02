//! App-figure bookkeeping: one [`Figure`] per `\app` statement in the
//! document, plus the app↔figure bindings.
//!
//! The document is the layout authority. A figure's *declared* size
//! comes from its `\app` statement's literal args; its *actual* rect
//! comes from `mathed_core::figures::figures_in_frame` on the laid-out
//! page. This module is the seam between those two plus the compositor:
//! [`FigureManager::sync`] reconciles them after every re-layout, and
//! [`FigureManager::claim_toplevel`] binds a freshly mapped client to a
//! figure (appending a new figure to the document when every existing
//! one is taken).

use std::ops::Range;

use mathed_core::figures::FigureSpec;
use smithay::utils::{Logical, Point, Rectangle};

use crate::docui::layout::DocLayoutCache;
use crate::docui::model::DocModel;

/// One `\app` statement in the document, as a placed figure.
#[derive(Debug, Clone)]
pub struct Figure {
    /// `f<stmt-index>` — stable for the lifetime of the statement.
    pub key: String,
    /// Index of the `\app` statement in `MarkerScan::stmts`.
    pub stmt: usize,
    /// The statement's parsed arguments (declared size + binding id).
    pub spec: FigureSpec,
    /// Doc byte range of the caption span (`#s` end .. `#f` start).
    pub span: Range<usize>,
    /// Doc byte ranges of the `w` / `h` literal args — what a resize
    /// rewrites.
    pub w_arg: Range<usize>,
    pub h_arg: Range<usize>,
    /// The page this figure landed on, if its page is known.
    pub page: Option<usize>,
    /// Screen-space (letterboxed, logical px) rect on `page`. Zero-sized
    /// when the figure is not on the visible page.
    pub rect: Rectangle<i32, Logical>,
    /// `AppManager::AppWindow::window_id` bound to this figure, if any.
    pub app_id: Option<u64>,
    /// The app's last reported title (for the dormant stand-in and the
    /// control-plane `list_state`).
    pub title: Option<String>,
}

impl Figure {
    /// True when no app is currently mapped into this figure.
    pub fn is_dormant(&self) -> bool {
        self.app_id.is_none()
    }

    /// Top-left of the figure in output-local logical pixels.
    pub fn loc(&self) -> Point<i32, Logical> {
        self.rect.loc
    }

    /// (w, h) of the figure's placed rect.
    pub fn size(&self) -> (i32, i32) {
        (self.rect.size.w, self.rect.size.h)
    }

    /// The binding key other figures mirror this one by, if any.
    pub fn mirror_id(&self) -> Option<&str> {
        self.spec.id.as_deref()
    }
}

/// The document's figures and their app bindings.
#[derive(Debug, Default)]
pub struct FigureManager {
    figures: Vec<Figure>,
}

impl FigureManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// All figures, in document order.
    pub fn figures(&self) -> &[Figure] {
        &self.figures
    }

    pub fn len(&self) -> usize {
        self.figures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.figures.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&Figure> {
        self.figures.iter().find(|f| f.key == key)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Figure> {
        self.figures.iter_mut().find(|f| f.key == key)
    }

    /// Bind `app_id` to the figure named `key`. Returns false when the
    /// key doesn't exist.
    pub fn bind(&mut self, key: &str, app_id: u64) -> bool {
        match self.get_mut(key) {
            Some(figure) => {
                figure.app_id = Some(app_id);
                true
            }
            None => false,
        }
    }

    /// Unbind every figure holding `app_id` (the app's surface died).
    /// Returns the keys that were released, in document order.
    ///
    /// The `\app` statements stay put: the figures go dormant, they
    /// don't disappear.
    pub fn release_app(&mut self, app_id: u64) -> Vec<String> {
        let mut released = Vec::new();
        for figure in &mut self.figures {
            if figure.app_id == Some(app_id) {
                figure.app_id = None;
                released.push(figure.key.clone());
            }
        }
        released
    }

    /// The figure bound to `app_id`.
    pub fn figure_of_app(&self, app_id: u64) -> Option<&Figure> {
        self.figures.iter().find(|f| f.app_id == Some(app_id))
    }

    pub fn figure_of_app_mut(&mut self, app_id: u64) -> Option<&mut Figure> {
        self.figures.iter_mut().find(|f| f.app_id == Some(app_id))
    }

    /// Every figure bound to `app_id` — mirrors are several `\app`
    /// statements sharing one binding id.
    pub fn figures_of_app(&self, app_id: u64) -> Vec<&Figure> {
        self.figures
            .iter()
            .filter(|f| f.app_id == Some(app_id))
            .collect()
    }

    /// Rebuild the figure list from the document and the latest layout.
    ///
    /// Declarative state (spec, spans, arg ranges) comes from the doc;
    /// geometry (page, rect) comes from the laid-out pages. Bindings
    /// (`app_id`, `title`) are carried over by *key* so a re-layout
    /// doesn't drop a live app — and a figure whose key vanished (its
    /// `\app` statement was deleted) releases its binding, which the
    /// caller turns into a `close` IPC.
    pub fn sync(&mut self, model: &DocModel, layout: &mut DocLayoutCache) -> Vec<u64> {
        let _ = model.scan();
        let mut figures = Vec::new();
        let mut released = Vec::new();

        for seg in model.segments() {
            if !seg.kind.is_app() {
                continue;
            }
            let Some(figure) = crate::docui::edit::figure_from_segment(seg) else {
                continue;
            };
            // Carry over the binding this statement had last time.
            let carried = self.get(&figure.key);
            figures.push(Figure {
                page: None,
                rect: Rectangle::default(),
                app_id: carried.and_then(|f| f.app_id),
                title: carried.and_then(|f| f.title.clone()),
                ..figure
            });
        }

        // Drop the figures whose statements went away, reporting the
        // apps that no longer have a home.
        let live: Vec<&str> = figures.iter().map(|f| f.key.as_str()).collect();
        for gone in self
            .figures
            .iter()
            .filter(|f| !live.contains(&f.key.as_str()))
        {
            if let Some(app_id) = gone.app_id {
                released.push(app_id);
            }
        }

        // Attach geometry from the layout, and move any figure that a
        // reflow pushed onto a different page.
        for figure in &mut figures {
            if let Some(placed) = layout.place_figure(&figure.key) {
                figure.page = Some(placed.page);
                figure.rect = placed.rect;
            } else {
                figure.page = None;
                figure.rect = Rectangle::default();
            }
        }

        self.figures = figures;
        released
    }

    /// The topmost figure whose rect contains `pos` (output-local
    /// logical pixels).
    pub fn figure_under(&self, pos: Point<f64, Logical>) -> Option<&Figure> {
        // Reverse order: later figures are visually on top.
        self.figures.iter().rev().find(|f| contains(&f.rect, pos))
    }

    /// The screen-space origin of the app's figure on `visible_page`,
    /// if it has one. Used for IME origin translation: a client reports
    /// caret rects in its own surface-local frame, which becomes
    /// output-local only once the figure's rect is added back in.
    pub fn origin_on_page(&self, app_id: u64, visible_page: usize) -> Option<Point<i32, Logical>> {
        self.figures_of_app(app_id)
            .into_iter()
            .find(|f| f.page == Some(visible_page))
            .map(|f| f.loc())
    }
}

fn contains(rect: &Rectangle<i32, Logical>, pos: Point<f64, Logical>) -> bool {
    let x = pos.x;
    let y = pos.y;
    let r = rect;
    x >= f64::from(r.loc.x)
        && x < f64::from(r.loc.x + r.size.w)
        && y >= f64::from(r.loc.y)
        && y < f64::from(r.loc.y + r.size.h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn figure(key: &str, x: i32, y: i32, w: i32, h: i32) -> Figure {
        Figure {
            key: key.to_string(),
            stmt: 0,
            spec: FigureSpec { w, h, id: None },
            span: 0..0,
            w_arg: 0..0,
            h_arg: 0..0,
            page: Some(0),
            rect: Rectangle::new((x, y).into(), (w, h).into()),
            app_id: None,
            title: None,
        }
    }

    #[test]
    fn figure_under_picks_the_topmost() {
        let mut mgr = FigureManager::new();
        mgr.figures = vec![figure("f0", 0, 0, 100, 100), figure("f1", 50, 50, 100, 100)];
        // Overlapping point: the later figure wins.
        let hit = mgr.figure_under((60.0, 60.0).into()).expect("hit");
        assert_eq!(hit.key, "f1");
        // Non-overlapping point: the earlier one.
        let hit = mgr.figure_under((10.0, 10.0).into()).expect("hit");
        assert_eq!(hit.key, "f0");
        // Outside everything.
        assert!(mgr.figure_under((500.0, 500.0).into()).is_none());
    }

    #[test]
    fn figure_under_excludes_the_right_and_bottom_edges() {
        let mut mgr = FigureManager::new();
        mgr.figures = vec![figure("f0", 10, 10, 100, 50)];
        // Left/top are inside; right/bottom belong to the next thing.
        assert!(mgr.figure_under((10.0, 10.0).into()).is_some());
        assert!(mgr.figure_under((109.9, 59.9).into()).is_some());
        assert!(mgr.figure_under((110.0, 30.0).into()).is_none());
        assert!(mgr.figure_under((30.0, 60.0).into()).is_none());
    }

    #[test]
    fn mirror_lookup_groups_figures_by_app() {
        let mut mgr = FigureManager::new();
        let mut a = figure("f0", 0, 0, 10, 10);
        a.app_id = Some(7);
        let mut b = figure("f1", 20, 0, 10, 10);
        b.app_id = Some(7);
        let mut c = figure("f2", 40, 0, 10, 10);
        c.app_id = Some(8);
        mgr.figures = vec![a, b, c];

        let keys: Vec<_> = mgr
            .figures_of_app(7)
            .into_iter()
            .map(|f| f.key.clone())
            .collect();
        assert_eq!(keys, ["f0", "f1"]);
        assert_eq!(mgr.figure_of_app(8).expect("bound").key, "f2");
        assert!(mgr.figure_of_app(9).is_none());
    }

    #[test]
    fn dormant_means_unbound() {
        let mut f = figure("f0", 0, 0, 10, 10);
        assert!(f.is_dormant());
        f.app_id = Some(1);
        assert!(!f.is_dormant());
    }
}
