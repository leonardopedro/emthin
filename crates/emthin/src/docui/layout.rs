//! Document layout: the paged render cache plus the coordinate map
//! between the document's page space and the host window.
//!
//! Paging is Typst's own page model (via `mathed_mini::layout_doc_paged`),
//! never pixel slicing, so a figure that doesn't fit a page moves to the
//! next one exactly like a figure in a PDF would. The window shows
//! exactly **one** page, letterboxed and centred — the "PDF viewer"
//! look.
//!
//! Three coordinate spaces, after the driftwm canvas model
//! (`driftwm/src/canvas.rs`) adapted from camera+zoom to scroll+page:
//!
//! - **doc space** — page-local points (== raster px at 1px/pt), plus
//!   which page. This is what `figures_in_frame` reports.
//! - **page space** — the placed page's rect in output-local logical px.
//! - **output space** — the host window, 0,0 at the top-left.
//!
//! Zoom is fixed at 1.0 in v1; the map is written so adding it is a
//! scale factor on `doc_to_screen` and nothing else.

use mathed_core::figures::FigureRect;
use mathed_mini::{layout_doc_paged, PageLayout, RenderError};
use smithay::utils::{Logical, Point, Rectangle, Size};

/// Where one figure landed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedFigure {
    /// Index of the page the figure is on.
    pub page: usize,
    /// The figure's rect in output-local logical px.
    pub rect: Rectangle<i32, Logical>,
}

/// The rendered document: every page's raster + glyph index + figure
/// rects, plus the letterbox mapping for the visible page.
///
/// Not `Debug`: `mathed_mini::PageLayout` carries raster buffers, and
/// dumping a whole page's pixels into a log line helps nobody.
#[derive(Default)]
pub struct DocLayoutCache {
    pages: Vec<PageLayout>,
    /// Size of one page in logical px (from the first page's raster).
    page_size: Size<i32, Logical>,
    /// Output size the letterbox was computed for.
    viewport: Size<i32, Logical>,
    /// Index of the visible page.
    current_page: usize,
}

impl DocLayoutCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Re-lay out the document. Cheap no-op when nothing changed.
    pub fn rebuild(
        &mut self,
        doc_text: &str,
        opts: &mathed_core::TransformOptions,
    ) -> Result<(), RenderError> {
        self.pages = layout_doc_paged(doc_text, opts)?;
        self.page_size = self
            .pages
            .first()
            .map(|p| Size::from((p.width as i32, p.height as i32)))
            .unwrap_or_default();
        // A shorter document can leave the viewer past the end.
        if self.current_page >= self.page_count() {
            self.current_page = self.page_count().saturating_sub(1);
        }
        Ok(())
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    pub fn current_page(&self) -> usize {
        self.current_page
    }

    pub fn set_current_page(&mut self, page: usize) {
        self.current_page = page.min(self.page_count().saturating_sub(1));
    }

    /// The visible page's layout, if the document has pages.
    pub fn page(&self) -> Option<&PageLayout> {
        self.pages.get(self.current_page)
    }

    pub fn page_at(&self, idx: usize) -> Option<&PageLayout> {
        self.pages.get(idx)
    }

    /// Glyph index for caret/selection geometry on the visible page.
    pub fn glyphs(&self) -> Option<&mathed_core::GlyphIndex> {
        self.page().map(|p| &p.glyphs)
    }

    /// The size of one page, in logical px.
    pub fn page_size(&self) -> Size<i32, Logical> {
        self.page_size
    }

    /// The output size the letterbox was last computed for.
    pub fn viewport(&self) -> Size<i32, Logical> {
        self.viewport
    }

    /// The visible page's rect in output-local logical px: the page
    /// scaled to fit and centred (the "letterbox").
    pub fn page_rect(&self) -> Rectangle<i32, Logical> {
        letterbox(self.page_size, self.viewport)
    }

    /// Where `key` landed, if it is on any page.
    pub fn place_figure(&self, key: &str) -> Option<PlacedFigure> {
        let page = self
            .pages
            .iter()
            .position(|p| p.figures.iter().any(|f| f.key == key))?;
        let rect = self.page_rect_for(page);
        let FigureRect { rect: fr, .. } = self.pages[page]
            .figures
            .iter()
            .find(|f| f.key == key)
            .expect("position() found the key in this page");
        Some(PlacedFigure {
            page,
            rect: translate(fr, rect.loc),
        })
    }

    /// The letterboxed rect of page `page`, in output-local logical px.
    fn page_rect_for(&self, page: usize) -> Rectangle<i32, Logical> {
        if page == self.current_page {
            return self.page_rect();
        }
        // Off-page figures are still reported (the figure list needs
        // their page index) but with no meaningful screen rect — the
        // caller only reads `page` for them.
        Rectangle::new(
            (0, 0).into(),
            Size::from((self.page_size.w, self.page_size.h)),
        )
    }

    /// Convert a doc-space point (page-local px) on the visible page to
    /// output-local logical px.
    pub fn doc_to_screen(&self, doc: Point<f64, Logical>) -> Point<i32, Logical> {
        let page = self.page_rect();
        Point::from((
            page.loc.x + doc.x.round() as i32,
            page.loc.y + doc.y.round() as i32,
        ))
    }

    /// Convert an output-local logical position to doc-space points on
    /// the visible page. Returns `None` when the point is outside the
    /// page (in the letterbox margin).
    pub fn screen_to_doc(&self, screen: Point<f64, Logical>) -> Option<Point<f64, Logical>> {
        let page = self.page_rect();
        if page.size.w <= 0 || page.size.h <= 0 {
            return None;
        }
        let x = screen.x - f64::from(page.loc.x);
        let y = screen.y - f64::from(page.loc.y);
        if x < 0.0 || y < 0.0 || x >= f64::from(page.size.w) || y >= f64::from(page.size.h) {
            return None;
        }
        Some(Point::new(x, y))
    }

    /// The caret rect for a doc byte, in output-local logical px.
    pub fn caret_rect(&self, doc_byte: usize) -> Option<Rectangle<i32, Logical>> {
        let geom = self.glyphs()?.caret_for_byte(doc_byte)?;
        let origin = self.doc_to_screen(Point::new(f64::from(geom.x), f64::from(geom.top)));
        Some(Rectangle::new(
            origin,
            Size::from((geom.width.ceil() as i32, geom.height.ceil() as i32)),
        ))
    }

    /// Selection highlight rects for a doc range, in output-local
    /// logical px.
    pub fn selection_rects(&self, range: std::ops::Range<usize>) -> Vec<Rectangle<i32, Logical>> {
        let Some(glyphs) = self.glyphs() else {
            return Vec::new();
        };
        glyphs
            .rects_for_range(range)
            .into_iter()
            .map(|r| {
                let origin = self.doc_to_screen(Point::new(f64::from(r.x0), f64::from(r.y0)));
                Rectangle::new(
                    origin,
                    Size::from(((r.x1 - r.x0).ceil() as i32, (r.y1 - r.y0).ceil() as i32)),
                )
            })
            .collect()
    }

    /// Update the output size the letterbox is computed against.
    pub fn set_viewport(&mut self, size: Size<i32, Logical>) {
        self.viewport = size;
    }

    /// Display name for a page, used by the ext-workspace bar.
    pub fn page_name(&self, page: usize) -> String {
        format!("Page {}", page + 1)
    }
}

