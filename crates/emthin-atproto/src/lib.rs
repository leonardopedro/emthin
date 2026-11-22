//! Optional AT Protocol publication adapter for emthin (parked item).
//!
//! This crate publishes **session descriptors only** — small, signed discovery
//! records that say which document is shared, who owns it, and where the live
//! `emthin-collab` endpoint is. It never carries Loro deltas, snapshots, raw
//! version tokens, document text, or presence. The live channel remains the
//! only path to current bytes; see `docs/atproto-session.md` in the workspace.
//!
//! The transport is a trait so the network lives here, not in `emthin-collab`.
//! A scripted in-memory PDS ships for tests and for local-only deployments; a
//! real PDS client can be added without changing the descriptor model or the
//! live protocol.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Provisional NSID. A deployment publishes its own lexicon under a domain it
/// controls; the name is not part of the transport contract.
pub const NSID: &str = "org.emthin.collab.session";
/// The only live protocol this descriptor may advertise.
pub const LIVE_PROTOCOL: &str = "emthin-collab/1";

/// Identity of the shared document. `id` is stable across machines and
/// renames; it is deliberately not a path or a figure key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentIdentity {
    pub id: String,
    pub kind: String,
    pub title: String,
}

/// A joinable live endpoint. A host behind NAT publishes a relay URL; a local
/// Unix path is never advertised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveEndpoint {
    pub transport: String,
    pub endpoint: String,
    pub protocol: String,
}

/// A coarse checkpoint hint. `digest` is a hash of the snapshot or of the
/// Loro version token — never the token itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

/// How a joiner is admitted. The descriptor is **not** the authorization
/// token; the live endpoint decides admission independently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinPolicy {
    /// `delegation`, `invite`, or `open`.
    pub policy: String,
    pub capability: String,
}

/// The signed discovery record. Field names match `docs/atproto-session.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDescriptor {
    #[serde(rename = "$type")]
    pub ty: String,
    pub document: DocumentIdentity,
    pub owner: String,
    pub live: LiveEndpoint,
    pub checkpoint: Checkpoint,
    pub join: JoinPolicy,
    pub issued_at: String,
    pub expires_at: String,
}

impl SessionDescriptor {
    pub fn builder() -> DescriptorBuilder {
        DescriptorBuilder::default()
    }

    /// Wire round-trip: the JSON shape is the publication contract.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a descriptor is serializable")
    }

    pub fn from_json(text: &str) -> Result<Self, Error> {
        serde_json::from_str(text).map_err(|e| Error::Invalid(e.to_string()))
    }

    /// Structural checks that keep forbidden data out of the record and
    /// catch a descriptor that cannot be used (expired, wrong protocol).
    pub fn validate(&self) -> Result<(), Error> {
        if self.ty != NSID {
            return Err(Error::Invalid(format!("unexpected $type: {}", self.ty)));
        }
        if self.live.protocol != LIVE_PROTOCOL {
            return Err(Error::Invalid(format!(
                "unexpected live protocol: {}",
                self.live.protocol
            )));
        }
        if !self.owner.starts_with("did:") {
            return Err(Error::Invalid("owner must be a DID".into()));
        }
        if self.live.endpoint.starts_with("/") || self.live.endpoint.contains("/run/") {
            return Err(Error::Invalid(
                "live endpoint must be joinable, not a local path".into(),
            ));
        }
        if !matches!(self.join.policy.as_str(), "delegation" | "invite" | "open") {
            return Err(Error::Invalid(format!(
                "unknown join policy: {}",
                self.join.policy
            )));
        }
        // A descriptor must not smuggle a raw version token into the digest.
        if self
            .checkpoint
            .digest
            .as_deref()
            .is_some_and(|d| d.starts_with("loro:") || d.starts_with("vv:"))
        {
            return Err(Error::Invalid(
                "digest must not carry a version token".into(),
            ));
        }
        Ok(())
    }

    /// Whether the descriptor has expired at `unix_secs`. Parsing is done by
    /// the caller so this crate needs no time/chrono dependency.
    pub fn is_expired_at(&self, unix_secs: i64) -> bool {
        parse_timestamp(&self.expires_at).is_some_and(|expires| expires <= unix_secs)
    }
}

/// Builder that defaults `$type` and the live protocol, so callers cannot
/// accidentally publish a record the joiner will reject.
#[derive(Debug, Clone)]
pub struct DescriptorBuilder {
    document: Option<DocumentIdentity>,
    owner: String,
    live: LiveEndpoint,
    checkpoint: Checkpoint,
    join: JoinPolicy,
    issued_at: String,
    expires_at: String,
}

impl Default for DescriptorBuilder {
    fn default() -> Self {
        Self {
            document: None,
            owner: String::new(),
            live: LiveEndpoint {
                transport: "wss".into(),
                endpoint: String::new(),
                protocol: LIVE_PROTOCOL.into(),
            },
            checkpoint: Checkpoint {
                revision: 0,
                snapshot: None,
                digest: None,
            },
            join: JoinPolicy {
                policy: "delegation".into(),
                capability: "emthin.collab.join".into(),
            },
            issued_at: String::new(),
            expires_at: String::new(),
        }
    }
}

