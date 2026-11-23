# Native Kyneta features for emthin

This plan adds Kyneta's local-first capabilities to the existing emthin
document, recovery, collaboration and navigation paths. It is a continuation
of the [desktop plan](../DEVELOPMENT_PLAN.md) and the
[Android plan](../emthin-android/DEVELOPMENT_PLAN.md). The Rust model remains
the source for both frontends. Build, offline use, syncing, tests and desktop
and Android UI all work without a JavaScript engine, Node, Bun, npm, React or
a browser view. The optional MM0 browser fixture is test infrastructure for a
separate workload. The native Android test companion also works independently.

## Feature scope and ownership

The source reference is `../kyneta` at `83d6e0d06f4b`. It separates typed schema
and observation, domain storage, exchange, peer policy, transport and derived
views. Adopt the useful behavior through existing emthin contracts. Kyneta's
JavaScript API, package ecosystem and wire interchange are independent
targets and are outside this integration.

| Kyneta feature | Evidence in Kyneta | Native emthin decision |
|---|---|---|
| Typed documents, validation and migrations | `packages/schema/README.md`, `docs/migrations-design.md`, `packages/schema/backends/loro/TECHNICAL.md` | Rust typed projections over the existing scanner, marker/figure/semantic model and Loro data. Validate incoming envelopes and new writes at the owning boundary. Keep half-written source editable and report diagnostics by source range. |
| Batched changes, origins and reactive views | `packages/schema/README.md`, `packages/changefeed/README.md`, `packages/machine/README.md` | Publish one committed change summary per user edit or accepted import, with origin and owner. Reuse Rust caches and frontend subscriptions for affected views. |
| Local-first data, readiness and recoverable storage | `packages/exchange/README.md` (Storage, Sync Status), `src/store/store-program.ts` within that package | Use the current snapshot/recovery lanes. Report local availability, durable acknowledgements, remote catch-up and refusal separately. |
| Durable multi-writer, authoritative and ephemeral data | `packages/exchange/README.md` (Three sync modes), Loro schema backend | Existing Loro content is multi-writer. The share owner controls enrollment and grants. Presence is ephemeral. Each has its existing owner and lifetime. |
| Ordered reconnect, relay and optional transport | `packages/exchange/README.md` (Peer Lifecycle, Line), `packages/transport/README.md`, `examples/unix-socket-sync` | Extend `emthin-collab`'s Rust exchange, line, wire and Unix host. Add a native remote transport only for a concrete phone/desktop or multi-host use. |
| Share, accept and write policy | `packages/exchange/README.md` (Governance Predicates) | Authenticated membership and separate read/write scopes over one explicitly shared document. Check outgoing and incoming paths and preserve refused offline edits locally. |
| Stable positions, selection, undo and history | `packages/schema/src/position.ts`, `docs/findings/undo-probes.md`, exchange README (Undo) | Reuse biased Loro anchors and undo. Test local undo after remote changes and expose bounded, read-only history inspection. |
| Reactive indexes and relation queries | `packages/index/README.md`, `packages/datalog/README.md`, `packages/zset/README.md` | Query the existing semantic facts and typed proof result references. Use an incremental index for a concrete problem if workload data justifies it. |
| Evidence and debugging | `packages/exchange/PRODUCT.md`, `packages/devtools/README.md` | Extend existing IPC status and Rust tracing with bounded session diagnostics and replayable fixture events. |
| Constraint-derived state | `packages/perspective/README.md` | Apply pure, checked rules to derived diagnostics, such as unresolved proof dependencies. Existing document source and Typst continue to resolve geometry. A generalized constraint store, pluggable CRDTs, generic Datalog and React bindings have no required emthin use in this plan. |

## Implementation order

| Checkpoint | Cards | Result |
|---|---|---|
| Local document | K1–K2 | One validated typed view, atomic change notifications and independently reported recovery/readiness. |
| Scoped Rust exchange | K3–K5 | Two clients edit the shared note, reconnect safely, show anchored presence and undo local edits without reverting a peer's work. |
| Native device and diagnostics | K6–K8 | Android can use the same native session model, diagnose a refused or stale session, and navigate proof dependencies. |
| AustralVM policy seam | K9 | A compiled pure Austral rule is evaluated against the same typed inputs and cases as the native implementation if it provides a concrete benefit. |

D2.1, D3.1, D4.1, D6.1 and D7.1 already cover parts of these contracts.
Implement each shared behavior once under the card that owns the affected
code. Use a cheaper model for individual cards, fixtures and UI wiring.
Review admission, migrations and the Austral ABI before other cards depend
on them.

K1 is the shared edit contract. K2 and K3 can branch from it. K4 and K5
use the scoped exchange from K3. K6 requires K2–K5 before using any
real shared data. K7 starts with K2's states and adds exchange events
as K3–K4 land. K8 builds on K1 and the MM0 test fixture. K9 uses K8's
read-only results and can stay outside the shipping app during evaluation.

### K1. Typed edits and change notifications in Rust

Reuse `mathed_core::{doc,sync,semantics}`, shared `DocModel` and the present
Android and desktop diagnostic code. Define typed changes at the existing
document edit/import boundary. A committed change names its document
instance, Loro version/frontier, origin (local, peer, undo, redo or restore),
and which content or presentation changed. Commit grouped edits before
publishing their summary. Preserve the scanner as the authority for
mathed source positions and Typst as the authority for placement.
Read-only status, presence and view changes use their own projection and
do not advance the durable document version.

