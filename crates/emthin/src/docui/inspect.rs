//! Opt-in, bounded session diagnostics and replay (Kyneta K7).
//!
//! Events describe *metadata only* — run/share correlation, peer, frontier,
//! connection epoch, a closed kind and a reason. Raw note text, terminal
//! bytes, presence blobs and keys are never recorded. The same
//! [`reduce`] folds a live event stream and an NDJSON replay, so both
//! frontends settle on the same state. Observation is off by default: no log,
//! no file and no queue is created until it is explicitly enabled.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

/// Maximum retained events. Older entries fall off the front so observation
/// cannot grow without bound.
pub const EVENT_LIMIT: usize = 512;

/// What happened. The closed set keeps a replay deterministic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Load,
    Recovery,
    Handshake,
    CatchUp,
    AcceptedChanges,
    Ack,
    Retry,
    PeerJoined,
    PeerLeft,
    Refused,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Recovery => "recovery",
            Self::Handshake => "handshake",
            Self::CatchUp => "catch_up",
            Self::AcceptedChanges => "accepted_changes",
            Self::Ack => "ack",
            Self::Retry => "retry",
            Self::PeerJoined => "peer_joined",
            Self::PeerLeft => "peer_left",
            Self::Refused => "refused",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "load" => Self::Load,
            "recovery" => Self::Recovery,
            "handshake" => Self::Handshake,
            "catch_up" => Self::CatchUp,
            "accepted_changes" => Self::AcceptedChanges,
            "ack" => Self::Ack,
            "retry" => Self::Retry,
            "peer_joined" => Self::PeerJoined,
            "peer_left" => Self::PeerLeft,
            "refused" => Self::Refused,
            _ => return None,
        })
    }
}

/// One metadata-only diagnostic event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEvent {
    pub at_ms: u64,
    pub run: String,
    pub share: String,
    pub peer: String,
    pub frontier: String,
    pub epoch: u64,
    pub kind: EventKind,
    /// Counts or coarse codes only, never document content.
    pub count: u64,
    /// A reason for a refusal/incompatibility, or an empty string.
    pub reason: String,
}

impl SessionEvent {
    pub fn new(kind: EventKind) -> Self {
        Self {
            at_ms: now_ms(),
            run: String::new(),
            share: String::new(),
            peer: String::new(),
            frontier: String::new(),
            epoch: 0,
            kind,
            count: 0,
            reason: String::new(),
        }
    }

    pub fn with(mut self, run: &str, share: &str, peer: &str) -> Self {
        self.run = run.to_string();
        self.share = share.to_string();
        self.peer = peer.to_string();
        self
    }

    /// Serialize one NDJSON line. Only closed metadata fields are emitted.
    pub fn to_ndjson(&self) -> String {
        serde_json::json!({
            "at_ms": self.at_ms,
            "run": self.run,
            "share": self.share,
            "peer": self.peer,
            "frontier": self.frontier,
            "epoch": self.epoch,
            "kind": self.kind.as_str(),
            "count": self.count,
            "reason": self.reason,
        })
        .to_string()
    }

    /// Parse one NDJSON line. An unknown kind or missing field is rejected so
    /// a malformed replay cannot silently change the settled state.
    pub fn from_ndjson(line: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|error| format!("bad JSON: {error}"))?;
        let string = |key: &str| -> Result<String, String> {
            value[key]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("missing string field {key}"))
        };
        let kind_name = string("kind")?;
        let kind = EventKind::parse(&kind_name)
            .ok_or_else(|| format!("unknown event kind {kind_name}"))?;
        Ok(Self {
            at_ms: value["at_ms"].as_u64().ok_or("missing at_ms")?,
            run: string("run")?,
            share: string("share")?,
            peer: string("peer")?,
            frontier: string("frontier")?,
            epoch: value["epoch"].as_u64().ok_or("missing epoch")?,
            kind,
            count: value["count"].as_u64().ok_or("missing count")?,
            reason: string("reason")?,
        })
    }
}