impl DescriptorBuilder {
    pub fn document(mut self, document: DocumentIdentity) -> Self {
        self.document = Some(document);
        self
    }
    pub fn owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = owner.into();
        self
    }
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.live.endpoint = endpoint.into();
        self
    }
    pub fn transport(mut self, transport: impl Into<String>) -> Self {
        self.live.transport = transport.into();
        self
    }
    pub fn revision(mut self, revision: u64) -> Self {
        self.checkpoint.revision = revision;
        self
    }
    pub fn checkpoint(mut self, snapshot: Option<String>, digest: Option<String>) -> Self {
        self.checkpoint.snapshot = snapshot;
        self.checkpoint.digest = digest;
        self
    }
    pub fn join(mut self, policy: impl Into<String>, capability: impl Into<String>) -> Self {
        self.join = JoinPolicy {
            policy: policy.into(),
            capability: capability.into(),
        };
        self
    }
    pub fn validity(mut self, issued_at: impl Into<String>, expires_at: impl Into<String>) -> Self {
        self.issued_at = issued_at.into();
        self.expires_at = expires_at.into();
        self
    }
    pub fn build(self) -> Result<SessionDescriptor, Error> {
        let descriptor = SessionDescriptor {
            ty: NSID.into(),
            document: self
                .document
                .ok_or_else(|| Error::Invalid("no document identity".into()))?,
            owner: self.owner,
            live: self.live,
            checkpoint: self.checkpoint,
            join: self.join,
            issued_at: self.issued_at,
            expires_at: self.expires_at,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }
}

/// Minimal RFC 3339 timestamp check (`YYYY-MM-DDTHH:MM:SSZ`). Timezone math
/// is intentionally not attempted — expiry is a coarse bound, and a malformed
/// string is treated as "not expired" so a bad record cannot lock a joiner out
/// before the live handshake decides.
fn parse_timestamp(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let num = |a: usize, b: usize| -> Option<i64> {
        std::str::from_utf8(&bytes[a..b]).ok()?.parse().ok()
    };
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    // Days since epoch (Howard Hinnant's civil-from-days inverse), enough for
    // a coarse expiry comparison without a date crate.
    let y = year - if month <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + hour * 3600 + minute * 60 + second)
}

/// A committed record on the PDS. `cid` stands in for the content id; the
/// adapter never exposes document bytes through it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoCommit {
    pub rkey: String,
    pub cid: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The PDS is unreachable. Editing and saves continue locally.
    Offline,
    /// The caller's checkpoint revision is older than the published one.
    Stale {
        published: u64,
        attempted: u64,
    },
    /// A record with this key already exists with a newer commit.
    Replay {
        rkey: String,
    },
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Offline => write!(f, "PDS unavailable"),
            Self::Stale {
                published,
                attempted,
            } => {
                write!(
                    f,
                    "stale descriptor: published {published}, attempted {attempted}"
                )
            }
            Self::Replay { rkey } => write!(f, "replayed record {rkey}"),
            Self::Invalid(reason) => write!(f, "invalid descriptor: {reason}"),
        }
    }
}

impl std::error::Error for Error {}

/// The network boundary. Kept as a trait so `emthin-collab` never links an
/// HTTP client and a scripted PDS can prove the contract offline.
pub trait PdsTransport {
    fn put_record(&mut self, rkey: &str, record: &SessionDescriptor) -> Result<RepoCommit, Error>;
    fn get_record(&mut self, rkey: &str) -> Result<Option<SessionDescriptor>, Error>;
}

/// Publish a descriptor. Best-effort by design: an `Offline` error is for the
/// caller to log and ignore — it is never on the save or edit path.
pub fn publish(
    transport: &mut dyn PdsTransport,
    rkey: &str,
    descriptor: &SessionDescriptor,
) -> Result<RepoCommit, Error> {
    descriptor.validate()?;
    transport.put_record(rkey, descriptor)
}

/// Fetch and validate a descriptor. This is discovery only; the live
/// `Hello` handshake still decides the real missing delta.
pub fn fetch(
    transport: &mut dyn PdsTransport,
    rkey: &str,
) -> Result<Option<SessionDescriptor>, Error> {
    match transport.get_record(rkey)? {
        Some(descriptor) => {
            descriptor.validate()?;
            Ok(Some(descriptor))
        }
        None => Ok(None),
    }
}

/// An in-memory PDS for tests and local-only deployments. Fault injection
/// covers the three documented failure modes without network access.
#[derive(Debug, Default)]
pub struct MockPdsTransport {
    records: BTreeMap<String, (SessionDescriptor, RepoCommit)>,
    offline: bool,
}

