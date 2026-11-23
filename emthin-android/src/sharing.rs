//! Native sharing bridge for the pad (Kyneta K6).
//!
//! The worker thread owns the transport-side replica and the collaboration
//! `Session`. The live `DocModel` stays on the UI thread: it only receives
//! encoded deltas (one grouped apply) and publishes its own encoded deltas.
//!
//! Frames leave the worker two ways. With a peer link attached
//! ([`ShareHandle::connect`]) the worker writes them to that Unix socket in the
//! shared `emthin_collab::wire` framing and reads inbound frames back; with no
//! link they are published as [`ShareEvent::Outbound`] so a scripted transport
//! (the host tests) or a future gateway adapter can deliver them.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use emthin_collab::unix::{LinkRead, UnixLink};
use emthin_collab::wire::{ShareId, Wire};
use emthin_collab::{Outbound, ReceiveOutcome, Session};
use mathed_core::doc::MathDoc;
use serde_json::{json, Value};

/// The peer id a socket link's remote end is registered under. The pad dials a
/// host it cannot name, so one stable label stands in for its identity.
const LINK_PEER: &str = "host";
/// How long the worker blocks for a command while a link is attached. Short
/// enough that inbound frames are serviced promptly without busy-waiting.
const LINK_TICK: Duration = Duration::from_millis(10);

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// A command sent from the UI thread to the transport worker.
enum WorkerCommand {
    /// Join with the given Loro frontier; the catch-up delta comes back as a
    /// `ShareEvent::CatchUp` for one grouped apply on the live model.
    Join {
        peer: String,
        version: Vec<u8>,
    },
    /// Publish one local edit. The worker imports it into its replica, then
    /// computes the delta each peer missed.
    Publish {
        delta: Vec<u8>,
    },
    /// Accept an inbound frame from a peer.
    Receive {
        peer: String,
        seq: u64,
        delta: Vec<u8>,
        scope: Option<u64>,
    },
    /// Dial a peer's Unix socket and carry frames over it.
    Connect {
        path: PathBuf,
    },
    /// Drop the socket link without leaving the share.
    Disconnect,
    Leave,
    Stop,
}

/// An event the worker publishes for the UI thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareEvent {
    /// The handshake's catch-up delta, to be applied as one grouped edit.
    CatchUp {
        delta: Vec<u8>,
    },
    /// One or more accepted remote deltas, to be applied as one grouped edit.
    Applied {
        deltas: Vec<Vec<u8>>,
    },
    /// One frame the transport must deliver to `peer`. Without this the
    /// worker's computed deltas would have no way back out of the thread, so
    /// a share could receive but never send.
    Outbound {
        peer: String,
        seq: u64,
        delta: Vec<u8>,
    },
    /// The peer snapshot after any state change.
    State {
        peers: Vec<String>,
        joined: bool,
    },
    Error(String),
}

/// The UI-thread handle. It never owns the replica or the live document.
pub struct ShareHandle {
    commands: Sender<WorkerCommand>,
    events: Receiver<ShareEvent>,
    share_id: ShareId,
    worker: Option<JoinHandle<()>>,
}

impl ShareHandle {
    /// Start a worker with a transport-side replica restored from `snapshot`.
    pub fn start(
        snapshot: &[u8],
        local_peer: impl Into<String>,
        local_name: impl Into<String>,
    ) -> Result<Self, String> {
        let replica = MathDoc::from_snapshot(snapshot)
            .map_err(|error| format!("bad share snapshot: {error}"))?;
        let worker_state = WorkerState {
            session: Session::new(local_peer, local_name, 30_000),
            replica,
            joined: false,
            link: None,
        };
        let share_id = *worker_state.session.share_id();
        let (commands, command_rx) = channel();
        let (event_tx, events) = channel();
        let worker = std::thread::Builder::new()
            .name("emthin-pad-share".to_string())
            .spawn(move || worker_state.run(command_rx, event_tx))
            .map_err(|error| format!("could not start sharing worker: {error}"))?;
        Ok(Self {
            commands,
            events,
            share_id,
            worker: Some(worker),
        })
    }

    pub fn share_id(&self) -> &ShareId {
        &self.share_id
    }

    pub fn join(&self, peer: impl Into<String>, version: Vec<u8>) -> Result<(), String> {
        self.commands
            .send(WorkerCommand::Join {
                peer: peer.into(),
                version,
            })
            .map_err(|error| error.to_string())
    }

    pub fn publish(&self, delta: Vec<u8>) -> Result<(), String> {
        self.commands
            .send(WorkerCommand::Publish { delta })
            .map_err(|error| error.to_string())
    }