Use existing type definitions for validated write boundaries and existing
Loro bytes for snapshots and deltas. Validate imported envelope type,
compatibility and bounds before applying data. Invalid or incomplete
expressions remain editable. Reject structurally invalid commands without
partially applying document edits. Give a new persisted format its own
version and a deterministic migration, preserve the original on failure,
and keep current valid files readable.

**Done.** One grouped edit produces one published notification and undo step.
Mark-only changes invalidate affected projections even with identical source
text. Failed validation changes no committed state. Irrelevant presence or
pan events cause zero document rebuilds. Desktop and Android agree on a
golden set of marker, citation and proof-note diagnostics.

**Result (2026-11-21).**

- `docui/change.rs` defines the contract: `DocChange` (sequence, document
  instance, content generation, committed Loro version vector, `ChangeOrigin`,
  `ChangeScope`, affected byte ranges, source/content digests), the
  `validate_import` envelope gate (bounds, non-emptiness, version token) and a
  structured `Diagnostic` golden set with `DiagnosticCode`/`Severity` and
  source ranges. It depends only on `mathed_core` and `std`, so the pad
  compiles it verbatim alongside `docui/model.rs`.
- `DocModel` records committed changes at the edit boundary. Every edit in a
  tick is coalesced into one change drained by `DocUi::take_changes`; the fast
  gate is the content generation, and the real test is the pair of digests so
  a marks-only edit (identical source digest, different content digest) still
  publishes while caret/pan/reading-size changes publish nothing. Undo, redo
  and peer deltas tag their change with the matching origin. Import bounds are
  validated before a snapshot decoder runs; a rejected import leaves no
  partial model.
- `OutgoingMessage::DocChanged` publishes each change over IPC with the owner,
  origin, scope, version hex and both digests; `docs/ipc.md` documents it and
  `scripts/check-ipc-docsync` is in sync (24 client→server, 20 server→client).
  No new persisted format was introduced, so no migration is owed.
- Tests: one grouped edit → one change + one undo step; a marks-only edit keeps
  the source digest; caret/selection/reading-size changes publish nothing;
  undo/redo/peer origins are attributed and one-shot; an invalid snapshot
  leaves no partial document; the proof-note golden set (marker pair, dangling
  figure, missing proof result, unresolved name) is stable and source-ranged.
  Desktop and Android run the same golden-set assertion. Fifteen Rust tests
  cover the new behavior.
- `cargo test --locked --workspace` exits 0: **618 passed, 0 failed, 2
  ignored** (up from 608). Per binary: 431 `emthin` lib (2 ignored live-logos
  tests), 2 `collab_ipc`, 1 `docui_async_restore`, 18 `xwayland_satellite`, 5
  `xwayland_satellite_watch`, 14 `emthin-clipboard`, 37 `emthin-collab`, 23
  `emthin-core-client`, 1 `scripted_peer`, 1 `unix_fixture`, 40 `emthin-dbus`,
  23 `emthin-vt`, 15 `ghostty-vt`, 5 `herdr-gateway-fixtures`; doc-tests 0.
  `cargo clippy --locked --workspace --all-targets -- -D warnings` exits 0.
  `bash scripts/verify-invariants` passes 7/7; `python3 scripts/check-ipc-docsync`
  and `python3 scripts/check-links` on the touched docs pass.
- Gaps (updated 2026-11-22): the live compositor/control-client gap is closed
  by `scripts/e2e/live-ipc.sh` (13/13 checks, exit 0) — a real headless sway +
  emthin instance answers `inspect_state`, `history_list` and the parked-item
  methods over the control socket, including a live rename reflected in the
  next `semantic_outline`. The sibling `mathed_core`/`mathed_mini` lane also
  runs here via `cd emthin && nix develop -c bash -c 'cd ../velysterm && cargo
  test -p mathed_core -p mathed_mini'` (the sibling's own flake has no aarch64
  devshell); it passes. An earlier full-workspace run hit a flaky
  `xwayland_satellite` script-launch assertion under parallel load; it passes
  in isolation and the final full run was green.

### K2. Readiness, snapshots and acknowledgements in Rust

Extend D2.1 and the Android A2–A4 recovery paths. Represent local states as
loading, available, missing or failed. Represent the share separately as
not shared, catching up, current to a specific peer frontier, offline or
refused. A confirmed recovery write, a confirmed explicit file write
and a peer's acknowledgement each identify the generation they covered.
A peer acknowledgement confirms applied operations and carries no claim
about its disk unless a separate durable acknowledgement is implemented.

A missing local recovery copy alone does not prove a shared document is empty.
A new local document can seed once on confirmed local creation. A shared
document seeds only on an explicit create decision at its owning host.
An offline client continues editing a previously loaded note and preserves
those operations for reconnect or export. Failed writes retain the last
valid recovery pair and the latest owed checkpoint.

**Done.** Exercise delayed load, late peer response, offline editing, a genuine
new document, write failure and relaunch. An offline timeout is shown as
offline. The proof note never seeds over a recovered or late-arriving note,
and a late completion never marks a newer revision saved.

**Result (2026-11-21).**

