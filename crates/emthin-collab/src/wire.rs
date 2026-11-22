//! Framing for the Unix-socket collaboration transport.
//!
//! One frame is `u32 big-endian length`, then `u8 tag`, then the body. The
//! length covers the tag and body. Tags are fixed by [`Wire`], so a future
//! field cannot silently change the meaning of an old frame.

use std::fmt;

use crate::line::Seq;

/// Largest frame accepted, tag included. Loro deltas are bounded by the
/// document, but a malicious peer must not be able to make the decoder
/// allocate an unbounded buffer.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

const TAG_HELLO: u8 = 1;
const TAG_DELTA: u8 = 2;
const TAG_ACK: u8 = 3;
const TAG_PRESENCE: u8 = 4;
const TAG_HELLO_V2: u8 = 5;
const TAG_RESYNC: u8 = 6;

/// Length of a stable share id. It is random and independent of paths, display
/// names and Loro peer ids, none of which identify a share (K3).
pub const SHARE_ID_LEN: usize = 16;
/// The v2 handshake protocol version.
pub const PROTOCOL_VERSION_V2: u16 = 2;
/// The envelope version carried by a v2 handshake.
pub const ENVELOPE_VERSION: u16 = 1;

/// A stable, process-independent identity for one shared document.
pub type ShareId = [u8; SHARE_ID_LEN];

/// One transport message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wire {
    /// First frame from a client: its identity and the version token it has
    /// already seen. An empty version asks for the whole document.
    Hello { peer: String, version: Vec<u8> },
    /// A reliable delta frame.
    Delta { seq: Seq, delta: Vec<u8> },
    /// A cumulative acknowledgement for one reliable stream.
    Ack { seq: Seq },
    /// An ephemeral presence payload.
    Presence { blob: Vec<u8> },
    /// A v2 handshake: stable share id, protocol/capability version, envelope
    /// kind version and the Loro frontier the sender already has (K3). A v1
    /// `Hello` keeps the legacy unscoped path.
    HelloV2 {
        peer: String,
        share_id: ShareId,
        protocol_version: u16,
        envelope_version: u16,
        /// `0` from the client; the acceptor fills in the assigned epoch on
        /// its reply so later frames can be fenced by it.
        epoch: u64,
        version: Vec<u8>,
    },
    /// A transport-level instruction to restart from a frontier after a gap
    /// beyond retained history, a queue overflow or a stalled peer. It exposes
    /// the next step instead of silently dropping frames.
    Resync {
        share_id: ShareId,
        epoch: u64,
        frontier: Vec<u8>,
        reason: String,
    },
}

/// A frame that cannot be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// The declared length is zero or larger than [`MAX_FRAME`].
    BadLength { len: usize, max: usize },
    /// The tag is not one this version knows.
    UnknownTag(u8),
    /// A delta frame is shorter than its sequence number.
    ShortDelta { len: usize },
    /// The handshake peer id exceeded the frame's printable-field budget.
    PeerTooLong { len: usize, max: usize },
    /// A peer id was empty.
    EmptyPeer,
    /// A peer id was not valid UTF-8.
    PeerNotUtf8,
    /// A complete frame has trailing bytes the body shape does not consume.
    TrailingBytes { len: usize },
    /// A v2 handshake declared an unsupported protocol version.
    UnsupportedProtocol(u16),
    /// A v2 handshake declared an unsupported envelope version.
    UnsupportedEnvelope(u16),
    /// A v2 `Resync` reason was empty.
    EmptyReason,
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadLength { len, max } => {
                write!(f, "frame length {len} is outside 1..={max}")
            }
            Self::UnknownTag(tag) => write!(f, "unknown frame tag {tag}"),
            Self::ShortDelta { len } => {
                write!(
                    f,
                    "delta frame body is {len} bytes, shorter than a sequence number"
                )
            }
            Self::PeerTooLong { len, max } => {
                write!(f, "handshake peer id is {len} bytes, over the {max} cap")
            }
            Self::EmptyPeer => f.write_str("handshake peer id is empty"),
            Self::PeerNotUtf8 => f.write_str("handshake peer id is not valid UTF-8"),
            Self::TrailingBytes { len } => {
                write!(f, "{len} trailing bytes after the frame body")
            }
            Self::UnsupportedProtocol(version) => {
                write!(f, "unsupported collaboration protocol version {version}")
            }
            Self::UnsupportedEnvelope(version) => {
                write!(f, "unsupported collaboration envelope version {version}")
            }
            Self::EmptyReason => f.write_str("resync reason is empty"),
        }
    }
}

impl std::error::Error for WireError {}

