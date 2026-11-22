//! Local/share readiness, acknowledged generations and the seed gate (Kyneta K2).
//!
//! Readiness is a projection of facts owned elsewhere — the session store owns
//! the durable snapshot, `DocUi` owns the open document, and the collaboration
//! session owns the peer frontier. This module gives those facts one vocabulary
//! and, critically, one rule for late completions: an acknowledgement is only
//! meaningful for the generation it covered and never for a newer one.

use std::time::{Duration, Instant};

/// How long without a peer response before a share is reported offline.
pub const OFFLINE_TIMEOUT: Duration = Duration::from_secs(5);

/// Local document availability, independent of any share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalState {
    Loading,
    Available,
    Missing,
    Failed(String),
}

impl LocalState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Available => "available",
            Self::Missing => "missing",
            Self::Failed(_) => "failed",
        }
    }
}

/// Share readiness, reported separately from the local document. A peer ack
/// confirms applied operations only — it says nothing about that peer's disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareState {
    NotShared,
    CatchingUp { frontier: String },
    Current { frontier: String },
    Offline,
    Refused(String),
}

impl ShareState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotShared => "not_shared",
            Self::CatchingUp { .. } => "catching_up",
            Self::Current { .. } => "current",
            Self::Offline => "offline",
            Self::Refused(_) => "refused",
        }
    }
}

/// The generation a confirmed write or acknowledgement covered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Generation {
    /// The document instance. A replacement document starts a new instance, so
    /// an ack from an older instance can never confirm a newer document.
    pub document: u64,
    /// The content generation at the moment of confirmation.
    pub content_generation: u64,
    /// The committed Loro version vector (hex) — the frontier the ack covered.
    pub version: String,
}

impl Generation {
    pub fn new(document: u64, content_generation: u64, version: String) -> Self {
        Self {
            document,
            content_generation,
            version,
        }
    }

    /// Whether this generation is still the one described by the live
    /// document. A late completion for an older generation is stale and must
    /// not mark the current revision saved.
    pub fn covers(&self, live_document: u64, live_generation: u64) -> bool {
        self.document == live_document && self.content_generation >= live_generation
    }
}

/// Which kind of work an acknowledgement confirms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckKind {
    /// A confirmed recovery/session snapshot write.
    Recovery,
    /// A confirmed explicit document-file write.
    ExplicitFile,
    /// A peer's acknowledgement of applied operations.
    Peer,
}

impl AckKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recovery => "recovery",
            Self::ExplicitFile => "explicit_file",
            Self::Peer => "peer",
        }
    }
}

/// A confirmed write or acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acknowledgement {
    pub kind: AckKind,
    pub generation: Generation,
    /// Recovery and explicit-file acks are durable disk claims; a peer ack is
    /// not (it only confirms the peer applied the operations).
    pub durable_disk: bool,
}

impl Acknowledgement {
    pub fn disk(kind: AckKind, generation: Generation) -> Self {
        Self {
            kind,
            generation,
            durable_disk: true,
        }
    }

    pub fn peer(generation: Generation) -> Self {
        Self {
            kind: AckKind::Peer,
            generation,
            durable_disk: false,
        }
    }
}

/// Decides whether the open document may seed a share.
///
/// Seeding is deliberately narrow: only a genuine new local document with a
/// confirmed local creation, or a shared document whose owning host made an
/// explicit create decision. A recovered note or one that arrived from a peer
/// is never seeded over, and seeding happens at most once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeedGate {
    local_creation_confirmed: bool,
    explicit_create_decided: bool,
    recovered: bool,
    late_arrival: bool,
    seeded: bool,
}

impl SeedGate {
    /// Confirm this is a genuinely new local document, not a restored one.
    pub fn confirm_local_creation(&mut self) {
        self.local_creation_confirmed = true;
    }

    /// Record an explicit create decision at the owning host.
    pub fn decide_explicit_create(&mut self) {
        self.explicit_create_decided = true;
    }

    /// The document was loaded from recovery; it must not be seeded over.
    pub fn note_recovered(&mut self) {
        self.recovered = true;
    }

    /// Content arrived late from a peer; it must not be seeded over.
    pub fn note_late_arrival(&mut self) {
        self.late_arrival = true;
    }

    pub fn can_seed(&self, shared: bool) -> bool {
        if self.seeded || self.recovered || self.late_arrival {
            return false;
        }
        if shared {
            self.explicit_create_decided
        } else {
            self.local_creation_confirmed
        }
    }

