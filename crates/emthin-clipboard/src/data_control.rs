//! Wayland data-control backend (ext_data_control_v1 / zwlr_data_control_v1).
//!
//! Uses `ext_data_control_v1` (preferred) or `zwlr_data_control_manager_v1`
//! (fallback) to monitor and control the host's clipboard without requiring
//! keyboard focus. Owns its own Wayland connection via `$WAYLAND_DISPLAY`.

use std::collections::HashMap;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::Arc;

use wayland_client::backend::{ObjectData, ObjectId};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};

use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::{self, ExtDataControlManagerV1},
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::{self, ZwlrDataControlManagerV1},
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
    zwlr_data_control_source_v1::{self, ZwlrDataControlSourceV1},
};

use crate::backend::{ClipboardBackend, ClipboardEvent, Driver, SelectionKind};

// ---------------------------------------------------------------------------
// Protocol abstraction — enum wrappers over ext / wlr variants
// ---------------------------------------------------------------------------

enum DataControlManager {
    ExtDataControl(ExtDataControlManagerV1),
    WlrDataControl(ZwlrDataControlManagerV1),
}

impl DataControlManager {
    fn create_data_source(
        &self,
        qh: &QueueHandle<ClipboardState>,
        role: SourceRole,
    ) -> DataControlSource {
        match self {
            Self::ExtDataControl(m) => {
                DataControlSource::ExtDataControl(m.create_data_source(qh, role))
            }
            Self::WlrDataControl(m) => {
                DataControlSource::WlrDataControl(m.create_data_source(qh, role))
            }
        }
    }

    fn get_data_device(
        &self,
        seat: &wl_seat::WlSeat,
        qh: &QueueHandle<ClipboardState>,
    ) -> DataControlDevice {
        match self {
            Self::ExtDataControl(m) => {
                DataControlDevice::ExtDataControl(m.get_data_device(seat, qh, ()))
            }
            Self::WlrDataControl(m) => {
                DataControlDevice::WlrDataControl(m.get_data_device(seat, qh, ()))
            }
        }
    }

    fn protocol_name(&self) -> &'static str {
        match self {
            Self::ExtDataControl(_) => "ext_data_control_v1",
            Self::WlrDataControl(_) => "zwlr_data_control_v1",
        }
    }
}

enum DataControlDevice {
    ExtDataControl(ExtDataControlDeviceV1),
    WlrDataControl(ZwlrDataControlDeviceV1),
}

impl DataControlDevice {
    fn set_selection(&self, source: Option<&DataControlSource>) {
        match (self, source) {
            (Self::ExtDataControl(d), Some(DataControlSource::ExtDataControl(s))) => {
                d.set_selection(Some(s));
            }
            (Self::ExtDataControl(d), None) => d.set_selection(None),
            (Self::WlrDataControl(d), Some(DataControlSource::WlrDataControl(s))) => {
                d.set_selection(Some(s));
            }
            (Self::WlrDataControl(d), None) => d.set_selection(None),
            _ => unreachable!("protocol variant mismatch"),
        }
    }

    fn set_primary_selection(&self, source: Option<&DataControlSource>) {
        match (self, source) {
            (Self::ExtDataControl(d), Some(DataControlSource::ExtDataControl(s))) => {
                d.set_primary_selection(Some(s));
            }
            (Self::ExtDataControl(d), None) => d.set_primary_selection(None),
            (Self::WlrDataControl(d), Some(DataControlSource::WlrDataControl(s))) => {
                d.set_primary_selection(Some(s));
            }
            (Self::WlrDataControl(d), None) => d.set_primary_selection(None),
            _ => unreachable!("protocol variant mismatch"),
        }
    }
}

enum DataControlOffer {
    ExtDataControl(ExtDataControlOfferV1),
    WlrDataControl(ZwlrDataControlOfferV1),
}

impl DataControlOffer {
    fn id(&self) -> ObjectId {
        match self {
            Self::ExtDataControl(o) => o.id(),
            Self::WlrDataControl(o) => o.id(),
        }
    }

