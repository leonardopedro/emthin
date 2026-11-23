//! Desktop Core session sources.
//!
//! A `terminal:` figure names a Core terminal, not a Wayland client. This
//! module owns the parser/renderer for those terminals and exposes their
//! pixels to the figure compositor. It does not open sockets: the caller feeds
//! attach events from `emthin-core-client` into the matching source.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use emthin_core_client::unix::{Connection, TransportError};
use emthin_core_client::{controller::Endpoint, controller::Session, AttachRequest, Client, Event};
use emthin_vt::render::{Color, Image, Metrics, Renderer};
use emthin_vt::{DejaVuFont, Pane};
use mathed_core::figures::TerminalRef;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            gles::GlesRenderer,
        },
    },
    reexports::calloop::{
        generic::Generic, Interest, LoopHandle, Mode, PostAction, RegistrationToken,
    },
    utils::{Logical, Point, Rectangle, Size, Transform},
};

use crate::element::CustomElement;
use crate::EmthinState;

const CELL_WIDTH: u16 = 8;
const CELL_HEIGHT: u16 = 16;
const MAX_ATTACH_ATTEMPTS: u32 = 5;
const RETRY_BASE_DELAY: Duration = Duration::from_millis(250);
const RETRY_MAX_DELAY: Duration = Duration::from_secs(8);

/// One Core terminal's VT state, rendered image and viewer lease.
pub struct CoreSessionSource {
    reference: TerminalRef,
    pane: Pane,
    renderer: Renderer,
    font: DejaVuFont,
    image: Option<Image>,
    revision: u64,
    session: Session,
    buffer: Option<MemoryRenderBuffer>,
    buffer_revision: u64,
}

impl CoreSessionSource {
    pub fn new(
        endpoint: Endpoint,
        reference: TerminalRef,
        channel: u32,
        cols: u16,
        rows: u16,
    ) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        Self {
            session: Session::new(endpoint, reference.canonical()),
            reference,
            pane: Pane::new(channel, cols, rows),
            renderer: Renderer::new(
                Metrics {
                    cell_width: CELL_WIDTH,
                    cell_height: CELL_HEIGHT,
                    baseline: 3,
                },
                8 * 1024 * 1024,
                1024,
            ),
            font: DejaVuFont::new(),
            image: None,
            revision: 0,
            buffer: None,
            buffer_revision: u64::MAX,
        }
    }

    pub fn reference(&self) -> &TerminalRef {
        &self.reference
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }

    pub fn pane(&self) -> &Pane {
        &self.pane
    }

    pub fn pane_mut(&mut self) -> &mut Pane {
        &mut self.pane
    }

    pub fn image(&self) -> Option<&Image> {
        self.image.as_ref()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Feed one attach event. Returns generated replies owed to Core when the
    /// event changed this source; unrelated channels are ignored.
    pub fn apply(&mut self, event: &Event) -> Option<Vec<u8>> {
        let paint = self.pane.handle(event)?;
        self.renderer
            .update(&paint.screen, &paint.damage, &mut self.font);
        self.image = Some(self.renderer.compose(Color([18, 22, 28, 255])));
        self.revision = self.revision.saturating_add(1);
        Some(paint.replies)
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Option<Vec<u8>> {
        self.apply(&Event::Resize {
            channel: self.pane.channel(),
            cols,
            rows,
        })
    }

    /// Key bytes for the program's current cursor/kitty modes. Returns empty
    /// for keys this desktop bridge does not translate.
    pub fn key_bytes(&self, keysym: u32, mods: u16) -> Vec<u8> {
        let Some((key, ch)) = ghostty_key(keysym) else {
            return Vec::new();
        };
        let text = if mods & (2 | 4) != 0 || ch == '\0' {
            String::new()
        } else {
            ch.to_string()
        };
        self.pane
            .encode_key(key, mods, &text, unshifted_char(ch) as u32)
    }

    pub fn mouse_button(
        &self,
        rect: Rectangle<i32, Logical>,
        pos: Point<f64, Logical>,
        button: u8,
        press: bool,
        mods: (bool, bool, bool),
    ) -> Vec<u8> {
        let (x, y) = self.cell_at(rect, pos);
        self.pane
            .mouse_button(x, y, button, press, modifier_bits(mods))
    }

    pub fn mouse_motion(
        &self,
        rect: Rectangle<i32, Logical>,
        pos: Point<f64, Logical>,
        button: Option<u8>,
        mods: (bool, bool, bool),
    ) -> Vec<u8> {
        let (x, y) = self.cell_at(rect, pos);
        self.pane.mouse_motion(x, y, button, modifier_bits(mods))
    }

    /// One wheel notch: a program that requested mouse reports gets the
    /// report, otherwise the pane scrolls its retained history.
    pub fn wheel(&mut self, up: bool) -> Vec<u8> {
        let bytes = self.pane.wheel(up);
        if !bytes.is_empty() {
            return bytes;
        }
        self.scroll(if up { -3 } else { 3 });
        Vec::new()
    }

    pub fn scroll(&mut self, rows: i32) {
        let Some(paint) = self.pane.scroll(rows) else {
            return;
        };
        self.renderer
            .update(&paint.screen, &paint.damage, &mut self.font);
        self.image = Some(self.renderer.compose(Color([18, 22, 28, 255])));
        self.revision = self.revision.saturating_add(1);
    }

    fn cell_at(&self, rect: Rectangle<i32, Logical>, pos: Point<f64, Logical>) -> (u16, u16) {
        let (cols, rows) = self.pane.viewport();
        let x = ((pos.x - f64::from(rect.loc.x)) / f64::from(CELL_WIDTH)).floor();
        let y = ((pos.y - f64::from(rect.loc.y)) / f64::from(CELL_HEIGHT)).floor();
        (
            x.clamp(0.0, f64::from(cols.saturating_sub(1))) as u16,
            y.clamp(0.0, f64::from(rows.saturating_sub(1))) as u16,
        )
    }

    /// The cached GPU element for this terminal at `rect`.
    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        rect: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Option<CustomElement<GlesRenderer>> {
        let image = self.image.as_ref()?;
        if image.width == 0 || image.height == 0 || rect.size.w <= 0 || rect.size.h <= 0 {
            return None;
        }
        if self.buffer.is_none() || self.buffer_revision != self.revision {
            self.buffer = Some(MemoryRenderBuffer::from_slice(
                &image.pixels,
                Fourcc::Abgr8888,
                Size::new(image.width as i32, image.height as i32),
                1,
                Transform::Normal,
                None,
            ));
            self.buffer_revision = self.revision;
        }
        let buffer = self.buffer.as_ref()?;
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            rect.loc.to_f64().to_physical(scale),
            buffer,
            Some(1.0),
            Some(Rectangle::from_size(
                (f64::from(image.width), f64::from(image.height)).into(),
            )),
            Some(rect.size),
            Kind::Unspecified,
        )
        .map_err(|error| tracing::warn!("terminal figure texture import failed: {error}"))
        .ok()
        .map(Into::into)
    }
}

