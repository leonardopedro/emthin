//! A scripted Core peer: no socket, no Core, no Zig.
//!
//! One stream gets its credit replenished and keeps producing data. The other
//! deliberately withholds credit once its initial window is spent. The client
//! must keep the productive stream and channel-zero control responsive.

use std::collections::{BTreeMap, BTreeSet};

use emthin_core_client::wire::{self, Frame, FrameDecoder};
use emthin_core_client::{AttachRequest, Client, Event, SubscribeFilter, CREDIT};

struct ScriptedPeer {
    windows: BTreeMap<u32, usize>,
    withheld: BTreeSet<u32>,
}

impl ScriptedPeer {
    fn new() -> Self {
        Self {
            windows: BTreeMap::new(),
            withheld: BTreeSet::new(),
        }
    }

    fn open(&mut self, channel: u32, window: usize) {
        self.windows.insert(channel, window);
    }

    fn withhold_credit(&mut self, channel: u32) {
        self.withheld.insert(channel);
    }

    fn data(&mut self, channel: u32, bytes: &[u8]) -> Option<Frame> {
        let window = self.windows.get_mut(&channel)?;
        if *window < bytes.len() {
            return None;
        }
        *window -= bytes.len();
        Some(Frame::data(channel, bytes))
    }

    fn credit(&mut self, channel: u32, bytes: usize) {
        if self.withheld.contains(&channel) {
            return;
        }
        if let Some(window) = self.windows.get_mut(&channel) {
            *window += bytes;
        }
    }
}

fn reply(id: u64, result: serde_json::Value) -> Frame {
    Frame::data(
        0,
        serde_json::to_vec(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
        .expect("encode reply"),
    )
}

fn decode(frame: Frame) -> Vec<Frame> {
    FrameDecoder::default()
        .push(&wire::encode(frame).expect("encode frame"))
        .expect("decode frame")
}

#[test]
fn one_stream_produces_while_another_withholds_credit() {
    let mut peer = ScriptedPeer::new();
    let mut client = Client::new();

    let (hello_id, request) = client.hello().expect("hello request");
    assert_eq!(decode(request).len(), 1);
    let hello = client
        .handle(reply(
            hello_id,
            serde_json::json!({
                "generation": 1,
                "version": "0.1.21",
                "core": {"id": "scripted", "name": "fixture"},
                "framed": true,
                "methods": ["hello", "terminal.attach", "events.subscribe"],
                "features": ["resize-notifications"],
            }),
        ))
        .expect("hello reply");
    assert!(matches!(hello.events.as_slice(), [Event::Hello(_)]));
    peer.open(1, 16);
    peer.open(2, 4);

    let (attach_one, request) = client
        .attach(AttachRequest::viewer("one", 80, 24))
        .expect("attach one");
    assert_eq!(decode(request).len(), 1);
    let attached = client
        .handle(reply(
            attach_one,
            serde_json::json!({
                "channel": 1,
                "terminal": {"id": "one"},
                "resize_notifications": true,
                "automatic_input": true,
            }),
        ))
        .expect("attach reply one");
    assert!(matches!(
        attached.events.as_slice(),
        [Event::Attached { channel: 1, .. }]
    ));

    let (attach_two, request) = client
        .attach(AttachRequest::viewer("two", 80, 24))
        .expect("attach two");
    assert_eq!(decode(request).len(), 1);
    let attached = client
        .handle(reply(
            attach_two,
            serde_json::json!({
                "channel": 2,
                "terminal": {"id": "two"},
                "resize_notifications": true,
                "automatic_input": true,
            }),
        ))
        .expect("attach reply two");
    assert!(matches!(
        attached.events.as_slice(),
        [Event::Attached { channel: 2, .. }]
    ));

    // Stream one consumes its window and is replenished by the client's CREDIT.
    let one = peer.data(1, b"aaaa").expect("first stream window");
    let outcome = client.handle(one).expect("stream one data");
    assert_eq!(outcome.events.len(), 1);
    let credit = &outcome.outgoing[0];
    assert_eq!(credit.kind, CREDIT);
    peer.credit(
        1,
        u32::from_le_bytes(credit.payload[..].try_into().unwrap()) as usize,
    );
    let live = peer.data(1, b"bbbb").expect("stream one stays live");
    let outcome = client.handle(live).expect("stream one data again");
    let credit = &outcome.outgoing[0];
    peer.credit(
        1,
        u32::from_le_bytes(credit.payload[..].try_into().unwrap()) as usize,
    );

    // Stream two spends its window, then the peer withholds its credit.
    let two = peer.data(2, b"cccc").expect("second stream window");
    let outcome = client.handle(two).expect("stream two data");
    assert_eq!(outcome.events.len(), 1);
    let credit = &outcome.outgoing[0];
    peer.withhold_credit(2);
    peer.credit(
        2,
        u32::from_le_bytes(credit.payload[..].try_into().unwrap()) as usize,
    );
    assert!(
        peer.data(2, b"d").is_none(),
        "withheld credit must stop stream two"
    );

    // Control and stream one still make progress while stream two is starved.
    let (subscribe_id, _) = client
        .subscribe(&SubscribeFilter::default())
        .expect("subscribe request");
    let control = client
        .handle(reply(
            subscribe_id,
            serde_json::json!({"channel": 9, "seq": 1, "terminals": []}),
        ))
        .expect("control reply");
    assert!(matches!(
        control.events.as_slice(),
        [Event::Subscribed { channel: 9, .. }]
    ));
    let one = peer.data(1, b"still-live").expect("stream one credit");
    assert!(matches!(
        client
            .handle(one)
            .expect("stream one data")
            .events
            .as_slice(),
        [Event::Data { channel: 1, .. }]
    ));
}