    pub fn receive(
        &self,
        peer: impl Into<String>,
        seq: u64,
        delta: Vec<u8>,
        scope: Option<u64>,
    ) -> Result<(), String> {
        self.commands
            .send(WorkerCommand::Receive {
                peer: peer.into(),
                seq,
                delta,
                scope,
            })
            .map_err(|error| error.to_string())
    }

    /// Dial a peer's Unix socket. Once attached, the worker writes every frame
    /// for that peer to the socket and feeds inbound frames back into the
    /// session, so only frames for other peers reach `ShareEvent::Outbound`.
    pub fn connect(&self, path: impl Into<PathBuf>) -> Result<(), String> {
        self.commands
            .send(WorkerCommand::Connect { path: path.into() })
            .map_err(|error| error.to_string())
    }

    pub fn disconnect(&self) -> Result<(), String> {
        self.commands
            .send(WorkerCommand::Disconnect)
            .map_err(|error| error.to_string())
    }

    pub fn leave(&self) -> Result<(), String> {
        self.commands
            .send(WorkerCommand::Leave)
            .map_err(|error| error.to_string())
    }

    pub fn stop(&mut self) {
        let _ = self.commands.send(WorkerCommand::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    /// Drain all events currently available.
    pub fn poll(&self) -> Vec<ShareEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            out.push(event);
        }
        out
    }

    /// The events as the JSON shape returned over JNI.
    pub fn poll_json(&self) -> String {
        let events: Vec<Value> = self
            .poll()
            .into_iter()
            .map(|event| match event {
                ShareEvent::CatchUp { delta } => json!({
                    "kind": "catch_up",
                    "delta": bytes_to_hex(&delta),
                }),
                ShareEvent::Applied { deltas } => json!({
                    "kind": "applied",
                    "deltas": deltas.iter().map(|d| bytes_to_hex(d)).collect::<Vec<_>>(),
                }),
                ShareEvent::Outbound { peer, seq, delta } => json!({
                    "kind": "outbound", "peer": peer, "seq": seq,
                    "delta": bytes_to_hex(&delta),
                }),
                ShareEvent::State { peers, joined } => json!({
                    "kind": "state", "peers": peers, "joined": joined,
                }),
                ShareEvent::Error(message) => json!({"kind": "error", "message": message}),
            })
            .collect();
        serde_json::to_string(&json!({
            "share_id": bytes_to_hex(&self.share_id),
            "events": events,
        }))
        .unwrap_or_else(|_| "{\"share_id\":\"\",\"events\":[]}".to_string())
    }
}

impl Drop for ShareHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Everything the worker owns. The live DocModel is never part of this struct.
struct WorkerState {
    session: Session,
    replica: MathDoc,
    joined: bool,
    /// An attached peer socket, when one is connected.
    link: Option<UnixLink>,
}

