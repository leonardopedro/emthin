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
//! *View zoom* multiplies the fit-to-window letterbox; panning moves the zoomed
//! page inside the window. Both are view state only — they never reflow the
//! document. *Reading size* is a separate control: `text_size_pt` makes Typst
//! reflow the document at a larger or smaller font size, which can change the
//! page count, while the letterbox keeps each resulting page fitted to the
//! window.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use mathed_core::figures::FigureRect;
use mathed_mini::{layout_doc_paged_with_text_size, PageLayout, RenderError, DEFAULT_TEXT_SIZE_PT};
use smithay::utils::{Logical, Point, Rectangle, Size};

/// Where one figure landed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedFigure {
    /// Index of the page the figure is on.
    pub page: usize,
    /// The figure's rect in output-local logical px.
    pub rect: Rectangle<i32, Logical>,
}

/// Which document incarnation and source generation a layout belongs to.
/// Accepted hit-testing data is usable only while this matches the live model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LayoutOwner {
    pub document: u64,
    pub content: u64,
}

/// One layout request. The worker owns the text and transform options for the
/// duration of the job, so the UI can keep editing.
struct LayoutRequest {
    generation: u64,
    owner: LayoutOwner,
    text: String,
    options: mathed_core::TransformOptions,
    text_size: f32,
    /// Page whose raster the caller needs first.
    page: usize,
}

struct PageRequest {
    owner: LayoutOwner,
    page: usize,
}

struct LayoutResult {
    generation: u64,
    owner: LayoutOwner,
    pages: Vec<PageLayout>,
    materialized_page: usize,
    compile_time: Duration,
    glyph_time: Duration,
    raster_time: Duration,
    error: Option<String>,
}

enum LayoutEvent {
    Layout(LayoutResult),
    Page {
        owner: LayoutOwner,
        page: usize,
        result: Result<PageLayout, String>,
    },
}

/// The timing of an applied layout. `compile_time` is Typst compilation,
/// `glyph_time` is source-map/glyph extraction for every page, and
/// `raster_time` is the one materialized page.
#[derive(Debug, Clone, Copy)]
pub struct AppliedLayout {
    pub generation: u64,
    pub compile_time: Duration,
    pub glyph_time: Duration,
    pub raster_time: Duration,
    pub apply_time: Duration,
}

/// One active layout, at most one owed replacement, and page materialization
/// requests served from the retained compiled generation.
struct LayoutWorker {
    requests: Sender<LayoutJob>,
    results: Receiver<LayoutEvent>,
    active: Option<u64>,
    pending: Option<LayoutRequest>,
    newest: u64,
    ready_pages: std::collections::VecDeque<(LayoutOwner, usize, PageLayout)>,
}

enum LayoutJob {
    Layout(Box<LayoutRequest>),
    Page(PageRequest),
}

