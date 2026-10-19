//! Host keyboard keymap passthrough for the nested compositor.
//!
//! smithay's winit backend forwards only the raw scancode of a host key
//! event (`WinitKeyboardInputEvent::key_code`), not the keysym the host
//! keymap translated it to, so the seat re-translates every key with the
//! keymap *emthin* compiles. When the two keymaps disagree — a German
//! layout host, or the synthetic per-invocation keymap `wtype` installs —
//! the same physical key produces the wrong character in the document and
//! in every embedded client (they share the seat's keymap).
//!
//! The fix prescribed under "Key Gotchas" in AGENTS.md: connect a separate
//! wayland-client on winit's host connection, snoop `wl_keyboard.keymap`,
//! and feed it to the seat with `KeyboardHandle::set_keymap_from_string`.
//! The listener needs no surface or focus of its own — the compositor
//! sends the keymap on `wl_keyboard` creation and whenever the keymap
//! changes (layout switch, virtual keyboard activation).
//!
//! Why the foreign-display piggyback (the same pattern as
//! `emthin-clipboard`'s wl_data_device fallback): winit owns the socket
//! and drains it as part of its own dispatch cycle, so by the time a
//! host key event reaches `process_input_event`, the keymap event queued
//! ahead of it is already demuxed into our queue's buffer. Dispatching
//! there — instead of on a side thread racing the input path — means a
//! keymap change is applied before the very key it garbles is
//! translated.

use std::ffi::c_void;
use std::os::fd::OwnedFd;

use wayland_client::backend::Backend;
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_registry;
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};

/// State for the wayland-client Dispatch callbacks.
struct ListenerState {
    seat: Option<WlSeat>,
    keyboard: Option<WlKeyboard>,
    /// A keymap that arrived but has not been applied to the seat yet.
    pending_keymap: Option<String>,
}

/// Host-side `wl_keyboard` snoop; see the module docs.
pub struct HostKeymapListener {
    #[allow(dead_code)]
    connection: Connection,
    queue: EventQueue<ListenerState>,
    inner: ListenerState,
}

impl HostKeymapListener {
    /// Connect to the host compositor on `display_ptr` and bind a
    /// `wl_keyboard` to snoop the host keymap. Returns `None` when the
    /// host exposes no `wl_seat` or a roundtrip fails (on X11 hosts
    /// `host_wl_display_ptr` is `None` to begin with).
    ///
    /// # Safety
    /// `display_ptr` must be a valid `*mut wl_display` that stays live
    /// for the listener's entire lifetime — the same contract as the
    /// `clipboard_bridge` foreign-display backends. Storing the listener
    /// in `EmthinState::host_keymap` (declared after `backend`) discharges
    /// it: default field-drop order releases the listener first.
    pub unsafe fn connect(display_ptr: *mut c_void) -> Option<Self> {
        // SAFETY: forwarded from the caller's contract.
        let backend = unsafe { Backend::from_foreign_display(display_ptr.cast()) };
        let connection = Connection::from_backend(backend);
        let mut queue = connection.new_event_queue::<ListenerState>();
        let qh = queue.handle();

        let _registry = connection.display().get_registry(&qh, ());
        let mut inner = ListenerState {
            seat: None,
            keyboard: None,
            pending_keymap: None,
        };

        // Roundtrip 1: discover wl_seat + its capabilities. The keyboard is
        // created by the Capabilities handler, never unconditionally:
        // `get_keyboard` on a seat without the Keyboard capability is
        // wl_seat_error.missing_capability — a protocol error that kills
        // the connection. (A headless host can legitimately have no
        // keyboard at startup: the harness's sway gains one only when
        // wtype installs its virtual keyboard.)
        if let Err(e) = queue.roundtrip(&mut inner) {
            tracing::debug!("host_keymap roundtrip 1 failed: {e}");
            return None;
        }
        inner.seat.as_ref()?;

        // Roundtrip 2: the initial keymap (sent on wl_keyboard creation,
        // if the capability was already there).
        if let Err(e) = queue.roundtrip(&mut inner) {
            tracing::debug!("host_keymap roundtrip 2 failed: {e}");
            return None;
        }
        tracing::info!(
            "host keymap listener ready (keyboard={}, initial keymap {})",
            if inner.keyboard.is_some() {
                "attached"
            } else {
                "awaited"
            },
            if inner.pending_keymap.is_some() {
                "received"
            } else {
                "pending"
            }
        );
        Some(Self {
            connection,
            queue,
            inner,
        })
    }