struct LiveSession {
    client: Client,
    connection: Connection,
    channel: u32,
    token: Option<RegistrationToken>,
}

struct RetryState {
    attempts: u32,
    next_at: Instant,
}

/// All desktop Core sources, keyed by canonical terminal reference.
#[derive(Default)]
pub struct CoreSessions {
    sources: BTreeMap<String, CoreSessionSource>,
    live: BTreeMap<String, LiveSession>,
    retries: BTreeMap<String, RetryState>,
    focused: Option<String>,
    socket: Option<PathBuf>,
    loop_handle: Option<LoopHandle<'static, EmthinState>>,
}

impl CoreSessions {
    /// The local Core endpoint used for live terminal figures. Without it,
    /// sources remain caller-fed (the headless/test path).
    pub fn set_socket(&mut self, socket: Option<PathBuf>) {
        self.socket = socket;
        self.retries.clear();
    }

    pub fn socket(&self) -> Option<&Path> {
        self.socket.as_deref()
    }

    /// True when a failed live source is due for another attach attempt. The
    /// idle tick uses this to request the render pass that calls `ensure_live`.
    pub fn retry_due(&self) -> bool {
        self.retries
            .iter()
            .any(|(key, _)| retry_ready(&self.retries, key))
    }

    /// Human-readable reconnect state for a terminal figure overlay.
    pub fn retry_status(&self, reference: &TerminalRef) -> Option<String> {
        let retry = self.retries.get(&reference.canonical())?;
        if retry.attempts >= MAX_ATTACH_ATTEMPTS {
            return Some("Terminal disconnected — retries exhausted".to_owned());
        }
        Some(format!(
            "Terminal reconnecting — attempt {}/{} after {} ms",
            retry.attempts + 1,
            MAX_ATTACH_ATTEMPTS,
            retry_delay(retry.attempts).as_millis()
        ))
    }

    pub fn focused(&self) -> Option<&TerminalRef> {
        let key = self.focused.as_deref()?;
        self.sources.get(key).map(CoreSessionSource::reference)
    }

    pub fn focus(&mut self, reference: &TerminalRef) -> bool {
        let key = reference.canonical();
        if !self.sources.contains_key(&key) {
            return false;
        }
        self.focused = Some(key);
        true
    }

    pub fn clear_focus(&mut self) {
        self.focused = None;
    }

