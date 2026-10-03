pub mod apps;
pub mod cursor;
pub mod dbus;
pub mod focus;
pub mod host;
pub mod ime;
pub mod page;
pub mod xwayland;

// Type re-exports for common shorthands (kept for historical call sites
// that used `crate::KeyboardFocusTarget` before state/ existed).
pub use focus::KeyboardFocusTarget;

use std::{ffi::OsString, sync::Arc};

use smithay::{
    backend::{renderer::gles::GlesRenderer, winit::WinitGraphicsBackend},
    desktop::{PopupManager, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    reexports::{
        calloop::{
            generic::Generic, EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction,
        },
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{Logical, Point, Rectangle},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        cursor_shape::CursorShapeManagerState,
        dmabuf::{DmabufGlobal, DmabufState},
        fractional_scale::FractionalScaleManagerState,
        output::OutputManagerState,
        relative_pointer::RelativePointerManagerState,
        selection::{data_device::DataDeviceState, primary_selection::PrimarySelectionState},
        selection::{
            ext_data_control::DataControlState as ExtDataControlState,
            wlr_data_control::DataControlState as WlrDataControlState,
        },
        shell::xdg::{decoration::XdgDecorationState, XdgShellState},
        shm::ShmState,
        socket::ListeningSocketSource,
        viewporter::ViewporterState,
    },
};

use smithay::wayland::seat::WaylandFocus;

/// Tracks where the active selection came from, so paste requests are
/// routed to the correct data source.
///
/// - `Wayland`: a wayland client on emthin owns a data source that can
///   be pulled via `request_data_device_client_selection`. X clients
///   running under `xwayland-satellite` also fall into this variant —
///   satellite translates X selections into Wayland data sources before
///   they ever reach emthin.
/// - `Host`: emthin received the selection from the host compositor
///   via `inject_host_selection` and holds only an offer — actual data
///   must be pulled back from the host via `ClipboardProxy`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SelectionOrigin {
    #[default]
    Wayland,
    Host,
}

/// Kind of focus override currently in effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusOverride {
    /// Emthin's window itself lost host-level focus (Alt+Tab away).
    Host,
}

/// Focus-override state — one saved focus per active override kind.
///
/// Replaces three independent `Option<KeyboardFocusTarget>` slots with
/// a typed `enter`/`exit`/`is_active` API so callers don't manipulate
/// raw fields. Saved focus is itself `Option` because there may not
/// have been a focused surface at the moment the override fired.
#[derive(Default)]
pub struct FocusState {
    saves: std::collections::HashMap<FocusOverride, Option<crate::KeyboardFocusTarget>>,
    /// Last embedded app that had keyboard focus. Used by WakeUp toggle.
    pub last_app_focus: Option<crate::KeyboardFocusTarget>,
}

impl FocusState {
    /// Save `current` as the focus to restore when this override exits.
    /// **Always overwrites.**
    pub fn enter(&mut self, kind: FocusOverride, current: Option<crate::KeyboardFocusTarget>) {
        self.saves.insert(kind, current);
    }

    /// Exit override; returns saved focus to restore. Outer `Some`
    /// means the override was active; inner `Option` is the actual
    /// saved focus (which itself may be `None` if nothing was focused
    /// when the override fired).
    pub fn exit(&mut self, kind: FocusOverride) -> Option<Option<crate::KeyboardFocusTarget>> {
        self.saves.remove(&kind)
    }

    pub fn is_active(&self, kind: FocusOverride) -> bool {
        self.saves.contains_key(&kind)
    }

    /// Clear every saved-focus slot and the last-app focus.
    /// Called on page switch: the saved targets may reference surfaces
    /// whose figures just left the visible page. Without this, a
    /// focus-away → page-switch → focus-back sequence would restore
    /// focus to a surface on a hidden page (sending `wl_keyboard.enter`
    /// to a client nobody is looking at).
    pub fn reset_on_page_switch(&mut self) {
        self.saves.clear();
        self.last_app_focus = None;
    }
}