impl LayoutWorker {
    fn new() -> Self {
        let (requests, request_rx) = mpsc::channel::<LayoutJob>();
        let (result_tx, results) = mpsc::channel::<LayoutEvent>();
        std::thread::Builder::new()
            .name("emthin-layout".into())
            .spawn(move || {
                let mut retained: Option<mathed_mini::RetainedPaged> = None;
                let mut current_owner: Option<LayoutOwner> = None;
                while let Ok(job) = request_rx.recv() {
                    match job {
                        LayoutJob::Layout(request) => {
                            let request = *request;
                            let started = Instant::now();
                            let compiled = mathed_mini::RetainedPaged::compile(
                                &request.text,
                                &request.options,
                                request.text_size,
                            );
                            let compile_time = started.elapsed();
                            let result = match compiled {
                                Ok(retained_doc) => {
                                    let glyph_started = Instant::now();
                                    let mut pages = retained_doc.skeleton();
                                    let glyph_time = glyph_started.elapsed();
                                    let page = request.page.min(pages.len().saturating_sub(1));
                                    let raster_started = Instant::now();
                                    let materialized = pages.get_mut(page).map(|slot| {
                                        match retained_doc.page(page, 1.0) {
                                            Ok(layout) => *slot = layout,
                                            Err(error) => {
                                                tracing::warn!(
                                                    ?error,
                                                    page,
                                                    "materializing the visible page failed"
                                                );
                                            }
                                        }
                                        page
                                    });
                                    let raster_time = raster_started.elapsed();
                                    retained = Some(retained_doc);
                                    current_owner = Some(request.owner);
                                    LayoutResult {
                                        generation: request.generation,
                                        owner: request.owner,
                                        pages,
                                        materialized_page: materialized.unwrap_or(0),
                                        compile_time,
                                        glyph_time,
                                        raster_time,
                                        error: None,
                                    }
                                }
                                Err(error) => LayoutResult {
                                    generation: request.generation,
                                    owner: request.owner,
                                    pages: Vec::new(),
                                    materialized_page: 0,
                                    compile_time,
                                    glyph_time: Duration::ZERO,
                                    raster_time: Duration::ZERO,
                                    error: Some(format!("{error:?}")),
                                },
                            };
                            if result_tx.send(LayoutEvent::Layout(result)).is_err() {
                                break;
                            }
                        }
                        LayoutJob::Page(request) => {
                            let result = match (&retained, current_owner) {
                                (Some(retained_doc), Some(owner)) if owner == request.owner => {
                                    retained_doc
                                        .page(request.page, 1.0)
                                        .map_err(|error| format!("{error:?}"))
                                }
                                _ => Err("render layout was replaced; request a new frame".into()),
                            };
                            if result_tx
                                .send(LayoutEvent::Page {
                                    owner: request.owner,
                                    page: request.page,
                                    result,
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
            })
            .expect("start layout worker");
        Self {
            requests,
            results,
            active: None,
            pending: None,
            newest: 0,
            ready_pages: std::collections::VecDeque::new(),
        }
    }

    fn request(
        &mut self,
        owner: LayoutOwner,
        text: String,
        options: mathed_core::TransformOptions,
        text_size: f32,
        page: usize,
    ) -> u64 {
        self.newest = self.newest.wrapping_add(1);
        let request = LayoutRequest {
            generation: self.newest,
            owner,
            text,
            options,
            text_size,
            page,
        };
        if self.active.is_none() {
            self.send(request);
        } else {
            self.pending = Some(request);
        }
        self.newest
    }

    fn send(&mut self, request: LayoutRequest) {
        self.active = Some(request.generation);
        if self
            .requests
            .send(LayoutJob::Layout(Box::new(request)))
            .is_err()
        {
            self.active = None;
        }
    }

    fn request_page(&mut self, owner: LayoutOwner, page: usize) {
        let _ = self
            .requests
            .send(LayoutJob::Page(PageRequest { owner, page }));
    }

    fn start_pending(&mut self) {
        if let Some(request) = self.pending.take() {
            self.send(request);
        }
    }

    #[cfg(not(test))]
    fn is_pending(&self) -> bool {
        self.active.is_some() || self.pending.is_some()
    }

    fn poll(&mut self) -> Option<LayoutResult> {
        loop {
            match self.results.try_recv() {
                Ok(LayoutEvent::Page {
                    owner,
                    page,
                    result,
                }) => match result {
                    Ok(layout) => self.ready_pages.push_back((owner, page, layout)),
                    Err(error) => tracing::warn!(?owner, page, "page materialization: {error}"),
                },
                Ok(LayoutEvent::Layout(result)) => {
                    if self.active == Some(result.generation) {
                        self.active = None;
                        self.start_pending();
                    }
                    if result.generation == self.newest {
                        return Some(result);
                    }
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    self.active = None;
                    return None;
                }
            }
        }
    }

    fn poll_page(&mut self) -> Option<(LayoutOwner, usize, PageLayout)> {
        self.ready_pages.pop_front()
    }

    #[cfg(test)]
    fn wait_for_newest(&mut self) -> Option<LayoutResult> {
        loop {
            let event = self.results.recv().ok()?;
            if let LayoutEvent::Page { .. } = event {
                continue;
            }
            let LayoutEvent::Layout(result) = event else {
                unreachable!()
            };
            if self.active == Some(result.generation) {
                self.active = None;
                self.start_pending();
            }
            if result.generation == self.newest {
                return Some(result);
            }
        }
    }
}

/// The rendered document: every page's raster + glyph index + figure
/// rects, plus the letterbox mapping for the visible page.
///
/// Not `Debug`: `mathed_mini::PageLayout` carries raster buffers, and
/// dumping a whole page's pixels into a log line helps nobody.
pub struct DocLayoutCache {
    pages: Vec<PageLayout>,
    /// The source owner currently being requested (set before a rebuild is
    /// scheduled). Hit-testing stays disabled while this differs from the
    /// accepted owner below.
    owner: Option<LayoutOwner>,
    /// The owner of the pages and glyph indexes currently held.
    accepted_owner: Option<LayoutOwner>,
    /// Changes after every successful raster rebuild, even without a text edit.
    generation: u64,
    /// Size of one page in logical px (from the first page's raster).
    page_size: Size<i32, Logical>,
    /// Output size the letterbox was computed for.
    viewport: Size<i32, Logical>,
    /// Index of the visible page.
    current_page: usize,
    /// Reading size in points. Typst reflows the document at this size;
    /// this is not the fit-to-window letterbox factor.
    text_size_pt: f32,
    /// Multiplier on the fit-to-window scale. `1.0` is the plain letterbox.
    view_zoom: f64,
    /// Output-logical offset from the centred view. Only meaningful when the
    /// zoomed page is larger than the viewport on that axis.
    view_pan: Point<f64, Logical>,
    /// Number of layout/raster attempts. Tests use this to distinguish a
    /// caret repaint from a real recompile.
    rebuild_count: u64,
    /// Pages whose full raster is resident. At most one on the live worker.
    materialized: Vec<bool>,
    worker: LayoutWorker,
}

impl Default for DocLayoutCache {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            owner: None,
            accepted_owner: None,
            generation: 0,
            page_size: Size::default(),
            viewport: Size::default(),
            current_page: 0,
            text_size_pt: DEFAULT_TEXT_SIZE_PT,
            view_zoom: 1.0,
            view_pan: Point::from((0.0, 0.0)),
            rebuild_count: 0,
            materialized: Vec::new(),
            worker: LayoutWorker::new(),
        }
    }
}

impl DocLayoutCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark which document/source generation the next layout belongs to.
    /// Geometry from an older accepted owner is not valid hit-test data.
    pub(crate) fn set_owner(&mut self, owner: LayoutOwner) {
        self.owner = Some(owner);
    }

    /// Whether the retained glyphs/figures belong to `owner`.
    pub(crate) fn accepts(&self, owner: LayoutOwner) -> bool {
        self.accepted_owner == Some(owner) && self.owner == Some(owner)
    }

    fn geometry_current(&self) -> bool {
        match (self.owner, self.accepted_owner) {
            (None, _) => true,
            (Some(owner), Some(accepted)) => owner == accepted,
            (Some(_), None) => false,
        }
    }

    /// Re-lay out the document. Cheap no-op when nothing changed.
    pub fn rebuild(
        &mut self,
        doc_text: &str,
        opts: &mathed_core::TransformOptions,
    ) -> Result<(), RenderError> {
        self.rebuild_count = self.rebuild_count.wrapping_add(1);
        let started = Instant::now();
        let result = layout_doc_paged_with_text_size(doc_text, opts, self.text_size_pt);
        let elapsed_us = started.elapsed().as_micros() as u64;
        match &result {
            Ok(pages) => tracing::debug!(
                target: "emthin::timing",
                stage = "layout_raster",
                elapsed_us,
                pages = pages.len(),
                text_bytes = doc_text.len(),
                "document layout and raster"
            ),
            Err(error) => tracing::debug!(
                target: "emthin::timing",
                stage = "layout_raster",
                elapsed_us,
                text_bytes = doc_text.len(),
                error = ?error,
                "document layout and raster failed"
            ),
        }
        self.install_pages(result?, self.owner);
        Ok(())
    }

    /// Schedule a replacement layout. In tests the cache stays synchronous so
    /// existing geometry assertions remain deterministic; production uses the
    /// one-active/one-pending worker.
    pub(crate) fn request_rebuild(
        &mut self,
        doc_text: &str,
        opts: &mathed_core::TransformOptions,
        owner: LayoutOwner,
    ) -> Option<Result<AppliedLayout, String>> {
        self.set_owner(owner);
        #[cfg(test)]
        {
            Some(match self.rebuild(doc_text, opts) {
                Ok(()) => Ok(AppliedLayout {
                    generation: self.generation,
                    compile_time: Duration::ZERO,
                    glyph_time: Duration::ZERO,
                    raster_time: Duration::ZERO,
                    apply_time: Duration::ZERO,
                }),
                Err(error) => Err(format!("{error:?}")),
            })
        }
        #[cfg(not(test))]
        {
            self.rebuild_count = self.rebuild_count.wrapping_add(1);
            self.worker.request(
                owner,
                doc_text.to_owned(),
                opts.clone(),
                self.text_size_pt,
                self.current_page,
            );
            None
        }
    }

    /// Whether a requested layout has not been applied yet. Tests use the
    /// synchronous path, where a request is always applied immediately.
    pub fn is_pending(&self) -> bool {
        #[cfg(test)]
        {
            self.owner != self.accepted_owner && self.owner.is_some()
        }
        #[cfg(not(test))]
        {
            self.worker.is_pending()
        }
    }

    /// Apply the newest completed worker result. Stale generations are
    /// discarded; a failed newest result keeps the old pages.
    pub fn poll_rebuild(&mut self) -> Option<Result<AppliedLayout, String>> {
        let mut result = self.worker.poll()?;
        if Some(result.owner) != self.owner {
            tracing::debug!(
                owner = ?result.owner,
                current = ?self.owner,
                "discarding a layout from a superseded document/source owner"
            );
            return None;
        }
        if let Some(error) = result.error.take() {
            return Some(Err(error));
        }
        let apply_started = Instant::now();
        let materialized_page = result.materialized_page;
        self.install_pages(result.pages, Some(result.owner));
        self.materialized = vec![false; self.pages.len()];
        if let Some(slot) = self.materialized.get_mut(materialized_page) {
            *slot = true;
        }
        Some(Ok(AppliedLayout {
            generation: result.generation,
            compile_time: result.compile_time,
            glyph_time: result.glyph_time,
            raster_time: result.raster_time,
            apply_time: apply_started.elapsed(),
        }))
    }

    /// Ask the retained worker for the visible page's raster. A page turn
    /// inside one compiled generation never asks for a compile.
    pub fn ensure_page(&mut self) {
        let page = self.current_page;
        let Some(false) = self.materialized.get(page) else {
            return;
        };
        if let Some(owner) = self.accepted_owner {
            self.worker.request_page(owner, page);
        }
    }

    /// Apply a page raster from the retained generation, if one arrived.
    /// Only the visible page keeps pixels resident.
    pub fn poll_page(&mut self) -> bool {
        let Some((owner, page, layout)) = self.worker.poll_page() else {
            return false;
        };
        if Some(owner) != self.accepted_owner || page >= self.pages.len() {
            return false;
        }
        self.pages[page] = layout;
        self.materialized[page] = true;
        let stale: Vec<usize> = self
            .materialized
            .iter()
            .enumerate()
            .filter_map(|(index, resident)| (*resident && index != page).then_some(index))
            .collect();
        for index in stale {
            self.pages[index].release_raster();
            self.materialized[index] = false;
        }
        true
    }

    fn install_pages(&mut self, pages: Vec<PageLayout>, owner: Option<LayoutOwner>) {
        self.materialized = vec![true; pages.len()];
        self.pages = pages;
        self.accepted_owner = owner;
        self.generation = self.generation.wrapping_add(1);
        self.page_size = self
            .pages
            .first()
            .map(|p| Size::from((p.width as i32, p.height as i32)))
            .unwrap_or_default();
        // A shorter document can leave the viewer past the end.
        if self.current_page >= self.page_count() {
            self.current_page = self.page_count().saturating_sub(1);
        }
        self.clamp_view_pan();
    }

    /// Layout/raster attempts since this cache was created.
    pub fn rebuild_count(&self) -> u64 {
        self.rebuild_count
    }

    /// The reading size, in points, the next layout will use.
    pub fn text_size(&self) -> f32 {
        self.text_size_pt
    }

    /// Set the reading size. Returns whether it changed; the caller must
    /// relayout when it did. Values outside `MIN..=MAX` are clamped.
    pub fn set_text_size(&mut self, pt: f32) -> bool {
        let pt = pt.clamp(MIN_TEXT_SIZE_PT, MAX_TEXT_SIZE_PT);
        if (pt - self.text_size_pt).abs() < 0.01 {
            return false;
        }
        self.text_size_pt = pt;
        true
    }

    /// The next preset reading size in `larger`/`smaller` direction.
    ///
    /// Steps are discrete so repeated presses are predictable. A size
    /// between two steps moves to the adjacent one, and the ends stay at
    /// the end rather than wrapping.
    pub fn next_text_size(&self, larger: bool) -> f32 {
        let current = self.text_size_pt;
        if larger {
            TEXT_SIZE_STEPS
                .iter()
                .copied()
                .find(|&pt| pt > current + 0.01)
                .unwrap_or(MAX_TEXT_SIZE_PT)
        } else {
            TEXT_SIZE_STEPS
                .iter()
                .rev()
                .copied()
                .find(|&pt| pt < current - 0.01)
                .unwrap_or(MIN_TEXT_SIZE_PT)
        }
    }

    /// Current view zoom, as a multiplier on the fit-to-window scale.
    pub fn view_zoom(&self) -> f64 {
        self.view_zoom
    }

    /// Current pan offset in output-logical pixels.
    pub fn view_pan(&self) -> Point<f64, Logical> {
        self.view_pan
    }

    /// Step to the next preset view zoom in `larger`/`smaller` direction.
    pub fn next_view_zoom(&self, larger: bool) -> f64 {
        if larger {
            VIEW_ZOOM_STEPS
                .iter()
                .copied()
                .find(|&zoom| zoom > self.view_zoom + 0.001)
                .unwrap_or(MAX_VIEW_ZOOM)
        } else {
            VIEW_ZOOM_STEPS
                .iter()
                .rev()
                .copied()
                .find(|&zoom| zoom < self.view_zoom - 0.001)
                .unwrap_or(MIN_VIEW_ZOOM)
        }
    }

    /// Set the view zoom around the viewport centre. Returns whether it
    /// changed; the caller re-syncs placed geometry when it did.
    pub fn zoom_view(&mut self, larger: bool) -> bool {
        let target = self.next_view_zoom(larger);
        if (target - self.view_zoom).abs() < 0.001 {
            return false;
        }
        // Scale an existing pan by the same factor instead of deriving it from
        // the rounded old rect: keyboard zoom around the centre must not drift
        // the page by a pixel or two on odd-sized pages.
        let factor = target / self.view_zoom;
        self.view_zoom = target;
        self.view_pan.x *= factor;
        self.view_pan.y *= factor;
        self.clamp_view_pan();
        true
    }

    /// Set the view zoom around `anchor` in output-logical px, keeping the
    /// document point under the anchor stationary when possible.
    pub fn zoom_view_at(&mut self, factor: f64, anchor: Point<f64, Logical>) -> bool {
        if !factor.is_finite() || factor <= 0.0 {
            return false;
        }
        let target = (self.view_zoom * factor).clamp(MIN_VIEW_ZOOM, MAX_VIEW_ZOOM);
        if (target - self.view_zoom).abs() < 0.001 {
            return false;
        }

        let old = self.page_rect();
        let center: Point<f64, Logical> = Point::new(
            f64::from(self.viewport.w) / 2.0,
            f64::from(self.viewport.h) / 2.0,
        );
        let anchor = if old.size.w > 0 && old.size.h > 0 {
            Point::new(
                anchor
                    .x
                    .max(f64::from(old.loc.x))
                    .min(f64::from(old.loc.x + old.size.w)),
                anchor
                    .y
                    .max(f64::from(old.loc.y))
                    .min(f64::from(old.loc.y + old.size.h)),
            )
        } else {
            center
        };
        let fraction = if old.size.w > 0 && old.size.h > 0 {
            (
                (anchor.x - f64::from(old.loc.x)) / f64::from(old.size.w),
                (anchor.y - f64::from(old.loc.y)) / f64::from(old.size.h),
            )
        } else {
            (0.5, 0.5)
        };

        self.view_zoom = target;
        let centered = view_page_rect(
            self.page_size,
            self.viewport,
            self.view_zoom,
            Point::from((0.0, 0.0)),
        );
        self.view_pan = Point::new(
            anchor.x - fraction.0 * f64::from(centered.size.w) - f64::from(centered.loc.x),
            anchor.y - fraction.1 * f64::from(centered.size.h) - f64::from(centered.loc.y),
        );
        self.clamp_view_pan();
        true
    }

    /// Move the zoomed page by an output-logical delta. Returns whether the
    /// page actually moved (a page that fits its viewport cannot pan).
    pub fn pan_view(&mut self, dx: f64, dy: f64) -> bool {
        if !dx.is_finite() || !dy.is_finite() {
            return false;
        }
        let before = self.view_pan;
        self.view_pan.x += dx;
        self.view_pan.y += dy;
        self.clamp_view_pan();
        (self.view_pan.x - before.x).abs() > 0.01 || (self.view_pan.y - before.y).abs() > 0.01
    }

    /// Restore a saved zoom/pan pair. Invalid values are clamped.
    pub fn set_view(&mut self, zoom: f64, pan: Point<f64, Logical>) -> bool {
        if !zoom.is_finite() || !pan.x.is_finite() || !pan.y.is_finite() {
            return false;
        }
        let zoom = zoom.clamp(MIN_VIEW_ZOOM, MAX_VIEW_ZOOM);
        let before = (self.view_zoom, self.view_pan);
        self.view_zoom = zoom;
        self.view_pan = pan;
        self.clamp_view_pan();
        (before.0 - self.view_zoom).abs() > 0.001
            || (before.1.x - self.view_pan.x).abs() > 0.01
            || (before.1.y - self.view_pan.y).abs() > 0.01
    }

    /// Return to the fit-to-window view.
    pub fn reset_view(&mut self) -> bool {
        if (self.view_zoom - 1.0).abs() < 0.001
            && self.view_pan.x.abs() < 0.01
            && self.view_pan.y.abs() < 0.01
        {
            return false;
        }
        self.view_zoom = 1.0;
        self.view_pan = Point::from((0.0, 0.0));
        true
    }

    fn clamp_view_pan(&mut self) {
        if self.page_size.w <= 0 || self.page_size.h <= 0 {
            self.view_pan = Point::from((0.0, 0.0));
            return;
        }
        let placed = view_page_rect(
            self.page_size,
            self.viewport,
            self.view_zoom,
            Point::from((0.0, 0.0)),
        );
        self.view_pan.x = clamp_axis(
            self.view_pan.x,
            f64::from(placed.size.w),
            f64::from(self.viewport.w),
        );
        self.view_pan.y = clamp_axis(
            self.view_pan.y,
            f64::from(placed.size.h),
            f64::from(self.viewport.h),
        );
    }

    /// Revision of the rendered pixels, independent of the Loro revision.
    pub fn generation(&self) -> u64 {
        self.generation
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
        let page = page.min(self.page_count().saturating_sub(1));
        if page != self.current_page {
            // A page switch starts at the top, not wherever the previous page
            // happened to be panned. The zoom is a reading preference and stays.
            self.view_pan = Point::from((0.0, 0.0));
        }
        self.current_page = page;
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
        if !self.geometry_current() {
            return None;
        }
        self.page().map(|p| &p.glyphs)
    }

    /// The page holding a rendered glyph in `range`, if any.
    ///
    /// A find match can start in hidden marker text or other source the
    /// renderer does not place; that range has no page, and the caller should
    /// leave the view where it is. A rendered match moves the visible page so
    /// F3 works even when the next hit is on another page.
    pub fn page_for_range(&self, range: std::ops::Range<usize>) -> Option<usize> {
        if !self.geometry_current() {
            return None;
        }
        self.pages.iter().position(|page| {
            page.glyphs
                .entries
                .iter()
                .any(|entry| range.contains(&entry.doc_byte))
        })
    }

    /// The first rendered glyph intersecting the actual viewport, and its
    /// vertical displacement in page points from the viewport top. `None`
    /// when the page is not visible or its geometry is stale.
    pub fn reading_spot(&self) -> Option<(usize, f64)> {
        if !self.geometry_current() {
            return None;
        }
        let page = self.page()?;
        let rect = self.page_rect();
        if rect.size.w <= 0 || rect.size.h <= 0 {
            return None;
        }
        let scale = page_scale(self.page_size, rect.size);
        let doc_top = ((0.0 - f64::from(rect.loc.y)) / scale).max(0.0);
        let doc_bottom = ((f64::from(self.viewport.h) - f64::from(rect.loc.y)) / scale)
            .min(f64::from(self.page_size.h));
        if doc_bottom <= doc_top {
            return None;
        }
        let entry = page
            .glyphs
            .entries
            .iter()
            .filter(|entry| {
                page.glyphs
                    .bands
                    .get(entry.band as usize)
                    .is_some_and(|band| {
                        f64::from(band.bottom) > doc_top && f64::from(band.top) < doc_bottom
                    })
            })
            .min_by(|a, b| {
                let a_band = &page.glyphs.bands[a.band as usize];
                let b_band = &page.glyphs.bands[b.band as usize];
                a_band
                    .top
                    .partial_cmp(&b_band.top)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
            })?;
        let band = &page.glyphs.bands[entry.band as usize];
        Some((entry.doc_byte, f64::from(band.top) - doc_top))
    }

    /// Place `doc_byte` at `offset` page points below the viewport top,
    /// choosing the page that holds it and clamping the resulting pan. Returns
    /// whether the visible page or transform changed.
    pub(crate) fn place_reading_offset(&mut self, doc_byte: usize, offset: f64) -> bool {
        if !offset.is_finite() || !self.geometry_current() {
            return false;
        }
        let Some(page_idx) = self.pages.iter().position(|page| {
            page.glyphs
                .entries
                .iter()
                .any(|entry| entry.doc_byte >= doc_byte)
        }) else {
            return false;
        };
        let Some(entry) = self.pages[page_idx]
            .glyphs
            .entries
            .iter()
            .find(|entry| entry.doc_byte >= doc_byte)
        else {
            return false;
        };
        let Some(band) = self.pages[page_idx].glyphs.bands.get(entry.band as usize) else {
            return false;
        };
        let band_top = f64::from(band.top);

        let before = (self.current_page, self.view_pan);
        if page_idx != self.current_page {
            self.set_current_page(page_idx);
        }
        let rect = self.page_rect();
        let scale = page_scale(self.page_size, rect.size);
        let centered = view_page_rect(
            self.page_size,
            self.viewport,
            self.view_zoom,
            Point::from((0.0, 0.0)),
        );
        // `offset = band.top - doc_top`, so the line's page position at the
        // viewport top is `band.top - offset`; place that at screen y 0.
        self.view_pan.y = offset * scale - band_top * scale - f64::from(centered.loc.y);
        self.clamp_view_pan();
        self.current_page != before.0
            || (self.view_pan.x - before.1.x).abs() > 0.01
            || (self.view_pan.y - before.1.y).abs() > 0.01
    }

    /// The size of one page, in logical px.
    pub fn page_size(&self) -> Size<i32, Logical> {
        self.page_size
    }

    /// The output size the letterbox was last computed for.
    pub fn viewport(&self) -> Size<i32, Logical> {
        self.viewport
    }

    /// The visible page's rect in output-local logical px: the page scaled to
    /// fit, multiplied by the view zoom, and shifted by the pan.
    pub fn page_rect(&self) -> Rectangle<i32, Logical> {
        view_page_rect(self.page_size, self.viewport, self.view_zoom, self.view_pan)
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
        if !self.geometry_current() {
            return None;
        }
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
        self.clamp_view_pan();
    }

    /// Display name for a page, used by the ext-workspace bar.
    pub fn page_name(&self, page: usize) -> String {
        format!("Page {}", page + 1)
    }
}

/// Reading sizes the keyboard steps through. The default is included,
/// so `Ctrl+0` lands exactly on it rather than on a neighbouring step.
pub const TEXT_SIZE_STEPS: &[f32] = &[10.0, 12.0, 14.0, 17.0, 20.0, 24.0, 28.0, 32.0];

/// Bounds on a caller-supplied size. A size outside this range is a
/// mistake, not a request to render a blank or enormous page.
pub const MIN_TEXT_SIZE_PT: f32 = 8.0;
pub const MAX_TEXT_SIZE_PT: f32 = 32.0;

/// View zoom multipliers on the fit-to-window scale. `1.0` is the plain
/// letterbox; `0.5` shows the whole page smaller, `4.0` is close reading.
pub const VIEW_ZOOM_STEPS: &[f64] = &[0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0, 4.0];

pub const MIN_VIEW_ZOOM: f64 = 0.5;
pub const MAX_VIEW_ZOOM: f64 = 4.0;

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

/// The placed page for a given zoom and pan. The fit scale is the plain
/// letterbox (never upscaling past 1:1); `zoom` multiplies it, and `pan` is an
/// output-logical offset from the centred position.
fn view_page_rect(
    content: Size<i32, Logical>,
    viewport: Size<i32, Logical>,
    zoom: f64,
    pan: Point<f64, Logical>,
) -> Rectangle<i32, Logical> {
    if content.w <= 0 || content.h <= 0 || viewport.w <= 0 || viewport.h <= 0 {
        return Rectangle::new((0, 0).into(), content);
    }
    let fit = (viewport.w as f64 / content.w as f64)
        .min(viewport.h as f64 / content.h as f64)
        .min(1.0);
    let scale = fit * zoom.clamp(MIN_VIEW_ZOOM, MAX_VIEW_ZOOM);
    let w = (content.w as f64 * scale).round().max(1.0) as i32;
    let h = (content.h as f64 * scale).round().max(1.0) as i32;
    Rectangle::new(
        Point::from((
            ((f64::from(viewport.w) - f64::from(w)) / 2.0 + pan.x).round() as i32,
            ((f64::from(viewport.h) - f64::from(h)) / 2.0 + pan.y).round() as i32,
        )),
        Size::from((w, h)),
    )
}

/// Clamp one pan axis so the page never leaves a blank band where the viewport
/// could still show page. A page smaller than the viewport stays centred.
fn clamp_axis(pan: f64, page: f64, viewport: f64) -> f64 {
    if page <= viewport {
        return 0.0;
    }
    let limit = (page - viewport) / 2.0;
    pan.clamp(-limit, limit)
}

#[cfg(test)]
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
    fn text_size_steps_discretely_and_clamps() {
        let mut cache = DocLayoutCache::new();
        assert_eq!(cache.text_size(), DEFAULT_TEXT_SIZE_PT);
        assert_eq!(cache.next_text_size(true), 20.0);
        assert_eq!(cache.next_text_size(false), 14.0);

        assert!(cache.set_text_size(24.0));
        assert_eq!(cache.text_size(), 24.0);
        assert!(!cache.set_text_size(24.0), "an unchanged size is a no-op");

        cache.set_text_size(1.0);
        assert_eq!(cache.text_size(), MIN_TEXT_SIZE_PT);
        cache.set_text_size(999.0);
        assert_eq!(cache.text_size(), MAX_TEXT_SIZE_PT);
        assert_eq!(cache.next_text_size(true), MAX_TEXT_SIZE_PT);
        assert_eq!(cache.next_text_size(false), 28.0);
    }

    #[test]
    fn the_worker_keeps_one_pending_and_discards_older_work() {
        let mut worker = LayoutWorker::new();
        let options = mathed_core::TransformOptions::default();
        let owner = LayoutOwner {
            document: 1,
            content: 1,
        };
        let first = worker.request(
            owner,
            "first".into(),
            options.clone(),
            DEFAULT_TEXT_SIZE_PT,
            0,
        );
        let second = worker.request(
            owner,
            "second".into(),
            options.clone(),
            DEFAULT_TEXT_SIZE_PT,
            0,
        );
        let third = worker.request(owner, "third".into(), options, DEFAULT_TEXT_SIZE_PT, 0);
        assert!(first < second && second < third);

        let result = worker.wait_for_newest().expect("worker result");
        assert_eq!(result.generation, third, "the newest request wins");
        assert!(result.error.is_none(), "a simple text layout must succeed");
        assert!(!result.pages.is_empty());
    }

    #[test]
    fn the_worker_rasterizes_only_requested_pages() {
        let mut worker = LayoutWorker::new();
        let options = mathed_core::TransformOptions::default();
        let owner = LayoutOwner {
            document: 2,
            content: 2,
        };
        let long = "lorem ipsum dolor sit amet consectetur adipiscing elit\n".repeat(220);
        let generation = worker.request(owner, long, options, DEFAULT_TEXT_SIZE_PT, 0);
        let result = worker.wait_for_newest().expect("worker result");
        assert_eq!(result.generation, generation);
        assert!(result.pages.len() > 1, "fixture must paginate");
        assert!(result.pages[0].image.width > 100, "visible page has pixels");
        assert_eq!(
            result.pages[1].image.width, 1,
            "offscreen page keeps only metadata"
        );

        worker.request_page(owner, 1);
        let (page_owner, page, layout) = loop {
            let _ = worker.poll();
            if let Some(page) = worker.poll_page() {
                break page;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(page_owner, owner);
        assert_eq!(page, 1);
        assert!(layout.image.width > 100, "later page materializes");
        assert_eq!(
            worker.newest, generation,
            "a page turn inside the retained generation does not request a compile"
        );
    }

    #[test]
    fn a_larger_reading_size_reflows_the_document() {
        let mut cache = DocLayoutCache::new();
        cache.set_viewport(Size::from((1200, 900)));
        let doc = "reading size changes how many words fit on a page. ".repeat(300);
        let opts = mathed_core::TransformOptions::default();
        cache.set_text_size(12.0);
        cache.rebuild(&doc, &opts).expect("12pt layout");
        let small_pages = cache.page_count();
        cache.set_text_size(28.0);
        cache.rebuild(&doc, &opts).expect("28pt layout");
        assert!(
            cache.page_count() > small_pages,
            "28pt must paginate longer than 12pt: {} vs {small_pages}",
            cache.page_count()
        );
    }

    #[test]
    fn stale_owners_keep_pixels_but_withhold_hit_geometry() {
        let mut cache = DocLayoutCache::new();
        cache.set_viewport(Size::from((1200, 900)));
        let opts = mathed_core::TransformOptions::default();
        let first = LayoutOwner {
            document: 1,
            content: 1,
        };
        cache.set_owner(first);
        cache.rebuild("first document", &opts).expect("layout");
        assert!(cache.accepts(first));
        let page = cache.page().expect("retained pixels").image.data.clone();
        let point = cache.doc_to_screen(Point::from((20.0, 20.0)));
        assert!(cache.screen_to_doc(point.to_f64()).is_some());

        let newer = LayoutOwner {
            document: 1,
            content: 2,
        };
        cache.set_owner(newer);
        assert!(!cache.accepts(newer), "the old layout is not accepted yet");
        assert_eq!(
            cache.page().expect("old pixels stay visible").image.data,
            page
        );
        assert!(
            cache.screen_to_doc(point.to_f64()).is_none(),
            "stale hit geometry must be withheld while the replacement runs"
        );
        assert!(cache.glyphs().is_none());

        cache
            .rebuild("second document", &opts)
            .expect("replacement");
        assert!(
            cache.accepts(newer),
            "the accepted replacement restores hits"
        );
        assert!(cache.glyphs().is_some());
    }

    #[test]
    fn successful_rebuilds_invalidate_rendered_page_even_without_loro_edits() {
        let mut cache = DocLayoutCache::new();
        let opts = mathed_core::TransformOptions::default();
        cache.rebuild("First document", &opts).unwrap();
        let generation = cache.generation();
        let first = cache.page().unwrap().image.data.clone();
        cache.rebuild("A different document", &opts).unwrap();
        assert_ne!(cache.generation(), generation);
        assert_ne!(cache.page().unwrap().image.data, first);
    }

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

    /// A find match on a later page names that page, so F3 can bring it into
    /// view; a range that no page rendered has no page at all.
    #[test]
    fn a_rendered_range_reports_its_page_and_a_hidden_range_reports_none() {
        let mut cache = DocLayoutCache::new();
        cache.set_viewport(Size::from((1200, 900)));
        cache
            .rebuild(
                &"searchable words on every page ".repeat(400),
                &mathed_core::TransformOptions::default(),
            )
            .expect("layout");
        assert!(cache.page_count() > 1, "the fixture must paginate");
        let entry = cache
            .page_at(1)
            .expect("page 1")
            .glyphs
            .entries
            .first()
            .expect("rendered glyphs on page 1");
        assert_eq!(
            cache.page_for_range(entry.doc_byte..entry.doc_byte + 1),
            Some(1)
        );
        assert_eq!(cache.page_for_range(1_000_000..1_000_001), None);
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

    /// Free 2D placement stays in the document: an `dx:`/`dy:` offset moves the
    /// figure's placed rect without any compositor-side position state.
    #[test]
    fn a_doc_offset_moves_the_placed_figure_rect() {
        let rect_of = |doc: &str| {
            let mut cache = DocLayoutCache::new();
            cache.set_viewport(Size::from((1280, 800)));
            cache
                .rebuild(doc, &mathed_core::TransformOptions::default())
                .expect("layout");
            cache.place_figure("f0").expect("a figure").rect
        };
        let flow = rect_of("#1 app #2 \\app(#1, #2, 240, 160, \"a\")");
        let shifted = rect_of("#1 app #2 \\app(#1, #2, 240, 160, \"a\", dx: 30, dy: 20)");
        assert_eq!(shifted.size, flow.size);
        // The page may be downscaled to fit the viewport; the document offset
        // is scaled with it, but must still be non-zero and in the right
        // direction.
        assert!(shifted.loc.x > flow.loc.x, "dx moved the figure right");
        assert!(shifted.loc.y > flow.loc.y, "dy moved the figure down");
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

    #[test]
    fn inline_figure_reflows_and_wraps_with_surrounding_prose() {
        let rect_of = |doc: &str| {
            let mut cache = DocLayoutCache::new();
            cache.set_viewport(Size::from((260, 600)));
            cache
                .rebuild(doc, &mathed_core::TransformOptions::default())
                .expect("layout");
            cache.place_figure("f0").expect("inline figure").rect
        };
        let short = rect_of("Hi #1#2 \\app(#1, #2, 60, 24, \"pill\", inline: true) end");
        let pushed_right =
            rect_of("Hi there my #1#2 \\app(#1, #2, 60, 24, \"pill\", inline: true) end");
        let wrapped = rect_of(
            "Hi there my friend this paragraph is long enough to wrap #1#2 \\app(#1, #2, 60, 24, \"pill\", inline: true) end",
        );
        assert!(
            pushed_right.loc.x > short.loc.x,
            "inline figure slides right with preceding words"
        );
        assert_eq!(pushed_right.loc.y, short.loc.y);
        assert!(
            wrapped.loc.y > pushed_right.loc.y,
            "inline figure wraps onto next line automatically"
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

    /// View zoom multiplies the fit-to-window letterbox around the centre; it
    /// does not reflow the document, so the page size stays put.
    #[test]
    fn view_zoom_scales_around_the_centre() {
        let mut cache = DocLayoutCache::new();
        cache.page_size = Size::from((600, 800));
        cache.set_viewport(Size::from((1200, 900)));
        let base = cache.page_rect();
        assert_eq!(base, Rectangle::new((300, 50).into(), (600, 800).into()));

        assert!(cache.zoom_view(true));
        assert_eq!(cache.view_zoom(), 1.25);
        let zoomed = cache.page_rect();
        assert_eq!(zoomed.size, Size::from((750, 1000)));
        assert_eq!(zoomed.loc, Point::from((225, -50)));
        assert_eq!(cache.page_size, Size::from((600, 800)));

        assert!(cache.reset_view());
        assert_eq!(cache.page_rect(), base);
        assert_eq!(cache.view_pan(), Point::from((0.0, 0.0)));
    }

    /// A zoomed page pans only along axes that overflow the viewport, and the
    /// clamp never lets the page leave a blank band.
    #[test]
    fn panning_a_zoomed_page_is_clamped_to_its_overflow() {
        let mut cache = DocLayoutCache::new();
        cache.page_size = Size::from((600, 800));
        cache.set_viewport(Size::from((1200, 900)));
        assert!(!cache.pan_view(50.0, 50.0), "the fitted page cannot pan");

        assert!(cache.zoom_view(true));
        assert!(cache.pan_view(0.0, 1_000.0), "the page moves to its clamp");
        assert!(!cache.pan_view(0.0, 1.0), "it cannot pan past the clamp");
        assert_eq!(cache.view_pan().y, 50.0);
        assert_eq!(
            cache.page_rect().loc.y,
            0,
            "the page still covers the viewport"
        );
        assert_eq!(cache.view_pan().x, 0.0, "no horizontal overflow at 1.25x");
    }

    /// Zooming at a pointer anchor keeps the document point under that pointer
    /// stationary, which is what makes wheel/cursor zoom feel attached to the
    /// page rather than to the window centre.
    #[test]
    fn zooming_at_an_anchor_keeps_the_doc_point_under_it() {
        let mut cache = DocLayoutCache::new();
        cache.page_size = Size::from((600, 800));
        // Both axes overflow after zooming: width 600 -> 900 > 500.
        cache.set_viewport(Size::from((500, 900)));
        let doc = Point::new(200.0, 300.0);
        let anchor = cache.doc_to_screen(doc).to_f64();

        assert!(cache.zoom_view_at(1.5, anchor));
        let back = cache
            .screen_to_doc(Point::new(anchor.x, anchor.y))
            .expect("the anchor is still on the page");
        assert!(
            (back.x - doc.x).abs() < 1.5 && (back.y - doc.y).abs() < 1.5,
            "doc point moved from {doc:?} to {back:?} under anchor {anchor:?}"
        );
    }

    /// Keyboard zoom is centred: an un-panned page must stay exactly centred,
    /// even when the page/viewport dimensions are odd and round to a pixel.
    #[test]
    fn centre_zoom_does_not_drift_the_pan() {
        let mut cache = DocLayoutCache::new();
        cache.page_size = Size::from((795, 1123));
        cache.set_viewport(Size::from((1199, 901)));
        assert_eq!(cache.view_pan(), Point::from((0.0, 0.0)));
        assert!(cache.zoom_view(true));
        assert_eq!(cache.view_pan(), Point::from((0.0, 0.0)));
        assert!(cache.zoom_view(true));
        assert_eq!(cache.view_pan(), Point::from((0.0, 0.0)));
    }

    /// A view smaller than the fit shows the whole page, centred, and stays
    /// put: there is nothing to pan to.
    #[test]
    fn zooming_below_the_fit_centres_and_cannot_pan() {
        let mut cache = DocLayoutCache::new();
        cache.page_size = Size::from((600, 800));
        cache.set_viewport(Size::from((1200, 900)));
        assert!(cache.zoom_view(false));
        assert_eq!(cache.view_zoom(), 0.75);
        assert_eq!(cache.page_rect().size, Size::from((450, 600)));
        assert_eq!(cache.page_rect().loc, Point::from((375, 150)));
        assert!(!cache.pan_view(100.0, 100.0));
    }

    /// The zoom/pan mapping must round-trip through `screen_to_doc` like the
    /// plain letterbox does.
    #[test]
    fn zoomed_and_panned_doc_screen_round_trip() {
        let mut cache = DocLayoutCache::new();
        cache.page_size = Size::from((600, 800));
        cache.set_viewport(Size::from((400, 400)));
        assert!(cache.zoom_view(true));
        assert!(cache.zoom_view(true));
        assert!(cache.pan_view(30.0, -40.0));
        for doc in [Point::new(50.0, 60.0), Point::new(300.0, 400.0)] {
            let screen = cache.doc_to_screen(doc);
            let back = cache
                .screen_to_doc(screen.to_f64())
                .unwrap_or_else(|| panic!("{doc:?} mapped to {screen:?} outside the page"));
            assert!(
                (back.x - doc.x).abs() <= 1.5 && (back.y - doc.y).abs() <= 1.5,
                "round trip drifted: {doc:?} -> {screen:?} -> {back:?}"
            );
        }
    }
}