impl MockPdsTransport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_offline(&mut self, offline: bool) {
        self.offline = offline;
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

impl PdsTransport for MockPdsTransport {
    fn put_record(&mut self, rkey: &str, record: &SessionDescriptor) -> Result<RepoCommit, Error> {
        if self.offline {
            return Err(Error::Offline);
        }
        if let Some((_, commit)) = self.records.get(rkey) {
            if commit.revision >= record.checkpoint.revision {
                return Err(if commit.revision == record.checkpoint.revision {
                    Error::Replay { rkey: rkey.into() }
                } else {
                    Error::Stale {
                        published: commit.revision,
                        attempted: record.checkpoint.revision,
                    }
                });
            }
        }
        let commit = RepoCommit {
            rkey: rkey.into(),
            cid: format!(
                "bafy{}",
                rkey.chars()
                    .map(|c| char::from(b'0' + (c as u8 % 10)))
                    .collect::<String>()
            ),
            revision: record.checkpoint.revision,
        };
        self.records
            .insert(rkey.into(), (record.clone(), commit.clone()));
        Ok(commit)
    }

    fn get_record(&mut self, rkey: &str) -> Result<Option<SessionDescriptor>, Error> {
        if self.offline {
            return Err(Error::Offline);
        }
        Ok(self.records.get(rkey).map(|(record, _)| record.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(revision: u64) -> SessionDescriptor {
        SessionDescriptor::builder()
            .document(DocumentIdentity {
                id: "emthin:sha256:6f1c".into(),
                kind: "mathed".into(),
                title: "Shared notes".into(),
            })
            .owner("did:web:notes.example.org")
            .endpoint("wss://notes.example.org/emthin/session/7Q2")
            .revision(revision)
            .checkpoint(None, Some("sha256:9a3e".into()))
            .validity("2026-11-21T10:00:00Z", "2026-11-28T10:00:00Z")
            .build()
            .expect("a valid descriptor")
    }

    #[test]
    fn a_descriptor_round_trips_through_the_published_json_shape() {
        let original = descriptor(412);
        let json = original.to_json();
        assert!(json.contains("\"$type\":\"org.emthin.collab.session\""));
        assert!(json.contains("\"protocol\":\"emthin-collab/1\""));
        assert_eq!(
            SessionDescriptor::from_json(&json).expect("parse"),
            original
        );
    }

    #[test]
    fn forbidden_fields_are_rejected_not_published() {
        let mut bad = descriptor(1);
        bad.checkpoint.digest = Some("loro:0102".into());
        assert!(matches!(bad.validate(), Err(Error::Invalid(_))));
        bad = descriptor(1);
        bad.live.endpoint = "/run/user/1000/emthin.sock".into();
        assert!(matches!(bad.validate(), Err(Error::Invalid(_))));
        bad = descriptor(1);
        bad.live.protocol = "something-else/2".into();
        assert!(matches!(bad.validate(), Err(Error::Invalid(_))));
    }

    #[test]
    fn publication_and_fetch_use_only_the_descriptor() {
        let mut pds = MockPdsTransport::new();
        let record = descriptor(412);
        let commit = publish(&mut pds, "3KQ", &record).expect("publish");
        assert_eq!(commit.revision, 412);
        assert_eq!(fetch(&mut pds, "3KQ").expect("fetch"), Some(record));
        assert_eq!(fetch(&mut pds, "missing").expect("fetch"), None);
    }

    #[test]
    fn a_pds_outage_does_not_change_local_state() {
        let mut pds = MockPdsTransport::new();
        publish(&mut pds, "3KQ", &descriptor(1)).expect("first publish");
        pds.set_offline(true);
        assert_eq!(
            publish(&mut pds, "3KQ", &descriptor(2)),
            Err(Error::Offline)
        );
        assert_eq!(fetch(&mut pds, "3KQ"), Err(Error::Offline));
        // The earlier record is untouched: a failed publication is best-effort.
        pds.set_offline(false);
        assert_eq!(
            fetch(&mut pds, "3KQ")
                .expect("fetch")
                .map(|d| d.checkpoint.revision),
            Some(1)
        );
    }

    #[test]
    fn stale_and_replayed_publications_are_rejected() {
        let mut pds = MockPdsTransport::new();
        publish(&mut pds, "3KQ", &descriptor(10)).expect("publish");
        assert!(matches!(
            publish(&mut pds, "3KQ", &descriptor(5)),
            Err(Error::Stale { .. })
        ));
        assert!(matches!(
            publish(&mut pds, "3KQ", &descriptor(10)),
            Err(Error::Replay { .. })
        ));
    }

    #[test]
    fn expiry_is_a_coarse_bound() {
        let record = descriptor(1);
        // 2026-11-21T10:00:00Z = 1763714400; expiry is one week later.
        let issued = parse_timestamp("2026-11-21T10:00:00Z").expect("parse");
        assert!(!record.is_expired_at(issued));
        assert!(record.is_expired_at(issued + 7 * 86400));
    }
}