- `docui/readiness.rs` defines the vocabulary: `LocalState`
  (`loading`/`available`/`missing`/`failed`), `ShareState` (`not_shared`/
  `catching_up`/`current`/`offline`/`refused`), `Generation` (document instance,
  content generation, Loro version hex), `Acknowledgement` (recovery, explicit
  file or peer — only the first two claim disk), a `SeedGate` and `Readiness`
  with a snapshot projection. An acknowledgement covers only its own
  generation, the newest per kind wins, a quiet share goes offline after the
  timeout, and seeding is refused for a recovered or late-arriving note and
  allowed at most once.
- `DocUi` owns `Readiness` and records each transition: loading on load,
  available/missing/failed from the session result, and an acknowledged
  generation on each successful recovery or explicit write. A peer ack from
  `ipc_collab_ack` is non-durable and advances share readiness only for a
  joined share; stale/duplicate/gapped deltas mark late arrival. The tick
  refreshes the offline timeout. A failed recovery write records `failed`
  without replacing the last valid pair, so the newer content stays owed.
- `list_state` now reports `local_state`, `share_state`, their detail text,
  `seed_granted` and the three generation-carrying `last_*_ack` objects.
  `docs/ipc.md` documents them and `scripts/check-ipc-docsync` is in sync.
- Tests: a late ack never marks a newer revision saved; a later ack replaces
  the older one; a peer ack claims no disk; a missing local copy does not seed
  a shared document; a recovered/late-arriving note never seeds; seeding is
  once-only; a quiet share goes offline; a failed local state keeps its detail;
  and at the `DocUi` level a genuine new document seeds once, a recovery reload
  never seeds, a late peer ack does not claim newer text saved, an offline
  client keeps editing without a durable claim, a failed write keeps the last
  valid pair, and a quiet share goes offline.
- `cargo test --locked --workspace` exits 0: **633 passed, 0 failed, 2 ignored**
  (up from 618). Per binary: 446 `emthin` lib (2 ignored live-logos tests), 2
  `collab_ipc`, 1 `docui_async_restore`, 18 `xwayland_satellite`, 5
  `xwayland_satellite_watch`, 14 `emthin-clipboard`, 37 `emthin-collab`, 23
  `emthin-core-client`, 1 `scripted_peer`, 1 `unix_fixture`, 40 `emthin-dbus`,
  23 `emthin-vt`, 15 `ghostty-vt`, 5 `herdr-gateway-fixtures`; doc-tests 0.
  `cargo clippy --locked --workspace --all-targets -- -D warnings` exits 0.
- Gaps (updated 2026-11-22): the live two-instance relay with a real timeout
  now runs — `collab_ipc::a_two_instance_relay_detects_a_real_offline_timeout`
  handshakes two real IPC peers, exchanges deltas, arms the peer clock with a
  cumulative ack, drops one peer and pumps the other past the full 5-second
  `OFFLINE_TIMEOUT` (no backdated clock) until `share_state` reports `offline`
  (passes in ~5s). Writing it exposed a production bug: `ipc_collab_join`
  never called `DocUi::note_share_joined`, so the share stayed `not_shared`,
  `note_peer_ack` returned early and the offline timeout in `tick.rs` could
  never fire; the join path now arms it. The pad already keeps
  `savedFingerprint` separate from the durable fingerprint, so no Android
  source changed in this card; its host lane remains green.

### K3. Reliable scoped exchange in Rust

Build on D6.1, `emthin-collab::Session`, `Exchange`, `Line` and the existing
length-prefixed codec. Sharing assigns a persistent random share ID to
one document and maps it locally to a document instance. Paths, display
names and Loro peer IDs alone do not identify the share. A v2 handshake
carries share ID, protocol capability/version, envelope kind/version and
Loro frontier. Each accepted connection has a fresh epoch. Unscoped
local IPC compatibility remains as documented. A mismatched or replaced
scope cannot apply deltas, acks or presence.

Merge committed Loro operations on the active session and relay only to
authorized recipients. Use contiguous applied sequence for cumulative
acknowledgement. Bound frames, queue bytes and retained retransmits using
the current wire cap as a hard ceiling. A gap beyond retained history or
an overflow starts a fresh handshake from a frontier. It must not discard
uncommitted local edits. Coalesce presence, preserve durable edits in Loro,
and isolate a stalled peer. Drop/fail signaling exposes the expected next
step to the frontend.

**Done.** Two writers edit text and marks offline, reconnect through a
local relay and converge. Duplicates, reorder, partial frames, malformed
envelopes, stale epochs and queue overflow have deterministic tests. A
stalled peer neither delays the healthy peer nor creates an unbounded queue.
A deliberate share switch discards old runtime state while preserving
the previously saved document.

**Result (2026-11-21).**

- `emthin-collab::wire` gained a v2 handshake (`HelloV2`) carrying peer id,
  a stable share id, protocol version 2, envelope version 1, an epoch field and
  the Loro frontier; and a `Resync` frame carrying share id, epoch, frontier
  and a reason. Both validate bounds and versions before a frame is accepted;
  v1 `Hello`/`Delta`/`Ack`/`Presence` are unchanged, so the local IPC path keeps
  its legacy shape.
