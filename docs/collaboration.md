# Collaboration transport

Status: the document and command primitives exist; a std-only, non-blocking
Unix-socket transport lives in `emthin-collab`, and emthin exposes the
document side plus ephemeral presence over its IPC socket.
`emthin/crates/emthin/tests/collab_ipc.rs` drives two IPC clients through join,
delta exchange, presence, expiry and reconnect. No remote peer is configured
by default, and the AT Protocol publication boundary is design-only (see
[`atproto-session.md`](atproto-session.md)).

## Layers

| Concern | Owner | State |
|---|---|---|
| Document convergence | `mathed_core::sync` (`MathDoc::version_vector`, `export_delta_since`, `import_delta`) | implemented |
| Live presence | `mathed_core::sync::PresenceStore` (Loro `EphemeralStore`) | implemented |
| Reliable command ordering | `emthin-collab` (`Line`, cumulative acks, replay) | implemented |
| Delta handshake | `emthin-collab::Exchange` | implemented |
| Multi-peer coordination | `emthin-collab::Session` (join, reconnect, edit fanout, relay, acks, presence) | implemented |
| Framing and transport | `emthin-collab::wire` + `emthin-collab::unix` | implemented (Unix sockets) |
| Control-plane bridge | emthin IPC (`collab_*`) | implemented; forwards local edits and presence each tick |

`emthin-collab` has no third-party networking dependency. `wire` is a
length-prefixed frame codec (16 MiB cap, incremental decoder); `unix::Host` and
`unix::UnixLink` are the socket implementation, including `Hello`, `Delta`,
`Ack`, and `Presence`. The host is non-blocking across peers: accepted
connections wait in a bounded handshake queue, `Host::poll` services every link
until it would block, and output is buffered per link so a slow reader does not
stall the others. A `Session` is the host side of a star: it applies a remote
delta and forwards the relevant operations to the other peers. A client that
only talks to the host needs `Exchange` and `Line` directly.

emthin itself exposes the document side over its existing control IPC:
`collab_version`, `collab_join`, `collab_delta`, `collab_ack`,
`collab_leave`, `collab_presence`, and `collab_name` in, `collab_delta` /
`collab_version` / `collab_presence` out. Binary fields are hex encoded. The
event loop compares the committed Loro version vector each tick, so text and
mark-only edits are forwarded once; a remote import updates the vector so it
is not echoed back.

Each session has a document-scope generation. Open and a successful ASCII
import clear peers, reliable lines and remote presence, advance the scope, and
publish an empty peer snapshot immediately. Scope-aware frames carry the
generation in an optional `scope` field; a frame from the previous document is
ignored even if the same peer id rejoins. Frames without `scope` keep the
legacy path: after a document replacement, a legacy relay must leave and join
again before it can send. Presence remains ephemeral and out of snapshots.

### Scoped exchange (K3)

`Session` also owns a stable random **share id** (16 bytes), independent of
the document path, display names and Loro peer ids. Changing the document —
Open, a successful ASCII import, or an explicit scope reset — replaces the
share id as well as the scope generation, so a delayed frame from the previous
share is rejected before any document state is touched.

A v2 handshake uses the `HelloV2` frame: peer id, share id, protocol version
(2), envelope version (1), an epoch field and the Loro frontier the sender has
already seen. A v1 `Hello` keeps the legacy unscoped path unchanged. The
acceptor validates share id and both versions before creating a peer, assigns a
**fresh epoch** to every accepted connection (`join_v2` / `reconnect_v2`), and
echoes the share id and epoch in its reply. `receive_v2`, `ack_v2` and
`apply_presence_v2` reject a stale epoch or scope before importing a delta,
advancing a reliable cursor or merging presence. A deliberate share switch
therefore discards old runtime state without touching the saved document.

Each reliable line has a byte ceiling (the 16 MiB wire cap by default).
`send_checked` / `receive_checked` account for retained and buffered bytes; on
overflow the peer is isolated rather than allowed to grow an unbounded queue,
so a stalled reader neither delays a healthy peer nor exhausts memory.
`Line` still deduplicates, reorders and replays contiguous commands. A gap
beyond retained history, a queue overflow, or an isolated peer is signalled by
a `Resync` frame carrying the share id, epoch, frontier and a reason — the next
step is explicit, and the frontier is the live document's version vector, so
uncommitted local edits are never discarded. Legacy frames keep `Wire::Delta`
/ `Ack` / `Presence` exactly as before.

