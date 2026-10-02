//! Pointer grabs: `MoveDialogGrab` for dragging floating dialogs,
//! `ResizeGrab` for a client-initiated resize, and `FigureResizeGrab`
//! for resizing a figure by dragging its edges — which rewrites the
//! `\app` arguments in the document.

use crate::EmthinState;
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
        GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab,
        PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, Size},
};

pub struct MoveDialogGrab {
    pub start_data: GrabStartData<EmthinState>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
}

impl PointerGrab<EmthinState> for MoveDialogGrab {
    fn motion(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // Suppress pointer focus while dragging — the dialog tracks
        // the pointer wholesale, no other surface should think it's
        // hovered.
        handle.motion(data, None, event);

        let delta = event.location - self.start_data.location;
        let new_location = self.initial_window_location.to_f64() + delta;
        data.page.active_space.map_element(
            self.window.clone(),
            new_location.to_i32_round::<i32>(),
            true,
        );
    }

    fn relative_motion(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(&mut self, data: &mut EmthinState, handle: &mut PointerInnerHandle<'_, EmthinState>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &GrabStartData<EmthinState> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut EmthinState) {}
}

/// Pointer grab for resizing an embedded app window via its edges.
/// Activated when the client sends an `xdg_toplevel.resize_request`.
///
/// A figure-bound app gets its size from the document, so this grab is
/// only reachable for a floating dialog (see `FigureResizeGrab` for the
/// document-side gesture).
pub struct ResizeGrab {
    pub start_data: GrabStartData<EmthinState>,
    pub window: Window,
    pub window_id: u64,
    pub initial_location: Point<i32, Logical>,
    pub initial_size: Size<i32, Logical>,
    /// Tracks the last computed location during drag — updated on every
    /// motion event so that final IPC reflects the actual final geometry.
    pub current_location: Point<i32, Logical>,
    /// Tracks the last computed size during drag.
    pub current_size: Size<i32, Logical>,
    pub edges: ResizeEdge,
}

impl ResizeGrab {
    fn compute_geometry(&mut self, delta: Point<i32, Logical>) {
        self.current_location = self.initial_location;
        self.current_size = self.initial_size;

        // Determine which axes are being resized based on the edge.
        // ResizeEdge is a plain enum (not bitflags), so we match each
        // variant individually. Corner variants affect two axes.
        let (resize_left, resize_right, resize_top, resize_bottom) = match self.edges {
            ResizeEdge::Top => (false, false, true, false),
            ResizeEdge::Bottom => (false, false, false, true),
            ResizeEdge::Left => (true, false, false, false),
            ResizeEdge::Right => (false, true, false, false),
            ResizeEdge::TopLeft => (true, false, true, false),
            ResizeEdge::TopRight => (false, true, true, false),
            ResizeEdge::BottomLeft => (true, false, false, true),
            ResizeEdge::BottomRight => (false, true, false, true),
            ResizeEdge::None => (false, false, false, false),
            _ => (false, false, false, false),
        };

        if resize_left {
            let dx = delta.x.min(self.initial_size.w - 50);
            self.current_location.x = self.initial_location.x + dx;
            self.current_size.w = self.initial_size.w - dx;
        }
        if resize_top {
            let dy = delta.y.min(self.initial_size.h - 50);
            self.current_location.y = self.initial_location.y + dy;
            self.current_size.h = self.initial_size.h - dy;
        }
        if resize_right {
            self.current_size.w = (self.initial_size.w + delta.x).max(50);
        }
        if resize_bottom {
            self.current_size.h = (self.initial_size.h + delta.y).max(50);
        }
    }

    fn send_window_resized(&self, data: &mut EmthinState) {
        let rect = crate::ipc::IpcRect {
            x: self.current_location.x,
            y: self.current_location.y,
            w: self.current_size.w,
            h: self.current_size.h,
        };
        tracing::debug!(
            "resize grab committed at ({},{}) {}x{}",
            rect.x,
            rect.y,
            rect.w,
            rect.h
        );
        data.needs_redraw = true;
    }
}

impl PointerGrab<EmthinState> for ResizeGrab {
    fn motion(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);

        let delta = (event.location - self.start_data.location).to_i32_round();
        self.compute_geometry(delta);

        if let Some(toplevel) = self.window.toplevel() {
            toplevel.with_pending_state(|s| {
                s.size = Some(self.current_size);
                s.states.set(
                    smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::TiledLeft,
                );
                s.states.set(
                    smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::TiledRight,
                );
                s.states.set(
                    smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::TiledTop,
                );
                s.states.set(
                    smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::TiledBottom,
                );
            });
            toplevel.send_pending_configure();
        }
        data.page
            .active_space
            .map_element(self.window.clone(), self.current_location, true);
    }

    fn relative_motion(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            self.send_window_resized(data);
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(&mut self, data: &mut EmthinState, handle: &mut PointerInnerHandle<'_, EmthinState>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &GrabStartData<EmthinState> {
        &self.start_data
    }

    fn unset(&mut self, data: &mut EmthinState) {
        self.send_window_resized(data);
    }
}

/// How close to a figure's edge the pointer must be to start a resize.
pub const FIGURE_EDGE_PX: f64 = 6.0;

/// Snap granularity for figure resizes, so a drag lands on round
/// numbers and small jitters don't dirty the document.
pub const FIGURE_SNAP_PX: i32 = 8;

/// Pointer grab for resizing a **document figure** by dragging its
/// edges.
///
/// Unlike a window-manager resize, this edits the *document*: on
/// release the grab rewrites the figure's `\app(#s, #f, W, H)`
/// literals, and the ordinary re-layout path then reflows the page and
/// reconfigures the bound app. That keeps one source of truth — a
/// figure's size lives in the text, not in compositor state.
///
/// The live preview is a compositor-side overlay (the figure rect is
/// painted directly), so no document write happens until release: a
/// drag is one CRDT op, not four hundred.
pub struct FigureResizeGrab {
    pub start_data: GrabStartData<EmthinState>,
    /// The figure being resized (`f<stmt-index>`).
    pub figure_key: String,
    /// The figure's rect when the drag started.
    pub initial_rect: Rectangle<i32, Logical>,
    /// Live preview rect during the drag.
    pub current_rect: Rectangle<i32, Logical>,
    pub edges: ResizeEdge,
    /// Set once the document write has happened, so `unset` (which
    /// runs after `button`) doesn't emit a duplicate IPC event.
    committed: bool,
}

impl FigureResizeGrab {
    pub fn new(
        start_data: GrabStartData<EmthinState>,
        figure_key: String,
        initial_rect: Rectangle<i32, Logical>,
        edges: ResizeEdge,
    ) -> Self {
        Self {
            start_data,
            figure_key,
            initial_rect,
            current_rect: initial_rect,
            edges,
            committed: false,
        }
    }

    /// Which figure edge (if any) a point is within [`FIGURE_EDGE_PX`]
    /// of. Corners win over edges so a corner drag resizes both axes.
    pub fn edge_at(rect: &Rectangle<i32, Logical>, pos: Point<f64, Logical>) -> ResizeEdge {
        let (w, h) = (f64::from(rect.size.w), f64::from(rect.size.h));
        if w <= 0.0 || h <= 0.0 {
            return ResizeEdge::None;
        }
        let x = pos.x - f64::from(rect.loc.x);
        let y = pos.y - f64::from(rect.loc.y);
        if x < 0.0 || y < 0.0 || x > w || y > h {
            return ResizeEdge::None;
        }
        let near_left = x <= FIGURE_EDGE_PX;
        let near_right = w - x <= FIGURE_EDGE_PX;
        let near_top = y <= FIGURE_EDGE_PX;
        let near_bottom = h - y <= FIGURE_EDGE_PX;
        match (near_left, near_right, near_top, near_bottom) {
            (true, _, true, _) => ResizeEdge::TopLeft,
            (_, true, true, _) => ResizeEdge::TopRight,
            (true, _, _, true) => ResizeEdge::BottomLeft,
            (_, true, _, true) => ResizeEdge::BottomRight,
            (true, _, _, _) => ResizeEdge::Left,
            (_, true, _, _) => ResizeEdge::Right,
            (_, _, true, _) => ResizeEdge::Top,
            (_, _, _, true) => ResizeEdge::Bottom,
            _ => ResizeEdge::None,
        }
    }

    /// Snap a dimension to the resize grid (at least
    /// [`crate::docui::edit::MIN_FIGURE`]).
    ///
    /// Round-to-nearest, not floor: a drag that lands on 647 should snap
    /// up to 648, not down to 640 and then feel "sticky" as the user
    /// pushes past the halfway point.
    fn snap(value: i32) -> i32 {
        let half = FIGURE_SNAP_PX / 2;
        let snapped = value.div_euclid(FIGURE_SNAP_PX) * FIGURE_SNAP_PX
            + if value.rem_euclid(FIGURE_SNAP_PX) >= half {
                FIGURE_SNAP_PX
            } else {
                0
            };
        snapped.clamp(
            crate::docui::edit::MIN_FIGURE,
            crate::docui::edit::MAX_FIGURE,
        )
    }

    fn compute_rect(&mut self, delta: Point<i32, Logical>) {
        let loc = self.initial_rect.loc;
        let mut size = self.initial_rect.size;

        // A figure lives in the document's flow: only its bottom-right
        // corner is free. Dragging the left/top edges therefore grows or
        // shrinks it *in place* (the flow position is the document's
        // business, not the pointer's) rather than moving the origin.
        if matches!(
            self.edges,
            ResizeEdge::Right | ResizeEdge::TopRight | ResizeEdge::BottomRight
        ) {
            size.w = Self::snap(self.initial_rect.size.w + delta.x);
        }
        if matches!(
            self.edges,
            ResizeEdge::Bottom | ResizeEdge::BottomLeft | ResizeEdge::BottomRight
        ) {
            size.h = Self::snap(self.initial_rect.size.h + delta.y);
        }
        if matches!(
            self.edges,
            ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft
        ) {
            size.w = Self::snap(self.initial_rect.size.w - delta.x);
        }
        if matches!(
            self.edges,
            ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight
        ) {
            size.h = Self::snap(self.initial_rect.size.h - delta.y);
        }

        // Location never moves: the flow owns it. Kept explicit so the
        // intent survives the next reader wondering why `loc` is
        // constant.
        self.current_rect = Rectangle::new(loc, size);
    }

    /// Commit the drag into the document. Idempotent: `unset` also runs
    /// after `button`, so a double call must not write twice.
    fn commit(&mut self, data: &mut EmthinState) {
        if self.committed {
            return;
        }
        self.committed = true;
        let (w, h) = (self.current_rect.size.w, self.current_rect.size.h);
        let Some(figure) = data.doc.figures().get(&self.figure_key).cloned() else {
            return;
        };
        if crate::docui::edit::set_figure_size(data.doc.model_mut(), &figure, w, h) {
            tracing::info!(
                "figure {} resized to {w}x{h} (\\app args rewritten)",
                self.figure_key
            );
            data.doc.relayout();
        }
        // Reflow has settled: reconfigure the app and report the new
        // geometry to any control client.
        crate::handlers::apps::reconfigure_after_resize(data, &self.figure_key);
    }
}

impl PointerGrab<EmthinState> for FigureResizeGrab {
    fn motion(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // No surface focus while dragging: the pointer is manipulating
        // the page, not pointing at a client.
        handle.motion(data, None, event);
        let delta = (event.location - self.start_data.location).to_i32_round();
        self.compute_rect(delta);

        // Live preview: paint the preview rect straight into the figure
        // manager so the render pass picks it up, without touching the
        // document.
        if let Some(figure) = data.doc.figures_mut().get_mut(&self.figure_key) {
            figure.rect = self.current_rect;
        }
        data.needs_redraw = true;
    }

    fn button(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            self.commit(data);
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn unset(&mut self, data: &mut EmthinState) {
        // `unset` runs after `button` already committed; `commit` is
        // guarded so this is a no-op in that case.
        self.commit(data);
    }

    fn axis(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(&mut self, data: &mut EmthinState, handle: &mut PointerInnerHandle<'_, EmthinState>) {
        handle.frame(data);
    }

    fn relative_motion(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut EmthinState,
        handle: &mut PointerInnerHandle<'_, EmthinState>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &GrabStartData<EmthinState> {
        &self.start_data
    }
}

#[cfg(test)]
mod figure_resize_tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn edge_detection_finds_each_border() {
        let r = rect(100, 100, 400, 300);
        assert_eq!(
            FigureResizeGrab::edge_at(&r, (102.0, 250.0).into()),
            ResizeEdge::Left
        );
        assert_eq!(
            FigureResizeGrab::edge_at(&r, (498.0, 250.0).into()),
            ResizeEdge::Right
        );
        assert_eq!(
            FigureResizeGrab::edge_at(&r, (300.0, 102.0).into()),
            ResizeEdge::Top
        );
        assert_eq!(
            FigureResizeGrab::edge_at(&r, (300.0, 398.0).into()),
            ResizeEdge::Bottom
        );
    }

    #[test]
    fn corners_win_over_edges() {
        let r = rect(0, 0, 400, 300);
        assert_eq!(
            FigureResizeGrab::edge_at(&r, (1.0, 1.0).into()),
            ResizeEdge::TopLeft
        );
        assert_eq!(
            FigureResizeGrab::edge_at(&r, (399.0, 299.0).into()),
            ResizeEdge::BottomRight
        );
    }

    #[test]
    fn the_middle_of_a_figure_is_not_an_edge() {
        let r = rect(0, 0, 400, 300);
        assert_eq!(
            FigureResizeGrab::edge_at(&r, (200.0, 150.0).into()),
            ResizeEdge::None
        );
    }

    #[test]
    fn points_outside_the_figure_are_not_edges() {
        let r = rect(100, 100, 400, 300);
        assert_eq!(
            FigureResizeGrab::edge_at(&r, (10.0, 10.0).into()),
            ResizeEdge::None
        );
        assert_eq!(
            FigureResizeGrab::edge_at(&r, (900.0, 10.0).into()),
            ResizeEdge::None
        );
    }

    #[test]
    fn a_degenerate_rect_has_no_edges() {
        let r = rect(0, 0, 0, 0);
        assert_eq!(
            FigureResizeGrab::edge_at(&r, (0.0, 0.0).into()),
            ResizeEdge::None
        );
    }

    #[test]
    fn snap_rounds_to_the_grid_and_respects_the_minimum() {
        assert_eq!(FigureResizeGrab::snap(640), 640);
        assert_eq!(FigureResizeGrab::snap(643), 640);
        assert_eq!(FigureResizeGrab::snap(647), 648);
        assert_eq!(FigureResizeGrab::snap(1), crate::docui::edit::MIN_FIGURE);
        assert_eq!(
            FigureResizeGrab::snap(100_000),
            crate::docui::edit::MAX_FIGURE
        );
    }
}
