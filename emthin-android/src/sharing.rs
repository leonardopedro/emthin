//! Native sharing bridge for the pad (Kyneta K6).
//!
//! The worker thread owns the transport-side replica and the collaboration
//! `Session`. The live `DocModel` stays on the UI thread: it only receives
//! encoded deltas (one grouped apply) and publishes its own encoded deltas.
//! The transport is scripted through channels here; a real socket/gateway
//! adapter is the device-lane gap recorded in the plan.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::JoinHandle;

use emthin_collab::wire::ShareId;
use emthin_collab::{Outbound, ReceiveOutcome, Session};
use mathed_core::doc::MathDoc;
use serde_json::{json, Value};

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
}

impl WorkerState {
    fn run(self, commands: Receiver<WorkerCommand>, events: Sender<ShareEvent>) {
        let mut state = self;
        while let Ok(command) = commands.recv() {
            match command {
                WorkerCommand::Join { peer, version } => {
                    match state.session.join(&state.replica, peer, &version) {
                        Ok(out) => {
                            state.joined = true;
                            let _ = events.send(ShareEvent::CatchUp { delta: out.delta });
                            state.publish_state(&events);
                        }
                        Err(error) => {
                            let _ = events.send(ShareEvent::Error(error.to_string()));
                        }
                    }
                }
                WorkerCommand::Publish { delta } => {
                    if let Err(error) = state.replica.import_delta(&delta) {
                        let _ = events.send(ShareEvent::Error(error.to_string()));
                        continue;
                    }
                    match state.session.local_edit(&state.replica) {
                        Ok(out) => state.publish_outbounds(out, &events),
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
                    match state.session.receive_scoped(
                        &mut state.replica,
                        &peer,
                        seq,
                        &delta,
                        scope,
                    ) {
                        Ok(ReceiveOutcome::Applied { outbound, .. }) => {
                            let _ = events.send(ShareEvent::Applied {
                                deltas: vec![delta],
                            });
                            state.publish_outbounds(outbound, &events);
                            state.publish_state(&events);
                        }
                        Ok(_) => {}
                        Err(error) => {
                            let _ = events.send(ShareEvent::Error(error.to_string()));
                        }
                    }
                }
                WorkerCommand::Leave => {
                    state.session.clear_peers();
                    state.joined = false;
                    state.publish_state(&events);
                }
                WorkerCommand::Stop => break,
            }
        }
    }

    fn publish_outbounds(&self, outbounds: Vec<Outbound>, events: &Sender<ShareEvent>) {
        // Outbounds are the frames a real transport would send. Each one is
        // published with its peer, sequence and delta, so a transport (or a
        // scripted test relay) can deliver it instead of losing it in the
        // worker.
        for out in outbounds {
            let _ = events.send(ShareEvent::Outbound {
                peer: out.peer,
                seq: out.seq,
                delta: out.delta,
            });
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