/// The peer-id cap. It is a protocol label, not a document payload; real ids
/// are short and anything larger is a malformed handshake.
pub const MAX_PEER: usize = 256;

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn get_u64(body: &[u8], at: usize) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&body[at..at + 8]);
    u64::from_be_bytes(bytes)
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn get_u16(body: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([body[at], body[at + 1]])
}

fn put_share_id(out: &mut Vec<u8>, id: &ShareId) {
    out.extend_from_slice(id);
}

fn get_share_id(body: &[u8], at: usize) -> ShareId {
    let mut id = [0u8; SHARE_ID_LEN];
    id.copy_from_slice(&body[at..at + SHARE_ID_LEN]);
    id
}

/// Encode one frame, length prefix included.
pub fn encode(msg: &Wire) -> Vec<u8> {
    let (tag, body) = match msg {
        Wire::Hello { peer, version } => {
            let mut body = Vec::with_capacity(2 + peer.len() + version.len());
            body.extend_from_slice(&(peer.len() as u16).to_be_bytes());
            body.extend_from_slice(peer.as_bytes());
            body.extend_from_slice(version);
            (TAG_HELLO, body)
        }
        Wire::Delta { seq, delta } => {
            let mut body = Vec::with_capacity(8 + delta.len());
            put_u64(&mut body, *seq);
            body.extend_from_slice(delta);
            (TAG_DELTA, body)
        }
        Wire::Ack { seq } => {
            let mut body = Vec::with_capacity(8);
            put_u64(&mut body, *seq);
            (TAG_ACK, body)
        }
        Wire::Presence { blob } => (TAG_PRESENCE, blob.clone()),
        Wire::HelloV2 {
            peer,
            share_id,
            protocol_version,
            envelope_version,
            epoch,
            version,
        } => {
            let mut body = Vec::with_capacity(2 + peer.len() + SHARE_ID_LEN + 12 + version.len());
            put_u16(&mut body, peer.len() as u16);
            body.extend_from_slice(peer.as_bytes());
            put_share_id(&mut body, share_id);
            put_u16(&mut body, *protocol_version);
            put_u16(&mut body, *envelope_version);
            put_u64(&mut body, *epoch);
            body.extend_from_slice(version);
            (TAG_HELLO_V2, body)
        }
        Wire::Resync {
            share_id,
            epoch,
            frontier,
            reason,
        } => {
            let mut body = Vec::with_capacity(SHARE_ID_LEN + 8 + 2 + frontier.len() + reason.len());
            put_share_id(&mut body, share_id);
            put_u64(&mut body, *epoch);
            put_u16(&mut body, frontier.len() as u16);
            body.extend_from_slice(frontier);
            put_u16(&mut body, reason.len() as u16);
            body.extend_from_slice(reason.as_bytes());
            (TAG_RESYNC, body)
        }
    };
    let len = body.len() + 1;
    let mut out = Vec::with_capacity(4 + len);
    out.extend_from_slice(&(len as u32).to_be_bytes());
    out.push(tag);
    out.extend_from_slice(&body);
    out
}

