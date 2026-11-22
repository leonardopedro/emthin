//! A reliable, ordered command stream.
//!
//! Each endpoint keeps one [`Line`] per peer. Outbound commands get
//! monotonically increasing sequence numbers starting at 1 and stay in
//! `unacked` until the peer acknowledges them. Inbound commands are buffered
//! when they arrive ahead of a gap and delivered in sequence as soon as the
//! gap fills; a command at or below the delivery cursor is a duplicate and is
//! never delivered twice.
//!
//! The transport is responsible for framing and for resending
//! [`Line::resend_from`] after a reconnect. This type does not serialize
//! commands, choose a transport, or route by policy.

use std::collections::BTreeMap;
use std::fmt;

/// A command sequence number. Sequence 0 is reserved as "nothing yet".
pub type Seq = u64;

/// One retained command with the byte count the caller supplied for it.
type Entry<T> = (T, usize);

/// The result of delivering one inbound command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery<T> {
    /// One or more commands are now deliverable, in order. Usually this is
    /// just the received command; filling a gap can release a run of commands
    /// that arrived earlier.
    Commands(Vec<T>),
    /// The command was already delivered, or is already buffered.
    Duplicate,
    /// The command arrived ahead of a gap. It is buffered; `expected` is the
    /// sequence number that must arrive next.
    Gap { expected: Seq },
}

/// A malformed sequence number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineError {
    /// Sequence numbers start at 1; 0 means "nothing yet" and is never a
    /// valid command.
    ZeroSequence,
    /// A peer's retained or buffered bytes would exceed its bound. The caller
    /// must isolate the peer and restart from a frontier.
    QueueOverflow { bytes: usize, max: usize },
}

impl fmt::Display for LineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroSequence => f.write_str("sequence numbers start at 1"),
            Self::QueueOverflow { bytes, max } => {
                write!(f, "queue would hold {bytes} bytes, over the {max} cap")
            }
        }
    }
}

impl std::error::Error for LineError {}

/// One direction of a reliable command stream with a peer.
///
/// `Line` is intentionally generic over the command type: the caller decides
/// how commands are encoded for its Unix socket, WebSocket, or in-process
/// bridge.
#[derive(Debug, Clone)]
pub struct Line<T> {
    next_send: Seq,
    unacked: BTreeMap<Seq, Entry<T>>,
    acked_through: Seq,
    next_recv: Seq,
    buffered: BTreeMap<Seq, Entry<T>>,
    /// Bytes held for retransmit / gap buffering and the hard ceiling. The
    /// caller supplies the byte count because `T` is opaque here.
    retained_bytes: usize,
    buffered_bytes_count: usize,
    max_bytes: usize,
}

impl<T> Default for Line<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Line<T> {
    /// Create an empty line. The first command sent or expected is 1.
    pub fn new() -> Self {
        Self::with_byte_limit(usize::MAX)
    }

    /// A line with an explicit byte ceiling. Frame bytes are counted by the
    /// caller through [`Self::send_checked`] / [`Self::receive_checked`].
    pub fn with_byte_limit(max_bytes: usize) -> Self {
        Self {
            next_send: 1,
            unacked: BTreeMap::new(),
            acked_through: 0,
            next_recv: 1,
            buffered: BTreeMap::new(),
            retained_bytes: 0,
            buffered_bytes_count: 0,
            max_bytes,
        }
    }

    /// Assign the next sequence number and retain `command` for replay until
    /// the peer acknowledges it. Legacy path: no byte accounting.
    pub fn send(&mut self, command: T) -> Seq {
        self.send_checked(command, 0)
            .expect("an unbounded line cannot overflow")
    }

    /// Like [`Self::send`], but refuses to retain `bytes` more than the line's
    /// byte ceiling. A stalled peer's queue cannot grow without bound.
    pub fn send_checked(&mut self, command: T, bytes: usize) -> Result<Seq, LineError> {
        let next = self.retained_bytes.saturating_add(bytes);
        if next > self.max_bytes {
            return Err(LineError::QueueOverflow {
                bytes: next,
                max: self.max_bytes,
            });
        }
        let seq = self.next_send;
        self.next_send = self
            .next_send
            .checked_add(1)
            .expect("command sequence exhausted");
        self.retained_bytes = next;
        self.unacked.insert(seq, (command, bytes));
        Ok(seq)
    }