/// The result of folding a trace. This is what both frontends display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledState {
    pub local: String,
    pub share: String,
    pub frontier: String,
    pub accepted: u64,
    pub peers: usize,
    pub reason: Option<String>,
}

impl Default for SettledState {
    fn default() -> Self {
        Self {
            local: "loading".to_string(),
            share: "not_shared".to_string(),
            frontier: String::new(),
            accepted: 0,
            peers: 0,
            reason: None,
        }
    }
}

/// Fold events into one settled state. Used by the live log's `state()` and by
/// `replay` for a fixture trace — one reducer, no frontend-specific branch.
pub fn reduce(events: impl IntoIterator<Item = SessionEvent>) -> SettledState {
    let mut state = SettledState::default();
    for event in events {
        match event.kind {
            EventKind::Load => state.local = "available".to_string(),
            EventKind::Recovery => {
                // `count` disambiguates a successful recovery (0) from a failed
                // one (1); `reason` carries the detail in both cases.
                state.local = if event.count == 0 {
                    "recovered".to_string()
                } else {
                    "failed".to_string()
                };
                if !event.reason.is_empty() {
                    state.reason = Some(event.reason);
                }
            }
            EventKind::Handshake | EventKind::CatchUp => {
                state.share = "catching_up".to_string();
                if !event.frontier.is_empty() {
                    state.frontier = event.frontier.clone();
                }
            }
            EventKind::AcceptedChanges => {
                state.share = "current".to_string();
                state.accepted += event.count;
                if !event.frontier.is_empty() {
                    state.frontier = event.frontier.clone();
                }
            }
            EventKind::Ack | EventKind::Retry => {}
            EventKind::PeerJoined => state.peers += 1,
            EventKind::PeerLeft => state.peers = state.peers.saturating_sub(1),
            EventKind::Refused => {
                state.share = "refused".to_string();
                state.reason = Some(if event.reason.is_empty() {
                    "refused without a reason".to_string()
                } else {
                    event.reason
                });
            }
        }
    }
    state
}

/// Replay NDJSON lines through the same reducer. Returns the settled state.
pub fn replay(lines: &str) -> Result<SettledState, String> {
    let events = lines
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(SessionEvent::from_ndjson)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(reduce(events))
}

/// A bounded, opt-in event log. `None` when observation is off — no buffer is
/// allocated and nothing is emitted.
#[derive(Debug, Clone)]
pub struct EventLog {
    events: Option<VecDeque<SessionEvent>>,
    limit: usize,
}

impl Default for EventLog {
    fn default() -> Self {
        Self::off()
    }
}

impl EventLog {
    pub fn off() -> Self {
        Self {
            events: None,
            limit: EVENT_LIMIT,
        }
    }

    pub fn on() -> Self {
        Self::with_limit(EVENT_LIMIT)
    }

    pub fn with_limit(limit: usize) -> Self {
        Self {
            events: Some(VecDeque::with_capacity(limit.max(1))),
            limit: limit.max(1),
        }
    }

    pub fn is_on(&self) -> bool {
        self.events.is_some()
    }

    pub fn enable(&mut self) {
        if self.events.is_none() {
            self.events = Some(VecDeque::with_capacity(self.limit));
        }
    }

    pub fn disable(&mut self) {
        self.events = None;
    }

    /// Record an event when observation is on. A no-op otherwise.
    pub fn push(&mut self, event: SessionEvent) {
        if let Some(events) = &mut self.events {
            if events.len() == self.limit {
                events.pop_front();
            }
            events.push_back(event);
        }
    }

    pub fn events(&self) -> impl Iterator<Item = &SessionEvent> {
        self.events.iter().flat_map(|events| events.iter())
    }

    pub fn ndjson(&self) -> String {
        let mut out = String::new();
        for event in self.events() {
            out.push_str(&event.to_ndjson());
            out.push('\n');
        }
        out
    }

