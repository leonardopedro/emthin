# AT Protocol session-descriptor boundary

Status: design only. No AT Protocol client, PDS, or lexicon ships in this
repository. The live document channel remains `emthin-collab`; this document
defines the narrow publication surface that may sit beside it.

Normative context: `unfer/docs/LORO_APROTO_DIVISION.md` assigns real-time
convergence, presence, and document bytes to Loro, and
identity-bound publication to the AT Protocol side. [`collaboration.md`](collaboration.md)
records why AT Protocol records must not carry deltas or presence.

## What is published

A **session descriptor** is one small, signed record that answers:

- which document is being shared;
- which identity owns the session;
- where the live `emthin-collab` endpoint is;
- which checkpoint the owner last published;
- what a joiner must present to be admitted.

It does **not** contain:

- Loro deltas, snapshots, or raw version tokens;
- document text or figure geometry;
- presence, carets, or display names;
- the long-term document history (that stays in the Loro/Zenodo chain);
- a second authorization decision that the live endpoint trusts blindly.

The record is discovery and attribution. The live handshake is still the only
way to obtain the current version and the bytes needed to converge.

## Record shape

Provisional NSID: `org.emthin.collab.session`. A deployment must publish the
lexicon under a domain it controls; the name is not part of the transport
contract.

```json
{
  "$type": "org.emthin.collab.session",
  "document": {
    "id": "emthin:sha256:6f1c...",
    "kind": "mathed",
    "title": "Shared notes"
  },
  "owner": "did:web:notes.example.org",
  "live": {
    "transport": "wss",
    "endpoint": "wss://notes.example.org/emthin/session/7Q2...",
    "protocol": "emthin-collab/1"
  },
  "checkpoint": {
    "revision": 412,
    "snapshot": "ipfs://bafy.../checkpoint.loro",
    "digest": "sha256:9a3e..."
  },
  "join": {
    "policy": "delegation",
    "capability": "emthin.collab.join"
  },
  "issuedAt": "2026-11-16T10:00:00Z",
  "expiresAt": "2026-11-23T10:00:00Z"
}
```

Field rules:

| Field | Rule |
|---|---|
| `document.id` | Stable across machines and renames. It is **not** `stable_document_key` (a canonical path) and not a figure key. Mint it from the document's genesis snapshot or store a random 128-bit id in document metadata. |
| `owner` | The `did:web` of the authority that signs the descriptor. The PDS account may be a hot key delegated with the `atproto-signing` capability. |
| `live.endpoint` | A joinable endpoint, not a local Unix path. A desktop host behind NAT publishes a relay/tunnel URL or omits the descriptor until one exists. |
| `live.protocol` | `emthin-collab/1`; it identifies the framed `Hello`/`Delta`/`Ack` protocol, not the Loro schema version. |
| `checkpoint.revision` | The document revision at publication time. It is a hint for staleness, not an ordering authority. |
| `checkpoint.snapshot` / `digest` | Optional content-addressed checkpoint. The snapshot bytes are not in the record. If absent, the joiner must obtain the document from the live peer. |
| `checkpoint.digest` | Digest of the snapshot or of the Loro version token, never the token itself. Publishing the digest lets a joiner detect a stale descriptor without exposing document history metadata. |
| `join.policy` | `delegation` (Arctic certificate), `invite` (out-of-band capability), or `open` (explicitly unauthenticated; not recommended). |
| `join.capability` | The capability string the live endpoint checks. `emthin.collab.join` is the default; a narrower capability may be defined later. |
| `expiresAt` | Bounds descriptor and admission lifetime. It does not replace certificate expiry or the live session lease. |

The record is small enough to update on explicit publication or coarse
checkpointing (a few minutes, a page break, or user action). It is never
rewritten per keystroke; that is the write-amplification failure the division
document rejects.

## Components and ownership

```text
 owner                                                        joiner
 ─────                                                        ──────
 Emthin DocUi ── current version/revision ──┐
                                            ▼
                                  SessionDescriptor builder
                                            │
                          AT Proto repo write│ (PDS; optional relay mirror)
                                            ▼
                                  PDS record / DID document
                                            │
                                            │ fetch descriptor + verify repo commit
                                            ▼
                                   JoinRequest(did, cert, capability)
                                            │
                         ┌──────────────────┴──────────────────┐
                         │  live endpoint (emthin-collab)      │
                         │  Session::join / Hello handshake    │
                         └──────────────────┬──────────────────┘
                                            │ framed Delta/Ack/Presence
                                            ▼
                                      Loro convergence
```

| Concern | Owner |
|---|---|
| Descriptor data model and serialization | An optional adapter crate (working name `emthin-atproto`), not `emthin-collab` |
| PDS/repo writes, DID resolution, record fetching | The adapter's AT Protocol client |
| Live channel framing and ordering | `emthin-collab::wire` / `Session` |
| Endpoint authentication and admission | The live host, via `australVM/arctic_authority` |
| `did:web` key document and threshold authority | `dynamic-arctic` |
| `did:unfer` registry | `unfer/unfer_identity` |
| Document bytes and convergence | `mathed_core` / `emthin-collab` |