Presence publishes the local caret on a one-second heartbeat and immediately
when the caret, document focus or display name changes. The same notification
carries the live peer snapshot (`peers`) and the optional local `blob` for the
transport to relay. Inbound blobs are merged through `Session::apply_presence`;
remote cursors are clamped to the local text length, drawn only on the page
where their byte already has a glyph, and disappear when the heartbeat times
out. Presence never enters Loro history, document revisions, undo or saves.
The local peer id is stable for the emthin instance, and reconnecting an
existing peer reuses its presence entry rather than duplicating it.

## AT Protocol in this workspace

There is no AT Protocol transport implementation to reuse:

- **`claurst`** does not contain atproto, XRPC, or firehose code. Its `/share`
  command uploads a session as a secret GitHub Gist, and its editor integration
  speaks the Agent Client Protocol.
- **`unfer/docs/LORO_APROTO_DIVISION.md`** is the normative split. It assigns
  real-time convergence and presence to Loro, and identity-bound publication
  to the AT Protocol side. It also says an atproto firehose, when built, is a
  mirror of committed log entries, never a second path into kernel state.
- **`unfer/docs/QUEPAXA_PLAN.md`** keeps the federation wire protocol as
  `unfer_protocol`, explicitly not atproto XRPC.
- **`dynamic-arctic`** implements the authority half: a `did:web` document and
  threshold delegation certificates with an `atproto-signing` capability. It
  is a signing authority, not a delta or presence transport.
- **`australVM/arctic_authority`** verifies those delegation certificates for
  authorization. It does not carry document traffic.

So “use atproto” would currently mean building a new client, choosing a PDS
and lexicon, and deciding an authentication model — not adapting an existing
integration.

## Decision: not the live document channel

The live channel needs low latency, privacy, and ephemeral messages. AT
Protocol records are durable, signed, public by default, and written through a
repository. Those are the right properties for publication and identity, and
the wrong ones for every keystroke:

1. **Durability is a cost, not a feature, for deltas.** Every edit would
   become a signed repository write. Loro already persists the document as a
   CRDT; mirroring every operation into a public log duplicates the byte
   stream that the normative division says must have one home.
2. **Presence must never be durable.** `PresenceStore` is explicitly gossip:
   no history, no identity, no authority. Publishing cursor heartbeats as
   records would persist exactly what the presence layer promises to discard.
3. **Write rate and fanout do not fit editing.** A relay firehose fans out all
   repositories to all subscribers. Editing needs a per-document channel, not
   a global public event stream.
4. **AT Protocol does not improve ordering.** Loro’s version vectors and
   `Line`’s sequence numbers already give causal delivery and replay. A
   repository commit order adds a second ordering mechanism without removing
   the first.

## Where it does make sense

AT Protocol is a good fit for the **session publication boundary**, not the
document stream:

- publish a small session descriptor — owner DID, document id, live endpoint,
  and the current version or checkpoint reference — under a custom lexicon;
- authenticate a joiner with the `did:web` delegation certificate that
  `dynamic-arctic` already issues, then hand them the live `Line`/`Exchange`
  channel;
- publish coarse checkpoints or “this document exists” records for discovery,
  with the bulk bytes fetched from the live peer or a content store.

That would be an optional adapter crate or feature. It must not move Loro
delta bytes or presence into records, and it must not become a second durable
home for the document. The live channel remains a framed byte stream whose
transport the host chooses. The proposed record shape, ownership boundary, and
join/revocation flow are specified in
[`atproto-session.md`](atproto-session.md).

## Consequences for the current code

- `emthin-collab` stays dependency-light; the Unix transport is std-only.
- `Session` supplies multi-client fanout and presence storage; the socket host
  owns the blocking reads and writes.
- emthin's IPC is the local bridge, not a second collaboration protocol: an
  external process can own the remote socket and relay frames through it.
- An AT Protocol adapter, if added, belongs beside the live channel as a
  discovery/identity surface, not underneath `Exchange`.
