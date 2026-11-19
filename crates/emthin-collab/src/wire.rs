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
}