/// Clipboard/selection routing state grouped together.
#[derive(Default)]
pub struct SelectionState {
    /// Clipboard synchronization proxy (Wayland or X11 backend).
    pub clipboard: Option<Box<dyn emthin_clipboard::ClipboardBackend>>,
    /// Direct handle on the *host* clipboard for the document's own
    /// copy/paste. Separate from `clipboard` on purpose: see
    /// `clipboard_bridge::set_host_clipboard`. `None` until first use
    /// (constructing it acquires the X11 clipboard ownership).
    pub doc_clipboard: Option<arboard::Clipboard>,
    /// Where the current clipboard selection came from.
    pub clipboard_origin: SelectionOrigin,
    /// Where the current primary selection came from.
    pub primary_origin: SelectionOrigin,
    /// Cached payload for a compositor-owned clipboard selection.
    /// `(mime_types, data)` — `send_selection` writes `data` into
    /// the fd when the requested mime_type matches.
    pub clipboard_cache: Option<(Vec<String>, Vec<u8>)>,
}

/// Smithay Wayland protocol state — pure bookkeeping for compositor protocols.
pub struct WaylandState {
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<EmthinState>,
    pub data_device_state: DataDeviceState,
    pub primary_selection_state: PrimarySelectionState,
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    pub viewporter_state: ViewporterState,
    pub xdg_decoration_state: XdgDecorationState,
    pub cursor_shape_manager_state: CursorShapeManagerState,
    /// Advertise `zwlr_data_control_v1` and `ext_data_control_v1` to
    /// emthin's own internal clients so they can exchange selections
    /// without needing keyboard focus. Mirrors what real wlroots /
    /// cosmic / KDE ≥ 6.2 do for their clients, and lets tools like
    /// wl-copy / wl-paste inside emthin skip the wl_data_device focus
    /// dance entirely.
    pub wlr_data_control_state: WlrDataControlState,
    pub ext_data_control_state: ExtDataControlState,
    pub dmabuf_state: DmabufState,
    /// Keep-alive: dropping this removes the linux-dmabuf global from the display.
    pub dmabuf_global: Option<DmabufGlobal>,
    /// Exposes `zwp_relative_pointer_manager_v1` — delivers raw mouse deltas
    /// to clients that bind the protocol (required for FPS camera control).
    pub relative_pointer_manager_state: RelativePointerManagerState,
    pub popups: PopupManager,
}

pub struct EmthinState {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,

    pub ipc: crate::ipc::IpcServer,
    pub apps: crate::apps::AppManager,

    /// Workspace model: the document's page state. Pages replace the
    /// old Emacs-frame workspaces; there is exactly **one** `Space`
    /// (the compositor never holds a second screenful of app surfaces
    /// — an app's surface is composited over a figure rect in the
    /// current page's raster).
    pub page: crate::page::PageState,

    pub loop_signal: LoopSignal,
    pub loop_handle: LoopHandle<'static, EmthinState>,

    /// Winit graphics backend (renderer + window). Stored here so
    /// `DmabufHandler::dmabuf_imported` can access the renderer.
    pub backend: Option<WinitGraphicsBackend<GlesRenderer>>,

    // Smithay protocol state (grouped for clarity).
    pub wl: WaylandState,

    /// XWayland supervisor state — display number, xwayland-satellite
    /// integration handle, and the `--command` deferred-spawn mailbox.
    pub xwayland: xwayland::XwaylandState,

    pub seat: Seat<Self>,

    // --- emthin specific ---
    /// The emthin host window + spawned child processes.
    pub host: host::HostState,

    /// The rendered document: `MathDoc` text, its scan/segments, the
    /// paged layout (raster + glyph index + figure rects), and the
    /// figure↔app bindings. The document is the layout authority —
    /// editing `\app` args reflows figures, which reconfigures app
    /// toplevels.
    pub doc: crate::docui::DocUi,

    /// The current page's raster, cached on the GPU. Owned by the
    /// compositor rather than by `docui` because it needs a live
    /// `GlesRenderer` to import into, and only the render pass has one.
    pub doc_page: crate::doc_render::DocPageTexture,

    /// Rasterized dormant-figure labels. Same reasoning as `doc_page`: it needs
    /// a renderer to upload into, and it is compositor state rather than
    /// document state because only the compositor knows what is bound.
    pub dormant_labels: crate::dormant_label::LabelCache,

