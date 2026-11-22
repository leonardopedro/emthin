//! A transport-independent Herdr Core protocol client.
//!
//! The caller owns the socket or WebSocket. This crate owns the protocol:
//! framed bytes, JSON-RPC request correlation, the hello/attach handshake,
//! per-channel stream state, and the credit returned after consuming output.
//! The optional [`unix`] adapter is a thin local-socket transport.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;
use serde_json::{json, Value};

pub mod controller;
#[cfg(unix)]
pub mod unix;
pub mod wire;

pub use wire::{Frame, FrameDecoder, FrameError, LineDecoder, CREDIT, DATA, MAX_PAYLOAD, RESIZE};

const GENERATION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Protocol(String),
    Frame(FrameError),
    Rpc {
        code: i64,
        kind: String,
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(message) => write!(f, "protocol: {message}"),
            Self::Frame(error) => write!(f, "frame: {error}"),
            Self::Rpc {
                code,
                kind,
                message,
            } => {
                write!(f, "core error {code} ({kind}): {message}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<FrameError> for Error {
    fn from(error: FrameError) -> Self {
        Self::Frame(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub generation: u32,
    pub version: String,
    pub core_id: String,
    pub core_name: String,
    pub methods: Vec<String>,
    pub features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Hello(Hello),
    Subscribed {
        request_id: u64,
        channel: u32,
        seq: u64,
        terminals: Vec<Value>,
    },
    EventLine {
        channel: u32,
        line: Vec<u8>,
    },
    Attached {
        request_id: u64,
        channel: u32,
        observe: bool,
        resize_notifications: bool,
    },
    Data {
        channel: u32,
        bytes: Vec<u8>,
    },
    Resize {
        channel: u32,
        cols: u16,
        rows: u16,
    },
    Credit {
        channel: u32,
        bytes: u32,
    },
    Closed {
        channel: u32,
        reason: String,
    },
}

#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub events: Vec<Event>,
    /// Frames the caller must write, in order. DATA consumption produces a
    /// CREDIT here; the client never silently forgets to acknowledge output.
    pub outgoing: Vec<Frame>,
}

#[derive(Debug, Clone, Default)]
pub struct SubscribeFilter {
    pub terminal: Option<String>,
    pub children_of: Option<String>,
    pub docs: Vec<String>,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct AttachRequest {
    pub terminal: String,
    pub cols: u16,
    pub rows: u16,
    pub observe: bool,
    pub resize_notifications: bool,
    pub history: u32,
    pub caller: Option<String>,
}

impl AttachRequest {
    pub fn viewer(terminal: impl Into<String>, cols: u16, rows: u16) -> Self {
        Self {
            terminal: terminal.into(),
            cols,
            rows,
            observe: false,
            resize_notifications: false,
            history: 0,
            caller: None,
        }
    }

    pub fn observer(terminal: impl Into<String>, cols: u16, rows: u16) -> Self {
        Self {
            observe: true,
            resize_notifications: true,
            ..Self::viewer(terminal, cols, rows)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Fresh,
    Helloed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    Hello,
    Subscribe,
    Attach {
        observe: bool,
        resize_notifications: bool,
    },
}

enum Stream {
    Attach { observe: bool },
    Events { lines: LineDecoder },
}

pub struct Client {
    phase: Phase,
    next_id: u64,
    pending: BTreeMap<u64, Pending>,
    streams: BTreeMap<u32, Stream>,
    hello: Option<Hello>,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        Self {
            phase: Phase::Fresh,
            next_id: 0,
            pending: BTreeMap::new(),
            streams: BTreeMap::new(),
            hello: None,
        }
    }

    pub fn hello(&mut self) -> Result<(u64, Frame), Error> {
        if self.phase != Phase::Fresh {
            return Err(Error::Protocol("hello has already been sent".into()));
        }
        let id = self.id();
        self.pending.insert(id, Pending::Hello);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "hello",
            "params": {"framed": true, "generation": GENERATION},
        });
        Ok((id, control_frame(&body)?))
    }

    pub fn subscribe(&mut self, filter: &SubscribeFilter) -> Result<(u64, Frame), Error> {
        if self.phase != Phase::Helloed {
            return Err(Error::Protocol(
                "subscribe requires a completed hello".into(),
            ));
        }
        let id = self.id();
        self.pending.insert(id, Pending::Subscribe);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "events.subscribe",
            "params": {
                "terminal": filter.terminal,
                "children_of": filter.children_of,
                "docs": filter.docs,
                "labels": filter.labels,
            },
        });
        Ok((id, control_frame(&body)?))
    }

    pub fn attach(&mut self, request: AttachRequest) -> Result<(u64, Frame), Error> {
        if self.phase != Phase::Helloed {
            return Err(Error::Protocol("attach requires a completed hello".into()));
        }
        let id = self.id();
        self.pending.insert(
            id,
            Pending::Attach {
                observe: request.observe,
                resize_notifications: request.resize_notifications,
            },
        );
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "terminal.attach",
            "params": {
                "terminal": request.terminal,
                "cols": request.cols,
                "rows": request.rows,
                "observe": request.observe,
                "resize_notifications": request.resize_notifications,
                "history": request.history,
                "caller": request.caller,
            },
        });
        Ok((id, control_frame(&body)?))
    }

    pub fn handle(&mut self, frame: Frame) -> Result<Outcome, Error> {
        let mut outcome = Outcome::default();
        if frame.channel == 0 {
            if frame.kind != DATA {
                return Err(Error::Protocol(format!(
                    "control channel got frame kind {}",
                    frame.kind
                )));
            }
            self.handle_control(&frame.payload, &mut outcome)?;
            return Ok(outcome);
        }

        if !self.streams.contains_key(&frame.channel) {
            return Err(Error::Protocol(format!(
                "frame for unattached channel {}",
                frame.channel
            )));
        }
        let events_channel = matches!(
            self.streams.get(&frame.channel),
            Some(Stream::Events { .. })
        );
        match frame.kind {
            DATA => {
                let len = frame.payload.len() as u32;
                if events_channel {
                    let lines = match self.streams.get_mut(&frame.channel) {
                        Some(Stream::Events { lines }) => lines.push(&frame.payload)?,
                        _ => unreachable!("events_channel checked above"),
                    };
                    for line in lines {
                        outcome.events.push(Event::EventLine {
                            channel: frame.channel,
                            line,
                        });
                    }
                } else {
                    outcome.events.push(Event::Data {
                        channel: frame.channel,
                        bytes: frame.payload,
                    });
                }
                outcome.outgoing.push(Frame {
                    channel: frame.channel,
                    kind: CREDIT,
                    payload: len.to_le_bytes().to_vec(),
                });
            }
            RESIZE => {
                if frame.payload.len() != 4 {
                    return Err(Error::Protocol(format!(
                        "RESIZE payload is {} bytes, want 4",
                        frame.payload.len()
                    )));
                }
                let cols = u16::from_le_bytes([frame.payload[0], frame.payload[1]]);
                let rows = u16::from_le_bytes([frame.payload[2], frame.payload[3]]);
                outcome.events.push(Event::Resize {
                    channel: frame.channel,
                    cols,
                    rows,
                });
            }
            wire::CREDIT => {
                if frame.payload.len() != 4 {
                    return Err(Error::Protocol("CREDIT payload must be 4 bytes".into()));
                }
                let bytes = u32::from_le_bytes(
                    frame.payload.as_slice().try_into().expect("checked length"),
                );
                outcome.events.push(Event::Credit {
                    channel: frame.channel,
                    bytes,
                });
            }
            wire::CLOSE => {
                self.streams.remove(&frame.channel);
                outcome.events.push(Event::Closed {
                    channel: frame.channel,
                    reason: String::from_utf8_lossy(&frame.payload).into_owned(),
                });
            }
            kind => {
                return Err(Error::Protocol(format!(
                    "core sent unexpected frame kind {kind} on channel {}",
                    frame.channel
                )));
            }
        }
        Ok(outcome)
    }

    pub fn input(&self, channel: u32, bytes: impl Into<Vec<u8>>) -> Result<Frame, Error> {
        self.writable(channel, "input")?;
        Ok(Frame::data(channel, bytes))
    }

    pub fn resize(&self, channel: u32, cols: u16, rows: u16) -> Result<Frame, Error> {
        self.writable(channel, "resize")?;
        Ok(Frame {
            channel,
            kind: RESIZE,
            payload: [cols.to_le_bytes(), rows.to_le_bytes()].concat(),
        })
    }

    pub fn close(&self, channel: u32) -> Result<Frame, Error> {
        if !self.streams.contains_key(&channel) {
            return Err(Error::Protocol(format!(
                "close for unattached channel {channel}"
            )));
        }
        Ok(Frame {
            channel,
            kind: wire::CLOSE,
            payload: Vec::new(),
        })
    }

    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }

    fn writable(&self, channel: u32, action: &str) -> Result<(), Error> {
        match self.streams.get(&channel) {
            Some(Stream::Attach { observe: false }) => Ok(()),
            Some(Stream::Attach { observe: true }) => Err(Error::Protocol(format!(
                "watch channel {channel} cannot {action}"
            ))),
            Some(Stream::Events { .. }) => Err(Error::Protocol(format!(
                "event channel {channel} cannot {action}"
            ))),
            None => Err(Error::Protocol(format!(
                "{action} for unattached channel {channel}"
            ))),
        }
    }

    pub fn hello_info(&self) -> Option<&Hello> {
        self.hello.as_ref()
    }

    fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn handle_control(&mut self, payload: &[u8], outcome: &mut Outcome) -> Result<(), Error> {
        let value: Value =
            serde_json::from_slice(payload).map_err(|e| Error::Protocol(e.to_string()))?;
        if let Some(error) = value.get("error") {
            return Err(rpc_error(error));
        }
        let id = value
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::Protocol("reply has no numeric id".into()))?;
        let pending = self
            .pending
            .remove(&id)
            .ok_or_else(|| Error::Protocol(format!("reply for unknown request {id}")))?;
        let result = value
            .get("result")
            .cloned()
            .ok_or_else(|| Error::Protocol(format!("reply {id} has no result")))?;
        match pending {
            Pending::Hello => {
                let reply: HelloReply = serde_json::from_value(result)
                    .map_err(|e| Error::Protocol(format!("bad hello reply: {e}")))?;
                if !reply.framed {
                    return Err(Error::Protocol(
                        "core did not accept framed mode (capability downgrade)".into(),
                    ));
                }
                if reply.generation != GENERATION {
                    return Err(Error::Protocol(format!(
                        "core speaks generation {}, want {GENERATION}",
                        reply.generation
                    )));
                }
                if !reply.methods.iter().any(|m| m == "terminal.attach") {
                    return Err(Error::Protocol(
                        "core does not advertise terminal.attach".into(),
                    ));
                }
                let hello = Hello {
                    generation: reply.generation,
                    version: reply.version,
                    core_id: reply.core.id,
                    core_name: reply.core.name,
                    methods: reply.methods,
                    features: reply.features,
                };
                self.hello = Some(hello.clone());
                self.phase = Phase::Helloed;
                outcome.events.push(Event::Hello(hello));
            }
            Pending::Subscribe => {
                let reply: SubscribeReply = serde_json::from_value(result)
                    .map_err(|e| Error::Protocol(format!("bad subscribe reply: {e}")))?;
                let channel = reply.channel.ok_or_else(|| {
                    Error::Protocol("framed subscribe returned no channel".into())
                })?;
                if channel == 0 {
                    return Err(Error::Protocol(
                        "subscribe reply used control channel 0".into(),
                    ));
                }
                if self.streams.contains_key(&channel) {
                    return Err(Error::Protocol(format!(
                        "subscribe reply reused channel {channel}"
                    )));
                }
                self.streams.insert(
                    channel,
                    Stream::Events {
                        lines: LineDecoder::default(),
                    },
                );
                outcome.events.push(Event::Subscribed {
                    request_id: id,
                    channel,
                    seq: reply.seq,
                    terminals: reply.terminals,
                });
            }
            Pending::Attach {
                observe,
                resize_notifications,
            } => {
                let reply: AttachReply = serde_json::from_value(result)
                    .map_err(|e| Error::Protocol(format!("bad attach reply: {e}")))?;
                if reply.channel == 0 {
                    return Err(Error::Protocol(
                        "attach reply used control channel 0".into(),
                    ));
                }
                if self.streams.contains_key(&reply.channel) {
                    return Err(Error::Protocol(format!(
                        "attach reply reused channel {}",
                        reply.channel
                    )));
                }
                self.streams
                    .insert(reply.channel, Stream::Attach { observe });
                outcome.events.push(Event::Attached {
                    request_id: id,
                    channel: reply.channel,
                    observe,
                    resize_notifications,
                });
            }
        }
        Ok(())
    }
}