    /// Unacknowledged commands from `seq` onward, in sequence order.
    ///
    /// A value below the acknowledgement cursor starts at the first command
    /// that still needs replay, so a caller can pass a stale cursor after a
    /// reconnect without re-sending acknowledged work.
    pub fn resend_from(&self, seq: Seq) -> impl Iterator<Item = (Seq, &T)> {
        let start = seq.max(self.acked_through.saturating_add(1));
        self.unacked
            .range(start..)
            .map(|(&seq, (command, _))| (seq, command))
    }

    /// Acknowledge every command through `seq`.
    ///
    /// Acknowledgements are cumulative and monotonic: a stale or duplicate
    /// acknowledgement is ignored. A cursor beyond the highest sequence sent
    /// is clamped to that sequence rather than trusting a remote peer to
    /// prune commands that were never sent.
    pub fn ack(&mut self, seq: Seq) {
        if seq <= self.acked_through {
            return;
        }
        let highest_sent = self.next_send.saturating_sub(1);
        let seq = seq.min(highest_sent);
        if seq <= self.acked_through {
            return;
        }
        let all = std::mem::take(&mut self.unacked);
        let (acked, keep): (BTreeMap<Seq, Entry<T>>, BTreeMap<Seq, Entry<T>>) = all
            .into_iter()
            .partition(|(command_seq, _)| *command_seq <= seq);
        for (_, bytes) in acked.values() {
            self.retained_bytes = self.retained_bytes.saturating_sub(*bytes);
        }
        self.unacked = keep;
        self.acked_through = seq;
    }

    /// Deliver one inbound command, buffering it if an earlier command is
    /// still missing. Legacy path: no byte accounting.
    pub fn receive(&mut self, seq: Seq, command: T) -> Result<Delivery<T>, LineError> {
        self.receive_checked(seq, command, 0)
    }

    /// Like [`Self::receive`], but refuses to buffer `bytes` more than the
    /// line's byte ceiling.
    pub fn receive_checked(
        &mut self,
        seq: Seq,
        command: T,
        bytes: usize,
    ) -> Result<Delivery<T>, LineError> {
        if seq == 0 {
            return Err(LineError::ZeroSequence);
        }
        if seq < self.next_recv {
            return Ok(Delivery::Duplicate);
        }
        if seq > self.next_recv {
            if self.buffered.contains_key(&seq) {
                return Ok(Delivery::Duplicate);
            }
            let next = self.buffered_bytes_count.saturating_add(bytes);
            if next > self.max_bytes {
                return Err(LineError::QueueOverflow {
                    bytes: next,
                    max: self.max_bytes,
                });
            }
            self.buffered_bytes_count = next;
            self.buffered.insert(seq, (command, bytes));
            return Ok(Delivery::Gap {
                expected: self.next_recv,
            });
        }

        let mut commands = vec![command];
        self.next_recv += 1;
        while let Some((command, bytes)) = self.buffered.remove(&self.next_recv) {
            self.buffered_bytes_count = self.buffered_bytes_count.saturating_sub(bytes);
            commands.push(command);
            self.next_recv += 1;
        }
        Ok(Delivery::Commands(commands))
    }

    /// The highest contiguous sequence number received. This is the cursor to
    /// send to the peer as an acknowledgement; 0 means nothing yet.
    pub fn ack_cursor(&self) -> Seq {
        self.next_recv - 1
    }

    /// The next inbound sequence number that can be delivered without a gap.
    pub fn expected(&self) -> Seq {
        self.next_recv
    }

    /// The highest sequence number acknowledged by the peer. 0 means nothing
    /// has been acknowledged yet.
    pub fn acked_through(&self) -> Seq {
        self.acked_through
    }

    /// Number of outbound commands retained for replay.
    pub fn unacked_len(&self) -> usize {
        self.unacked.len()
    }

    /// Number of inbound commands waiting for a gap to fill.
    pub fn buffered_len(&self) -> usize {
        self.buffered.len()
    }