    pub fn mark_seeded(&mut self) {
        self.seeded = true;
    }

    pub fn is_seeded(&self) -> bool {
        self.seeded
    }

    /// Reset the per-document transient flags when the document is replaced.
    pub fn reset_for_document(&mut self) {
        *self = Self::default();
    }
}

/// A snapshot of readiness for status and IPC projections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessSnapshot {
    pub local: &'static str,
    pub share: &'static str,
    pub local_detail: Option<String>,
    pub share_detail: Option<String>,
    pub seeded: bool,
    pub last_recovery_ack: Option<Generation>,
    pub last_explicit_ack: Option<Generation>,
    pub last_peer_ack: Option<Generation>,
}

/// The readiness state owned by `DocUi`.
#[derive(Debug, Clone)]
pub struct Readiness {
    pub local: LocalState,
    pub share: ShareState,
    acks: Vec<Acknowledgement>,
    pub seed: SeedGate,
    last_peer_response: Option<Instant>,
}

impl Default for Readiness {
    fn default() -> Self {
        Self {
            local: LocalState::Loading,
            share: ShareState::NotShared,
            acks: Vec::new(),
            seed: SeedGate::default(),
            last_peer_response: None,
        }
    }
}

impl Readiness {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn acknowledge(&mut self, ack: Acknowledgement) {
        // Keep only the newest acknowledgement of each kind. "Newest" is the
        // generation it covered, not the order it arrived in: a completion
        // that lands late for an older generation must not overwrite the
        // record of a later, newer one.
        match self
            .acks
            .iter_mut()
            .find(|existing| existing.kind == ack.kind)
        {
            Some(existing) => {
                let newer_document = ack.generation.document > existing.generation.document;
                let same_document_not_older = ack.generation.document
                    == existing.generation.document
                    && ack.generation.content_generation >= existing.generation.content_generation;
                if newer_document || same_document_not_older {
                    *existing = ack;
                }
            }
            None => self.acks.push(ack),
        }
    }

    pub fn last_ack(&self, kind: AckKind) -> Option<&Acknowledgement> {
        self.acks.iter().rev().find(|ack| ack.kind == kind)
    }

    /// Whether an acknowledgement of `kind` covers the live generation. A late
    /// completion for an older generation returns false, so callers cannot
    /// claim a newer revision is saved.
    pub fn ack_is_current(&self, kind: AckKind, live_document: u64, live_generation: u64) -> bool {
        self.last_ack(kind)
            .is_some_and(|ack| ack.generation.covers(live_document, live_generation))
    }

    pub fn note_peer_response(&mut self, now: Instant) {
        self.last_peer_response = Some(now);
    }

    /// A share that has gone quiet is offline, not merely "not current".
    pub fn refresh_share_timeout(&mut self, now: Instant) {
        if matches!(self.share, ShareState::NotShared | ShareState::Refused(_)) {
            return;
        }
        if let Some(last) = self.last_peer_response {
            if now.saturating_duration_since(last) >= OFFLINE_TIMEOUT {
                self.share = ShareState::Offline;
            }
        }
    }