impl WorkerState {
    fn run(mut self, commands: Receiver<WorkerCommand>, events: Sender<ShareEvent>) {
        loop {
            // With a link attached, command waits must not stall inbound frames.
            let command = if self.link.is_some() {
                match commands.recv_timeout(LINK_TICK) {
                    Ok(command) => Some(command),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match commands.recv() {
                    Ok(command) => Some(command),
                    Err(_) => break,
                }
            };
            let mut stop = false;
            if let Some(command) = command {
                stop = self.handle(command, &events);
            }
            self.pump_link(&events);
            if stop {
                break;
            }
        }
    }

    /// Run one command. Returns `true` when the worker must stop.
    fn handle(&mut self, command: WorkerCommand, events: &Sender<ShareEvent>) -> bool {
        match command {
            WorkerCommand::Join { peer, version } => {
                match self.session.join(&self.replica, peer, &version) {
                    Ok(out) => {
                        self.joined = true;
                        let _ = events.send(ShareEvent::CatchUp {
                            delta: out.delta.clone(),
                        });
                        self.send_outbound(out, events);
                        self.publish_state(events);
                    }
                    Err(error) => {
                        let _ = events.send(ShareEvent::Error(error.to_string()));
                    }
                }
            }
            WorkerCommand::Publish { delta } => {
                if let Err(error) = self.replica.import_delta(&delta) {
                    let _ = events.send(ShareEvent::Error(error.to_string()));
                    return false;
                }
                match self.session.local_edit(&self.replica) {
                    Ok(out) => {
                        for out in out {
                            self.send_outbound(out, events);
                        }
                    }
                    Err(error) => {
                        let _ = events.send(ShareEvent::Error(error.to_string()));
                    }
                }
            }
            WorkerCommand::Receive {
                peer,
                seq,
                delta,
                scope,
            } => {
                match self
                    .session
                    .receive_scoped(&mut self.replica, &peer, seq, &delta, scope)
                {
                    Ok(ReceiveOutcome::Applied { outbound, .. }) => {
                        let _ = events.send(ShareEvent::Applied {
                            deltas: vec![delta],
                        });
                        for out in outbound {
                            self.send_outbound(out, events);
                        }
                        self.publish_state(events);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        let _ = events.send(ShareEvent::Error(error.to_string()));
                    }
                }
            }
            WorkerCommand::Connect { path } => self.connect(&path, events),
            WorkerCommand::Disconnect => {
                self.drop_link();
                self.publish_state(events);
            }
            WorkerCommand::Leave => {
                self.drop_link();
                self.session.clear_peers();
                self.joined = false;
                self.publish_state(events);
            }
            WorkerCommand::Stop => return true,
        }
        false
    }

    /// Dial `path` and handshake. The link carries every frame for [`LINK_PEER`]
    /// from here on, so the worker is a transport peer rather than a scripted
    /// relay.
    fn connect(&mut self, path: &Path, events: &Sender<ShareEvent>) {
        self.drop_link();
        let stream = match UnixStream::connect(path) {
            Ok(stream) => stream,
            Err(error) => {
                let _ = events.send(ShareEvent::Error(format!("share link: {error}")));
                return;
            }
        };
        if let Err(error) = stream.set_nonblocking(true) {
            let _ = events.send(ShareEvent::Error(format!("share link: {error}")));
            return;
        }
        let mut link = UnixLink::new(stream);
        let hello = Wire::Hello {
            peer: self.session.local_peer().to_string(),
            version: Vec::new(),
        };
        if let Err(error) = link.send(&hello) {
            let _ = events.send(ShareEvent::Error(format!("share link: {error}")));
            return;
        }
        // Register the remote end so inbound frames have a reliable line and
        // outbound frames a destination. The handshake delta travels over the
        // same socket as everything else.
        if !self.session.has_peer(LINK_PEER) {
            match self.session.join(&self.replica, LINK_PEER, &[]) {
                Ok(out) => {
                    if let Err(error) = link.send(&Wire::Delta {
                        seq: out.seq,
                        delta: out.delta,
                    }) {
                        let _ = events.send(ShareEvent::Error(format!("share link: {error}")));
                        return;
                    }
                    self.joined = true;
                }
                Err(error) => {
                    let _ = events.send(ShareEvent::Error(error.to_string()));
                    return;
                }
            }
        }
        self.link = Some(link);
        self.publish_state(events);
    }

    /// Read and apply everything the link has without waiting.
    fn pump_link(&mut self, events: &Sender<ShareEvent>) {
        let Some(mut link) = self.link.take() else {
            return;
        };
        let mut pending: Vec<Outbound> = Vec::new();
        let mut alive = true;
        let mut ack = None;
        loop {
            match link.try_recv() {
                Ok(LinkRead::Message(Wire::Delta { seq, delta })) => {
                    match self
                        .session
                        .receive(&mut self.replica, LINK_PEER, seq, &delta)
                    {
                        Ok(ReceiveOutcome::Applied { outbound, .. }) => {
                            let _ = events.send(ShareEvent::Applied {
                                deltas: vec![delta],
                            });
                            pending.extend(outbound);
                            ack = Some(seq);
                        }
                        // A duplicate is still received: acknowledge it so the
                        // host releases the retained copy.
                        Ok(_) => ack = Some(seq),
                        Err(error) => {
                            let _ = events.send(ShareEvent::Error(error.to_string()));
                        }
                    }
                }
                // Resync and any frame the link's other direction owns are
                // not document traffic for this replica.
                Ok(LinkRead::Message(_)) => {}
                Ok(LinkRead::Pending) => break,
                Ok(LinkRead::Closed) => {
                    alive = false;
                    break;
                }
                Err(error) => {
                    let _ = events.send(ShareEvent::Error(format!("share link: {error}")));
                    alive = false;
                    break;
                }
            }
        }
        if alive {
            // The host retains every frame until it is acknowledged; without
            // this a long share grows its per-peer queue until it isolates
            // the pad. One cumulative ack covers everything read this pass.
            if let Some(seq) = ack {
                let _ = link.send(&Wire::Ack { seq });
            }
            let _ = link.flush();
            self.link = Some(link);
        } else {
            self.session.leave(LINK_PEER);
            self.publish_state(events);
        }
        for out in pending {
            self.send_outbound(out, events);
        }
    }

    /// Write one frame to the attached link, or publish it for a relay when the
    /// link is absent or belongs to another peer.
    fn send_outbound(&mut self, out: Outbound, events: &Sender<ShareEvent>) {
        if out.peer == LINK_PEER {
            if let Some(link) = self.link.as_mut() {
                let frame = Wire::Delta {
                    seq: out.seq,
                    delta: out.delta.clone(),
                };
                match link.send(&frame) {
                    Ok(()) => return,
                    Err(error) => {
                        let _ = events.send(ShareEvent::Error(format!("share link: {error}")));
                    }
                }
            }
            self.drop_link();
        }
        let _ = events.send(ShareEvent::Outbound {
            peer: out.peer,
            seq: out.seq,
            delta: out.delta,
        });
    }

    fn drop_link(&mut self) {
        if let Some(link) = self.link.take() {
            let _ = link.shutdown();
            self.session.leave(LINK_PEER);
        }
    }

    fn publish_state(&self, events: &Sender<ShareEvent>) {
        let _ = events.send(ShareEvent::State {
            peers: Vec::new(),
            joined: self.joined,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle() -> ShareHandle {
        let doc = MathDoc::with_text("shared");
        ShareHandle::start(&doc.snapshot(), "pad", "Pad").unwrap()
    }

    fn wait_for(
        label: &str,
        handle: &ShareHandle,
        predicate: impl Fn(&ShareEvent) -> bool,
    ) -> ShareEvent {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut last = Vec::new();
        loop {
            for event in handle.poll() {
                if predicate(&event) {
                    return event;
                }
                last.push(event);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {label}: saw {last:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn the_worker_owns_the_replica_and_never_the_live_model() {
        let mut handle = handle();
        assert_eq!(handle.share_id().len(), 16);
        // The worker's replica is private to it; the handle exposes only
        // commands/events, never a MathDoc or DocModel.
        handle.join("peer", Vec::new()).unwrap();
        let event = wait_for("catch_up", &handle, |event| {
            matches!(event, ShareEvent::CatchUp { .. })
        });
        assert!(matches!(event, ShareEvent::CatchUp { .. }));
        handle.stop();
    }

    fn catch_up(handle: &ShareHandle, label: &str) -> Vec<u8> {
        match wait_for(label, handle, |event| {
            matches!(event, ShareEvent::CatchUp { .. })
        }) {
            ShareEvent::CatchUp { delta } => delta,
            _ => unreachable!(),
        }
    }

    /// The worker's replica text, observed the way a transport would: join a
    /// fresh peer and apply its catch-up delta to an empty document.
    fn replica_text(handle: &ShareHandle, probe: &str) -> String {
        handle.join(probe, Vec::new()).unwrap();
        let delta = catch_up(handle, &format!("{probe} catch_up"));
        let mut doc = MathDoc::new();
        doc.import_delta(&delta).unwrap();
        doc.text().to_string()
    }

    #[test]
    fn an_offline_then_reconnect_exchange_converges() {
        let doc = MathDoc::with_text("base");
        let snapshot = doc.snapshot();
        let mut a = ShareHandle::start(&snapshot, "a", "A").unwrap();
        let mut b = ShareHandle::start(&snapshot, "b", "B").unwrap();

        // Each side edits while nobody is connected: the operation stays in
        // its worker's replica and produces no traffic.
        let mut a_live = MathDoc::from_snapshot(&snapshot).unwrap();
        let mut b_live = MathDoc::from_snapshot(&snapshot).unwrap();
        a_live.insert(0, "A");
        b_live.insert(b_live.text().len(), "B");
        a.publish(a_live.export_delta()).unwrap();
        b.publish(b_live.export_delta()).unwrap();
        for event in a.poll().iter().chain(b.poll().iter()) {
            assert!(
                !matches!(event, ShareEvent::Outbound { .. }),
                "an offline edit produced traffic: {event:?}"
            );
        }

        // Reconnect: each host handshakes from an empty frontier and the
        // transport delivers the other side's catch-up to its replica.
        a.join("b", Vec::new()).unwrap();
        b.join("a", Vec::new()).unwrap();
        let a_catch = catch_up(&a, "a catch_up");
        let b_catch = catch_up(&b, "b catch_up");
        b.receive("a", 1, a_catch, None).unwrap();
        a.receive("b", 1, b_catch, None).unwrap();
        wait_for("b applied", &b, |event| {
            matches!(event, ShareEvent::Applied { .. })
        });
        wait_for("a applied", &a, |event| {
            matches!(event, ShareEvent::Applied { .. })
        });

        // A connected edit comes back out of the worker as a frame the
        // transport can deliver.
        a_live.insert(a_live.text().len(), "!");
        a.publish(a_live.export_delta()).unwrap();
        let (seq, delta) = match wait_for("outbound frame", &a, |event| {
            matches!(event, ShareEvent::Outbound { .. })
        }) {
            ShareEvent::Outbound { peer, seq, delta } => {
                assert_eq!(peer, "b");
                (seq, delta)
            }
            _ => unreachable!(),
        };
        b.receive("a", seq, delta, None).unwrap();
        wait_for("b applied the connected edit", &b, |event| {
            matches!(event, ShareEvent::Applied { .. })
        });

        // Both replicas converged on every edit.
        let a_text = replica_text(&a, "probe-a");
        let b_text = replica_text(&b, "probe-b");
        assert_eq!(a_text, b_text, "the two replicas diverged");
        for marker in ["A", "B", "!"] {
            assert!(a_text.contains(marker), "{marker} missing from {a_text:?}");
        }

        a.stop();
        b.stop();
    }

    /// A real socket peer, not a scripted relay: the worker dials an
    /// `emthin_collab::unix::Host` and frames travel in the shared wire
    /// framing. A connected peer's edit reaches the replica, and a published
    /// local edit reaches the host.
    #[test]
    fn a_socket_link_carries_edits_both_ways() {
        use emthin_collab::unix::Host;
        use std::time::{Duration, Instant};

        let base = MathDoc::with_text("base");
        let snapshot = base.snapshot();
        let dir = std::env::temp_dir().join(format!("emthin-pad-share-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("share.sock");
        let mut host = Host::bind(
            &path,
            MathDoc::from_snapshot(&snapshot).unwrap(),
            "host",
            "Host",
            60_000,
        )
        .expect("bind the share socket");
        let handle = ShareHandle::start(&snapshot, "pad", "Pad").unwrap();
        handle.connect(&path).unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            host.poll().expect("host poll");
            let _ = handle.poll();
            if host.has_peer("pad") {
                break;
            }
            assert!(Instant::now() < deadline, "the pad's link never handshook");
            std::thread::sleep(Duration::from_millis(5));
        }

        // Host -> worker: the frame arrives over the socket and applies to the
        // transport replica, without any relay in the test.
        host.doc_mut().insert(0, "H");
        host.local_edit().unwrap();
        assert!(
            !host.session().resend("pad", 0).unwrap().is_empty(),
            "the host retains a frame until the pad acknowledges it"
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut applied = false;
        while !applied {
            host.poll().expect("host poll");
            applied = handle
                .poll()
                .iter()
                .any(|event| matches!(event, ShareEvent::Applied { .. }));
            assert!(Instant::now() < deadline, "the host's edit never arrived");
            std::thread::sleep(Duration::from_millis(5));
        }
        let after_host = replica_text(&handle, "probe-host");
        assert!(after_host.contains('H'), "replica text was {after_host:?}");

        // Worker -> host: a published local edit is written to the socket by
        // the worker itself, so no outbound event is needed for this peer.
        let mut live = MathDoc::from_snapshot(&snapshot).unwrap();
        live.insert(live.text().len(), "P");
        handle.publish(live.export_delta()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            host.poll().expect("host poll");
            let events = handle.poll();
            assert!(
                !events.iter().any(|event| matches!(
                    event,
                    ShareEvent::Outbound { peer, .. } if peer == LINK_PEER
                )),
                "a connected frame must not also be published for a relay: {events:?}"
            );
            if host.doc().text().contains('P') {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the pad's edit never reached the host"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        // Both replicas agree on both edits.
        let published = replica_text(&handle, "probe-pad");
        assert_eq!(published, host.doc().text());
        assert!(published.contains('H') && published.contains('P'));

        // The pad acknowledges what it received, so the host releases its
        // retained frames instead of growing the queue until it isolates the
        // peer. Without the ack this list keeps every sent frame.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            host.poll().expect("host poll");
            let _ = handle.poll();
            if host.session().resend("pad", 0).unwrap().is_empty() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the pad never acknowledged the host's stream"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn leave_and_stop_are_clean() {
        let mut handle = handle();
        handle.join("peer", Vec::new()).unwrap();
        let _ = handle.poll();
        handle.leave().unwrap();
        wait_for("leave state", &handle, |event| {
            matches!(event, ShareEvent::State { joined: false, .. })
        });
        handle.stop();
        assert!(handle.worker.is_none());
    }
}