    /// The same reducer, applied to the live stream.
    pub fn settled(&self) -> SettledState {
        reduce(self.events().cloned())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: EventKind, frontier: &str, reason: &str, count: u64) -> SessionEvent {
        let mut event = SessionEvent::new(kind).with("run", "share", "peer");
        event.frontier = frontier.to_string();
        event.reason = reason.to_string();
        event.count = count;
        event
    }

    #[test]
    fn a_delayed_recovery_and_a_reconnect_trace_settle_identically_live_and_replayed() {
        let trace = [
            event(EventKind::Load, "", "", 0),
            event(EventKind::Recovery, "", "loaded the backup pair", 0),
            event(EventKind::Handshake, "v1", "", 0),
            event(EventKind::CatchUp, "v2", "", 0),
            event(EventKind::AcceptedChanges, "v3", "", 4),
            event(EventKind::PeerJoined, "", "", 0),
        ];
        let mut log = EventLog::on();
        for event in trace.iter().cloned() {
            log.push(event);
        }
        let ndjson = log.ndjson();
        let replayed = replay(&ndjson).unwrap();
        assert_eq!(log.settled(), replayed);
        assert_eq!(replayed.local, "recovered");
        assert_eq!(replayed.share, "current");
        assert_eq!(replayed.accepted, 4);
        assert_eq!(replayed.peers, 1);
    }

    #[test]
    fn a_refusal_trace_settles_the_same_way_on_both_paths() {
        let trace = [
            event(EventKind::Load, "", "", 0),
            event(EventKind::Handshake, "v1", "", 0),
            event(EventKind::Refused, "", "read-only peer", 0),
            event(EventKind::Retry, "v1", "", 1),
        ];
        let state = reduce(trace.iter().cloned());
        let replayed = replay(
            &trace
                .iter()
                .map(SessionEvent::to_ndjson)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        assert_eq!(state, replayed);
        assert_eq!(state.share, "refused");
        assert_eq!(state.reason.as_deref(), Some("read-only peer"));
    }

    #[test]
    fn observation_off_creates_no_buffer_or_events() {
        let mut log = EventLog::off();
        assert!(!log.is_on());
        log.push(event(EventKind::AcceptedChanges, "v1", "", 9));
        assert_eq!(log.events().count(), 0);
        assert_eq!(log.ndjson(), "");
        assert_eq!(log.settled(), SettledState::default());
    }

    #[test]
    fn a_bounded_log_drops_oldest_events() {
        let mut log = EventLog::with_limit(2);
        log.push(event(EventKind::Load, "", "", 0));
        log.push(event(EventKind::PeerJoined, "", "", 0));
        log.push(event(EventKind::PeerLeft, "", "", 0));
        assert_eq!(log.events().count(), 2);
        let settled = log.settled();
        assert_eq!(settled.peers, 0, "the join fell off the front");
    }

    #[test]
    fn a_malformed_replay_line_is_rejected() {
        assert!(replay("{\"kind\":\"nonsense\"}").is_err());
        assert!(replay("not json").is_err());
        let mut event = event(EventKind::Load, "", "", 0);
        event.kind = EventKind::Load;
        let mut value: serde_json::Value = serde_json::from_str(&event.to_ndjson()).unwrap();
        value.as_object_mut().unwrap().remove("run");
        assert!(replay(&value.to_string()).is_err());
    }

    #[test]
    fn events_never_carry_raw_document_content() {
        let line = event(EventKind::AcceptedChanges, "v1", "", 3).to_ndjson();
        assert!(!line.contains("note text"));
        // The closed field set is exactly the metadata contract.
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        let expected = [
            "at_ms", "run", "share", "peer", "frontier", "epoch", "kind", "count", "reason",
        ];
        assert_eq!(
            value.as_object().unwrap().len(),
            expected.len(),
            "no content field was added"
        );
    }
}