    fn receive(&self, mime_type: String, fd: BorrowedFd<'_>) {
        match self {
            Self::ExtDataControl(o) => o.receive(mime_type, fd),
            Self::WlrDataControl(o) => o.receive(mime_type, fd),
        }
    }

    fn destroy(&self) {
        match self {
            Self::ExtDataControl(o) => o.destroy(),
            Self::WlrDataControl(o) => o.destroy(),
        }
    }
}

enum DataControlSource {
    ExtDataControl(ExtDataControlSourceV1),
    WlrDataControl(ZwlrDataControlSourceV1),
}

impl DataControlSource {
    fn offer(&self, mime_type: String) {
        match self {
            Self::ExtDataControl(s) => s.offer(mime_type),
            Self::WlrDataControl(s) => s.offer(mime_type),
        }
    }

    fn destroy(&self) {
        match self {
            Self::ExtDataControl(s) => s.destroy(),
            Self::WlrDataControl(s) => s.destroy(),
        }
    }
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

/// Role tag stored as user data on data control source objects.
#[derive(Clone, Debug)]
enum SourceRole {
    Clipboard,
    Primary,
}

/// State for wayland-client Dispatch callbacks.
struct ClipboardState {
    manager: Option<DataControlManager>,
    device: Option<DataControlDevice>,
    seat: Option<wl_seat::WlSeat>,

    clipboard_offer: Option<DataControlOffer>,
    primary_offer: Option<DataControlOffer>,

    /// Offers being assembled (MIME types accumulating via offer events).
    pending_offers: HashMap<ObjectId, Vec<String>>,

    clipboard_source: Option<DataControlSource>,
    primary_source: Option<DataControlSource>,

    events: Vec<ClipboardEvent>,

    /// Anti-loop: number of host selection echo events to suppress.
    /// Incremented each time we call set_selection/set_primary_selection on
    /// the host; decremented when the corresponding echo event arrives.
    /// A counter (not a bool) handles clients that set selection multiple
    /// times rapidly (e.g. Firefox sets twice — once without SAVE_TARGETS,
    /// then again with it).
    suppress_clipboard: u32,
    suppress_primary: u32,
}

impl Drop for ClipboardState {
    fn drop(&mut self) {
        for offer in [self.clipboard_offer.take(), self.primary_offer.take()]
            .into_iter()
            .flatten()
        {
            offer.destroy();
        }
        for source in [self.clipboard_source.take(), self.primary_source.take()]
            .into_iter()
            .flatten()
        {
            source.destroy();
        }
    }
}

// ---------------------------------------------------------------------------
// ClipboardState — shared event handlers (protocol-agnostic logic)
// ---------------------------------------------------------------------------

/// Decide whether a host selection event is our own echo coming back.
///
/// A counter, not a bool: a client may set the selection more than once in quick
/// succession (Firefox does, once without `SAVE_TARGETS` and again with it), and
/// a bool would eat the *second* echo too and lose a real host change.
///
/// Returns true when the caller should emit the change, false when this event is
/// the echo of something we just sent and must stay invisible. Decrements the
/// counter either way it consumes one.
fn claim_echo(suppress: &mut u32) -> bool {
    match *suppress {
        0 => true,
        _ => {
            *suppress -= 1;
            false
        }
    }
}

/// Close out an offer sequence: return the MIME types belonging to the offer the
/// selection event names, and drop every other entry as stale.
///
/// The clear is not tidiness. A `selection` event finalizes the sequence, and
/// `pending_offers` is also fed by drag-and-drop, so anything left over belongs
/// to an offer that will never be selected. Leaving it in the map would let a
/// later selection pick up MIME types from an unrelated offer.
///
/// `ObjectId` trips clippy's `mutable_key_type` because it holds an
/// `Arc<Atomic<bool>>` liveness flag. That flag is not part of the object's
/// identity: wayland-backend documents that two ids compare equal only when they
/// represent the same protocol object, and recycles ids on destruction, so its
/// `Eq`/`Hash` are exactly what a map key needs. The lint only appeared once the
/// type showed up in a signature -- as a struct field it never fired.
#[allow(clippy::mutable_key_type)]
fn finalize_offers(
    pending: &mut HashMap<ObjectId, Vec<String>>,
    selected: Option<ObjectId>,
) -> Vec<String> {
    let mime_types = selected
        .and_then(|id| pending.remove(&id))
        .unwrap_or_default();
    pending.clear();
    mime_types
}

impl ClipboardState {
    fn source_and_suppress(
        &mut self,
        kind: SelectionKind,
    ) -> (&mut Option<DataControlSource>, &mut u32) {
        match kind {
            SelectionKind::Clipboard => (&mut self.clipboard_source, &mut self.suppress_clipboard),
            SelectionKind::Primary => (&mut self.primary_source, &mut self.suppress_primary),
        }
    }