fn control_frame(value: &Value) -> Result<Frame, Error> {
    let payload = serde_json::to_vec(value).map_err(|e| Error::Protocol(e.to_string()))?;
    Ok(Frame::data(0, payload))
}

fn rpc_error(value: &Value) -> Error {
    let code = value.get("code").and_then(Value::as_i64).unwrap_or(-32000);
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("core error")
        .to_owned();
    let kind = value
        .get("data")
        .and_then(|d| d.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("unavailable")
        .to_owned();
    Error::Rpc {
        code,
        kind,
        message,
    }
}

#[derive(Deserialize)]
struct HelloReply {
    generation: u32,
    version: String,
    core: CoreInfo,
    framed: bool,
    methods: Vec<String>,
    #[serde(default)]
    features: Vec<String>,
}

#[derive(Deserialize)]
struct CoreInfo {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct AttachReply {
    channel: u32,
}

#[derive(Deserialize)]
struct SubscribeReply {
    channel: Option<u32>,
    seq: u64,
    #[serde(default)]
    terminals: Vec<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(id: u64, result: Value) -> Frame {
        Frame::data(
            0,
            serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "result": result})).unwrap(),
        )
    }

    fn hello_reply(id: u64) -> Frame {
        reply(
            id,
            json!({
                "generation": 1,
                "version": "0.1.21",
                "core": {"id": "core-1", "name": "host"},
                "framed": true,
                "methods": ["hello", "terminal.attach"],
                "features": ["resize-notifications"]
            }),
        )
    }

    fn attach_reply(id: u64, channel: u32) -> Frame {
        reply(
            id,
            json!({
                "channel": channel,
                "terminal": {"id": "t1"},
                "resize_notifications": true,
                "automatic_input": true
            }),
        )
    }

    fn hello(client: &mut Client) {
        let (id, _) = client.hello().unwrap();
        let events = client.handle(hello_reply(id)).unwrap().events;
        assert!(matches!(events.as_slice(), [Event::Hello(_)]));
    }

    fn attach(client: &mut Client, channel: u32) {
        let (id, _) = client.attach(AttachRequest::viewer("t1", 80, 24)).unwrap();
        let events = client.handle(attach_reply(id, channel)).unwrap().events;
        assert!(matches!(
            events.as_slice(),
            [Event::Attached { channel: got, .. }] if *got == channel
        ));
    }

    #[test]
    fn hello_reply_and_a_following_frame_can_be_coalesced() {
        let mut client = Client::new();
        let (id, _) = client.hello().unwrap();
        let mut bytes = wire::encode(hello_reply(id)).unwrap();
        bytes.extend(wire::encode(Frame::data(4, b"early")).unwrap());
        let mut decoder = FrameDecoder::default();
        let frames = decoder.push(&bytes).unwrap();
        client.handle(frames[0].clone()).unwrap();
        let error = client.handle(frames[1].clone()).unwrap_err();
        assert!(matches!(error, Error::Protocol(_)));
    }

    #[test]
    fn attach_reply_installs_the_channel_before_data() {
        let mut client = Client::new();
        hello(&mut client);
        let (id, _) = client.attach(AttachRequest::viewer("t1", 80, 24)).unwrap();
        let attached = client.handle(attach_reply(id, 7)).unwrap().events;
        assert_eq!(attached.len(), 1);
        let outcome = client.handle(Frame::data(7, b"hello")).unwrap();
        assert_eq!(
            outcome.events,
            vec![Event::Data {
                channel: 7,
                bytes: b"hello".to_vec()
            }]
        );
        assert_eq!(outcome.outgoing.len(), 1);
        assert_eq!(outcome.outgoing[0].kind, CREDIT);
        assert_eq!(outcome.outgoing[0].payload, 5u32.to_le_bytes());
    }

    #[test]
    fn an_observer_receives_resize_before_data() {
        let mut client = Client::new();
        hello(&mut client);
        let (id, _) = client
            .attach(AttachRequest::observer("t1", 80, 24))
            .unwrap();
        client.handle(attach_reply(id, 3)).unwrap();
        let resized = client
            .handle(Frame {
                channel: 3,
                kind: RESIZE,
                payload: vec![80, 0, 24, 0],
            })
            .unwrap();
        let data = client.handle(Frame::data(3, b"x")).unwrap();
        assert!(matches!(
            resized.events[0],
            Event::Resize {
                cols: 80,
                rows: 24,
                ..
            }
        ));
        assert!(matches!(data.events[0], Event::Data { .. }));
    }

    #[test]
    fn two_channels_do_not_stall_or_mix_each_other() {
        let mut client = Client::new();
        hello(&mut client);
        attach(&mut client, 1);
        attach(&mut client, 2);
        let one = client.handle(Frame::data(1, b"one")).unwrap();
        let two = client.handle(Frame::data(2, b"two")).unwrap();
        assert_eq!(
            one.events[0],
            Event::Data {
                channel: 1,
                bytes: b"one".to_vec()
            }
        );
        assert_eq!(
            two.events[0],
            Event::Data {
                channel: 2,
                bytes: b"two".to_vec()
            }
        );
        assert_eq!(one.outgoing[0].channel, 1);
        assert_eq!(two.outgoing[0].channel, 2);
    }

    #[test]
    fn subscribe_events_split_and_coalesce_and_do_not_accept_input() {
        let mut client = Client::new();
        hello(&mut client);
        let (id, _) = client.subscribe(&SubscribeFilter::default()).unwrap();
        let subscribed = client
            .handle(reply(
                id,
                json!({
                    "channel": 9,
                    "seq": 41,
                    "terminals": [{"id": "t1"}]
                }),
            ))
            .unwrap();
        assert!(matches!(
            subscribed.events.as_slice(),
            [Event::Subscribed {
                channel: 9,
                seq: 41,
                ..
            }]
        ));

        let first = client
            .handle(Frame::data(9, b"{\"event\":\"a\"}\n{\"event\""))
            .unwrap();
        let second = client.handle(Frame::data(9, b":\"b\"}\n")).unwrap();
        assert_eq!(first.events.len(), 1);
        assert_eq!(second.events.len(), 1);
        assert_eq!(first.outgoing[0].kind, CREDIT);
        assert!(client.input(9, b"x").is_err());
    }

    #[test]
    fn a_non_json_control_payload_is_rejected() {
        let mut client = Client::new();
        let error = client.handle(Frame::data(0, b"not json")).unwrap_err();
        assert!(matches!(error, Error::Protocol(_)));
    }

    #[test]
    fn a_watch_channel_cannot_type_or_resize() {
        let mut client = Client::new();
        hello(&mut client);
        let (id, _) = client
            .attach(AttachRequest::observer("t1", 80, 24))
            .unwrap();
        client.handle(attach_reply(id, 4)).unwrap();
        assert!(client.input(4, b"x").is_err());
        assert!(client.resize(4, 90, 30).is_err());
    }

    #[test]
    fn closure_removes_the_stream_and_rejects_late_data() {
        let mut client = Client::new();
        hello(&mut client);
        attach(&mut client, 7);
        let closed = client
            .handle(Frame {
                channel: 7,
                kind: wire::CLOSE,
                payload: b"done".to_vec(),
            })
            .unwrap();
        assert_eq!(
            closed.events,
            vec![Event::Closed {
                channel: 7,
                reason: "done".into()
            }]
        );
        assert_eq!(client.stream_count(), 0);
        assert!(client.handle(Frame::data(7, b"late")).is_err());
    }

    #[test]
    fn a_generation_mismatch_is_rejected_as_a_capability_downgrade() {
        let mut client = Client::new();
        let (id, _) = client.hello().unwrap();
        let error = client
            .handle(reply(
                id,
                json!({
                    "generation": 2,
                    "version": "9",
                    "core": {"id": "c", "name": "h"},
                    "framed": true,
                    "methods": ["terminal.attach"],
                    "features": []
                }),
            ))
            .unwrap_err();
        assert!(matches!(error, Error::Protocol(message) if message.contains("generation")));
    }

    #[test]
    fn an_rpc_error_keeps_its_stable_kind() {
        let mut client = Client::new();
        let (id, _) = client.hello().unwrap();
        let error = client
            .handle(Frame::data(
                0,
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32000, "message": "nope", "data": {"kind": "not_found"}}
                }))
                .unwrap(),
            ))
            .unwrap_err();
        assert_eq!(
            error,
            Error::Rpc {
                code: -32000,
                kind: "not_found".into(),
                message: "nope".into()
            }
        );
    }
}