    /// Connect and attach a live figure, retrying a failed or closed endpoint
    /// with bounded exponential backoff. The compositor calls this on every
    /// render pass, so a not-yet-due retry is a no-op rather than a storm.
    pub fn ensure_live(
        &mut self,
        reference: &TerminalRef,
        rect: Rectangle<i32, Logical>,
        handle: &LoopHandle<'static, EmthinState>,
    ) {
        let key = reference.canonical();
        if self.live.contains_key(&key) || !retry_ready(&self.retries, &key) {
            return;
        }
        let Some(socket) = self.socket.clone() else {
            return;
        };
        let cols = (rect.size.w.max(1) as u16).div_ceil(CELL_WIDTH).max(1);
        let rows = (rect.size.h.max(1) as u16).div_ceil(CELL_HEIGHT).max(1);
        let (connection, client, channel) = match connect_live(&socket, reference, cols, rows) {
            Ok(attached) => attached,
            Err(error) => {
                tracing::warn!("terminal {key}: live attach failed: {error}");
                record_retry(&mut self.retries, &key);
                return;
            }
        };
        let stream = match connection.try_clone_stream() {
            Ok(stream) => stream,
            Err(error) => {
                tracing::warn!("terminal {key}: duplicate socket: {error}");
                record_retry(&mut self.retries, &key);
                return;
            }
        };
        let poll_key = key.clone();
        let token = match handle.insert_source(
            Generic::new(stream, Interest::READ, Mode::Level),
            move |_, _, state: &mut EmthinState| {
                state.core_sessions.poll_live(&poll_key);
                Ok(PostAction::Continue)
            },
        ) {
            Ok(token) => token,
            Err(error) => {
                tracing::warn!("terminal {key}: event-loop registration: {error}");
                record_retry(&mut self.retries, &key);
                return;
            }
        };
        let endpoint = Endpoint {
            core_id: core_id(reference),
            socket,
        };
        if self
            .open(endpoint, reference.clone(), channel, cols, rows)
            .is_none()
        {
            handle.remove(token);
            record_retry(&mut self.retries, &key);
            return;
        }
        self.retries.remove(&key);
        self.live.insert(
            key,
            LiveSession {
                client,
                connection,
                channel,
                token: Some(token),
            },
        );
        self.loop_handle = Some(handle.clone());
    }

