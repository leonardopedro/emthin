//! Herdr Core framed transport.
//!
//! A frame is `[channel u32 LE][kind u8][len u32 LE][payload]`. Channel zero
//! carries JSON-RPC; attach streams use their own channels. This module owns
//! only the bytes and their limits, never a socket.

use std::fmt;

pub const HEADER: usize = 9;
pub const MAX_PAYLOAD: usize = 16 << 20;
pub const WINDOW: usize = 256 * 1024;

pub const DATA: u8 = 0;
pub const CREDIT: u8 = 1;
pub const CLOSE: u8 = 2;
pub const RESIZE: u8 = 3;
pub const FOCUS: u8 = 4;
pub const FACTS: u8 = 5;
pub const AUTOMATIC_INPUT: u8 = 6;

/// A frame is bounded before its payload is allocated. Unknown kinds are
/// rejected rather than silently discarded, so a protocol mismatch is visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub channel: u32,
    pub kind: u8,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn data(channel: u32, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            channel,
            kind: DATA,
            payload: payload.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    Oversized(usize),
    UnknownKind(u8),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Oversized(len) => write!(f, "frame payload of {len} bytes exceeds {MAX_PAYLOAD}"),
            Self::UnknownKind(kind) => write!(f, "unknown frame kind {kind}"),
        }
    }
}

impl std::error::Error for FrameError {}

pub fn encode(frame: Frame) -> Result<Vec<u8>, FrameError> {
    if frame.payload.len() > MAX_PAYLOAD {
        return Err(FrameError::Oversized(frame.payload.len()));
    }
    if frame.kind > AUTOMATIC_INPUT {
        return Err(FrameError::UnknownKind(frame.kind));
    }
    let mut out = Vec::with_capacity(HEADER + frame.payload.len());
    out.extend_from_slice(&frame.channel.to_le_bytes());
    out.push(frame.kind);
    out.extend_from_slice(&(frame.payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&frame.payload);
    Ok(out)
}

/// Incremental decoder: a header or payload may arrive in any number of reads,
/// and several frames may arrive in one read.
#[derive(Default)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
}

impl FrameDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Frame>, FrameError> {
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();
        loop {
            if self.buffer.len() < HEADER {
                break;
            }
            let channel = u32::from_le_bytes(self.buffer[0..4].try_into().expect("four bytes"));
            let kind = self.buffer[4];
            let len =
                u32::from_le_bytes(self.buffer[5..9].try_into().expect("four bytes")) as usize;
            if len > MAX_PAYLOAD {
                return Err(FrameError::Oversized(len));
            }
            if kind > AUTOMATIC_INPUT {
                return Err(FrameError::UnknownKind(kind));
            }
            if self.buffer.len() < HEADER + len {
                break;
            }
            let payload = self.buffer[HEADER..HEADER + len].to_vec();
            self.buffer.drain(..HEADER + len);
            frames.push(Frame {
                channel,
                kind,
                payload,
            });
        }
        Ok(frames)
    }

    /// Reject a truncated frame at end of stream instead of treating it as a
    /// clean close.
    pub fn finish(&mut self) -> Result<(), FrameError> {
        if self.buffer.is_empty() {
            Ok(())
        } else {
            Err(FrameError::Oversized(self.buffer.len()))
        }
    }
}

/// Unframed mode is one JSON value per line. This decoder keeps the same
/// 16 MiB bound and handles an event split across reads or several per read.
#[derive(Default)]
pub struct LineDecoder {
    buffer: Vec<u8>,
}

impl LineDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > MAX_PAYLOAD {
            return Err(FrameError::Oversized(self.buffer.len()));
        }
        let mut lines = Vec::new();
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let line = self.buffer.drain(..=end).collect::<Vec<_>>();
            let line = line[..line.len() - 1].to_vec();
            if !line.is_empty() {
                lines.push(line);
            }
        }
        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(channel: u32, kind: u8, payload: &[u8]) -> Vec<u8> {
        encode(Frame {
            channel,
            kind,
            payload: payload.to_vec(),
        })
        .expect("encode")
    }

    #[test]
    fn a_header_and_payload_survive_arbitrary_splits() {
        let wire = bytes(7, DATA, b"hello");
        let mut decoder = FrameDecoder::default();
        assert!(decoder.push(&wire[..3]).expect("partial header").is_empty());
        assert!(decoder.push(&wire[3..10]).expect("partial body").is_empty());
        let frames = decoder.push(&wire[10..]).expect("complete");
        assert_eq!(frames, vec![Frame::data(7, b"hello".to_vec())]);
        decoder.finish().expect("clean end");
    }

    #[test]
    fn coalesced_frames_are_returned_in_order() {
        let mut wire = bytes(1, DATA, b"one");
        wire.extend(bytes(2, RESIZE, &[80, 0, 24, 0]));
        let frames = FrameDecoder::default().push(&wire).expect("decode");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], Frame::data(1, b"one".to_vec()));
        assert_eq!(frames[1].kind, RESIZE);
    }

    #[test]
    fn the_payload_ceiling_is_checked_before_allocation() {
        let mut header = bytes(1, DATA, b"");
        header[5..9].copy_from_slice(&((MAX_PAYLOAD + 1) as u32).to_le_bytes());
        let error = FrameDecoder::default()
            .push(&header)
            .expect_err("oversized");
        assert_eq!(error, FrameError::Oversized(MAX_PAYLOAD + 1));
    }

    #[test]
    fn an_unknown_kind_is_not_silently_ignored() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&1u32.to_le_bytes());
        raw.push(9);
        raw.extend_from_slice(&1u32.to_le_bytes());
        raw.push(b'x');
        let error = FrameDecoder::default()
            .push(&raw)
            .expect_err("unknown kind");
        assert_eq!(error, FrameError::UnknownKind(9));
    }

    #[test]
    fn a_truncated_frame_is_not_a_clean_close() {
        let wire = bytes(1, DATA, b"hello");
        let mut decoder = FrameDecoder::default();
        decoder.push(&wire[..11]).expect("partial");
        assert!(decoder.finish().is_err());
    }

    #[test]
    fn json_lines_split_and_coalesce_like_frames() {
        let mut decoder = LineDecoder::default();
        assert!(decoder.push(b"{\"a\":1}\n{\"b\"").expect("lines").len() == 1);
        let lines = decoder.push(b":2}\n").expect("rest");
        assert_eq!(lines, vec![b"{\"b\":2}".to_vec()]);
    }
}