    fn on_data_offer(&mut self, id: ObjectId) {
        self.pending_offers.insert(id, Vec::new());
    }

    fn on_offer_mime(&mut self, offer_id: ObjectId, mime_type: String) {
        if let Some(pending) = self.pending_offers.get_mut(&offer_id) {
            pending.push(mime_type);
        }
    }

    fn on_selection(&mut self, kind: SelectionKind, new_offer: Option<DataControlOffer>) {
        let mime_types = finalize_offers(
            &mut self.pending_offers,
            new_offer.as_ref().map(DataControlOffer::id),
        );

        let (offer_slot, suppress) = match kind {
            SelectionKind::Clipboard => (&mut self.clipboard_offer, &mut self.suppress_clipboard),
            SelectionKind::Primary => (&mut self.primary_offer, &mut self.suppress_primary),
        };

        if let Some(old) = offer_slot.take() {
            old.destroy();
        }
        *offer_slot = new_offer;

        if !claim_echo(suppress) {
            return;
        }

        self.events
            .push(ClipboardEvent::HostSelectionChanged { kind, mime_types });
    }

    fn on_device_finished(&mut self) {
        tracing::warn!("Data control device finished (seat destroyed?)");
        self.device = None;
    }

    fn on_source_send(&mut self, role: &SourceRole, mime_type: String, fd: OwnedFd) {
        let kind = match role {
            SourceRole::Clipboard => SelectionKind::Clipboard,
            SourceRole::Primary => SelectionKind::Primary,
        };
        self.events.push(ClipboardEvent::HostSendRequest {
            kind,
            mime_type,
            write_fd: fd,
            completion: None,
        });
    }

    fn on_source_cancelled(&mut self, role: &SourceRole) {
        let (kind, source_slot) = match role {
            SourceRole::Clipboard => (SelectionKind::Clipboard, &mut self.clipboard_source),
            SourceRole::Primary => (SelectionKind::Primary, &mut self.primary_source),
        };
        if let Some(s) = source_slot.take() {
            s.destroy();
        }
        self.events.push(ClipboardEvent::SourceCancelled { kind });
    }
}

// ---------------------------------------------------------------------------
// ClipboardProxy — public API
// ---------------------------------------------------------------------------

/// Data-control backend proxy.
pub(crate) struct ClipboardProxy {
    connection: Connection,
    queue: EventQueue<ClipboardState>,
    inner: ClipboardState,
}

impl ClipboardProxy {
    /// Connect to the host compositor and set up data control protocol.
    ///
    /// Prefers `ext_data_control_v1`, falls back to `zwlr_data_control_manager_v1`.
    /// Returns `None` if neither is supported.
    pub(crate) fn new() -> Option<Self> {
        let conn = match Connection::connect_to_env() {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("Cannot connect to host Wayland for clipboard: {e}");
                return None;
            }
        };
        let mut queue = conn.new_event_queue::<ClipboardState>();
        let qh = queue.handle();

        let _registry = conn.display().get_registry(&qh, ());

        let mut state = ClipboardState {
            manager: None,
            device: None,
            seat: None,
            clipboard_offer: None,
            primary_offer: None,
            pending_offers: HashMap::new(),
            clipboard_source: None,
            primary_source: None,
            events: Vec::new(),
            suppress_clipboard: 0,
            suppress_primary: 0,
        };