    /// Drain every ready frame from one live source and feed accepted attach
    /// events into its pane. Generated replies and CREDIT go back to Core.
    pub fn poll_live(&mut self, key: &str) {
        let Self {
            sources,
            live: live_sessions,
            retries,
            focused,
            loop_handle,
            ..
        } = self;
        let Some(live) = live_sessions.get_mut(key) else {
            return;
        };
        let mut closed = false;
        loop {
            let frame = match live.connection.receive() {
                Ok(Some(frame)) => frame,
                Ok(None) => {
                    closed = true;
                    break;
                }
                Err(TransportError::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(error) => {
                    tracing::warn!("terminal {key}: live read: {error}");
                    closed = true;
                    break;
                }
            };
            let outcome = match live.client.handle(frame) {
                Ok(outcome) => outcome,
                Err(error) => {
                    tracing::warn!("terminal {key}: protocol: {error}");
                    closed = true;
                    break;
                }
            };
            for outgoing in outcome.outgoing {
                if let Err(error) = live.connection.send(outgoing) {
                    tracing::warn!("terminal {key}: write: {error}");
                    closed = true;
                    break;
                }
            }
            if closed {
                break;
            }
            for event in outcome.events {
                let Some(source) = sources.get_mut(key) else {
                    continue;
                };
                if let Some(replies) = source.apply(&event) {
                    if !replies.is_empty() {
                        if let Ok(frame) = live.client.input(live.channel, replies) {
                            let _ = live.connection.send(frame);
                        }
                    }
                }
            }
        }
        let token = live.token;
        if closed {
            sources.remove(key);
            if focused.as_deref() == Some(key) {
                *focused = None;
            }
            if let (Some(handle), Some(token)) = (loop_handle.as_ref(), token) {
                handle.remove(token);
            }
            live_sessions.remove(key);
            record_retry(retries, key);
        }
    }

    pub fn send_input(&mut self, reference: &TerminalRef, bytes: &[u8]) -> bool {
        let key = reference.canonical();
        let Some(live) = self.live.get_mut(&key) else {
            return false;
        };
        let Ok(frame) = live.client.input(live.channel, bytes) else {
            return false;
        };
        live.connection.send(frame).is_ok()
    }

    /// Committed IME text for a focused Core terminal. Preedit is deliberately
    /// not forwarded: the terminal only receives text the IME committed.
    pub fn commit_text(&mut self, reference: &TerminalRef, text: &str) -> bool {
        !text.is_empty() && self.send_input(reference, text.as_bytes())
    }

    pub fn key_bytes(&self, reference: &TerminalRef, keysym: u32, mods: u16) -> Vec<u8> {
        self.get(reference)
            .map(|source| source.key_bytes(keysym, mods))
            .unwrap_or_default()
    }

    pub fn mouse_button(
        &self,
        reference: &TerminalRef,
        rect: Rectangle<i32, Logical>,
        pos: Point<f64, Logical>,
        button: u8,
        press: bool,
        mods: (bool, bool, bool),
    ) -> Vec<u8> {
        self.get(reference)
            .map(|source| source.mouse_button(rect, pos, button, press, mods))
            .unwrap_or_default()
    }

    pub fn mouse_motion(
        &self,
        reference: &TerminalRef,
        rect: Rectangle<i32, Logical>,
        pos: Point<f64, Logical>,
        button: Option<u8>,
        mods: (bool, bool, bool),
    ) -> Vec<u8> {
        self.get(reference)
            .map(|source| source.mouse_motion(rect, pos, button, mods))
            .unwrap_or_default()
    }

    pub fn wheel(&mut self, reference: &TerminalRef, up: bool) -> Option<Vec<u8>> {
        self.get_mut(reference).map(|source| source.wheel(up))
    }
    /// Register (or replace) one terminal source. A replacement deliberately
    /// drops the old lease: a new source is a new program/viewer generation.
    /// Returns `None` when a Core terminal reference and endpoint name
    /// different Cores.
    pub fn open(
        &mut self,
        endpoint: Endpoint,
        reference: TerminalRef,
        channel: u32,
        cols: u16,
        rows: u16,
    ) -> Option<&mut CoreSessionSource> {
        if let TerminalRef::Core { core_id, .. } = &reference {
            if core_id != &endpoint.core_id {
                return None;
            }
        }
        let key = reference.canonical();
        self.sources.insert(
            key.clone(),
            CoreSessionSource::new(endpoint, reference, channel, cols, rows),
        );
        self.sources.get_mut(&key)
    }

    pub fn get(&self, reference: &TerminalRef) -> Option<&CoreSessionSource> {
        self.sources.get(&reference.canonical())
    }

    pub fn get_mut(&mut self, reference: &TerminalRef) -> Option<&mut CoreSessionSource> {
        self.sources.get_mut(&reference.canonical())
    }

    pub fn apply(&mut self, reference: &TerminalRef, event: &Event) -> Option<Vec<u8>> {
        self.get_mut(reference)?.apply(event)
    }

    pub fn remove(&mut self, reference: &TerminalRef) -> bool {
        self.sources.remove(&reference.canonical()).is_some()
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        reference: &TerminalRef,
        rect: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Option<CustomElement<GlesRenderer>> {
        self.get_mut(reference)?.element(renderer, rect, scale)
    }
}

fn modifier_bits(mods: (bool, bool, bool)) -> u8 {
    u8::from(mods.0) | (u8::from(mods.1) << 1) | (u8::from(mods.2) << 2)
}

fn retry_delay(attempts: u32) -> Duration {
    let shift = attempts.saturating_sub(1).min(5);
    RETRY_BASE_DELAY
        .saturating_mul(1 << shift)
        .min(RETRY_MAX_DELAY)
}

fn retry_ready(retries: &BTreeMap<String, RetryState>, key: &str) -> bool {
    retries
        .get(key)
        .is_none_or(|retry| retry.attempts < MAX_ATTACH_ATTEMPTS && Instant::now() >= retry.next_at)
}

fn record_retry(retries: &mut BTreeMap<String, RetryState>, key: &str) {
    let attempts = retries
        .get(key)
        .map_or(1, |retry| retry.attempts.saturating_add(1))
        .min(MAX_ATTACH_ATTEMPTS);
    retries.insert(
        key.to_owned(),
        RetryState {
            attempts,
            next_at: Instant::now() + retry_delay(attempts),
        },
    );
}

fn core_parts(reference: &TerminalRef) -> Result<(&str, u32), String> {
    match reference {
        TerminalRef::Core { core_id, terminal } => Ok((core_id, *terminal)),
        TerminalRef::HerdrLocal { .. } => {
            Err("legacy herdr-local references are not served by a Core socket".into())
        }
    }
}

fn core_id(reference: &TerminalRef) -> String {
    core_parts(reference)
        .map(|(core_id, _)| core_id.to_owned())
        .unwrap_or_default()
}

fn connect_live(
    socket: &Path,
    reference: &TerminalRef,
    cols: u16,
    rows: u16,
) -> Result<(Connection, Client, u32), String> {
    let (expected_core, terminal) = core_parts(reference)?;
    let mut connection = Connection::connect(socket).map_err(|error| error.to_string())?;
    connection
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| error.to_string())?;

    let mut client = Client::new();
    let (_, hello) = client.hello().map_err(|error| error.to_string())?;
    connection.send(hello).map_err(|error| error.to_string())?;
    let mut hello_core = None;
    for _ in 0..8 {
        let Some(outcome) = connection
            .pump(&mut client)
            .map_err(|error| error.to_string())?
        else {
            return Err("Core closed during hello".into());
        };
        if let Some(Event::Hello(hello)) = outcome
            .events
            .into_iter()
            .find(|event| matches!(event, Event::Hello(_)))
        {
            hello_core = Some(hello.core_id);
            break;
        }
    }
    let hello_core = hello_core.ok_or_else(|| "Core did not answer hello".to_owned())?;
    if hello_core != expected_core {
        return Err(format!(
            "terminal reference names Core {expected_core}, socket names {hello_core}"
        ));
    }

    let (_, attach) = client
        .attach(AttachRequest::viewer(format!("t{terminal}"), cols, rows))
        .map_err(|error| error.to_string())?;
    connection.send(attach).map_err(|error| error.to_string())?;
    let mut channel = None;
    for _ in 0..8 {
        let Some(outcome) = connection
            .pump(&mut client)
            .map_err(|error| error.to_string())?
        else {
            return Err("Core closed during attach".into());
        };
        if let Some(Event::Attached {
            channel: attached, ..
        }) = outcome
            .events
            .into_iter()
            .find(|event| matches!(event, Event::Attached { .. }))
        {
            channel = Some(attached);
            break;
        }
    }
    let channel = channel.ok_or_else(|| "Core did not answer attach".to_owned())?;
    connection
        .set_read_timeout(None)
        .map_err(|error| error.to_string())?;
    connection
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    Ok((connection, client, channel))
}

/// Map the keysyms Smithay sees onto Ghostty key values. Printable ASCII uses
/// Ghostty's physical-key numbering; Unicode beyond ASCII is left to an IME.
fn unshifted_char(ch: char) -> char {
    match ch {
        '!' => '1',
        '@' => '2',
        '#' => '3',
        '$' => '4',
        '%' => '5',
        '^' => '6',
        '&' => '7',
        '*' => '8',
        '(' => '9',
        ')' => '0',
        '_' => '-',
        '+' => '=',
        '{' => '[',
        '}' => ']',
        '|' => '\\',
        ':' => ';',
        '"' => '\'',
        '<' => ',',
        '>' => '.',
        '?' => '/',
        '~' => '`',
        ch if ch.is_ascii_uppercase() => ch.to_ascii_lowercase(),
        ch => ch,
    }
}

fn ghostty_key(keysym: u32) -> Option<(i32, char)> {
    let ch = char::from_u32(keysym)?;
    let key = match ch {
        'a'..='z' => 20 + (ch as i32 - 'a' as i32),
        'A'..='Z' => 20 + (ch.to_ascii_lowercase() as i32 - 'a' as i32),
        '0'..='9' => 6 + (ch as i32 - '0' as i32),
        '`' | '~' => 1,
        '\\' | '|' => 2,
        '[' | '{' => 3,
        ']' | '}' => 4,
        ',' | '<' => 5,
        '=' | '+' => 16,
        '-' | '_' => 46,
        '.' | '>' => 47,
        '\'' | '"' => 48,
        ';' | ':' => 49,
        '/' | '?' => 50,
        ' ' => 63,
        '!' => 7,
        '@' => 8,
        '#' => 9,
        '$' => 10,
        '%' => 11,
        '^' => 12,
        '&' => 13,
        '*' => 14,
        '(' => 15,
        ')' => 16,
        _ => match keysym {
            0xff08 => 53,
            0xff09 => 64,
            0xff0d => 58,
            0xff1b => 120,
            0xffff => 68,
            0xff50 => 71,
            0xff57 => 69,
            0xff55 => 74,
            0xff56 => 73,
            0xff51 => 76,
            0xff52 => 78,
            0xff53 => 77,
            0xff54 => 75,
            0xffbe..=0xffc9 => 121 + (keysym - 0xffbe) as i32,
            _ => return None,
        },
    };
    Some((key, if keysym >= 0xff00 { '\0' } else { ch }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use emthin_core_client::wire::{Frame, DATA};
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::thread;
    use std::time::Instant;

    const CORE: &str = "6ab09a20fd1288983dfbbac2bb349231";

    fn reference() -> TerminalRef {
        TerminalRef::parse(&format!("hc://{CORE}/t3")).expect("canonical reference")
    }

    fn endpoint() -> Endpoint {
        Endpoint {
            core_id: CORE.into(),
            socket: PathBuf::from("/run/user/1000/hc.sock"),
        }
    }

    #[test]
    fn attach_retries_are_bounded_and_exponential() {
        let mut retries = BTreeMap::new();
        assert!(retry_ready(&retries, "k"));
        assert_eq!(retry_delay(1), RETRY_BASE_DELAY);
        assert_eq!(retry_delay(2), RETRY_BASE_DELAY * 2);
        assert_eq!(retry_delay(99), RETRY_MAX_DELAY);

        record_retry(&mut retries, "k");
        assert_eq!(retries["k"].attempts, 1);
        assert!(!retry_ready(&retries, "k"), "backoff delays the retry");
        for attempt in 2..=MAX_ATTACH_ATTEMPTS {
            retries.get_mut("k").unwrap().next_at = Instant::now();
            assert!(retry_ready(&retries, "k"));
            record_retry(&mut retries, "k");
            assert_eq!(retries["k"].attempts, attempt);
        }
        assert!(
            !retry_ready(&retries, "k"),
            "the attempt budget is exhausted"
        );

        let mut sessions = CoreSessions::default();
        let key = reference().canonical();
        sessions.retries.insert(
            key.clone(),
            RetryState {
                attempts: 1,
                next_at: Instant::now() + Duration::from_millis(250),
            },
        );
        let status = sessions.retry_status(&reference()).expect("retry status");
        assert!(status.contains("attempt 2/5"), "{status}");
        sessions.retries.get_mut(&key).unwrap().attempts = MAX_ATTACH_ATTEMPTS;
        assert_eq!(
            sessions.retry_status(&reference()).as_deref(),
            Some("Terminal disconnected — retries exhausted")
        );
    }

    #[test]
    fn attach_data_paints_a_source_and_bumps_its_revision() {
        let mut sessions = CoreSessions::default();
        let source = sessions
            .open(endpoint(), reference(), 3, 4, 2)
            .expect("matching Core");
        assert_eq!(source.image(), None);

        sessions
            .apply(
                &reference(),
                &Event::Data {
                    channel: 3,
                    bytes: b"hi".to_vec(),
                },
            )
            .expect("matching data");
        let source = sessions.get(&reference()).expect("source");
        let image = source.image().expect("painted");
        assert_eq!((image.width, image.height), (32, 32));
        assert_eq!(source.revision(), 1);
        assert!(
            image.pixels.chunks(4).any(|p| p != [18, 22, 28, 255]),
            "terminal text must reach the figure pixels"
        );
    }

    #[test]
    fn a_reference_for_another_core_is_refused() {
        let mut sessions = CoreSessions::default();
        let mut endpoint = endpoint();
        endpoint.core_id = "ffffffffffffffffffffffffffffffff".into();
        assert!(sessions.open(endpoint, reference(), 3, 4, 2).is_none());
        assert!(sessions.is_empty());
    }

    #[test]
    fn unrelated_channels_and_unknown_terminals_are_ignored() {
        let mut sessions = CoreSessions::default();
        sessions
            .open(endpoint(), reference(), 3, 4, 2)
            .expect("matching Core");
        assert!(sessions
            .apply(
                &reference(),
                &Event::Data {
                    channel: 9,
                    bytes: b"wrong".to_vec(),
                },
            )
            .is_none());
        let other = TerminalRef::parse(&format!("hc://{CORE}/t4")).expect("reference");
        assert!(sessions
            .apply(
                &other,
                &Event::Data {
                    channel: 3,
                    bytes: b"x".to_vec(),
                },
            )
            .is_none());
        assert_eq!(sessions.get(&reference()).unwrap().revision(), 0);
    }

    #[test]
    fn a_real_attach_stream_reaches_the_source_and_closes_it() {
        let socket = std::env::temp_dir().join(format!(
            "emthin-core-session-{}-{}.sock",
            std::process::id(),
            CORE
        ));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind fixture");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut connection = Connection::from_stream(stream).expect("connection");
            let request = connection.receive().expect("hello read").expect("hello");
            let hello: serde_json::Value = serde_json::from_slice(&request.payload).unwrap();
            send_json(
                &mut connection,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": hello["id"],
                    "result": {
                        "generation": 1,
                        "version": "fixture",
                        "core": {"id": CORE, "name": "fixture"},
                        "framed": true,
                        "methods": ["hello", "terminal.attach"],
                        "features": []
                    }
                }),
            );
            let request = connection.receive().expect("attach read").expect("attach");
            let attach: serde_json::Value = serde_json::from_slice(&request.payload).unwrap();
            send_json(
                &mut connection,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": attach["id"],
                    "result": {"channel": 7}
                }),
            );
            connection.send(Frame::data(7, b"hi")).expect("attach data");
            while let Ok(Some(_)) = connection.receive() {}
        });

        let (connection, client, channel) =
            connect_live(&socket, &reference(), 80, 24).expect("live handshake");
        assert_eq!(channel, 7);
        let key = reference().canonical();
        let mut sessions = CoreSessions::default();
        sessions
            .open(endpoint(), reference(), channel, 4, 2)
            .expect("matching Core");
        sessions.live.insert(
            key.clone(),
            LiveSession {
                client,
                connection,
                channel,
                token: None,
            },
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while sessions
            .get(&reference())
            .is_some_and(|source| source.revision() == 0)
            && Instant::now() < deadline
        {
            sessions.poll_live(&key);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(sessions.get(&reference()).unwrap().revision(), 1);
        let image = sessions.get(&reference()).unwrap().image().unwrap();
        assert!(image
            .pixels
            .chunks(4)
            .any(|pixel| pixel != [18, 22, 28, 255]));

        drop(sessions);
        server.join().expect("fixture thread");
        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn live_socket_drives_figure_input_and_output() {
        let socket = std::env::temp_dir().join(format!(
            "emthin-core-live-e2e-{}-{}.sock",
            std::process::id(),
            CORE
        ));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind fixture");
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut connection = Connection::from_stream(stream).expect("connection");
            let request = connection.receive().expect("hello read").expect("hello");
            let hello: serde_json::Value = serde_json::from_slice(&request.payload).unwrap();
            send_json(
                &mut connection,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": hello["id"],
                    "result": {
                        "generation": 1,
                        "version": "fixture",
                        "core": {"id": CORE, "name": "fixture"},
                        "framed": true,
                        "methods": ["hello", "terminal.attach"],
                        "features": []
                    }
                }),
            );
            let request = connection.receive().expect("attach read").expect("attach");
            let attach: serde_json::Value = serde_json::from_slice(&request.payload).unwrap();
            send_json(
                &mut connection,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": attach["id"],
                    "result": {"channel": 7}
                }),
            );
            connection
                .send(Frame::data(7, b"\x1b[?1002h\x1b[?1006h"))
                .expect("initial screen");
            let mut input = Vec::new();
            while input.len() < 4 {
                let frame = connection.receive().expect("input read").expect("input");
                if frame.kind == DATA {
                    input.push(frame.payload);
                }
            }
            connection
                .send(Frame::data(7, b"done"))
                .expect("later screen");
            finish_rx.recv().expect("finish signal");
            input
        });

        let (connection, client, channel) =
            connect_live(&socket, &reference(), 80, 24).expect("live handshake");
        assert_eq!(channel, 7);
        let key = reference().canonical();
        let mut sessions = CoreSessions::default();
        sessions
            .open(endpoint(), reference(), channel, 80, 24)
            .expect("matching Core");
        sessions.live.insert(
            key.clone(),
            LiveSession {
                client,
                connection,
                channel,
                token: None,
            },
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while sessions.get(&reference()).unwrap().revision() == 0 && Instant::now() < deadline {
            sessions.poll_live(&key);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(sessions.get(&reference()).unwrap().revision(), 1);

        let keys = sessions.key_bytes(&reference(), 'x' as u32, 0);
        assert_eq!(keys, b"x");
        assert!(sessions.send_input(&reference(), &keys));
        let rect = Rectangle::from_size((640, 384).into());
        let pos = Point::new(9.0, 17.0);
        let press = sessions.mouse_button(&reference(), rect, pos, 0, true, (false, false, false));
        assert_eq!(press, b"\x1b[<0;2;2M");
        assert!(sessions.send_input(&reference(), &press));
        let motion = sessions.mouse_motion(&reference(), rect, pos, Some(0), (false, false, false));
        assert_eq!(motion, b"\x1b[<32;2;2M");
        assert!(sessions.send_input(&reference(), &motion));
        let ime = "界";
        assert!(sessions.commit_text(&reference(), ime));

        let deadline = Instant::now() + Duration::from_secs(2);
        while sessions.get(&reference()).unwrap().revision() < 2 && Instant::now() < deadline {
            sessions.poll_live(&key);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(sessions.get(&reference()).unwrap().revision(), 2);

        finish_tx.send(()).expect("release fixture");
        let input = server.join().expect("fixture thread");
        assert_eq!(
            input,
            vec![b"x".to_vec(), press, motion, ime.as_bytes().to_vec()]
        );
        drop(sessions);
        let _ = std::fs::remove_file(&socket);
    }

    #[test]
    fn a_closed_live_socket_reattaches_after_backoff() {
        let socket = std::env::temp_dir().join(format!(
            "emthin-core-reattach-{}-{}.sock",
            std::process::id(),
            CORE
        ));
        let _ = std::fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("bind fixture");
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("first accept");
            let mut connection = Connection::from_stream(stream).expect("connection");
            serve_handshake(&mut connection, 7);
            connection
                .send(Frame::data(7, b"first"))
                .expect("first data");
            drop(connection);

            let (stream, _) = listener.accept().expect("second accept");
            let mut connection = Connection::from_stream(stream).expect("connection");
            serve_handshake(&mut connection, 8);
            connection
                .send(Frame::data(8, b"back"))
                .expect("second data");
            finish_rx.recv().expect("finish signal");
        });

        let (connection, client, channel) =
            connect_live(&socket, &reference(), 80, 24).expect("first handshake");
        let key = reference().canonical();
        let mut sessions = CoreSessions::default();
        sessions
            .open(endpoint(), reference(), channel, 80, 24)
            .expect("matching Core");
        sessions.live.insert(
            key.clone(),
            LiveSession {
                client,
                connection,
                channel,
                token: None,
            },
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        while sessions.live.contains_key(&key) && Instant::now() < deadline {
            sessions.poll_live(&key);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(!sessions.live.contains_key(&key));
        assert!(sessions.get(&reference()).is_none());
        let retry = &sessions.retries[&key];
        assert_eq!(retry.attempts, 1);
        assert!(!sessions.retry_due(), "the retry is still backing off");

        sessions.retries.get_mut(&key).unwrap().next_at = Instant::now();
        assert!(sessions.retry_due());

        let (connection, client, channel) =
            connect_live(&socket, &reference(), 80, 24).expect("second handshake");
        assert_eq!(channel, 8);
        sessions
            .open(endpoint(), reference(), channel, 80, 24)
            .expect("matching Core");
        sessions.live.insert(
            key.clone(),
            LiveSession {
                client,
                connection,
                channel,
                token: None,
            },
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while sessions.get(&reference()).unwrap().revision() == 0 && Instant::now() < deadline {
            sessions.poll_live(&key);
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(sessions.get(&reference()).unwrap().revision(), 1);
        assert!(sessions.get(&reference()).unwrap().image().is_some());

        finish_tx.send(()).expect("release fixture");
        server.join().expect("fixture thread");
        drop(sessions);
        let _ = std::fs::remove_file(&socket);
    }

    fn serve_handshake(connection: &mut Connection, channel: u32) {
        let request = connection.receive().expect("hello read").expect("hello");
        let hello: serde_json::Value = serde_json::from_slice(&request.payload).unwrap();
        send_json(
            connection,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": hello["id"],
                "result": {
                    "generation": 1,
                    "version": "fixture",
                    "core": {"id": CORE, "name": "fixture"},
                    "framed": true,
                    "methods": ["hello", "terminal.attach"],
                    "features": []
                }
            }),
        );
        let request = connection.receive().expect("attach read").expect("attach");
        let attach: serde_json::Value = serde_json::from_slice(&request.payload).unwrap();
        send_json(
            connection,
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": attach["id"],
                "result": {"channel": channel}
            }),
        );
    }

    fn send_json(connection: &mut Connection, value: serde_json::Value) {
        connection
            .send(Frame::data(0, serde_json::to_vec(&value).unwrap()))
            .expect("send");
    }

    #[test]
    fn a_focused_terminal_encodes_keys_and_mouse_for_its_program() {
        let mut sessions = CoreSessions::default();
        sessions
            .open(endpoint(), reference(), 3, 10, 4)
            .expect("matching Core");
        assert!(sessions.focus(&reference()));
        assert_eq!(sessions.focused(), Some(&reference()));

        assert_eq!(sessions.key_bytes(&reference(), 'a' as u32, 0), b"a");
        assert_eq!(sessions.key_bytes(&reference(), '!' as u32, 1), b"!");
        assert_eq!(sessions.key_bytes(&reference(), 0xff0d, 0), b"\r");
        assert_eq!(sessions.key_bytes(&reference(), 'c' as u32, 2), b"\x03");
        assert_eq!(sessions.key_bytes(&reference(), 0xff52, 0), b"\x1b[A");

        sessions
            .apply(
                &reference(),
                &Event::Data {
                    channel: 3,
                    bytes: b"\x1b[?1000h\x1b[?1006h".to_vec(),
                },
            )
            .unwrap();
        let rect = Rectangle::from_size((80, 64).into());
        assert_eq!(
            sessions.mouse_button(
                &reference(),
                rect,
                Point::new(9.0, 17.0),
                0,
                true,
                (false, false, false),
            ),
            b"\x1b[<0;2;2M"
        );
        sessions.clear_focus();
        assert!(sessions.focused().is_none());
    }

    #[test]
    fn resize_and_replacement_change_the_source_dimensions() {
        let mut sessions = CoreSessions::default();
        sessions
            .open(endpoint(), reference(), 3, 4, 2)
            .expect("matching Core");
        sessions
            .apply(
                &reference(),
                &Event::Data {
                    channel: 3,
                    bytes: b"x".to_vec(),
                },
            )
            .unwrap();
        sessions
            .get_mut(&reference())
            .unwrap()
            .resize(6, 3)
            .expect("resize");
        let image = sessions.get(&reference()).unwrap().image().unwrap();
        assert_eq!((image.width, image.height), (48, 48));

        sessions
            .open(endpoint(), reference(), 7, 2, 1)
            .expect("matching Core");
        let source = sessions.get(&reference()).unwrap();
        assert_eq!(source.pane().channel(), 7);
        assert_eq!(source.image(), None, "a replacement starts un-rendered");
        assert_eq!(sessions.len(), 1);
        assert!(sessions.remove(&reference()));
        assert!(sessions.is_empty());
    }
}