- `Session` owns a random 16-byte share id and a connection epoch counter.
  `join_v2`/`reconnect_v2` validate share id and versions before creating a
  peer, then assign a fresh epoch; `receive_v2`, `ack_v2` and
  `apply_presence_v2` reject a stale epoch or scope before touching the
  document or a reliable cursor. `reset_document_scope` replaces the share id
  and drops old peers. `unix::Host` accepts both handshake shapes and echoes
  the assigned epoch.
- `Line` gained byte-checked send/receive with a hard ceiling (the 16 MiB wire
  cap by default) and byte accounting on acknowledgement. Overflow isolates
  only the offending peer; healthy peers keep their outbounds. `Resync`
  exposes the next step from the live version vector, so a gap beyond retained
  history never discards uncommitted local edits.
- Tests: v2 handshake round-trip and version/share rejection; share switch;
  stale epoch rejection for delta, ack and presence; bounded retention and
  stalled-peer isolation; offline reconnect convergence from a frontier.
  Existing duplicate/reorder/partial-frame/malformed-frame tests stay green.
- `cargo test --locked --workspace` exits 0: **643 passed, 0 failed, 2
  ignored** (up from 633), with 47 `emthin-collab` tests (up from 37).
  `cargo clippy --locked --workspace --all-targets -- -D warnings` exits 0;
  `bash scripts/verify-invariants` passes 7/7; `python3 scripts/check-ipc-docsync`
  and `python3 scripts/check-links` pass.
- Gaps (updated 2026-11-22): the wire-level gap is closed. `wire.rs` now has
  `DeltaV2`/`AckV2`/`PresenceV2` frames carrying epoch and optional scope, with
  round-trip tests and a check that the legacy tags still decode to legacy
  shapes. `UnixLink` upgrades established frames on a v2 handshake and the host
  rejects a stale epoch before touching document state (unit test on a
  socketpair). Convergence and fencing are covered by in-process `Session`
  tests and the two-IPC-client harness. A native network transport was not run;
  that is K6's real-phone lane (excluded from this pass).

### K4. Document grants and anchored presence in Rust

Model grants on the share owner as read, edit and invite/revoke
permissions, with verified peer membership from the transport. Local
Unix permission/peer credentials can establish the local principal. Any
remote access uses an independently authenticated principal. A display
name supplied in presence provides no permission. Enforce the owner
grant for discovery, snapshots, deltas, writes, relays and presence.
Denied and missing share lookup have the same outward result. Revoke
stops further exchange and invalidates pending work. It cannot recover
copies already received by someone else.

Keep presence within the share and connection epoch. Publish an
encoded biased Loro anchor scoped to the accepted frontier where
possible. Resolve only in a current compatible document view; hide a
caret while its source is missing. Coalesce heartbeat, expire on leave
or timeout, and clear on document replacement. For a denied writer, retain
local edits as a recoverable local copy with an explicit retry/export
choice. A read-only client can still copy authorized content and work on
a deliberate local fork.

**Done.** Each grant is checked on both stored load and relay. Read-only
and revoked peers cannot commit, discover other shares or acquire a terminal
permission. A collaborative insertion above a caret moves it with its
anchor. Disconnect/rejoin and a stale presence update cannot change the
document, undo history or saved file.

**Result (2026-11-21).**

- `emthin-collab::grants` defines `Grant::{Read, Edit, Invite}`, a
  transport-verified `Principal`, `ShareLookup::{Access, Unavailable}` (denied
  and missing lookups are identical) and an owner-held `GrantTable`. Revoke
  removes every grant and records the connection epoch as invalidated, so
  pending frames from that epoch are dropped; it cannot recall copies already
  received. A presence display name is never a principal.
- `emthin-collab::anchor` defines `CaretAnchor` — an encoded biased Loro cursor
  plus share id, epoch, captured frontier and fallback — with a bounded codec
  and an `AnchorStore`. It resolves only when the live document still contains
  the captured frontier; a missing or foreign source hides the caret instead
  of guessing an offset.
- `Session` owns both tables: `receive_granted` enforces edit permission and
  epoch revocation before a delta is applied, `relay_granted` filters
  outbounds, and `apply_anchor_blob` / `resolved_caret` enforce read permission
  and share/epoch scope. `collab_peers` resolves remote carets from anchors
  (raw byte cursor is the legacy fallback), and the compositor publishes a
  biased anchor on each presence heartbeat. An unconfigured share keeps the
  documented unscoped compatibility path; once any grant is made, joins,
  deltas and presence are checked.
- IPC gained `collab_shares`, `collab_grant`, `collab_revoke` and
  `collab_anchor` plus the `collab_share_access` notification;
  `docs/ipc.md` and `docs/collaboration.md` document them and
  `scripts/check-ipc-docsync` is in sync (28 client→server, 21 server→client).
- Tests: only the verified owner can grant/revoke; a read-only peer cannot
  commit but can read; revoke stops exchange and invalidates pending work;
  denied/missing lookups match; a forged display name has no authority; an
  anchor follows an insertion above it, is hidden for an incompatible view,
  and a stale epoch cannot publish one. The two-instance IPC harness grants,
  gates, replays and revokes a real delta and asserts text, revision and undo
  are unchanged while denied.
- `cargo test --locked --workspace` exits 0: **659 passed, 0 failed, 2
  ignored** (up from 643), with 62 `emthin-collab` tests and 4 `collab_ipc`
  tests. `cargo clippy --locked --workspace --all-targets -- -D warnings`
  exits 0; `bash scripts/verify-invariants`, `check-ipc-docsync` and
  `check-links` pass.