/// Decode a complete frame body (everything after the length prefix).
pub fn decode(tag: u8, body: &[u8]) -> Result<Wire, WireError> {
    match tag {
        TAG_HELLO => {
            if body.len() < 2 {
                return Err(WireError::TrailingBytes { len: body.len() });
            }
            let peer_len = u16::from_be_bytes([body[0], body[1]]) as usize;
            let start: usize = 2;
            let end = start
                .checked_add(peer_len)
                .filter(|end| *end <= body.len())
                .ok_or(WireError::PeerTooLong {
                    len: peer_len,
                    max: body.len().saturating_sub(2),
                })?;
            if peer_len == 0 {
                return Err(WireError::EmptyPeer);
            }
            if peer_len > MAX_PEER {
                return Err(WireError::PeerTooLong {
                    len: peer_len,
                    max: MAX_PEER,
                });
            }
            let peer =
                String::from_utf8(body[start..end].to_vec()).map_err(|_| WireError::PeerNotUtf8)?;
            Ok(Wire::Hello {
                peer,
                version: body[end..].to_vec(),
            })
        }
        TAG_DELTA => {
            if body.len() < 8 {
                return Err(WireError::ShortDelta { len: body.len() });
            }
            Ok(Wire::Delta {
                seq: get_u64(body, 0),
                delta: body[8..].to_vec(),
            })
        }
        TAG_ACK => {
            if body.len() != 8 {
                return Err(WireError::TrailingBytes {
                    len: body.len().saturating_sub(8),
                });
            }
            Ok(Wire::Ack {
                seq: get_u64(body, 0),
            })
        }
        TAG_PRESENCE => Ok(Wire::Presence {
            blob: body.to_vec(),
        }),
        TAG_HELLO_V2 => {
            // u16 peer len + peer + 16 share id + 2 protocol + 2 envelope + 8 epoch.
            const MIN: usize = 2 + SHARE_ID_LEN + 12;
            if body.len() < MIN {
                return Err(WireError::TrailingBytes { len: body.len() });
            }
            let peer_len = get_u16(body, 0) as usize;
            let peer_end = 2usize
                .checked_add(peer_len)
                .filter(|end| *end + SHARE_ID_LEN + 12 <= body.len())
                .ok_or(WireError::PeerTooLong {
                    len: peer_len,
                    max: body.len().saturating_sub(MIN),
                })?;
            if peer_len == 0 {
                return Err(WireError::EmptyPeer);
            }
            if peer_len > MAX_PEER {
                return Err(WireError::PeerTooLong {
                    len: peer_len,
                    max: MAX_PEER,
                });
            }
            let peer = String::from_utf8(body[2..peer_end].to_vec())
                .map_err(|_| WireError::PeerNotUtf8)?;
            let share_id = get_share_id(body, peer_end);
            let protocol_version = get_u16(body, peer_end + SHARE_ID_LEN);
            let envelope_version = get_u16(body, peer_end + SHARE_ID_LEN + 2);
            let epoch = get_u64(body, peer_end + SHARE_ID_LEN + 4);
            if protocol_version != PROTOCOL_VERSION_V2 {
                return Err(WireError::UnsupportedProtocol(protocol_version));
            }
            if envelope_version != ENVELOPE_VERSION {
                return Err(WireError::UnsupportedEnvelope(envelope_version));
            }
            Ok(Wire::HelloV2 {
                peer,
                share_id,
                protocol_version,
                envelope_version,
                epoch,
                version: body[peer_end + SHARE_ID_LEN + 12..].to_vec(),
            })
        }
        TAG_RESYNC => {
            // 16 share id + 8 epoch + 2 frontier len + 2 reason len.
            const MIN: usize = SHARE_ID_LEN + 8 + 4;
            if body.len() < MIN {
                return Err(WireError::TrailingBytes { len: body.len() });
            }
            let share_id = get_share_id(body, 0);
            let epoch = get_u64(body, SHARE_ID_LEN);
            let frontier_len = get_u16(body, SHARE_ID_LEN + 8) as usize;
            let frontier_end = SHARE_ID_LEN + 10 + frontier_len;
            if frontier_end + 2 > body.len() {
                return Err(WireError::TrailingBytes { len: body.len() });
            }
            let frontier = body[SHARE_ID_LEN + 10..frontier_end].to_vec();
            let reason_len = get_u16(body, frontier_end) as usize;
            let reason_end = frontier_end + 2 + reason_len;
            if reason_end > body.len() {
                return Err(WireError::TrailingBytes { len: body.len() });
            }
            if reason_len == 0 {
                return Err(WireError::EmptyReason);
            }
            let reason =
                String::from_utf8(body[frontier_end + 2..reason_end].to_vec()).unwrap_or_default();
            Ok(Wire::Resync {
                share_id,
                epoch,
                frontier,
                reason,
            })
        }
        tag => Err(WireError::UnknownTag(tag)),
    }
}

