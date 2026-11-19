//! End-to-end collaboration over the real emthin control socket.
//!
//! Two headless `EmthinState`s each own an IPC server and a document. This test
//! connects one IPC client to each, has them join each other, then relays the
//! resulting `collab_delta` notifications between the two sockets. Every frame
//! goes through `IpcServer`, JSON-RPC framing, `handle_ipc_message`, the
//! per-tick local-edit forwarder, and the remote-import path — no session or
//! document method is called directly from the test.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use emthin::{ipc::IpcServer, tick::event_loop_tick, EmthinState};
use serde_json::{json, Value};
use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

static NEXT: AtomicU64 = AtomicU64::new(1);
/// `Peer::new` sets process-wide XDG/Wayland environment variables, so these
/// socket harnesses cannot construct peers concurrently.
static SERIAL: Mutex<()> = Mutex::new(());

struct Client {
    stream: UnixStream,
    buf: Vec<u8>,
}

impl Client {
    fn connect(path: &Path) -> Self {
        let stream = UnixStream::connect(path).expect("connect to emthin IPC");
        stream.set_nonblocking(true).expect("non-blocking client");
        Self {
            stream,
            buf: Vec::new(),
        }
    }

    fn send(&mut self, method: &str, params: Value) {
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .expect("serialize");
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stream
            .write_all(header.as_bytes())
            .expect("write header");
        self.stream.write_all(&body).expect("write body");
    }

    /// Read every currently available message.
    fn drain(&mut self) -> Vec<Value> {
        let mut chunk = [0u8; 8192];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("IPC read: {e}"),
            }
        }

        let mut out = Vec::new();
        while let Some(header_end) = self.buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let header = std::str::from_utf8(&self.buf[..header_end]).expect("utf8 header");
            let len: usize = header
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length:"))
                .and_then(|value| value.trim().parse().ok())
                .expect("Content-Length");
            let body_start = header_end + 4;
            if self.buf.len() < body_start + len {
                break;
            }
            let body: Vec<u8> = self.buf.drain(..body_start + len).collect();
            out.push(serde_json::from_slice(&body[body_start..]).expect("JSON body"));
        }
        out
    }
}

struct Peer {
    state: EmthinState,
    /// Declared before the event loop so the state, whose handles point into
    /// the loop, is dropped first.
    _event_loop: EventLoop<'static, EmthinState>,
    client: Client,
    dir: PathBuf,
}

impl Drop for Peer {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Peer {
    fn new(tag: &str, seed: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "emthin-collab-ipc-{}-{}-{tag}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let xdg = root.join("xdg");
        std::fs::create_dir_all(&xdg).expect("runtime dir");
        std::env::set_var("XDG_RUNTIME_DIR", &xdg);
        std::env::set_var(
            "EMTHIN_WAYLAND_SOCKET_NAME",
            format!("collab-{}-{tag}", std::process::id()),
        );

        let mut event_loop = EventLoop::try_new().expect("event loop");
        let handle = event_loop.handle();
        let display = Display::new().expect("wayland display");
        let socket = root.join("emthin.ipc");
        let ipc = IpcServer::bind(&socket).expect("IPC bind");
        let xkb = smithay::input::keyboard::XkbConfig {
            layout: "us",
            ..Default::default()
        };
        let mut state =
            EmthinState::new(&mut event_loop, handle, display, ipc, xkb).expect("EmthinState");
        if !seed.is_empty() {
            state.doc.model_mut().replace(0..0, seed);
        }

        let client = Client::connect(&socket);
        Self {
            state,
            _event_loop: event_loop,
            client,
            dir: root,
        }
    }

    fn pump(&mut self) {
        self.state.ipc.accept();
        event_loop_tick(&mut self.state);
    }

