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
//! **scale space** — between page and output. The page is scaled to fit the
//! viewport (the letterbox), and that factor applies to *everything* mapped from
//! page space to output space: figures, caret, selection boxes, and click
//! hit-testing. It used to apply to the page raster alone, so every mapping
//! disagreed with what was drawn whenever the window was smaller than the page.
//!
//! Zoom is fixed at 1.0 in v1; this letterbox factor is not zoom — it is the
//! fit-to-window scale, and it is already accounted for.

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
            rect: translate(fr, rect.loc, page_scale(self.page_size, rect.size) as f32),
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
        let scale = page_scale(self.page_size, page.size);
        Point::from((
            page.loc.x + (doc.x * scale).round() as i32,
            page.loc.y + (doc.y * scale).round() as i32,
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
        // Back into page-local px. Without this a click landed at the wrong
        // glyph as soon as the window was smaller than the page.
        let scale = page_scale(self.page_size, page.size);
        Some(Point::new(x / scale, y / scale))
    }

    /// The caret rect for a doc byte, in output-local logical px.
    pub fn caret_rect(&self, doc_byte: usize) -> Option<Rectangle<i32, Logical>> {
        let geom = self.glyphs()?.caret_for_byte(doc_byte)?;
        // `doc_to_screen` scales the origin; the size has to be scaled by the same
        // factor or the caret keeps its page-local width and the glyphs it sits
        // between are the scaled ones.
        let page = self.page_rect();
        let scale = page_scale(self.page_size, page.size) as f32;
        let origin = self.doc_to_screen(Point::new(f64::from(geom.x), f64::from(geom.top)));
        Some(Rectangle::new(
            origin,
            Size::from((
                (geom.width * scale).ceil().max(1.0) as i32,
                (geom.height * scale).ceil().max(1.0) as i32,
            )),
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
                // Same factor as the origin, for the same reason: an unscaled
                // width made the highlight overrun the selection by 5% and run
                // past the page edge on a full-width line.
                let page = self.page_rect();
                let scale = page_scale(self.page_size, page.size) as f32;
                let origin = self.doc_to_screen(Point::new(f64::from(r.x0), f64::from(r.y0)));
                Rectangle::new(
                    origin,
                    Size::from((
                        ((r.x1 - r.x0) * scale).ceil().max(1.0) as i32,
                        ((r.y1 - r.y0) * scale).ceil().max(1.0) as i32,
                    )),
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
/// The letterbox factor between the page's own size and its placed rect.
///
/// Derived from the same `letterbox` that produced the placed rect, so the
/// factor and the rect it came from cannot drift apart. Returns 1.0 for a
/// degenerate page rather than dividing by zero.
fn page_scale(content: Size<i32, Logical>, placed: Size<i32, Logical>) -> f64 {
    if content.w <= 0 || content.h <= 0 || placed.w <= 0 || placed.h <= 0 {
        return 1.0;
    }
    f64::from(placed.w) / f64::from(content.w)
}

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

/// Map a page-local rect to output-local logical px.
///
/// `scale` is the letterbox factor, **not** 1.0. The page raster is drawn scaled
/// down to fit the viewport — an A4 page in the default 1280x800 window gets
/// about 0.95 — so a pure translation here placed every figure, caret, selection
/// box and dormant mark at unscaled doc coordinates while the page underneath was
/// scaled. At scale 0.95 a figure at doc x=600 was off by ~30 px and 5% too large,
/// and click hit-testing disagreed with what was on screen.
///
/// Every caller goes through `page_rect_for`, which derives the scale from the
/// same `letterbox` the page rect comes from, so the two cannot disagree.
fn translate(
    r: &mathed_core::RectF,
    origin: Point<i32, Logical>,
    scale: f32,
) -> Rectangle<i32, Logical> {
    Rectangle::new(
        Point::from((
            origin.x + (r.x0 * scale).round() as i32,
            origin.y + (r.y0 * scale).round() as i32,
        )),
        Size::from((
            ((r.x1 - r.x0) * scale).round().max(1.0) as i32,
            ((r.y1 - r.y0) * scale).round().max(1.0) as i32,
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

    /// Every page→output mapping must apply the letterbox factor, because the
    /// page raster is drawn scaled.
    ///
    /// The page is scaled to fit the viewport, and the factor used to reach only
    /// `page_rect` — so a figure, the caret, a selection box and a click all
    /// disagreed with the page underneath as soon as the window was smaller than
    /// the page. The default 1280x800 window against an A4 page is already a
    /// ~0.95 scale, so this was the default configuration, not an edge case.
    ///
    /// The round trip is the property: mapping a doc point to the screen and
    /// back must return the point it started from.
    #[test]
    fn doc_and_screen_round_trip_through_the_letterbox_scale() {
        let mut cache = DocLayoutCache::new();
        cache.set_viewport(Size::from((1280, 800)));
        cache
            .rebuild("Hello, world", &mathed_core::TransformOptions::default())
            .expect("layout");

        // Force a non-1:1 fit: a page taller than the viewport.
        let scale = page_scale(cache.page_size, cache.page_rect().size);
        assert!(
            scale < 1.0,
            "the fixture must actually downscale, got scale {scale} \
             (page {:?} in a 1280x800 viewport)",
            cache.page_size
        );

        for doc in [
            Point::new(10.0f64, 10.0),
            Point::new(200.0, 300.0),
            Point::new(
                cache.page_size.w as f64 - 1.0,
                cache.page_size.h as f64 - 1.0,
            ),
        ] {
            let screen = cache.doc_to_screen(doc);
            let back = cache
                .screen_to_doc(Point::new(f64::from(screen.x), f64::from(screen.y)))
                .unwrap_or_else(|| panic!("{doc:?} mapped to {screen:?}, outside the page"));
            assert!(
                (back.x - doc.x).abs() <= 1.0 && (back.y - doc.y).abs() <= 1.0,
                "round trip drifted: {doc:?} -> {screen:?} -> {back:?} (scale {scale})"
            );
        }
    }

    /// A figure's placed rect must fit inside the placed page, scaled with it.
    /// Before the fix it was placed at unscaled doc coordinates while the page
    /// was drawn scaled, so a figure near the right edge landed past the page
    /// boundary that the raster was actually drawn to.
    #[test]
    fn a_figure_stays_inside_the_scaled_page() {
        let mut cache = DocLayoutCache::new();
        cache.set_viewport(Size::from((1280, 800)));
        // A figure that fits the page: a 600pt figure would overflow a595pt page
        // even at 1:1, which would be the document asking for the impossible
        // rather than a mapping bug.
        let doc = "#1 app #2 \\app(#1, #2, 300, 200, \"a\")";
        cache
            .rebuild(doc, &mathed_core::TransformOptions::default())
            .expect("layout");
        let scale = page_scale(cache.page_size, cache.page_rect().size);
        assert!(scale < 1.0, "the fixture must downscale, got {scale}");

        let page = cache.page_rect();
        let placed = cache
            .place_figure("f0")
            .expect("the figure is in the layout");
        let fig = placed.rect;
        assert!(
            fig.loc.x >= page.loc.x && fig.loc.y >= page.loc.y,
            "figure {:?} starts before the page {page:?}",
            fig
        );
        assert!(
            fig.loc.x + fig.size.w <= page.loc.x + page.size.w
                && fig.loc.y + fig.size.h <= page.loc.y + page.size.h,
            "figure {:?} overflows the scaled page {page:?} (scale {scale})",
            fig
        );
    }

    /// A page that fits is still mapped 1:1 — the fix must not shrink a page
    /// that did not need it.
    #[test]
    fn a_page_that_fits_is_mapped_one_to_one() {
        let mut cache = DocLayoutCache::new();
        cache.set_viewport(Size::from((4000, 4000)));
        cache
            .rebuild("Hello", &mathed_core::TransformOptions::default())
            .expect("layout");
        assert_eq!(
            page_scale(cache.page_size, cache.page_rect().size),
            1.0,
            "a page smaller than the viewport must not be scaled"
        );
    }

    /// `page_scale` must not divide by zero on a degenerate page.
    #[test]
    fn a_degenerate_page_scales_by_one() {
        assert_eq!(page_scale(Size::from((0, 0)), Size::from((100, 100))), 1.0);
        assert_eq!(page_scale(Size::from((100, 100)), Size::from((0, 0))), 1.0);
    }

    /// The caret and selection boxes must be scaled by the same factor as the
    /// page they sit on.
    ///
    /// `d8f889c` scaled the *origins* of every page→output mapping but left the
    /// caret's and the selection's *sizes* in page-local pixels, which made the
    /// caret wider than the glyph it sits in and made a highlight overrun the
    /// selection — by 5% in the default 1280x800 window, and past the page edge
    /// on a full-width line.
    ///
    /// Asserted against the factor itself rather than against "looks plausible",
    /// because a loose bound passes with the bug in place.
    #[test]
    fn a_selection_box_is_the_doc_width_times_the_letterbox_scale() {
        let mut cache = DocLayoutCache::new();
        // Small viewport, so the page is genuinely downscaled.
        cache.set_viewport(Size::from((400, 400)));
        cache
            .rebuild("hello world", &mathed_core::TransformOptions::default())
            .expect("layout");
        let scale = page_scale(cache.page_size, cache.page_rect().size);
        assert!(scale < 0.9, "the fixture must downscale hard, got {scale}");

        let doc_width: f32 = cache
            .glyphs()
            .expect("a glyph index")
            .rects_for_range(0..5)
            .iter()
            .map(|r| r.x1 - r.x0)
            .sum();
        assert!(doc_width > 1.0, "the fixture must have a measurable width");

        let placed = cache.selection_rects(0..5);
        let width: i32 = placed.iter().map(|r| r.size.w).sum();
        let expected = (f64::from(doc_width) * scale).round() as i32;
        assert!(
            (width - expected).abs() <= 1,
            "selection width {width} should be the doc width {doc_width} at scale \
             {scale}, i.e. about {expected}"
        );
    }

    /// The caret must be as wide as the glyph it sits in — which it only is if
    /// both go through the same scale.
    #[test]
    fn the_caret_is_as_wide_as_the_glyph_under_it() {
        let mut cache = DocLayoutCache::new();
        cache.set_viewport(Size::from((400, 400)));
        cache
            .rebuild("hello world", &mathed_core::TransformOptions::default())
            .expect("layout");
        let scale = page_scale(cache.page_size, cache.page_rect().size);
        assert!(scale < 0.9, "the fixture must downscale, got {scale}");

        // One glyph, placed.
        let glyph = cache.selection_rects(0..1);
        let glyph_w: i32 = glyph.iter().map(|r| r.size.w).sum();
        // The caret straddling that same glyph.
        let caret = cache.caret_rect(1).expect("a caret after the first glyph");
        assert!(
            (caret.size.w - glyph_w).abs() <= 1,
            "caret width {} should match the placed glyph width {glyph_w} at scale {scale}",
            caret.size.w
        );
    }

    /// Scaling must not collapse the caret to nothing on a heavily downscaled
    /// page — a `.max(1.0)` floor, not a bare multiply.
    #[test]
    fn a_caret_stays_visible_under_a_harsh_downscale() {
        let mut cache = DocLayoutCache::new();
        cache.set_viewport(Size::from((80, 80)));
        cache
            .rebuild("hi", &mathed_core::TransformOptions::default())
            .expect("layout");
        let caret = cache.caret_rect(1).expect("a caret");
        assert!(
            caret.size.w >= 1 && caret.size.h >= 1,
            "the caret must not round away to nothing: {caret:?}"
        );
    }
}