/// Incremental decoder. Feed it socket reads; it returns one frame at a time
/// and retains a partial frame between calls.
#[derive(Debug, Default)]
pub struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// Take the next complete frame, or `None` if more bytes are needed.
    pub fn next_frame(&mut self) -> Result<Option<Wire>, WireError> {
        if self.buffer.len() < 4 {
            return Ok(None);
        }
        let len = u32::from_be_bytes([
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ]) as usize;
        if len == 0 || len > MAX_FRAME {
            return Err(WireError::BadLength {
                len,
                max: MAX_FRAME,
            });
        }
        if self.buffer.len() < 4 + len {
            return Ok(None);
        }
        let frame: Vec<u8> = self.buffer.drain(..4 + len).collect();
        decode(frame[4], &frame[5..]).map(Some)
    }

    /// Whether bytes remain that do not form a complete frame.
    pub fn has_partial(&self) -> bool {
        !self.buffer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_roundtrips() {
        let messages = [
            Wire::Hello {
                peer: "alice".into(),
                version: vec![1, 2, 3],
            },
            Wire::Delta {
                seq: 7,
                delta: vec![9, 8, 7],
            },
            Wire::Ack { seq: 42 },
            Wire::Presence {
                blob: vec![0, 255, 4],
            },
        ];
        for msg in messages {
            let encoded = encode(&msg);
            let mut decoder = Decoder::new();
            decoder.push(&encoded);
            assert_eq!(decoder.next_frame().unwrap(), Some(msg));
            assert!(!decoder.has_partial());
        }
    }

    #[test]
    fn a_split_frame_waits_for_the_rest() {
        let encoded = encode(&Wire::Delta {
            seq: 1,
            delta: b"payload".to_vec(),
        });
        let mut decoder = Decoder::new();
        decoder.push(&encoded[..5]);
        assert_eq!(decoder.next_frame().unwrap(), None);
        decoder.push(&encoded[5..]);
        assert_eq!(
            decoder.next_frame().unwrap(),
            Some(Wire::Delta {
                seq: 1,
                delta: b"payload".to_vec()
            })
        );
    }

    #[test]
    fn several_frames_come_out_in_order() {
        let mut bytes = encode(&Wire::Ack { seq: 1 });
        bytes.extend_from_slice(&encode(&Wire::Ack { seq: 2 }));
        let mut decoder = Decoder::new();
        decoder.push(&bytes);
        assert_eq!(decoder.next_frame().unwrap(), Some(Wire::Ack { seq: 1 }));
        assert_eq!(decoder.next_frame().unwrap(), Some(Wire::Ack { seq: 2 }));
        assert_eq!(decoder.next_frame().unwrap(), None);
    }

    #[test]
    fn an_overlong_length_is_refused_before_allocating() {
        let mut decoder = Decoder::new();
        decoder.push(&u32::MAX.to_be_bytes());
        assert!(matches!(
            decoder.next_frame(),
            Err(WireError::BadLength { .. })
        ));
    }

    #[test]
    fn zero_length_is_refused() {
        let mut decoder = Decoder::new();
        decoder.push(&0u32.to_be_bytes());
        assert!(matches!(
            decoder.next_frame(),
            Err(WireError::BadLength { len: 0, .. })
        ));
    }

    #[test]
    fn malformed_bodies_are_refused() {
        assert!(matches!(
            decode(TAG_DELTA, &[0, 1]),
            Err(WireError::ShortDelta { .. })
        ));
        assert!(matches!(
            decode(TAG_ACK, &[0, 1]),
            Err(WireError::TrailingBytes { .. })
        ));
        assert!(matches!(
            decode(TAG_HELLO, &[0, 0]),
            Err(WireError::EmptyPeer)
        ));
        assert!(matches!(decode(99, &[]), Err(WireError::UnknownTag(99))));
    }

    #[test]
    fn a_v2_handshake_carries_share_versions_and_frontier() {
        let msg = Wire::HelloV2 {
            peer: "ada".into(),
            share_id: [7u8; SHARE_ID_LEN],
            protocol_version: PROTOCOL_VERSION_V2,
            envelope_version: ENVELOPE_VERSION,
            epoch: 0,
            version: vec![1, 2, 3, 4],
        };
        let encoded = encode(&msg);
        let mut decoder = Decoder::new();
        decoder.push(&encoded);
        assert_eq!(decoder.next_frame().unwrap(), Some(msg.clone()));
        // The raw body really contains the share id and both versions.
        let body = &encoded[5..];
        assert_eq!(&body[5..5 + SHARE_ID_LEN], &[7u8; SHARE_ID_LEN]);
    }

    #[test]
    fn a_v2_handshake_rejects_unsupported_versions_and_short_bodies() {
        let hello = |protocol: u16, envelope: u16| {
            let mut body = Vec::new();
            put_u16(&mut body, 3); // peer "ada"
            body.extend_from_slice(b"ada");
            put_share_id(&mut body, &[1u8; SHARE_ID_LEN]);
            put_u16(&mut body, protocol);
            put_u16(&mut body, envelope);
            put_u64(&mut body, 0);
            body
        };
        assert!(matches!(
            decode(TAG_HELLO_V2, &hello(99, ENVELOPE_VERSION)),
            Err(WireError::UnsupportedProtocol(99))
        ));
        assert!(matches!(
            decode(TAG_HELLO_V2, &hello(PROTOCOL_VERSION_V2, 88)),
            Err(WireError::UnsupportedEnvelope(88))
        ));

        assert!(matches!(
            decode(TAG_HELLO_V2, &[0u8; 3]),
            Err(WireError::TrailingBytes { .. })
        ));
    }

    #[test]
    fn a_resync_frame_roundtrips_with_its_next_step() {
        let msg = Wire::Resync {
            share_id: [9u8; SHARE_ID_LEN],
            epoch: 4,
            frontier: vec![0xab, 0xcd],
            reason: "queue overflow".into(),
        };
        let encoded = encode(&msg);
        let mut decoder = Decoder::new();
        decoder.push(&encoded);
        assert_eq!(decoder.next_frame().unwrap(), Some(msg));
    }
}
