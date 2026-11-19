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
}

impl fmt::Display for LineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroSequence => f.write_str("sequence numbers start at 1"),
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
    unacked: BTreeMap<Seq, T>,
    acked_through: Seq,
    next_recv: Seq,
    buffered: BTreeMap<Seq, T>,
}

impl<T> Default for Line<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Line<T> {
    /// Create an empty line. The first command sent or expected is 1.
    pub fn new() -> Self {
        Self {
            next_send: 1,
            unacked: BTreeMap::new(),
            acked_through: 0,
            next_recv: 1,
            buffered: BTreeMap::new(),
        }
    }

    /// Assign the next sequence number and retain `command` for replay until
    /// the peer acknowledges it.
    pub fn send(&mut self, command: T) -> Seq {
        let seq = self.next_send;
        self.next_send = self
            .next_send
            .checked_add(1)
            .expect("command sequence exhausted");
        self.unacked.insert(seq, command);
        seq
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
            .map(|(&seq, command)| (seq, command))
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
        self.unacked = self.unacked.split_off(&(seq + 1));
        self.acked_through = seq;
    }

    /// Deliver one inbound command, buffering it if an earlier command is
    /// still missing.
    pub fn receive(&mut self, seq: Seq, command: T) -> Result<Delivery<T>, LineError> {
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
            self.buffered.insert(seq, command);
            return Ok(Delivery::Gap {
                expected: self.next_recv,
            });
        }

        let mut commands = vec![command];
        self.next_recv += 1;
        while let Some(command) = self.buffered.remove(&self.next_recv) {
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