        // Roundtrip 1: discover globals (manager + seat)
        if let Err(e) = queue.roundtrip(&mut state) {
            tracing::warn!("Clipboard roundtrip 1 failed: {e}");
            return None;
        }

        // Create data control device from manager + seat
        if let (Some(ref manager), Some(ref seat)) = (&state.manager, &state.seat) {
            state.device = Some(manager.get_data_device(seat, &qh));
        }

        if state.device.is_none() {
            tracing::warn!("Host supports neither ext_data_control_v1 nor zwlr_data_control_v1");
            return None;
        }

        let protocol_name = state
            .manager
            .as_ref()
            .map(|m| m.protocol_name())
            .unwrap_or("unknown");

        // Roundtrip 2: receive initial selection events
        if let Err(e) = queue.roundtrip(&mut state) {
            tracing::warn!("Clipboard roundtrip 2 failed: {e}");
            return None;
        }

        tracing::info!("Clipboard sync initialized ({protocol_name})");
        Some(Self {
            connection: conn,
            queue,
            inner: state,
        })
    }

    fn flush(&self) {
        if let Err(e) = self.connection.flush() {
            tracing::warn!("clipboard flush error: {e}");
        }
    }
}

impl ClipboardBackend for ClipboardProxy {
    fn driver(&self) -> Driver<'_> {
        Driver::OwnedFd(self.connection.as_fd())
    }

    fn dispatch(&mut self) {
        if let Some(guard) = self.queue.prepare_read() {
            if let Err(e) = guard.read() {
                tracing::warn!("clipboard read error: {e}");
            }
        }
        if let Err(e) = self.queue.dispatch_pending(&mut self.inner) {
            tracing::warn!("clipboard dispatch error: {e}");
        }
        // No flush here — this is a read path. Write paths (receive_from_host,
        // set_host_selection, clear_host_selection) flush after sending their
        // requests.
    }

    fn take_events(&mut self) -> Vec<ClipboardEvent> {
        std::mem::take(&mut self.inner.events)
    }

    fn receive_from_host(&mut self, kind: SelectionKind, mime_type: &str, fd: OwnedFd) {
        let offer = match kind {
            SelectionKind::Clipboard => self.inner.clipboard_offer.as_ref(),
            SelectionKind::Primary => self.inner.primary_offer.as_ref(),
        };
        if let Some(offer) = offer {
            offer.receive(mime_type.to_string(), fd.as_fd());
            self.flush();
        } else {
            tracing::warn!("receive_from_host: no active {kind:?} offer, fd dropped");
        }
    }

    fn set_host_selection(&mut self, kind: SelectionKind, mime_types: &[String]) {
        let Some(ref manager) = self.inner.manager else {
            return;
        };
        let Some(ref device) = self.inner.device else {
            return;
        };

        let qh = self.queue.handle();
        let role = match kind {
            SelectionKind::Clipboard => SourceRole::Clipboard,
            SelectionKind::Primary => SourceRole::Primary,
        };

        let source = manager.create_data_source(&qh, role);
        for mime in mime_types {
            source.offer(mime.clone());
        }

        match kind {
            SelectionKind::Clipboard => device.set_selection(Some(&source)),
            SelectionKind::Primary => device.set_primary_selection(Some(&source)),
        }
        let (source_slot, suppress) = self.inner.source_and_suppress(kind);
        if let Some(old) = source_slot.replace(source) {
            old.destroy();
        }
        *suppress += 1;
        self.flush();
    }

    fn clear_host_selection(&mut self, kind: SelectionKind) {
        let Some(ref device) = self.inner.device else {
            return;
        };

        match kind {
            SelectionKind::Clipboard => device.set_selection(None),
            SelectionKind::Primary => device.set_primary_selection(None),
        }
        let (source_slot, suppress) = self.inner.source_and_suppress(kind);
        if let Some(old) = source_slot.take() {
            old.destroy();
        }
        *suppress += 1;
        self.flush();
    }
}