`emthin-collab` stays dependency-light. It may accept an already-verified
`JoinIdentity { did, capabilities, expires_at }` and a `SessionDescriptor`
value, but it must not link an HTTP client or a PDS library. The adapter
performs all network and identity work and hands the live host verified facts.

## Publication flow

1. **Build.** Read the current document revision and, if checkpointing, export
   a Loro snapshot to a content store. Compute its digest. Do not export the
   raw version vector into the record.
2. **Sign and write.** The owner's AT Protocol repo writes the descriptor under
   a fresh TID key. The repo commit is signed by the owner's `did:web` key or by
   a hot key holding the `atproto-signing` delegation capability from
   `dynamic-arctic`.
3. **Mirror (optional).** A relay may mirror the record. It is a mirror of a
   committed record, never a second path into document state.
4. **Retract/refresh.** Replacing the descriptor uses a new record key or a
   compare-and-swap on the checkpoint revision. Deletion means "stop
   advertising"; it does not delete local document history.

Publication is explicit. The live session's local-edit path never calls the
adapter, and the adapter never calls `Session::local_edit`.

## Join flow

1. **Resolve.** The joiner resolves `owner` to a `did:web` document and verifies
   the service endpoint and signing key. A `did:unfer` participant presents an
   Arctic `DelegationCertificate` whose `issuer_did` is the owner and whose
   capabilities include `emthin.collab.join`.
2. **Fetch.** The joiner fetches the descriptor from the owner's repo (or a
   cached copy) and verifies the repo commit. The descriptor's `expiresAt` and
   `join.policy` are checked before dialing.
3. **Authorize.** The live endpoint verifies the certificate through
   `australVM/arctic_authority`. It checks the issuer, delegatee key, expiry,
   and the requested capability. The presence peer id is only a transport
   label and is never used for this decision.
4. **Handshake.** The joiner opens an `emthin-collab` `Hello` with its last
   known version token (empty for a new peer). The host computes the missing
   delta with `export_delta_since` and sends `Delta` frames over the reliable
   stream. If the descriptor carried a checkpoint, the joiner may start from
   that snapshot's version instead of an empty token.
5. **Converge.** `Delta` / `Ack` / `Presence` proceed exactly as in
   [`collaboration.md`](collaboration.md). No AT Protocol request is on the
   edit path.

## Authorization and revocation

- The descriptor is **not** the authorization token. A valid, signed record
  only proves who published it. Admission is decided by the live endpoint's
  certificate check.
- Certificate expiry and descriptor expiry are separate. The earlier deadline
  wins for new joins; an active session should be bounded by a host-side lease
  and re-checked at renewal.
- Revocation removes authority, not bytes. A revoked peer stops joining and its
  existing connection is closed at the next lease check. Loro operations it
  already contributed remain part of the CRDT; removing them would require an
  explicit document operation, not an identity revocation.
- A descriptor whose endpoint is unreachable is not replaced automatically by
  a new authority. The owner publishes a new record; joiners must not follow a
  redirect invented by the endpoint.

## Failure modes

| Symptom | Boundary response |
|---|---|
| PDS unavailable | Keep editing locally and over already-open live links. Publication is best-effort; it is not on the save path. |
| Descriptor stale | Join from the advertised checkpoint or empty version. The live `Hello` exchange, not the record, determines the real missing delta. |
| Version token rejected | Do not retry the same token. Request a fresh descriptor/checkpoint or join from empty. Map to `collab_failed` at the local IPC boundary. |
| Certificate expired or capability missing | Refuse the dial before `Hello`; do not create a `Session` peer. |
| Endpoint identity mismatch | Abort. Do not trust a descriptor that names an endpoint inconsistent with the owner's DID service entry. |
| Record replayed/older | AT Protocol repo commit order plus `issuedAt`/`revision` detects it; the live host still authorizes independently. |
| Relay duplicates a record | Idempotent: the record is data, not an event that mutates kernel state. |

## Explicit non-goals

- No document deltas, snapshots, or presence records.
- No AT Protocol-backed `DurableStore`.
- No global ordering of edits; QuePaxa consensus remains the global-order
  mechanism for committed effects, and Loro remains per-document.
- No second DID method. `did:web` stays in `dynamic-arctic`; `did:unfer` stays
  in `unfer_identity`.
- No trust in a PDS relay's ordering for live edits.
- No automatic publication of every local edit.

## Acceptance checks for a future implementation

- [ ] The adapter can publish and retract a descriptor without linking
      `emthin-collab` to an HTTP client.
- [ ] A join with a valid delegation certificate obtains the same delta as a
      direct `Session::join`; a join with a missing capability is refused
      before a peer is created.
- [ ] No document text, delta, version token, or presence blob appears in the
      serialized record.
- [ ] A stale descriptor still converges because the `Hello` version, not the
      record, drives `export_delta_since`.
- [ ] Editing continues with the PDS offline.
- [ ] `unfer/docs/LORO_APROTO_DIVISION.md`'s conformance checklist still holds.