    /// Clipboard/selection routing state.
    pub selection: SelectionState,

    /// Focus management state.
    pub focus: FocusState,

    /// IME (text_input_v3) bridge — host IME ↔ embedded Wayland clients.
    pub ime: crate::ime::ImeBridge,

    /// Cursor image tracking (Named / Surface) + raw pointer location
    /// for `zwp_relative_pointer_v1` delta synthesis.
    pub cursor: cursor::CursorState,

    /// Coarse damage flag for structural events (IPC, layer shell, input,
    /// page switch) that smithay's per-element OutputDamageTracker does
    /// not cover.  When true the next Redraw calls render_frame; cleared after.
    pub needs_redraw: bool,

    /// Bridge to the in-process DBus broker that impersonates fcitx5
    /// for embedded clients. Populated in `main.rs` after `init_winit`;
    /// stays [`dbus::DbusBridge::default`] (inert) if the host has no
    /// session bus.
    pub dbus: dbus::DbusBridge,
}

impl EmthinState {
    pub fn new(
        event_loop: &mut EventLoop<Self>,
        loop_handle: LoopHandle<'static, Self>,
        display: Display<Self>,
        ipc: crate::ipc::IpcServer,
        xkb_config: smithay::input::keyboard::XkbConfig<'_>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let start_time = std::time::Instant::now();
        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let popups = PopupManager::default();

        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<Self>(&dh);
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let cursor_shape_manager_state = CursorShapeManagerState::new::<Self>(&dh);
        let ime = crate::ime::ImeBridge::new(&dh);
        let relative_pointer_manager_state = RelativePointerManagerState::new::<Self>(&dh);
        let dmabuf_state = DmabufState::new();

        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        // Always expose DC to internal clients — internal clients
        // (Firefox, Electron apps, wl-clipboard, screen-grabs) prefer
        // DC over wl_data_device and therefore never need keyboard
        // focus to exchange selections with one another. Mirrors what
        // wlroots / cosmic / KDE ≥ 6.2 do for their clients.
        let wlr_data_control_state =
            WlrDataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let ext_data_control_state =
            ExtDataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);

        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "winit");

        seat.add_keyboard(xkb_config, 200, 25)
            .map_err(|e| format!("failed to initialize keyboard: {e:?}"))?;
        seat.add_pointer();

        let workspace_protocol = crate::protocols::workspace::WorkspaceProtocolState::new(&dh);

        let socket_name = Self::init_wayland_listener(display, event_loop)?;

        let loop_signal = event_loop.get_signal();

        Ok(Self {
            start_time,
            display_handle: dh,

            ipc,
            apps: crate::apps::AppManager::default(),

            page: crate::page::PageState::new(workspace_protocol),

            loop_signal,
            loop_handle,
            socket_name,

            backend: None,

            wl: WaylandState {
                compositor_state,
                xdg_shell_state,
                shm_state,
                output_manager_state,
                seat_state,
                data_device_state,
                primary_selection_state,
                fractional_scale_manager_state,
                viewporter_state,
                xdg_decoration_state,
                cursor_shape_manager_state,
                wlr_data_control_state,
                ext_data_control_state,
                dmabuf_state,
                dmabuf_global: None,
                relative_pointer_manager_state,
                popups,
            },
            xwayland: xwayland::XwaylandState::default(),
            seat,

            // emthin specific
            host: host::HostState::new(),
            doc: crate::docui::DocUi::new(),
            doc_page: crate::doc_render::DocPageTexture::new(),
            dormant_labels: crate::dormant_label::LabelCache::default(),
            selection: SelectionState::default(),
            focus: FocusState::default(),
            ime,
            cursor: cursor::CursorState::default(),
            needs_redraw: true,
            dbus: dbus::DbusBridge::default(),
        })
    }

    fn init_wayland_listener(
        display: Display<EmthinState>,
        event_loop: &mut EventLoop<Self>,
    ) -> Result<OsString, Box<dyn std::error::Error>> {
        // Pin the socket name when `--wayland-socket <NAME>` was passed on
        // the CLI (main.rs copies the flag value into this env var) or
        // when `EMTHIN_WAYLAND_SOCKET_NAME` is set directly. Used by E2E
        // tests so external Wayland clients (wl-copy, xclip, …) have a
        // predictable WAYLAND_DISPLAY. Otherwise fall through to `new_auto()`
        // which picks wayland-N.
        let listening_socket = match std::env::var_os("EMTHIN_WAYLAND_SOCKET_NAME") {
            Some(name) => ListeningSocketSource::with_name(&name.to_string_lossy())?,
            None => ListeningSocketSource::new_auto()?,
        };
        let socket_name = listening_socket.socket_name().to_os_string();

        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                if let Err(e) = state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                {
                    tracing::error!("Failed to insert Wayland client: {}", e);
                }
            })
            .map_err(|e| format!("failed to init wayland event source: {e}"))?;

        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    // SAFETY: `display` is owned by the Generic source and lives for
                    // the entire event loop. No other mutable reference to the Display
                    // exists during this callback, as calloop guarantees single-threaded
                    // dispatch. We never drop the display while the source is active.
                    unsafe {
                        if let Err(e) = display.get_mut().dispatch_clients(state) {
                            tracing::error!("dispatch_clients failed: {}", e);
                        }
                    }
                    // Flush responses immediately so clients don't wait until
                    // the next render frame for roundtrip replies (wl_display.sync).
                    let _ = state.display_handle.flush_clients();
                    state.needs_redraw = true;
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|e| format!("failed to init display event source: {e}"))?;

        Ok(socket_name)
    }

    /// Fullscreen geometry for the primary output (logical pixels).
    pub fn output_fullscreen_geo(&self) -> Option<Rectangle<i32, Logical>> {
        let output = self.page.active_space.outputs().next()?;
        let mode = output.current_mode()?;
        let scale = output.current_scale().fractional_scale();
        let logical = mode.size.to_f64().to_logical(scale).to_i32_round();
        Some(Rectangle::new((0, 0).into(), logical))
    }

    /// Re-letterbox the document page against the current output size.
    ///
    /// Called when the host window resizes and when a layer-shell client
    /// changes the non-exclusive zone: the page is centred in whatever
    /// space is usable, then every bound app is reconfigured to its (now
    /// moved and rescaled) figure rect.
    /// Re-lay out the document and close any app whose `\app` statement is gone.
    ///
    /// Every production path that edits the document goes through here rather
    /// than calling `DocUi::relayout` directly. `DocUi::relayout` returns the
    /// window ids of apps whose figure disappeared — its doc comment has said "so
    /// the caller can close them" from the start — and all seven call sites
    /// discarded that value as a bare statement expression.
    ///
    /// The result was that deleting a figure statement left the client mapped in
    /// the `Space` and registered in `AppManager`: it kept committing, kept
    /// receiving frame callbacks, and was never drawn, because `figure_render`
    /// only walks figures. An invisible client that never goes away, plus a
    /// monotonically growing `state.apps`.
    ///
    /// Closing uses `xdg_toplevel::send_close`, the same mechanism `ipc_close`
    /// uses, rather than killing anything — the client tears itself down and
    /// `cleanup_dead_apps` then does the unmapping and figure release it always
    /// did. A client that ignores `close` stays alive, which is the correct
    /// outcome: emthin does not own the client's process.
    pub fn relayout_document(&mut self) {
        let released = self.doc.relayout();
        self.close_apps_whose_figure_is_gone(&released);
    }

    fn close_apps_whose_figure_is_gone(&mut self, released: &[u64]) {
        for window_id in released {
            let Some(app) = self.apps.get(*window_id) else {
                continue;
            };
            let Some(toplevel) = app.window.toplevel() else {
                continue;
            };
            tracing::info!(
                "closing app {window_id}: its \\app statement was deleted from the document"
            );
            toplevel.send_close();
            // The binding is dropped now rather than waiting for the client to
            // actually die, so nothing can draw or reconfigure it in between.
            self.doc.figures_mut().release_app(*window_id);
        }
    }

    pub fn relayout_doc(&mut self) {
        let geo = self.usable_area();
        tracing::debug!(
            "relayout_doc: usable area ({},{}) {}x{}",
            geo.loc.x,
            geo.loc.y,
            geo.size.w,
            geo.size.h,
        );
        self.doc
            .set_viewport(smithay::utils::Size::from((geo.size.w, geo.size.h)));

        // A resize can move a figure to a different page (Typst's page
        // model depends on the page size, and the letterbox changes the
        // scale), so re-run the layout and reconfigure whatever moved.
        let before: Vec<(String, i32, i32)> = self
            .doc
            .figures()
            .figures()
            .iter()
            .map(|f| (f.key.clone(), f.spec.w, f.spec.h))
            .collect();
        let released = self.doc.relayout();
        self.close_apps_whose_figure_is_gone(&released);
        for (key, w, h) in &before {
            let changed = self
                .doc
                .figures()
                .get(key)
                .is_some_and(|f| (f.spec.w, f.spec.h) != (*w, *h));
            if changed {
                crate::handlers::apps::reconfigure_after_resize(self, key);
            }
        }
        self.needs_redraw = true;
    }

    /// Full output size in logical pixels. Note this ignores the
    /// layer-shell non-exclusive zone on purpose: the document page is
    /// letterboxed inside the usable area (see `relayout_doc`), but the
    /// output itself is what the output's mode describes.
    pub fn usable_area(&self) -> Rectangle<i32, Logical> {
        let Some(output) = self.page.active_space.outputs().next() else {
            return Rectangle::default();
        };
        let Some(mode) = output.current_mode() else {
            return Rectangle::default();
        };
        let scale = output.current_scale().fractional_scale();
        Rectangle::new(
            (0, 0).into(),
            mode.size.to_f64().to_logical(scale).to_i32_round(),
        )
    }

    /// Apply the window-manager's auto-focus policy when a new toplevel
    /// maps: grant it keyboard focus.
    ///
    /// Single entry point for xdg_shell `new_toplevel`. `_window_id` is
    /// unused today — focus is reported by the figure click path — but
    /// kept in the signature so callers don't have to know that.
    pub fn auto_focus_new_window(&mut self, window: Window, _window_id: u64) {
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(window.into()), serial);
        }
        // Remember it so the WakeUp key can toggle back into whichever
        // figure the user was last in.
        self.focus.last_app_focus = self.seat.get_keyboard().and_then(|k| k.current_focus());
        self.needs_redraw = true;
    }

    /// Resolve a `wl_surface` to the keyboard focus target that owns it.
    /// Searches toplevels in the active `Space` and tracked popups.
    pub fn focus_target_for_surface(
        &self,
        surface: &WlSurface,
    ) -> Option<crate::KeyboardFocusTarget> {
        if let Some(window) = self
            .page
            .active_space
            .elements()
            .find(|w| w.wl_surface().as_deref().is_some_and(|s| s == surface))
            .cloned()
        {
            return Some(crate::KeyboardFocusTarget::from(window));
        }
        if let Some(popup) = self.wl.popups.find_popup(surface) {
            return Some(crate::KeyboardFocusTarget::from(popup));
        }
        None
    }

    /// Change the visible document page.
    ///
    /// Apps bound to figures on other pages keep running but stop
    /// receiving frame callbacks, so they idle instead of repainting a
    /// screenful nobody is looking at. Popups of off-page figures are
    /// dismissed — an orphaned popup surface keeps its client committing
    /// forever.
    pub fn goto_page(&mut self, page: usize) -> bool {
        if page == self.page.current_page {
            return false;
        }
        if page >= self.doc.page_count() {
            tracing::warn!(
                "goto_page({page}) out of range ({} pages)",
                self.doc.page_count()
            );
            return false;
        }

        for window in self.page.active_space.elements() {
            dismiss_popups_for_window(window);
        }

        self.page.current_page = page;
        self.focus.reset_on_page_switch();
        self.ime.reset_on_page_switch();
        self.cursor.reset_on_page_switch();
        // The document keeps its own copy of the page index (the layout owns
        // everything visual reads), so it has to be told, not merely consulted.
        self.doc.on_page_changed(page);

        self.ipc
            .send(crate::ipc::OutgoingMessage::PageChanged { page });

        // Clear pointer focus so stale hover events don't reach surfaces
        // that are now on a hidden page.
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.motion(
                self,
                None,
                &smithay::input::pointer::MotionEvent {
                    location: pointer.current_location(),
                    serial,
                    time: 0,
                },
            );
            pointer.frame(self);
        }

        tracing::info!("switched to page {page}");
        self.needs_redraw = true;
        true
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        // Figures first. A figure's app is composited over the page by
        // `figure_render`, not laid out by `Space`, so
        // `Space::element_under` cannot find it — and worse, it would
        // return whatever the `Space` happens to have mapped there,
        // which is often a *different* app. Same ordering as the old
        // mirror-input path, for the same reason.
        if let Some((wl, local)) = self.figure_surface_under(pos) {
            return Some((wl, local));
        }
        self.page
            .active_space
            .element_under(pos)
            .and_then(|(window, location)| {
                window
                    .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + location).to_f64()))
            })
    }

    /// The client surface under `pos` if `pos` lands inside a figure
    /// bound to one, and the position mapped into that surface's own
    /// coordinates.
    fn figure_surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let figure = self.doc.figure_at(pos)?;
        let rect = crate::handlers::apps::app_figure_rect(self, figure.app_id?)?;
        let src = self.apps.get(figure.app_id?)?.geometry?;
        let wl = self.apps.get(figure.app_id?)?.wl_surface()?;
        // The figure is an aspect-fit box around a surface of the app's
        // own committed size, so both an offset and a scale apply.
        let ratio =
            crate::apps::AppManager::aspect_fit_ratio(src.size.to_f64(), rect.size.to_f64());
        let origin = Point::new(f64::from(rect.loc.x), f64::from(rect.loc.y));
        let rel = pos - origin;
        let local = match ratio {
            Some(r) => src.loc.to_f64() + rel.downscale(r),
            None => src.loc.to_f64() + rel,
        };
        Some((wl, local))
    }
}