- Gaps (updated 2026-11-22): local `SO_PEERCRED` extraction is now wired —
  `IpcConn::peer_credentials` reads pid/uid/gid from the kernel at accept time,
  and the `client_identity` IPC method exposes them (unit test on a socketpair
  plus a live assertion in `scripts/e2e/live-ipc.sh`). This is a verified local
  principal fact, not an authorization decision; admission still belongs to the
  live endpoint. A real authenticated remote transport and an explicit
  local-fork/export UI for a denied writer were not performed — the former is
  K6's real-phone lane (excluded from this pass), the latter has no workflow
  that needs it. The pad was unchanged.

### K5. Local undo and read-only history in Rust

Use the current Loro `UndoManager` and grouped edit APIs. The normal
Undo/Redo commands affect this editor's operations. Remote edits, hydration,
recovery and view state never enter the local stack. Ensure undo of a
mark-only batch emits a committed change. Use a read-only snapshot/clone
with the requested retained frontier for history comparison. Share ID,
frontier and ownership stay explicit. A history preview stays separate
from the live writable model; only an explicit copy export creates a
new local document. History unavailable after a compacted frontier has
an explicit status.

**Done.** Concurrent text and mark edits from two peers survive either
peer's Undo/Redo for its own group. Reconnect and recovery preserve
the documented Loro behavior. Previewing an older retained frontier
changes neither the live version nor its save/undo status.

**Result (2026-11-21).**

- `docui/history.rs` stores a bounded ring (30) of retained snapshots, each
  stamped with share id, frontier, owner, sequence and a text+marks content
  digest. `HistoryPreview` is read-only text/marks/digest data; only
  `into_local_document` materializes a writable document. `HistoryStatus::
  Unavailable { reason }` is explicit for a compacted frontier or another
  share.
- `DocUi` retains a snapshot for each drained committed change
  (`take_changes` in the tick), exposes `history_frontiers`, `history_preview`
  and `promote_history_copy` (the explicit copy export that replaces the live
  document and resets collaboration scope), and clears history on New/Open/
  import/export.
- Undo isolation is verified at the document boundary: `MathDoc::import_delta`
  never touches the `UndoManager`, and `from_snapshot` constructs a fresh
  manager, so remote edits, recovery and hydration never enter the local undo
  stack. A mark-only undo still emits one `DocChange` with `origin: undo` and
  an identical source digest but a changed content digest.
- IPC gained `history_list`, `history_preview`, `history_export` and the
  `history` / `history_preview` notifications; `docs/ipc.md` documents them
  and `scripts/check-ipc-docsync` is in sync (31 client→server, 23
  server→client).
- Tests: a preview leaves live text, revision, undo and digest unchanged;
  an explicit export creates a new local document; a compacted frontier and a
  foreign share report an explicit unavailable status; a remote import does
  not enter the undo stack; a mark-only undo emits a committed change;
  consecutive identical frontiers coalesce and marks are visible in the
  preview.
- `cargo test --locked --workspace` exits 0: **670 passed, 0 failed, 2
  ignored** (up from 659). `cargo clippy --locked --workspace --all-targets
  -- -D warnings` exits 0; `bash scripts/verify-invariants`,
  `check-ipc-docsync` and `check-links` pass.
- Gaps (updated 2026-11-22): the retention-across-relaunch gap is closed.
  `HistoryStore::{to_json,from_json}` persists the bounded ring to a plain-JSON
  sidecar next to the session file; a malformed row is skipped rather than
  erasing the ring. `DocUi::load/save` wire it in, with a DocUi-level test that
  retains a snapshot, saves, relaunches a fresh instance and reads the same
  frontiers back. The live control-client gap is also closed:
  `scripts/e2e/live-ipc.sh` asserts a live `history_list` reply with a
  `frontiers` field. The Android pad now has a K5 preview UI: it path-includes
  the same `docui::history` ring (no second model), exposes `historyRemember`,
  `historyFrontiers` and `historyPreview` over JNI, and shows a read-only
  snapshot list/dialog from the History toolbar action. `device-lane.sh
  history-preview` passes on arm64 Waydroid. A read-only boundary still exists
  at the IPC layer without an end-to-end UX beyond the documented grant checks.

### K6. Native sharing on desktop and Android

First connect two disposable desktop instances through the existing
Rust Unix host. Android reuses the shared Rust exchange through JNI and
a worker that owns transport, without handing its live `DocModel` across
threads. Apply accepted deltas as a grouped action on the Android
model owner. Reuse current recovery, page, caret and action/status code.
Expose Share/Join/Leave and watch/edit state through the existing
action surface. Initial configuration pairs explicit document shares
only.