    /// Pump the listener's queue and return the newest keymap not yet
    /// applied to the seat. Coalesces the usual create-then-change
    /// bursts into the last one; `None` when nothing new arrived.
    pub fn take_keymap(&mut self) -> Option<String> {
        if let Err(e) = self.queue.dispatch_pending(&mut self.inner) {
            tracing::warn!("host_keymap dispatch_pending error: {e}");
        }
        self.inner.pending_keymap.take()
    }
}

impl crate::EmthinState {
    /// Apply the newest host keymap the listener snooped (if any) to the
    /// seat. Called from the input path before a key is translated and
    /// once per event-loop tick, so a host layout switch lands before the
    /// next keystroke is interpreted — and is re-broadcast to embedded
    /// clients, which share the seat's keymap.
    pub fn sync_host_keymap(&mut self) {
        let Some(keymap) = self
            .host_keymap
            .as_mut()
            .and_then(HostKeymapListener::take_keymap)
        else {
            return;
        };
        // Every apply is a full keymap rebroadcast to all clients; skip
        // the unchanged case.
        if self.applied_host_keymap.as_deref() == Some(keymap.as_str()) {
            return;
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        match keyboard.set_keymap_from_string(self, keymap.clone()) {
            Ok(()) => {
                tracing::info!("adopted host keymap ({} bytes)", keymap.len());
                self.applied_host_keymap = Some(keymap);
            }
            Err(err) => {
                tracing::warn!("host keymap rejected ({err:?}); keeping the configured keymap");
            }
        }
    }
}

/// Decode a `wl_keyboard` keymap payload: the fd carries exactly `size`
/// bytes of `XKB_KEYMAP_FORMAT_TEXT_V1` text. Kept pure so it is unit
/// testable without a compositor.
fn parse_keymap_bytes(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text.starts_with("xkb_keymap") {
        Some(text.to_owned())
    } else {
        tracing::debug!("host keymap payload is not an xkb keymap; ignored");
        None
    }
}

/// Read the keymap fd to its end and decode it.
fn read_keymap_fd(fd: OwnedFd) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::from(fd).read_to_end(&mut bytes).ok()?;
    parse_keymap_bytes(&bytes)
}

// ---------------------------------------------------------------------------
// Dispatch impls
// ---------------------------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, ()> for ListenerState {
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
            if interface == "wl_seat" && state.seat.is_none() {
                state.seat = Some(registry.bind(name, version.min(5), qh, ()));
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for ListenerState {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event {
            let capabilities = wl_seat::Capability::from_bits_truncate(capabilities.into());
            if capabilities.contains(wl_seat::Capability::Keyboard) {
                if state.keyboard.is_none() {
                    state.keyboard = Some(seat.get_keyboard(qh, ()));
                }
            } else if let Some(keyboard) = state.keyboard.take() {
                if keyboard.version() >= 3 {
                    keyboard.release();
                }
            }
        }
    }
}

impl Dispatch<WlKeyboard, ()> for ListenerState {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Keymap { format, fd, .. } = event {
            if matches!(format, WEnum::Value(wl_keyboard::KeymapFormat::XkbV1)) {
                if let Some(keymap) = read_keymap_fd(fd) {
                    tracing::debug!("host keymap received ({} bytes)", keymap.len());
                    state.pending_keymap = Some(keymap);
                }
            } else {
                tracing::debug!("host keymap in unsupported format {format:?}; ignored");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::fd::OwnedFd;

    use super::{parse_keymap_bytes, read_keymap_fd};

    #[test]
    fn accepts_xkb_keymap_text() {
        let km = "xkb_keymap {\n\tkeycodes { };\n};\n";
        assert_eq!(parse_keymap_bytes(km.as_bytes()), Some(km.to_owned()));
    }

    #[test]
    fn rejects_non_keymap_payloads() {
        assert_eq!(parse_keymap_bytes(b"not a keymap"), None);
        // Invalid UTF-8.
        assert_eq!(parse_keymap_bytes(&[0xff, 0xfe]), None);
    }

    #[test]
    fn reads_keymap_from_fd() {
        let (mut writer, reader) = std::os::unix::net::UnixStream::pair().unwrap();
        let km = "xkb_keymap {\n};\n";
        writer.write_all(km.as_bytes()).unwrap();
        // Drop the writer so the reader sees EOF.
        drop(writer);
        let fd: OwnedFd = reader.into();
        assert_eq!(read_keymap_fd(fd), Some(km.to_owned()));
    }
}