    pub fn snapshot(&self) -> ReadinessSnapshot {
        ReadinessSnapshot {
            local: self.local.as_str(),
            share: self.share.as_str(),
            local_detail: match &self.local {
                LocalState::Failed(message) => Some(message.clone()),
                _ => None,
            },
            share_detail: match &self.share {
                ShareState::Refused(message) => Some(message.clone()),
                _ => None,
            },
            seeded: self.seed.is_seeded(),
            last_recovery_ack: self
                .last_ack(AckKind::Recovery)
                .map(|a| a.generation.clone()),
            last_explicit_ack: self
                .last_ack(AckKind::ExplicitFile)
                .map(|a| a.generation.clone()),
            last_peer_ack: self.last_ack(AckKind::Peer).map(|a| a.generation.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generation(instance: u64, gen: u64) -> Generation {
        Generation::new(instance, gen, format!("v{gen}"))
    }

    #[test]
    fn a_late_acknowledgement_never_marks_a_newer_revision_saved() {
        let mut readiness = Readiness::new();
        readiness.acknowledge(Acknowledgement::disk(AckKind::Recovery, generation(1, 3)));
        assert!(readiness.ack_is_current(AckKind::Recovery, 1, 3));
        // The document moved on; the old completion is now stale.
        assert!(!readiness.ack_is_current(AckKind::Recovery, 1, 4));
        assert!(!readiness.ack_is_current(AckKind::Recovery, 2, 3));
    }

    #[test]
    fn a_later_acknowledgement_replaces_the_older_one() {
        let mut readiness = Readiness::new();
        readiness.acknowledge(Acknowledgement::disk(
            AckKind::ExplicitFile,
            generation(1, 2),
        ));
        readiness.acknowledge(Acknowledgement::disk(
            AckKind::ExplicitFile,
            generation(1, 5),
        ));
        let snapshot = readiness.snapshot();
        assert_eq!(snapshot.last_explicit_ack, Some(generation(1, 5)));
        assert!(readiness.ack_is_current(AckKind::ExplicitFile, 1, 5));
    }

    #[test]
    fn a_late_acknowledgement_does_not_replace_a_newer_one() {
        let mut readiness = Readiness::new();
        readiness.acknowledge(Acknowledgement::disk(AckKind::Recovery, generation(1, 5)));
        // A completion for the older generation arrives afterwards.
        readiness.acknowledge(Acknowledgement::disk(AckKind::Recovery, generation(1, 3)));
        assert_eq!(
            readiness.snapshot().last_recovery_ack,
            Some(generation(1, 5))
        );
        assert!(readiness.ack_is_current(AckKind::Recovery, 1, 5));
        // A late completion from the previous document instance is stale too.
        readiness.acknowledge(Acknowledgement::disk(AckKind::Recovery, generation(0, 9)));
        assert_eq!(
            readiness.snapshot().last_recovery_ack,
            Some(generation(1, 5))
        );
    }

    #[test]
    fn a_peer_ack_confirms_operations_but_not_disk() {
        let mut readiness = Readiness::new();
        let ack = Acknowledgement::peer(generation(1, 4));
        assert!(!ack.durable_disk);
        readiness.acknowledge(ack);
        assert_eq!(
            readiness.last_ack(AckKind::Peer).unwrap().kind,
            AckKind::Peer
        );
        assert!(!readiness.last_ack(AckKind::Peer).unwrap().durable_disk);
    }

    #[test]
    fn a_missing_local_copy_does_not_seed_a_shared_document() {
        let mut gate = SeedGate::default();
        gate.confirm_local_creation();
        assert!(gate.can_seed(false));
        assert!(
            !gate.can_seed(true),
            "a share needs an explicit create decision"
        );
        gate.decide_explicit_create();
        assert!(gate.can_seed(true));
    }

    #[test]
    fn a_recovered_or_late_arriving_note_never_seeds() {
        let mut gate = SeedGate::default();
        gate.confirm_local_creation();
        gate.decide_explicit_create();
        gate.note_recovered();
        assert!(!gate.can_seed(false));
        assert!(!gate.can_seed(true));
        gate.reset_for_document();
        gate.confirm_local_creation();
        gate.note_late_arrival();
        assert!(!gate.can_seed(false));
    }

    #[test]
    fn seeding_happens_at_most_once() {
        let mut gate = SeedGate::default();
        gate.confirm_local_creation();
        assert!(gate.can_seed(false));
        gate.mark_seeded();
        assert!(!gate.can_seed(false));
        assert!(gate.is_seeded());
    }

    #[test]
    fn a_quiet_share_is_reported_offline() {
        let mut readiness = Readiness::new();
        readiness.share = ShareState::CatchingUp {
            frontier: "v1".into(),
        };
        let now = Instant::now();
        readiness.note_peer_response(now);
        readiness.refresh_share_timeout(now + OFFLINE_TIMEOUT / 2);
        assert!(matches!(readiness.share, ShareState::CatchingUp { .. }));
        readiness.refresh_share_timeout(now + OFFLINE_TIMEOUT + Duration::from_millis(1));
        assert_eq!(readiness.share, ShareState::Offline);
        assert_eq!(readiness.snapshot().share, "offline");
    }

    #[test]
    fn a_failed_local_state_keeps_its_detail() {
        let mut readiness = Readiness::new();
        readiness.local = LocalState::Failed("disk full".into());
        let snapshot = readiness.snapshot();
        assert_eq!(snapshot.local, "failed");
        assert_eq!(snapshot.local_detail.as_deref(), Some("disk full"));
    }
}