For the real-phone checkpoint, the Unix-only path needs a network
adapter. Reuse the approved Tailscale endpoint and verified device
identity from the [Herdr gateway design](HERDR_INTEGRATION_SPEC.md#h5-pairing-transport-and-access)
when available. Use a separate document route and scope, with the existing
Rust document framing and a native client. Keep the document adapter's
Rust policy check on the trusted host. The gateway supplies transport
identity using its existing authenticated context. A terminal grant or
a shared document link alone confers no document write permission.
Local/development tests use the Unix and scripted transports until the
gateway and device lanes are available.

**Done.** Android and desktop join a fixture share, edit offline then
converge on reconnect, and preserve the Android note and caret through
background/relaunch. Independent grant revocation works for the document
without dropping a permitted terminal. Report local and real-phone
results separately.

**Result (2026-11-21) — local lane only.**

- Desktop: `emthin-collab::unix` now has a two-instance test that binds two
  hosts from one share snapshot, cross-connects them with `Hello`, exchanges
  catch-up, edits each replica independently and verifies text **and** version
  vector converge. This is the disposable-desktop path through the existing
  Rust Unix host.
- Android: `emthin-collab` is a std-only path dependency of the pad. The new
  `sharing.rs` owns a worker thread with the transport-side `MathDoc` replica
  and `Session`; the live `DocModel` never crosses the thread boundary. The UI
  handle sends commands (join/publish/receive/leave) and drains typed events;
  `shareApply` applies an accepted delta as one grouped edit on the UI-thread
  model and reuses the existing `state_json`/caret/recovery path.
- JNI surface: `shareOpen`/`shareClose`/`shareJoin`/`sharePublish`/
  `shareReceive`/`shareLeave`/`sharePoll`/`shareApply` in `DocBridge.java`.
  `ActionCatalog` gained `SHARE_OPEN`/`SHARE_JOIN`/`SHARE_LEAVE`/`SHARE_WATCH`;
  `MainActivity` dispatches them, applies accepted deltas when not watch-only,
  and closes the worker on destroy. Java host tests pass (action catalog 56
  checks). Desktop Share/Join/Leave remain the documented `collab_*` IPC
  methods (K4) — no new keyboard binding was added for peer selection.
- Tests: the worker owns its replica and never the live model; offline-then-
  reconnect exchange converges; leave/stop are clean; two Unix hosts converge.
  Android host `cargo test --locked`: **113 passed** (up from 110).
  `cargo test --locked --workspace`: **671 passed, 0 failed, 2 ignored** (up
  from 670). Desktop `cargo clippy --workspace --all-targets -D warnings` and
  `bash scripts/verify-invariants` pass.
- **Real-phone lane: not run (named gap).** No Tailscale/tsnet endpoint,
  tsnet `WhoIs` identity check, WSS `/v1/stream` document adapter or separate
  document route exists in this checkout; `emthin-collab::unix` is filesystem
  Unix-socket only. The pad's `GatewayLeaseClient` is a terminal-only scripted
  client with no document grant binding. Consequently the real-phone
  checkpoint, and the "revoke document grant without dropping a permitted
  terminal" end-to-end proof, remain open until the gateway document adapter
  and device lane exist. Document grant revocation itself is covered by the K4
  tests; terminal lease state is independent in `SessionCatalog`.
- `emthin-android` clippy has 19 pre-existing findings elsewhere in the crate
  under this toolchain; none are in the new `sharing.rs`.

### K7. Inspect session state and debug replay in Rust

Extend existing IPC status and tracing with opt-in bounded events for
load/recovery, handshake, catch-up, accepted change counts, ack/retry,
peer lifecycle and validation/permission failure. Correlate by run/share,
peer, frontier and connection epoch. Report a reason for incompatibility
or refusal. Omit raw note text, terminal bytes, presence contents and
keys from default logs. Reuse the existing semantic preview and status
tools. A fixture CLI can record and replay metadata as NDJSON and verify
the expected state without a browser devtools panel.

**Done.** Replay a delayed recovery and a reconnect/failure trace through
the same event reducer. Both frontends show the same settled/refused
state. Turn observation off and confirm there is no file or event queue
created solely for it.

**Result (2026-11-21).**

- `docui/inspect.rs` defines a closed `EventKind` set (load, recovery,
  handshake, catch-up, accepted changes, ack, retry, peer joined/left,
  refused), a metadata-only `SessionEvent` (run, share, peer, frontier, epoch,
  count, reason — no note text, terminal bytes, presence blobs or keys), a
  bounded 512-entry opt-in `EventLog`, and one `EventReducer` used by both the
  live `settled()` and NDJSON `replay()`.
- `EmthinState` owns the log, off by default; `inspect_event` records
  correlation metadata from the live session. Handshake, accepted-change
  counts and permission refusals are recorded in the IPC collab paths.
  `inspect_start` / `inspect_stop` / `inspect_state` and the `inspect_state`
  notification expose the same reducer over IPC; `docs/ipc.md` is in sync
  (34 client→server, 24 server→client).
- A fixture CLI (`inspect-replay`) replays an NDJSON trace from stdin and
  prints the settled state; it reports a malformed line instead of settling
  on a wrong state. The pad compiles the same module via `#[path]`, and a host
  test asserts the desktop and Android reducers settle identically.
- Tests: delayed-recovery and reconnect traces settle identically live vs
  replayed; a refusal trace settles the same way on both paths; a bounded log
  drops oldest entries; observation off creates no buffer and yields the
  default state; malformed replay lines are rejected; events carry only the
  closed metadata field set.
- `cargo test --locked --workspace` exits 0: **677 passed, 0 failed, 2
  ignored** (up from 671). Android host tests: 120 passed (up from 113).
  `cargo clippy --locked --workspace --all-targets -D warnings`,
  `bash scripts/verify-invariants`, `check-ipc-docsync` and `check-links`
  pass. The CLI was exercised with a four-event recovery/reconnect fixture.
- Gaps (updated 2026-11-22): the on-disk trace gap is closed. `EventLog` gained
  an opt-in NDJSON sink (`enable_with`) that mirrors the bounded in-memory ring
  to a `.inspect.ndjson` sidecar next to the session file; `inspect_start` wires
  it when a primary session owns a state dir, and a sink failure degrades to
  in-memory observation. A unit test proves the file bytes equal `ndjson()` and
  that replaying them settles to the same state. The live control-client case
  is also closed (`scripts/e2e/live-ipc.sh`, `inspect_state` with `enabled`).
  The inspect count is now visible on the desktop status line after background
  work, so observation has a UI surface beyond `list_state`/IPC. The Android
  host test uses the shared reducer rather than a device run.

### K8. Derived proof and reference queries in Rust

The first index is a projection over the existing `SemanticIndex`, marker
pairs and validated references in the MM0 proof note fixture. Support
one concrete query, “show definitions and references used by this proof
note, with missing targets and the last known proof result.” Resolve
shared document keys and stable IDs within the open share. Proof result
is an independently fetched, read-only fact. The existing MM0 backend
remains the checker. Add a keyed view and update it from K1 events only
if source rebuilds are measurably costly. Capture result ownership so a
late result for a different proof or document does not overwrite the view.
For a bounded transitive dependency query, use the existing reference
edges, deduplicate nodes and handle cycles. Keep the query read-only.

**Done.** On the existing proof note fixture, rewrite one affected
definition or reference, change a mark and deliver a stale proof result.
Show the right affected rows and provenance. Compare the maintained
view with a full rebuild on the same fixtures. Use the current
navigation/status surface. A generic query language or generalized
constraint engine would require another demonstrated query beyond
these existing projections.

**Result (2026-11-21).**

- `docui/proof_index.rs` projects definitions, references and missing targets
  from mathed's existing `SemanticIndex`, marker pairs and `\formal` specs —
  no second parser or symbol table. `ProofView` carries an explicit
  `ViewOwner` (share id + document key) and a caller-supplied revision.
- The one concrete query is "definitions and references used by this proof
  note, with missing targets and the last known proof result." A bounded
  `dependencies(name, limit)` walks only the existing reference edges,
  deduplicates nodes and terminates on cycles.
- A `ProofResult` is an independently fetched read-only fact. `apply_result`
  accepts it only when both the owner and the document's proof id match, so a
  late or foreign result cannot overwrite the view or touch the document.
  No full-view incremental update was added: rebuilding from the shared
  engine is cheap on this fixture, and the plan allows keeping the direct
  projection when measurements do not justify a keyed cache.
- IPC gained `proof_query` / `proof_result` and the `proof_view` notification;
  `docs/ipc.md` is in sync (36 client→server, 25 server→client). The view
  reuses the existing status/navigation surface rather than adding a panel.
- Tests: the view lists definitions, references and an unresolved `$ghost$`
  target; rewriting the definition literal changes exactly the affected row
  after a rebuild; a marks-only change leaves text rows unchanged; a stale
  or foreign proof result is ignored while a matching one is accepted; a
  maintained view equals a full rebuild; a cyclic dependency query is bounded
  and deduplicated.
- `cargo test --locked --workspace` exits 0: **682 passed, 0 failed, 2
  ignored** (up from 677). `cargo clippy --locked --workspace --all-targets
  -D warnings`, `bash scripts/verify-invariants`, `check-ipc-docsync` and
  `check-links` pass.
- Gaps (updated 2026-11-22): both the live control-client case **and** a real
  verifier run are closed. `scripts/e2e/live-ipc.sh` delivers a `proof_result`
  with the document's own owner/proof id and asserts the next `proof_query`
  reports the verdict. Writing it exposed a production bug: `ipc_proof_query`
  rebuilt the proof view on every call, silently discarding an accepted verdict;
  `DocUi::proof_view_cached` now reuses the view when owner/generation match,
  with a regression test for retention and invalidation on edit.
  `scripts/e2e/live-mm0.sh` starts a loopback `mm0web`, verifies the real Peano
  `.mm0`/`.mm1` with mm0-rs + mm0-c, reads the backend's content-addressed
  proof id from its audit trail, writes a document citing that id via a new
  `\formal(..., proof: "id")` named argument, and feeds the backend's verdict
  into `proof_result` — all 5 checks pass (id `Peano-live-66bfb078`). The
  `proof:` argument is an additive shared-core change verified in both lanes
  (585 passed). A concrete second query now exists: `proof_search` filters
  definitions/references/unresolved names by case-insensitive substring and
  returns `(name,start,end)` rows so a client can key a bounded incremental
  view by source range (unit test + live `proof_matches` assertion). No
  general-purpose query language was introduced — only this second query over
  the existing facts, which is what the gap asked to demonstrate.

### K9. Compile a pure rule with AustralVM when justified

Use Rust for the shared document/schema types, Loro, persistence, protocol,
rendering and Android bridge. AustralVM is a candidate for a bounded pure
rule, such as deriving proof-dependency completeness from validated link
facts. Start with the K8 Rust result as the test oracle. Reuse the
build-time Austral-to-CPS artifact approach in `../australVM/refi` and the
existing Cranelift bridge. The Android JIT smoke demonstrates a closed
constant call and is only a linkage check. Evaluate whether a native
portable artifact is practical before requiring that bridge for the new
feature.

Define a narrow, versioned input/output ABI with bounded typed facts,
no host I/O and an explicit error value. Load only a known artifact built
with the application, with the existing deny-by-default capability gate.
Never compile or run code supplied by a document or peer. The rule's
result is advisory until it passes both platform lanes. If eventually
used for admission, evaluation failure denies the affected request and
keeps the last valid local state. Keep the Rust implementation if the
Austral variant cannot reduce drift, provide stronger checked invariants
or pass the native build/test gates.

**Done.** The same positive, negative, malformed, cyclic and bounded
fixtures have identical Rust/Austral results, including error meanings.
Run actual calls off the Android UI thread on arm64 and verify older
artifacts and malformed inputs fail cleanly. Record build/runtime
dependencies, artifact identity and measured cost.

**Result (2026-11-21) — evaluated, not adopted.**

- The rule candidate is defined in `docui/proof_index.rs` behind a narrow,
  versioned ABI (`RULE_ABI_VERSION = 1`): bounded typed `LinkFact` inputs,
  no host I/O, and an explicit `RuleResult::{Complete, Incomplete, Malformed}`.
  It derives proof-dependency completeness from validated reference edges.
  The positive, negative (missing target), malformed (empty id), cyclic and
  over-bound fixtures all have deterministic Rust results with explicit error
  meanings, and are covered by tests.
- The existing `austral_cranelift_bridge` in the pad is a linkage smoke: its
  CPS v1 mirror compiles closed constant-return bodies behind `jitRoundtrip`,
  with the `DenyAll` capability gate untouched. It has no array/bounded-record
  ABI, no versioned artifact identity path for this rule, and lives in an
  external checkout (`../australVM`) that the desktop crate does not depend
  on. A real Austral variant would require extending that ABI and artifact
  pipeline, then passing both the desktop and Android native lanes.
- Decision: **keep the Rust rule.** The plan explicitly allows this when the
  Austral variant cannot reduce drift, provide stronger checked invariants or
  pass the native build/test gates. Here it does none of the three: the facts
  already come from mathed's validated semantic index (one source of truth),
  Rust's `RuleResult` encodes the same checked invariants, and no native
  artifact for the bounded-record ABI exists to gate. The advisory result is
  therefore Rust-only and does not affect admission.
- Verification: `cargo test --locked --workspace` exits 0: **683 passed, 0
  failed, 2 ignored** (up from 682), including the rule fixtures.
  `cargo clippy --locked --workspace --all-targets -D warnings` exits 0.
- Gap closed (2026-11-22): a real Austral artifact now exists and runs.
  `emthin-android/austral/LinkRule.aum` implements the rule in Austral (bounded
  base-10 digit encoding over the scalar CPS ABI); `austral/build.sh` compiles
  it with the pinned local compiler to `LinkRule.cps` (CPS v1, 4 functions,
  1389 bytes, SHA-256 `157e22a1…43ca48`). `emthin-android/src/austral_rule.rs`
  embeds the artifact, refuses a fingerprint mismatch or any capability-bearing
  symbol before load, and calls `LinkRule:check` on a worker thread. The five
  host tests reproduce the Rust rule's positive/negative/malformed/cyclic/
  over-bound fixtures with identical results; the pad's JNI `australRule`
  entry is called from a Java background thread and `device-lane.sh
  austral-rule` asserts `complete/2` in logcat on the arm64 Waydroid device
  (**passed**). Fixing the bridge's `compile_cps_binary` to register the
  trapping-arithmetic symbols (missing there, present on the compiler's own
  JIT path) was required for the artifact to finalize. The rule remains
  advisory — this closes the artifact/reproducibility gap, not an adoption
  decision.

## Development testing

Extend the [MM0 proof workflow](../DEVELOPMENT_PLAN.md#development-testing-plan).
Use the existing scratch backend and Zenodo mock to keep it repeatable.

1. Create one shared proof note offline. Open it on two isolated peers,
   confirm mark-only sync, rotate Android during a late callback, then
   disconnect both and edit different spans. Reconnect and verify same
   text, marks and proof references. Repeat after retry and restart.
2. Make one peer read-only, then revoke it during a pending exchange.
   Confirm scoped rejection, retained local copy where applicable,
   expired presence and continued access by the authorized peer.
3. Undo one local batch following the peer's later edit. Compare the
   live result with a read-only earlier frontier. Reopen the recovered
   note and confirm its named file state, latest recovery and
   remote catch-up are reported separately.
4. Query the proof note while its publication moves from queued to published
   in the mock backend. Switch documents before delivering a delayed
   result and compare derived facts against a source rebuild.
5. Build the Rust desktop suite and the standalone Android host/APK
   lanes in an environment with no JavaScript tooling. Run the local
   and peer fixtures there. Review the dependency and packaged asset
   delta for JavaScript engines, JS bundles and browser requirements.
   Report Austral artifact checks and the real-phone run separately.

Record the two native build commands, revisions, package or APK hash,
capabilities, fixture results and any missing deployment/device evidence.
A synthetic replay proves ordering and scope, while the actual mobile run
proves lifecycle, input and transport across platforms.
