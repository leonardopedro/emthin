use smithay::{
    delegate_xdg_shell,
    desktop::{
        find_popup_root_surface, get_popup_toplevel_coords, PopupKeyboardGrab, PopupKind,
        PopupManager, PopupPointerGrab, PopupUngrabStrategy, Space, Window,
    },
    input::{pointer::Focus, Seat},
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::{wl_output::WlOutput, wl_seat, wl_surface::WlSurface},
    },
    utils::Serial,
    wayland::{
        compositor::with_states,
        shell::xdg::{
            PopupSurface, PositionerState, SurfaceCachedState, ToplevelSurface, XdgShellHandler,
            XdgShellState, XdgToplevelSurfaceData,
        },
    },
};

use crate::EmthinState;

impl XdgShellHandler for EmthinState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.wl.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Every toplevel is a document figure. There is no "shell"
        // client and no special first-toplevel case: `tick` binds this
        // surface to a figure (an existing one, or one it appends to the
        // document — see `docui::figures::claim_toplevel`) and
        // configures the toplevel to that figure's size.
        //
        // The classification (floating dialog vs. figure-bound app) is
        // deferred by one tick: at this point `set_parent` /
        // `set_min_size` / `set_max_size` from the same Wayland batch
        // may not have been processed yet (mirroring sway's
        // `wants_floating`, desktop/xdg_shell.c:228).
        //
        // Crucially, we do NOT pin a size here. A previous version
        // configured (1, 1) so the initial round-trip would
        // immediately tell the client "you're tiny" — but
        // xwayland-satellite faithfully forwards that configure to the
        // X client and clobbers its natural size. Leaving pending size
        // unset means the initial configure goes out as (0, 0), the
        // "client choose" semantic in xdg_shell — clients commit at
        // their natural size and we re-configure once classified.
        let window = Window::new_wayland_window(surface.clone());
        // Tag so handle_surface_commit knows to *defer* the initial
        // configure: until the toplevel is classified we don't know
        // whether to send (0, 0) for a dialog or the figure's size for
        // a bound app, and sending a half-baked configure now causes
        // some X11 clients (Feishu via xwayland-satellite) to give up
        // and exit.
        window
            .user_data()
            .insert_if_missing(crate::handlers::dialogs::PendingClassificationTag::default);
        self.page
            .active_space
            .map_element(window.clone(), (0, 0), false);
        self.page.pending_app_toplevels.push((surface, window));
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        if let Err(e) = self.wl.popups.track_popup(PopupKind::Xdg(surface)) {
            tracing::warn!("Failed to track popup: {}", e);
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        // Only floating dialogs are draggable. A figure's geometry is the
        // document's — it comes from the `\app` statement's width and
        // height — so letting a figure be moved would desync the document from
        // what is on screen.
        let Some(window) = self
            .page
            .active_space
            .elements()
            .find(|w| {
                w.toplevel()
                    .is_some_and(|t| t.wl_surface() == surface.wl_surface())
            })
            .cloned()
        else {
            return;
        };
        if window
            .user_data()
            .get::<crate::handlers::dialogs::FloatingDialogTag>()
            .is_none()
        {
            return;
        }

        let Some(seat) = Seat::<EmthinState>::from_resource(&seat) else {
            return;
        };
        let Some(pointer) = seat.get_pointer() else {
            return;
        };
        // The client must own the click that started this move.
        if !pointer.has_grab(serial) {
            return;
        }
        let Some(start_data) = pointer.grab_start_data() else {
            return;
        };
        // Click must have landed on a surface from the same client.
        use smithay::reexports::wayland_server::Resource;
        let same_client = start_data
            .focus
            .as_ref()
            .is_some_and(|(s, _)| s.id().same_client_as(&surface.wl_surface().id()));
        if !same_client {
            return;
        }
        let Some(initial_window_location) = self.page.active_space.element_location(&window) else {
            return;
        };

        let grab = crate::grabs::MoveDialogGrab {
            start_data,
            window,
            initial_window_location,
        };
        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        // Only figures are resizable, and a resize is a document edit that
        // rewrites the `\app` statement (see `docui::edit`). Dialogs have no
        // figure and therefore no AppManager entry.
        let Some(window_id) = self.apps.id_for_surface(surface.wl_surface()) else {
            return;
        };
        let Some(window) = self
            .page
            .active_space
            .elements()
            .find(|w| {
                w.toplevel()
                    .is_some_and(|t| t.wl_surface() == surface.wl_surface())
            })
            .cloned()
        else {
            return;
        };

        let Some(seat) = Seat::<EmthinState>::from_resource(&seat) else {
            return;
        };
        let Some(pointer) = seat.get_pointer() else {
            return;
        };
        if !pointer.has_grab(serial) {
            return;
        }
        let Some(start_data) = pointer.grab_start_data() else {
            return;
        };
        use smithay::reexports::wayland_server::Resource;
        let same_client = start_data
            .focus
            .as_ref()
            .is_some_and(|(s, _)| s.id().same_client_as(&surface.wl_surface().id()));
        if !same_client {
            return;
        }
        let Some(initial_location) = self.page.active_space.element_location(&window) else {
            return;
        };
        let initial_size = window.bbox().size;

        let grab = crate::grabs::ResizeGrab {
            start_data,
            window,
            window_id,
            initial_location,
            initial_size,
            current_location: initial_location,
            current_size: initial_size,
            edges,
        };
        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        tracing::debug!("popup grab requested, serial={:?}", serial);
        let Some(seat) = Seat::<EmthinState>::from_resource(&seat) else {
            tracing::warn!("popup grab: seat not found");
            return;
        };
        let kind = PopupKind::Xdg(surface);

        if let Ok(root) = find_popup_root_surface(&kind) {
            // PopupGrab needs the root as our KeyboardFocusTarget, not a bare
            // wl_surface. Map it back through the space.
            let Some(root_target) = self.focus_target_for_surface(&root) else {
                tracing::warn!("popup grab: root surface has no known focus target");
                return;
            };
            let ret = self.wl.popups.grab_popup(root_target, kind, &seat, serial);

            match ret {
                Ok(mut grab) => {
                    if let Some(keyboard) = seat.get_keyboard() {
                        if keyboard.is_grabbed()
                            && !(keyboard.has_grab(serial)
                                || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
                        {
                            tracing::debug!("popup grab: keyboard already grabbed, ungrabbing");
                            grab.ungrab(PopupUngrabStrategy::All);
                            return;
                        }
                        keyboard.set_focus(self, grab.current_grab(), serial);
                        keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
                    }
                    if let Some(pointer) = seat.get_pointer() {
                        if pointer.is_grabbed()
                            && !(pointer.has_grab(serial)
                                || pointer.has_grab(grab.previous_serial().unwrap_or(serial)))
                        {
                            tracing::debug!("popup grab: pointer already grabbed, ungrabbing");
                            grab.ungrab(PopupUngrabStrategy::All);
                            return;
                        }
                        pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
                        tracing::debug!("popup grab: pointer grab set successfully");
                    }
                }
                Err(e) => {
                    tracing::warn!("popup grab failed: {:?}", e);
                }
            }
        } else {
            tracing::warn!("popup grab: could not find root surface");
        }
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        // A figure-bound app asking for fullscreen is asking to leave
        // the figure metaphor: acknowledge the state (so the client
        // hides its toolbar/chrome) but keep its size pinned to the
        // figure rect. There is no "host" to fullscreen any more.
        if self.apps.id_for_surface(surface.wl_surface()).is_some() {
            Self::set_toplevel_state(&surface, xdg_toplevel::State::Fullscreen, true);
            tracing::debug!("figure app fullscreen request acknowledged (size stays figure-bound)");
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if self.apps.id_for_surface(surface.wl_surface()).is_some() {
            Self::set_toplevel_state(&surface, xdg_toplevel::State::Fullscreen, false);
            tracing::debug!("figure app unfullscreen request acknowledged");
        }
    }

    fn maximize_request(&mut self, _surface: ToplevelSurface) {
        // No-op by policy: a figure's size is the document's business.
        // Clients (GTK4/3) send `unmaximize_request` immediately on
        // connect if Maximized was in the initial configure; the state
        // is never set, so there is nothing to re-assert.
    }

    fn unmaximize_request(&mut self, _surface: ToplevelSurface) {}

    fn title_changed(&mut self, surface: ToplevelSurface) {
        let title =
            Self::get_toplevel_data(&surface, |d| d.lock().ok().and_then(|d| d.title.clone()));
        let Some(window_id) = self.apps.id_for_surface(surface.wl_surface()) else {
            return;
        };
        let Some(title) = title else { return };
        // The bound app owns the host window's title: it is the only
        // thing on screen that has a "title" in the ordinary sense.
        self.host.set_title(Some(&title));
        self.doc.on_app_title_changed(window_id, &title);
        self.ipc
            .send(crate::ipc::OutgoingMessage::AppTitleChanged { window_id, title });
    }

    fn app_id_changed(&mut self, surface: ToplevelSurface) {
        let Some(window_id) = self.apps.id_for_surface(surface.wl_surface()) else {
            return;
        };
        let app_id =
            Self::get_toplevel_data(&surface, |d| d.lock().ok().and_then(|d| d.app_id.clone()));
        if let Some(app_id) = app_id {
            self.host.set_app_id(Some(&app_id));
            self.doc.on_app_id_changed(window_id, &app_id);
        }
    }
}

/// Whether a toplevel should map as a floating dialog (centered, not
/// bound to a figure) instead of a figure-bound app.
///
/// Direct port of sway's `wants_floating` (sway/desktop/xdg_shell.c:228):
///
/// ```c
/// return (min_w != 0 && min_h != 0 &&
///         (min_w == max_w || min_h == max_h))
///        || toplevel->parent;
/// ```
///
/// Note the OR between `min_w == max_w` and `min_h == max_h` — a single
/// pinned axis is enough. We previously required both axes pinned plus
/// a 600×500 hard size cap, but the cap kept misclassifying wechat's
/// 560×760 login window (HiDPI doubles the X-side `WM_NORMAL_HINTS`
/// satellite forwards) as an embedded app.
///
/// X11 clients arrive through `xwayland-satellite`, which forwards
/// `WM_NORMAL_HINTS` → `xdg_toplevel.set_min_size` + `set_max_size`
/// (xwayland-satellite/src/server/mod.rs:978) and `WM_TRANSIENT_FOR` →
/// `set_parent`. Both `parent` and the cached min/max sizes are
/// populated only after the client's initial dispatch burst finishes —
/// call this from the drain pass in
/// `tick::process_pending_app_toplevels`, never inside
/// `XdgShellHandler::new_toplevel`.
pub fn wants_floating(_state: &EmthinState, surface: &ToplevelSurface) -> bool {
    let parent = with_states(surface.wl_surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|d| d.lock().ok())
            .and_then(|d| d.parent.clone())
    });
    if parent.is_some() {
        return true;
    }

    let (min, max) = with_states(surface.wl_surface(), |states| {
        let mut cached = states.cached_state.get::<SurfaceCachedState>();
        let current = cached.current();
        (current.min_size, current.max_size)
    });
    min.w > 0 && min.h > 0 && (min.w == max.w || min.h == max.h)
}