impl crate::xwayland_satellite::HasXwls for EmthinState {
    fn xwls_mut(&mut self) -> Option<&mut crate::xwayland_satellite::XwlsIntegration> {
        self.xwayland.integration_mut()
    }
}

/// Data associated with each wayland client connection.
#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

/// Dismiss all popups attached to the given toplevel surface.
fn dismiss_popups_for_window(window: &Window) {
    let Some(toplevel) = window.toplevel() else {
        return;
    };
    let surface = toplevel.wl_surface();
    for (popup, _) in PopupManager::popups_for_surface(surface) {
        let _ = PopupManager::dismiss_popup(surface, &popup);
    }
}

#[cfg(test)]
mod focus_state_tests {
    use super::*;

    // KeyboardFocusTarget wraps smithay types that need a live Wayland
    // Display+Client to construct, so we test with `None` saved-focus
    // (which is itself a valid sentinel: "override active, nothing was
    // focused at the moment it fired").

    #[test]
    fn default_has_no_active_overrides() {
        let f = FocusState::default();
        assert!(!f.is_active(FocusOverride::Host));
    }

    #[test]
    fn enter_then_is_active() {
        let mut f = FocusState::default();
        f.enter(FocusOverride::Host, None);
        assert!(f.is_active(FocusOverride::Host));
    }

    #[test]
    fn exit_returns_saved_and_clears() {
        let mut f = FocusState::default();
        f.enter(FocusOverride::Host, None);
        let saved = f.exit(FocusOverride::Host);
        assert_eq!(saved, Some(None), "exit returns the saved Option");
        assert!(!f.is_active(FocusOverride::Host));
    }

    #[test]
    fn exit_inactive_returns_none() {
        let mut f = FocusState::default();
        assert_eq!(f.exit(FocusOverride::Host), None);
    }

    #[test]
    fn reset_clears_all_overrides_and_last_app() {
        let mut f = FocusState::default();
        f.enter(FocusOverride::Host, None);
        f.last_app_focus = None; // simulate some prior state
        f.reset_on_page_switch();
        assert!(!f.is_active(FocusOverride::Host));
        assert!(f.last_app_focus.is_none());
    }
}