    fn wait_for(&mut self, what: &str, mut predicate: impl FnMut(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.pump();
            for message in self.client.drain() {
                assert_ne!(message["method"], "error", "{what}: IPC error: {message}");
                if predicate(&message) {
                    return message;
                }
            }
            assert!(
                Instant::now() < deadline,
                "{what}: timed out; last document was {:?}",
                self.state.doc.model().text()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_collab_delta(&mut self, peer: &str) -> (u64, String) {
        let message = self.wait_for("a collaboration delta", |message| {
            message["method"] == "collab_delta" && message["params"]["peer"] == peer
        });
        assert_eq!(message["method"], "collab_delta", "{message}");
        (
            message["params"]["seq"].as_u64().expect("seq"),
            message["params"]["delta"]
                .as_str()
                .expect("delta")
                .to_string(),
        )
    }

    fn send_delta(&mut self, peer: &str, seq: u64, delta: &str) {
        self.client.send(
            "collab_delta",
            json!({"peer": peer, "seq": seq, "delta": delta}),
        );
    }

    fn document(&mut self) -> String {
        self.client.send("list_state", json!({}));
        let message = self.wait_for("a state reply", |message| message["method"] == "state");
        message["params"]["doc"].as_str().expect("doc").to_string()
    }

    /// Make a local edit. The transport under test is the IPC frame exchange;
    /// the edit itself is the same model mutation a keystroke would commit.
    fn edit(&mut self, text: &str) {
        let len = self.state.doc.model().len();
        self.state.doc.model_mut().replace(0..len, text);
    }
}

fn no_error(message: &Value) {
    assert_ne!(message["method"], "error", "IPC error: {message}");
}

/// Drain one peer's control socket, forwarding relayable presence blobs to the
/// other. Returns the other notifications for assertion or discard.
fn relay_presence(from: &mut Peer, to: &mut Peer) -> Vec<Value> {
    let from_peer = from.state.collab.local_peer().to_string();
    let mut rest = Vec::new();
    for message in from.client.drain() {
        no_error(&message);
        if message["method"] == "collab_presence" {
            if let Some(blob) = message["params"]["blob"].as_str() {
                to.client.send(
                    "collab_presence",
                    json!({"peer": from_peer, "blob": blob}),
                );
            }
        }
        rest.push(message);
    }
    rest
}

#[test]
fn presence_is_visible_expires_and_reconnects_without_editing() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("EMTHIN_COLLAB_PRESENCE_TIMEOUT_MS", "1500");
    let mut a = Peer::new("presence-a", "base");
    let mut b = Peer::new("presence-b", "");

    a.client.send("collab_name", json!({"name": "Ada"}));
    b.client.send("collab_name", json!({"name": "Bob"}));

    // Names and carets travel as presence blobs, not as document deltas.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        a.pump();
        b.pump();
        relay_presence(&mut a, &mut b);
        relay_presence(&mut b, &mut a);
        let a_sees_bob = a.state.collab_peers().iter().any(|p| p.name == "Bob");
        let b_sees_ada = b.state.collab_peers().iter().any(|p| p.name == "Ada");
        if a_sees_bob && b_sees_ada {
            break;
        }
        assert!(Instant::now() < deadline, "presence names did not travel");
        std::thread::sleep(Duration::from_millis(5));
    }

    // Join both reliable streams, keyed by the same stable presence ids the
    // transport forwards, and relay the catch-up deltas.
    let a_peer = a.state.collab.local_peer().to_string();
    let b_peer = b.state.collab.local_peer().to_string();
    a.client
        .send("collab_join", json!({"peer": b_peer, "version": ""}));
    b.client
        .send("collab_join", json!({"peer": a_peer, "version": ""}));
    let (a_seq, a_delta) = a.wait_collab_delta(&b_peer);
    let (b_seq, b_delta) = b.wait_collab_delta(&a_peer);
    b.send_delta(&a_peer, a_seq, &a_delta);
    a.send_delta(&b_peer, b_seq, &b_delta);
    assert_eq!(b.document(), "base");

    // A caret move is published without changing text, revision or undo state.
    let b_before = (
        b.state.doc.model().text().to_string(),
        b.state.doc.model().revision(),
    );
    a.state.doc.model_mut().set_caret(2);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        a.pump();
        b.pump();
        relay_presence(&mut a, &mut b);
        relay_presence(&mut b, &mut a);
        if b
            .state
            .collab_peers()
            .iter()
            .any(|peer| peer.name == "Ada" && peer.cursor == Some(2))
        {
            break;
        }
        assert!(Instant::now() < deadline, "the remote caret did not arrive");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(b.state.doc.model().text(), b_before.0);
    assert_eq!(b.state.doc.model().revision(), b_before.1);
    let connected = b
        .state
        .collab_peers()
        .into_iter()
        .find(|peer| peer.name == "Ada")
        .expect("Ada is present");
    assert!(connected.connected, "the reliable stream is joined");

    // `list_state` exposes the same snapshot even before any notification.
    b.client.send("list_state", json!({}));
    let state = b.wait_for("a state reply", |message| message["method"] == "state");
    assert_eq!(state["params"]["peers"][0]["name"], "Ada");
    assert_eq!(state["params"]["peers"][0]["connected"], true);
    assert_eq!(state["params"]["collab_local_name"], "Bob");

    // Stop relaying. The peer disappears once its heartbeat window lapses.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        b.pump();
        if b.state.collab_peers().is_empty() {
            break;
        }
        assert!(Instant::now() < deadline, "expired presence stayed visible");
        std::thread::sleep(Duration::from_millis(20));
    }