// Xdg Shell
delegate_xdg_shell!(EmthinState);

// Xdg Decoration — always force server-side (no decorations drawn = borderless)
use smithay::delegate_xdg_decoration;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;

impl XdgDecorationHandler for EmthinState {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        toplevel.send_configure();
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: Mode) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        toplevel.send_pending_configure();
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        toplevel.send_pending_configure();
    }
}

delegate_xdg_decoration!(EmthinState);

pub fn handle_surface_commit(
    popups: &mut PopupManager,
    space: &Space<Window>,
    surface: &WlSurface,
) {
    if let Some(window) = space
        .elements()
        .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface))
        .cloned()
    {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|d| d.lock().ok())
                .map(|d| d.initial_configure_sent)
                .unwrap_or(true)
        });

        // Skip auto-configure while the toplevel is still awaiting
        // dialog-vs-app classification — see PendingClassificationTag.
        let pending_classification = window
            .user_data()
            .get::<crate::handlers::dialogs::PendingClassificationTag>()
            .is_some();

        if !initial_configure_sent && !pending_classification {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_configure();
            }
        }
    }

    // Handle popup commits.
    popups.commit(surface);
    if let Some(popup) = popups.find_popup(surface) {
        match popup {
            PopupKind::Xdg(ref xdg) => {
                if !xdg.is_initial_configure_sent() {
                    if let Err(e) = xdg.send_configure() {
                        tracing::warn!("initial popup configure failed: {e}");
                    }
                }
            }
            PopupKind::InputMethod(ref _input_method) => {}
        }
    }
}

impl EmthinState {
    fn set_toplevel_state(surface: &ToplevelSurface, state: xdg_toplevel::State, enabled: bool) {
        surface.with_pending_state(|s| {
            if enabled {
                s.states.set(state);
            } else {
                s.states.unset(state);
            }
        });
        surface.send_pending_configure();
    }

    fn get_toplevel_data<T>(
        surface: &ToplevelSurface,
        extractor: impl FnOnce(&XdgToplevelSurfaceData) -> Option<T>,
    ) -> Option<T> {
        with_states(surface.wl_surface(), |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(extractor)
        })
    }

    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let popup_kind = PopupKind::Xdg(popup.clone());
        let Ok(root) = find_popup_root_surface(&popup_kind) else {
            return;
        };
        let Some(window) = self
            .page
            .active_space
            .elements()
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == &root))
        else {
            return;
        };

        let Some(output) = self.page.active_space.outputs().next() else {
            return;
        };
        let Some(output_geo) = self.page.active_space.output_geometry(output) else {
            return;
        };
        let Some(window_geo) = self.page.active_space.element_geometry(window) else {
            return;
        };

        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(&popup_kind);
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}