    /// Bytes retained for retransmission.
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    /// Bytes buffered waiting for a gap.
    pub fn buffered_bytes(&self) -> usize {
        self.buffered_bytes_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_byte_bounded_line_refuses_to_retain_past_its_ceiling() {
        let mut line = Line::with_byte_limit(10);
        assert_eq!(line.send_checked("abcd", 4), Ok(1));
        assert_eq!(line.send_checked("efgh", 4), Ok(2));
        assert_eq!(line.retained_bytes(), 8);
        assert_eq!(
            line.send_checked("ijkl", 4),
            Err(LineError::QueueOverflow { bytes: 12, max: 10 })
        );
        // Acknowledging releases the retained bytes.
        line.ack(1);
        assert_eq!(line.retained_bytes(), 4);
        assert_eq!(line.send_checked("ijkl", 4), Ok(3));
    }

    #[test]
    fn send_assigns_monotonic_sequences() {
        let mut line = Line::new();
        assert_eq!(line.send("one"), 1);
        assert_eq!(line.send("two"), 2);
        assert_eq!(line.send("three"), 3);
        assert_eq!(line.unacked_len(), 3);
    }

    #[test]
    fn in_order_receive_delivers_and_advances_ack_cursor() {
        let mut line = Line::new();
        assert_eq!(line.receive(1, "one"), Ok(Delivery::Commands(vec!["one"])));
        assert_eq!(line.ack_cursor(), 1);
        assert_eq!(line.receive(2, "two"), Ok(Delivery::Commands(vec!["two"])));
        assert_eq!(line.ack_cursor(), 2);
    }

    #[test]
    fn out_of_order_commands_wait_for_the_gap() {
        let mut line = Line::new();
        assert_eq!(line.receive(2, "two"), Ok(Delivery::Gap { expected: 1 }));
        assert_eq!(line.buffered_len(), 1);
        assert_eq!(line.ack_cursor(), 0);

        assert_eq!(
            line.receive(1, "one"),
            Ok(Delivery::Commands(vec!["one", "two"]))
        );
        assert_eq!(line.ack_cursor(), 2);
        assert_eq!(line.buffered_len(), 0);
    }

    #[test]
    fn filling_a_gap_releases_only_the_contiguous_run() {
        let mut line = Line::new();
        assert_eq!(line.receive(3, "three"), Ok(Delivery::Gap { expected: 1 }));
        assert_eq!(line.receive(4, "four"), Ok(Delivery::Gap { expected: 1 }));
        assert_eq!(line.receive(1, "one"), Ok(Delivery::Commands(vec!["one"])));
        assert_eq!(line.expected(), 2);
        assert_eq!(line.buffered_len(), 2);

        assert_eq!(
            line.receive(2, "two"),
            Ok(Delivery::Commands(vec!["two", "three", "four"]))
        );
        assert_eq!(line.ack_cursor(), 4);
    }

    #[test]
    fn duplicate_receive_is_idempotent() {
        let mut line = Line::new();
        assert_eq!(line.receive(1, "one"), Ok(Delivery::Commands(vec!["one"])));
        assert_eq!(line.receive(1, "one"), Ok(Delivery::Duplicate));
        assert_eq!(line.receive(1, "different"), Ok(Delivery::Duplicate));
        assert_eq!(line.ack_cursor(), 1);
    }

    #[test]
    fn duplicate_buffered_command_is_not_overwritten() {
        let mut line = Line::new();
        assert_eq!(
            line.receive(2, "first copy"),
            Ok(Delivery::Gap { expected: 1 })
        );
        assert_eq!(line.receive(2, "second copy"), Ok(Delivery::Duplicate));
        assert_eq!(
            line.receive(1, "one"),
            Ok(Delivery::Commands(vec!["one", "first copy"]))
        );
    }

    #[test]
    fn ack_is_cumulative_and_stale_acks_are_ignored() {
        let mut line = Line::new();
        line.send("one");
        line.send("two");
        line.send("three");

        line.ack(2);
        assert_eq!(line.acked_through(), 2);
        assert_eq!(line.unacked_len(), 1);
        assert_eq!(
            line.resend_from(1).map(|(seq, _)| seq).collect::<Vec<_>>(),
            vec![3]
        );

        line.ack(1);
        assert_eq!(line.acked_through(), 2);
        assert_eq!(line.unacked_len(), 1);
    }

    #[test]
    fn ack_beyond_sent_is_clamped() {
        let mut line = Line::new();
        line.send("one");
        line.ack(99);
        assert_eq!(line.acked_through(), 1);
        assert_eq!(line.unacked_len(), 0);
    }

    #[test]
    fn replay_starts_at_the_first_unacknowledged_command() {
        let mut line = Line::new();
        line.send("one");
        line.send("two");
        line.send("three");
        line.ack(1);

        let replay: Vec<_> = line.resend_from(0).map(|(seq, _)| seq).collect();
        assert_eq!(replay, vec![2, 3]);
    }

    #[test]
    fn zero_sequence_is_rejected() {
        let mut line = Line::new();
        assert_eq!(line.receive(0, "zero"), Err(LineError::ZeroSequence));
        assert_eq!(line.ack_cursor(), 0);
    }
}