/// Fit `content` inside `viewport`, centred — the letterbox.
fn letterbox(content: Size<i32, Logical>, viewport: Size<i32, Logical>) -> Rectangle<i32, Logical> {
    if content.w <= 0 || content.h <= 0 || viewport.w <= 0 || viewport.h <= 0 {
        return Rectangle::new((0, 0).into(), content);
    }
    // Uniform scale down only: a page is never blown up past 1:1, so a
    // small document stays crisp instead of being magnified into blur.
    let scale = (viewport.w as f64 / content.w as f64)
        .min(viewport.h as f64 / content.h as f64)
        .min(1.0);
    let w = (content.w as f64 * scale).round().max(1.0) as i32;
    let h = (content.h as f64 * scale).round().max(1.0) as i32;
    Rectangle::new(
        Point::from(((viewport.w - w) / 2, (viewport.h - h) / 2)),
        Size::from((w, h)),
    )
}

fn translate(r: &mathed_core::RectF, origin: Point<i32, Logical>) -> Rectangle<i32, Logical> {
    Rectangle::new(
        Point::from((
            origin.x + r.x0.round() as i32,
            origin.y + r.y0.round() as i32,
        )),
        Size::from((
            (r.x1 - r.x0).round().max(1.0) as i32,
            (r.y1 - r.y0).round().max(1.0) as i32,
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_centres_and_never_upscales() {
        // Content smaller than the viewport: centred at 1:1.
        let r = letterbox(Size::from((400, 300)), Size::from((1000, 800)));
        assert_eq!(r.size, Size::from((400, 300)));
        assert_eq!(r.loc, Point::from((300, 250)));
        // Content wider than the viewport: scaled down to fit width,
        // centred vertically.
        let r = letterbox(Size::from((2000, 1000)), Size::from((1000, 800)));
        assert_eq!(r.size, Size::from((1000, 500)));
        assert_eq!(r.loc, Point::from((0, 150)));
    }

    #[test]
    fn letterbox_survives_degenerate_sizes() {
        let r = letterbox(Size::from((0, 0)), Size::from((800, 600)));
        assert_eq!(r.size, Size::from((0, 0)));
        let r = letterbox(Size::from((400, 300)), Size::from((0, 0)));
        assert_eq!(r.size, Size::from((400, 300)));
    }

    #[test]
    fn doc_and_screen_are_inverse() {
        let mut cache = DocLayoutCache::new();
        cache.page_size = Size::from((600, 800));
        cache.set_viewport(Size::from((1200, 900)));
        // A 600x800 page in a 1200x900 viewport is *centred* at 1:1:
        // loc = ((1200-600)/2, (900-800)/2) = (300, 50).
        let doc = Point::new(100.0, 200.0);
        let screen = cache.doc_to_screen(doc);
        assert_eq!(screen, Point::from((400, 250)));
        let back = cache
            .screen_to_doc(Point::new(f64::from(screen.x), f64::from(screen.y)))
            .expect("inside the page");
        assert!((back.x - 100.0).abs() < 1.0 && (back.y - 200.0).abs() < 1.0);
    }

    #[test]
    fn screen_to_doc_rejects_the_letterbox_margin() {
        let mut cache = DocLayoutCache::new();
        cache.page_size = Size::from((600, 800));
        cache.set_viewport(Size::from((1200, 900)));
        // Letterbox margin on the left.
        assert!(cache.screen_to_doc(Point::new(10.0, 10.0)).is_none());
        assert!(cache.screen_to_doc(Point::new(-1.0, 10.0)).is_none());
    }

    #[test]
    fn empty_layout_has_no_pages_and_no_caret() {
        let cache = DocLayoutCache::new();
        assert_eq!(cache.page_count(), 0);
        assert!(cache.page().is_none());
        assert!(cache.caret_rect(0).is_none());
        assert!(cache.screen_to_doc(Point::new(1.0, 1.0)).is_none());
    }

    #[test]
    fn current_page_clamps_into_range() {
        let mut cache = DocLayoutCache::new();
        cache.set_current_page(99);
        assert_eq!(cache.current_page(), 0);
    }
}