// ---------------------------------------------------------------------------
// Dispatch — wl_registry (shared, binds ext or wlr manager)
// ---------------------------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, ()> for ClipboardState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "ext_data_control_manager_v1" if state.manager.is_none() => {
                    let proxy = registry.bind::<ExtDataControlManagerV1, _, _>(name, 1, qh, ());
                    state.manager = Some(DataControlManager::ExtDataControl(proxy));
                }
                "zwlr_data_control_manager_v1" if state.manager.is_none() => {
                    let proxy = registry.bind::<ZwlrDataControlManagerV1, _, _>(
                        name,
                        version.min(3),
                        qh,
                        (),
                    );
                    state.manager = Some(DataControlManager::WlrDataControl(proxy));
                }
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(1), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for ClipboardState {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// ---------------------------------------------------------------------------
// Dispatch — ext_data_control_v1 (thin wrappers → shared ClipboardState methods)
// ---------------------------------------------------------------------------

impl Dispatch<ExtDataControlManagerV1, ()> for ClipboardState {
    fn event(
        _: &mut Self,
        _: &ExtDataControlManagerV1,
        _: ext_data_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for ClipboardState {
    fn event(
        state: &mut Self,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_data_control_device_v1::Event;
        match event {
            Event::DataOffer { id } => state.on_data_offer(id.id()),
            Event::Selection { id } => state.on_selection(
                SelectionKind::Clipboard,
                id.map(DataControlOffer::ExtDataControl),
            ),
            Event::PrimarySelection { id } => state.on_selection(
                SelectionKind::Primary,
                id.map(DataControlOffer::ExtDataControl),
            ),
            Event::Finished => state.on_device_finished(),
            _ => {}
        }
    }

    fn event_created_child(opcode: u16, qh: &QueueHandle<Self>) -> Arc<dyn ObjectData> {
        assert_eq!(opcode, 0, "unexpected child-creating opcode");
        qh.make_data::<ExtDataControlOfferV1, ()>(())
    }
}

impl Dispatch<ExtDataControlOfferV1, ()> for ClipboardState {
    fn event(
        state: &mut Self,
        offer: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.on_offer_mime(offer.id(), mime_type);
        }
    }
}

impl Dispatch<ExtDataControlSourceV1, SourceRole> for ClipboardState {
    fn event(
        state: &mut Self,
        _: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        role: &SourceRole,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                state.on_source_send(role, mime_type, fd)
            }
            ext_data_control_source_v1::Event::Cancelled => state.on_source_cancelled(role),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Dispatch — zwlr_data_control_v1 (thin wrappers → same ClipboardState methods)
// ---------------------------------------------------------------------------

impl Dispatch<ZwlrDataControlManagerV1, ()> for ClipboardState {
    fn event(
        _: &mut Self,
        _: &ZwlrDataControlManagerV1,
        _: zwlr_data_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for ClipboardState {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_data_control_device_v1::Event;
        match event {
            Event::DataOffer { id } => state.on_data_offer(id.id()),
            Event::Selection { id } => state.on_selection(
                SelectionKind::Clipboard,
                id.map(DataControlOffer::WlrDataControl),
            ),
            Event::PrimarySelection { id } => state.on_selection(
                SelectionKind::Primary,
                id.map(DataControlOffer::WlrDataControl),
            ),
            Event::Finished => state.on_device_finished(),
            _ => {}
        }
    }

    fn event_created_child(opcode: u16, qh: &QueueHandle<Self>) -> Arc<dyn ObjectData> {
        assert_eq!(opcode, 0, "unexpected child-creating opcode");
        qh.make_data::<ZwlrDataControlOfferV1, ()>(())
    }
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for ClipboardState {
    fn event(
        state: &mut Self,
        offer: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.on_offer_mime(offer.id(), mime_type);
        }
    }
}

impl Dispatch<ZwlrDataControlSourceV1, SourceRole> for ClipboardState {
    fn event(
        state: &mut Self,
        _: &ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        role: &SourceRole,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_source_v1::Event::Send { mime_type, fd } => {
                state.on_source_send(role, mime_type, fd)
            }
            zwlr_data_control_source_v1::Event::Cancelled => state.on_source_cancelled(role),
            _ => {}
        }
    }
}

#[cfg(test)]
#[allow(clippy::mutable_key_type)] // see finalize_offers
mod tests {
    use super::*;

    fn state() -> ClipboardState {
        ClipboardState {
            manager: None,
            device: None,
            seat: None,
            clipboard_offer: None,
            primary_offer: None,
            pending_offers: HashMap::new(),
            clipboard_source: None,
            primary_source: None,
            events: Vec::new(),
            suppress_clipboard: 0,
            suppress_primary: 0,
        }
    }

    // -- claim_echo ---------------------------------------------------------

    #[test]
    fn nothing_is_suppressed_when_no_echo_is_outstanding() {
        let mut n = 0;
        assert!(claim_echo(&mut n), "an unsuppressed change must be emitted");
        assert_eq!(0, n, "emitting must not consume a slot that was not ours");
    }

    #[test]
    fn one_echo_is_swallowed_and_the_next_change_gets_through() {
        let mut n = 1;
        assert!(!claim_echo(&mut n), "our own echo must not bounce back");
        assert_eq!(0, n);
        assert!(
            claim_echo(&mut n),
            "a real host change after one echo must not be eaten as well"
        );
    }

    #[test]
    fn consecutive_echoes_are_swallowed_one_for_one() {
        // Firefox sets the selection twice: once without SAVE_TARGETS, again with
        // it. Both echoes are ours; the third event is the host's.
        let mut n = 2;
        assert!(!claim_echo(&mut n));
        assert_eq!(1, n);
        assert!(!claim_echo(&mut n));
        assert_eq!(0, n);
        assert!(claim_echo(&mut n));
    }

    // -- finalize_offers ----------------------------------------------------

    #[test]
    fn the_selected_offers_types_are_returned() {
        let mut pending = HashMap::new();
        pending.insert(ObjectId::null(), vec!["text/plain".to_string()]);
        assert_eq!(
            vec!["text/plain".to_string()],
            finalize_offers(&mut pending, Some(ObjectId::null()))
        );
    }

    #[test]
    fn a_selection_with_no_offer_yields_no_types() {
        let mut pending = HashMap::new();
        assert!(finalize_offers(&mut pending, None).is_empty());
    }

    #[test]
    fn finalizing_drops_types_left_over_from_earlier_offers() {
        let mut pending = HashMap::new();
        pending.insert(ObjectId::null(), vec!["text/plain".to_string()]);
        finalize_offers(&mut pending, None);
        assert!(
            pending.is_empty(),
            "an unselected offer must not stay in the map for a later selection \
             to pick up MIME types from"
        );
    }

    #[test]
    fn types_are_collected_in_the_order_they_were_offered() {
        let mut pending = HashMap::new();
        let types: Vec<String> = (0..3).map(|i| format!("type/{i}")).collect();
        let expected = types.clone();
        pending.insert(ObjectId::null(), types);
        assert_eq!(
            expected,
            finalize_offers(&mut pending, Some(ObjectId::null()))
        );
    }

    // -- routing ------------------------------------------------------------

    #[test]
    fn clipboard_and_primary_have_independent_suppress_counters() {
        let mut st = state();
        st.suppress_clipboard = 1;
        st.suppress_primary = 0;

        // Scoped separately: each call borrows the state mutably, so the two
        // cannot be live at once.
        {
            let (_slot, suppress) = st.source_and_suppress(SelectionKind::Clipboard);
            assert_eq!(1, *suppress, "only the clipboard echo is outstanding");
        }
        {
            let (_slot, suppress) = st.source_and_suppress(SelectionKind::Primary);
            assert_eq!(0, *suppress);
        }
    }

    // ObjectId can only be null() from outside the wayland crate, so the
    // multi-offer case (a DnD offer that never gets selected) cannot be built
    // here. It is covered end to end by scripts/e2e/clip-bisect.sh instead.
}