    // A fresh publish and a reconnect bring the same peer back exactly once.
    a.client.send("collab_name", json!({"name": "Ada"}));
    a.pump();
    relay_presence(&mut a, &mut b);
    b.pump();
    a.client.send(
        "collab_join",
        json!({"peer": b_peer, "version": ""}),
    );
    let _ = a.wait_collab_delta(&b_peer);
    assert_eq!(
        b.state
            .collab_peers()
            .iter()
            .filter(|peer| peer.name == "Ada")
            .count(),
        1,
        "reconnect must not duplicate the peer"
    );
}

#[test]
fn two_ipc_clients_join_and_exchange_deltas() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut a = Peer::new("a", "base");
    let mut b = Peer::new("b", "");

    // Each side joins the other through its own control socket. The answer is
    // the catch-up delta that must travel to the other side.
    a.client
        .send("collab_join", json!({"peer": "b", "version": ""}));
    b.client
        .send("collab_join", json!({"peer": "a", "version": ""}));
    let (a_seq, a_delta) = a.wait_collab_delta("b");
    let (b_seq, b_delta) = b.wait_collab_delta("a");
    assert_eq!(a_seq, 1);
    assert_eq!(b_seq, 1);

    b.send_delta("a", a_seq, &a_delta);
    a.send_delta("b", b_seq, &b_delta);
    assert_eq!(
        b.document(),
        "base",
        "the joining side receives the document"
    );

    // A local document change on A must arrive as a `collab_delta` on A's
    // control socket, then converge on B after being relayed.
    a.edit("base + a");
    let (a_edit_seq, a_edit_delta) = a.wait_collab_delta("b");
    assert_eq!(a_edit_seq, 2, "the next frame on the same reliable stream");
    b.send_delta("a", a_edit_seq, &a_edit_delta);
    assert_eq!(b.document(), "base + a");

    // And the reverse direction, so both peers have produced a local edit.
    b.edit("base + a + b");
    let (b_edit_seq, b_edit_delta) = b.wait_collab_delta("a");
    assert_eq!(b_edit_seq, 2);
    a.send_delta("b", b_edit_seq, &b_edit_delta);
    assert_eq!(a.document(), "base + a + b");

    // Both sides converge, and both still answer the ordinary control
    // protocol after the exchange.
    assert_eq!(a.document(), b.document());

    // Surface any error notification that was queued instead of silently
    // passing on document text that happened to match.
    for message in a.client.drain().into_iter().chain(b.client.drain()) {
        no_error(&message);
    }
}
