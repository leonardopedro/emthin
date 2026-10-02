//! Page model: the document's pages replace Emacs-frame workspaces.
//!
//! A *page* is one page of the rendered Typst document (typst's own
//! pagination). The user flips between pages with PgUp/PgDn or an
//! ext-workspace-v1 bar; there is exactly **one** `Space<Window>`,
//! because emthin never holds two screenfuls of app surfaces — an app's
//! surface is composited over a figure rect inside the current page's
//! raster, and figures on other pages keep their apps alive but idle.
//!
//! This module owns only page-local bookkeeping. Cross-subsystem
//! operations (`goto_page`) stay on `EmthinState` because they touch
//! seat, IME, focus, the document, and IPC at once.

use smithay::desktop::{Space, Window};
use smithay::wayland::shell::xdg::ToplevelSurface;

/// Page-related fields grouped together.
pub struct PageState {
    /// The single compositor `Space`. Every bound app toplevel is
    /// mapped here so it keeps getting configures and frame callbacks;
    /// the *render* pass composites their surfaces over figure rects
    /// rather than letting `render_output` draw them in `Space` order.
    pub active_space: Space<Window>,
    /// Index of the visible page (0-based).
    pub current_page: usize,
    /// Toplevels awaiting the dialog-vs-figure classification.
    ///
    /// Deferred by one tick: the `dispatch_clients` pass that fires
    /// `new_toplevel` may not yet have processed the same client's
    /// `set_parent` / `set_min_size` / `set_max_size` requests (the
    /// same reasoning as sway's `wants_floating`,
    /// desktop/xdg_shell.c:228). Drained in `tick.rs`.
    pub pending_app_toplevels: Vec<(ToplevelSurface, Window)>,
    /// ext-workspace-v1 protocol state, one workspace per page.
    pub protocol: crate::protocols::workspace::WorkspaceProtocolState,
}

impl PageState {
    pub fn new(protocol: crate::protocols::workspace::WorkspaceProtocolState) -> Self {
        Self {
            active_space: Space::default(),
            current_page: 0,
            pending_app_toplevels: Vec::new(),
            protocol,
        }
    }
}

/// Process ext-workspace-v1 client actions.
///
/// `Activate(id)` is a page switch. `Remove(id)` has no analogue —
/// pages come from the document's own pagination and are not
/// independently creatable or destructible — so it is rejected
/// rather than silently swallowed.
pub(crate) fn process_page_actions(state: &mut crate::EmthinState) {
    let actions = state.page.protocol.take_pending_actions();
    if actions.is_empty() {
        return;
    }
    state.needs_redraw = true;
    for action in actions {
        use crate::protocols::workspace::WorkspaceAction;
        match action {
            // Workspace ids are page indices + 1: ext-workspace ids are
            // non-zero by protocol, page indices are not.
            WorkspaceAction::Activate(id) => {
                state.goto_page(id.saturating_sub(1) as usize);
            }
            other => tracing::warn!("ext-workspace: unhandled action {other:?}"),
        }
    }
}

/// Send ext-workspace-v1 protocol events reflecting the document's
/// current page count, then clean up dead protocol handles.
///
/// A page is named after the first heading inside it when it has one,
/// otherwise `Page N`. The protocol's compositor is the single source
/// of truth: the page list *is* the document's pagination.
pub(crate) fn refresh_page_state(state: &mut crate::EmthinState) {
    let current = state.page.current_page;
    let infos: Vec<crate::protocols::workspace::WorkspaceInfo> = (0..state.doc.page_count())
        .map(|page| crate::protocols::workspace::WorkspaceInfo {
            id: page as u64 + 1,
            name: state.doc.page_name(page),
            active: page == current,
        })
        .collect();

    if let Some(output) = state.page.active_space.outputs().next().cloned() {
        let dh = state.display_handle.clone();
        state.page.protocol.refresh(&dh, &infos, &output);
    }
    state.page.protocol.cleanup_dead();
}
