# emthin development plan — desktop and shared document core

Reviewed and expanded 2026-11-18. This is the development plan for `./`
(emthin's compositor, document UI, IPC, collaboration and shared file surface).
It is deliberately separate from the Android plan:

- [`emthin-android/DEVELOPMENT_PLAN.md`](emthin-android/DEVELOPMENT_PLAN.md)
  remains authoritative for `emthin-pad`.
- [`docs/REWRITE_PLAN.md`](docs/REWRITE_PLAN.md) is the historical rewrite
  design and work log. Its W1–W11 landed; do not reopen completed work.
- [`docs/MATHED_FOOT_PARITY.md`](docs/MATHED_FOOT_PARITY.md) is the feature
  inventory shared by both frontends.
- The [native terminal figure plan](docs/HERDR_INTEGRATION_PLAN.md) covers
  the separate Herdr, Tailscale and OpenShell integration milestone.

This review selects work that repairs or completes paths already present. It
does not add a second editor model, a window manager, a notebook runtime, a
plugin framework, or a network transport. The excluded new features are listed
with reasons in [Parked work](#parked-work).

The plan is implementation-ready for a cheaper model. Each card states the
current path, the required behavior, the files to read first, the acceptance
evidence, and what must not be built.

## Review baseline

| Item | Observation |
|---|---|
| Repository | `emthin` at `d86710c`, with existing uncommitted collaboration, Android and CI work. Preserve unrelated changes. |
| Workspace | Four Rust crates: `emthin`, `emthin-clipboard`, `emthin-collab`, `emthin-dbus`. |
| Document model | `MathDoc` (Loro), `DocModel`, paged Typst layout, `\app` figures, citations, find, voice and formal-verification seams. |
| Shared frontends | Desktop document UI and the Android pad both build on `mathed_core`/`mathed_mini`; desktop-only work must not assume the pad can follow it in the same change. |
| Documented tests | The old 353/347 counts were reconciled on 2026-11-18: after D8 the workspace runs 511 passed, 2 ignored, including an async-relayout restore regression test and the Core protocol client tests. (The D1 checkpoint was 471.) The desktop E2E also runs on 2026-11-18: `wtype` passes 28/28 and `xdotool` 36/36, 0 skipped; the xdotool extras are Find cancel plus fcitx5/rime preedit and commit checks. |
| E2E | `scripts/e2e/run.sh` runs 28 checks in `wtype` and 36 in `xdotool`: typing, document copy, both clipboard directions, multi-page PgDn/PgUp navigation, per-document preferences, touch panning, no-X11 transport, state read, graceful shutdown, and (XTEST only) Find cancel plus an fcitx5/rime preedit followed by its commit. It does not measure rendering quality or latency. |
| Device lane | `emthin-android/device-lane.sh` is the pad lane; it is not a desktop gate. |
| Sibling revisions | kyneta `83d6e0d06f4b`; pinetrie `0f4483f4aa86`; desktop `06102dcd77f5`; fresh `4a5940ed6063`; foot `152487e49499`; helix `a714a818a3a4`; atuin `6e57c7e3dc8d`; starship `e3b64da0b660`; Infinity `e1a3e4b36e5f`. |

## Findings and priority

| ID | Priority | Finding | Consequence |
|---|---|---|---|
| F1 | P0 — addressed by D1 | At review, `DocUi::save` wrote the Loro snapshot with `std::fs::write`; `SessionFile::save` wrote JSON the same way; `SessionFile::load` silently returned defaults on parse errors. | A crash during autosave could tear either file. D1 now writes a digest-verified pair through same-directory temporaries, keeps a paired backup, migrates legacy metadata, and blocks autosave when no valid snapshot can be recovered. |
| F2 | P0 | `ipc_save_doc` writes `DocUi::model().text()` directly. `DocUi::open_document` only reads UTF-8 source. | An explicit save loses Loro marks. Opening a `.mathed`/Loro file is unsupported even though the session path already uses snapshots. Neither write is atomic or checks for an outside change. |
| F3 | P1 | `tick.rs` calls `state.relayout_document()`; `DocUi::relayout` calls `DocLayoutCache::rebuild`, which runs the whole paged Typst compile and raster on the event-loop thread. `DocModel::set_caret` also marks a document with revealable content dirty. | Caret movement and every accepted edit can stall the compositor. There is no latest-wins worker, no visible-page prioritisation, and no latency measurement. |
| F4 | P1 | `DocumentPreferences` stores page, zoom, pan, reading size and citation format, but no caret, selection or reading anchor. | Relaunch restores the page but not the place in it. Concurrent edits can move the logical location while the stored page stays valid-looking. |
| F5 | P1 | `FindBar::close` only flips `open`; it does not restore the pre-search caret, selection, page or pan. `goto_citation` and page jumps do not enter a return history. | Escape after a search leaves the reader at the last match. There is no Back/Forward for navigation, so a jump cannot be undone without remembering the old position. |
| F6 | P1 | `emthin-collab::Session` already has `set_cursor`, `presence_blob`, `apply_presence`, `present` and `remove_outdated_presence`, and the handlers expose `collab_join`/`collab_delta`/`collab_ack`. emthin only forwards deltas. | Presence is implemented but invisible. `list_state` has no peers, there is no heartbeat, no remote caret, and no peer status. |
| F7 | P2 | The keymap is a static `match`; `DOCUMENTED_KEYMAP` still calls `Ctrl+Shift+Return` "open the app launcher" although the launcher is a relaunch action with no prompt. `DocUi::last_error` is stored but never rendered. | Documentation and behavior disagree, and a failed layout is silent to the user. There is no single place that says which actions currently apply. |
| F8 | P2 | Verification has no latency budget, no crash-injection case, no two-instance presence check, and stale test counts. The E2E still names the IME commit as an uncovered leg. | The riskiest new paths (persistence, async layout, presence) would land without checks that can fail for the right reason. |
| F9 | parked | Semantic outline/definition navigation, rendered-text accessibility, notebook/kernel execution, and AT Protocol publication are real gaps but are not selected here. | They need shared-core or new transport work beyond this plan's "complete existing paths" boundary. See [Parked work](#parked-work). |

The ordering above is intentional. Persistence is P0 because it can destroy a
document; responsiveness is next because it makes every other workflow feel
broken; navigation and presence are user-visible completions; status and
verification make the results observable.

## What the siblings contribute

These are source observations at the revisions above, not claims that the
sibling applications were executed. Paths are evidence locators; they are not
links because the siblings are review-only checkouts.

| Project | Mechanism inspected | Transfer to emthin | Boundary |
|---|---|---|---|
| kyneta | `packages/exchange/src/store/store-program.ts`: explicit `unwritten`/`idle`/`writing` phases, one `owed` follow-up write, acknowledged persistence; `packages/schema/src/position.ts`: side-bias positions that resolve after edits; `packages/transport/src/task-scope.ts`: cancellation periods and owned background work. | D1's acknowledged, one-owed session save; D4's document-bound anchors; D3/D6's owned jobs and cancellation. | No Kyneta store/schema substrate, React runtime, or second document model. |
| pinetrie | `packages/interface/search.ts`: search state belongs to one canvas and its corpus is explicit; ADR-0058: a reader's spot is keyed by message, not a pixel scroll offset; ADR-0213: tests name the place they require and a missing lane is reported with a reason. | D5's scoped search state and origin restoration; D4's anchored reading spot; D8's named requirements. | No conversation tree, agent lanes, columns, boards, or fuzzy matcher. Keep mathed's literal smart-case search. |
| Atuin Desktop (`desktop`) | `src/state/runbooks/snapshot.ts`: snapshots carry runbook identity separate from runtime state; `operation_processor.ts`: unprocessed operations are retried only when online; `src/lib/std/async_queue.ts`: priority concurrency with one release per checkout. | D2's document identity and save-copy/conflict boundary; D3's bounded job admission; D1's durable identity separate from view state. | No runbook execution, secrets, variables, or server sync. |
| fresh | `crates/fresh-editor/src/view/viewport.rs`: one byte anchor plus row displacement, explicit `Follow`/`Hold` ownership; `app/async_dispatch.rs`: a 50 ms per-tick dispatch budget with deferred backlog; `services/recovery/storage.rs`: atomic recovery storage. | D3's bounded/latest-wins work; D4's document-relative reader position; D1's atomic recovery. | No buffer/split/plugin system, piece tree, or QuickJS. |
| foot | `render.c`: content rendering and damage/refresh scheduling are separate; `search.c`: search saves the original viewport and follows only while the reader is at the end; `ime.c`: pending preedit and commit are distinct until applied. | D3's overlay-only repaint; D5's cancel/accept search semantics; D7's composition-aware status. | No terminal grid, PTY, escape sequences, or PRIMARY-selection policy in the document model. |
| helix | `book/src/jumplist.md`: bounded significant-movement history with Back/Forward; `helix-view/src/view.rs`: forward-branch truncation and document removal; `document.rs`: atomic save, mtime conflict detection, and diagnostics mapped through edits. | D5's 30-entry return list; D2's conflict check; D7's source-linked diagnostics. | No modal editor, LSP, multi-cursor, rope, or replacement document model. |
| atuin | `search/keybindings/keymap.rs`: ordered conditional rules, first match wins; `search/engines.rs`: search mode and filters are part of one state; `search/engines/db.rs`: acceptance can return a result without executing it. | D5's context-specific Find behavior; D7's action availability predicates. | No shell-history database, SQL/fuzzy query language, or executable results. |
| starship | `src/context/mod.rs`: `OnceLock` caches, scan/command timeouts, and only applicable modules; `configs/starship_root.rs`: module order is a configuration, not a second state model. | D3's cached derived data and bounded probes; D7's compact status projection. | No prompt, shell probes, arbitrary status commands, or settings DSL. |
| Infinity | `system/local/driver.rs`: one in-flight step paired with its cancellation handle; `system/defer.rs`: synthetic work waits while a tool call is outstanding; `infinity-daemon/src/session_store.rs`: session lifecycle is persisted separately from conversation bytes. | D3's one-owner render/layout jobs and cancellation; D6's peer lifecycle; D1's separate durable state. | No agent runtime, network protocol, or durable replay of page renders. |

The existing [Android sibling study](emthin-android/SIBLING_STUDY.md) and
[feature specification](emthin-android/FEATURE_SPEC.md) already record the
pad-specific adaptations. This plan does not repeat them. Shared changes must
keep those contracts green.

## Design constraints

1. **The document remains the layout authority.** Figure geometry still comes
   from `\app` statements and Typst frame introspection. Do not add compositor
   placement state.
2. **Use the existing document engine.** No second parser, scanner, undo stack,
   search engine or editor model. Extend `DocModel`, `DocLayoutCache`,
   `FindBar`, `Session` or the IPC catalog.
3. **One owner per asynchronous job.** A render/save/presence job has one
   identity and one result slot; a newer request may replace an unstarted
   request, never publish over a newer applied result.
4. **Keep the last good page visible.** A failed layout or a pending worker must
   not blank the document. The same rule already applies to a malformed
   statement mid-typing.
5. **Presence is ephemeral.** It never enters Loro history and never becomes a
   durable document edit. AT Protocol remains design-only.
6. **No silent data loss.** If persistence cannot prove a valid copy, keep the
   bytes, report the failure, and stop writing over them until the user acts.
7. **Shared-core changes require both lanes.** A change in `mathed_core` or
   `mathed_mini` must run the Android host tests at minimum; a pad-visible
   behavior change needs the device lane or an explicit named gap.

## Workstreams and task cards

Sizes are implementation scope, not elapsed-time promises: S is one focused
change, M is two or three, L is a small subsystem with a design check. Every
card must leave a buildable tree and carry evidence for its own boundary.

Every selected card extends a path that already exists: D5 adds return history
to existing jumps, D6 exposes existing presence, D7 projects existing state.
A wholly new capability is not a card here; it is in [Parked work](#parked-work)
until it has a separate scope decision.

| Order | Card | Depends on | Size | Implementer |
|---|---|---|---|---|
| 0 | D0 Re-establish the measured baseline | — | S | Cheaper LLM |
| 1 | D1 Make session persistence crash-safe | D0 | M | Cheaper LLM; owner reviews the on-disk contract |
| 2 | D2 Make document files truthful | D1 | M | Cheaper LLM |
| 3 | D3a Separate caret/selection repaint from layout | D1 | S | Cheaper LLM |
| 4 | D3b Move layout/raster to a latest-wins worker | D3a | L | Cheaper LLM; design check for `Send` and page lifetime |
| 5 | D4 Restore the reader's place with document anchors | D1, D3a | M | Cheaper LLM |
| 6 | D5 Make find cancel and jumps returnable | D4 | M | Cheaper LLM |
| 7 | D6 Wire collaboration presence and peer status | D4, D7 (status hook) | M | Cheaper LLM |
| 8 | D7 Add one action/status surface | D1, D3a | M | Cheaper LLM |
| 9 | Throughout | D8 Extend verification with each card | all | S–M | Cheaper LLM; owner runs the desktop E2E if the machine cannot |

Checkpoints:

- **E0 — no data loss (D0–D2).** A crash or outside write cannot silently
  destroy marks, preferences or the last good snapshot.
- **E1 — responsive editing (D3).** Caret movement does not recompile the
  document; a layout in flight cannot blank or overwrite the visible page.
- **E2 — returnable reading (D4–D5).** Relaunch, Find cancel and navigation
  jumps restore the reader's place.
- **E3 — visible collaboration and state (D6–D7).** Peers, save/render failures
  and the active action context are visible without reading logs.

### D0 — Re-establish the measured baseline

**Current.** The documented counts are now reconciled: the 2026-11-18 baseline
ran 471 passed, 2 ignored after D1, and the completed plan runs 511 passed, 2
ignored (D8), including the async-relayout restore regression test and the Core
protocol client tests. `cargo clippy` and `verify-invariants` pass;
`cargo fmt --check` is red on pre-existing modified files, which must not be
reformatted wholesale. The E2E's 28/36 checks and the cargo suite measure
different seams. The `wtype` E2E lane runs headless sway against the flake's
Mesa, and `xdotool` runs sway's X11 backend on Xvfb;
`GBM_BACKENDS_PATH` and `LIBGL_DRIVERS_PATH` keep Mesa from searching the
absent system path `/run/opengl-driver`. The 2026-11-18 results are 28/28
(`wtype`) and 36/36 (`xdotool`), 0 skipped. Timing now covers layout + CPU raster, document relayout, snapshot
encode/write, save and autosave, and the event-loop tick. No live-session
latency number has been captured yet.

**Work.**

- Run `cargo fmt --check`, `cargo clippy --locked --workspace --all-targets --
  -D warnings`, `cargo test --locked --workspace`, `bash
  scripts/verify-invariants`, `python3 scripts/check-links` and `python3
  scripts/check-ipc-docsync` at the review HEAD.
- Run the desktop E2E if the machine has the input configuration; otherwise
  report the exact missing authority. Do not convert a skipped lane into a
  pass.
- Record the fresh counts and the exact sibling revisions used. Update
  `README.md` and `docs/measurement-surfaces.md` only from the run's log.
- Timing is emitted on the `emthin::timing` target: `debug` shows layout,
  relayout, save, autosave and slow ticks; `trace` adds every tick. Capture a
  live-session baseline from `RUST_LOG=emthin::timing=trace` when a compositor
  is available. This is instrumentation, not a new metrics service.
- Define the requirements for a crash/restart test and a two-instance
  collaboration test now, so later cards do not invent acceptance after the
  fact.

**Acceptance.** A short report names each command, its exit status and the
counts it produced; every unavailable lane names the missing requirement. The
timing log is visible only at debug/trace level and does not change behavior.

**Do not.** Add a benchmark framework, a dashboard, a new CI service, or
rewrite the existing gates.

### D1 — Make session persistence crash-safe

**Current.** Implemented 2026-11-18. `session::store::SessionStore` owns the
snapshot and metadata pair. Metadata carries a SHA-256 of its snapshot; writes
use same-directory temporaries, `sync_all`, rename and a paired backup. Load
returns `Loaded` (primary, backup or legacy), `Missing`, or `Unrecoverable`, and
quarantines copies of files it cannot verify. `DocUi` keeps the failure as
`persistence_error` and suppresses autosave until an explicit open.

**Required behavior.**

- Introduce one persistence module (extend `session.rs` or add
  `session/store.rs`) that owns reads and writes for both the snapshot and its
  metadata. `DocUi` calls it instead of touching `std::fs` directly.
- Keep the public meaning of `--session-file`: the primary snapshot path stays
  the path the user supplied. A backup beside it is an implementation detail.
- Write through a temporary file in the same directory, flush the file, rename
  it over the primary, and retain the previous valid primary as a backup. A
  failed rename or flush leaves the old file intact. Do not use a temporary
  directory on another filesystem.
- Add an integrity check to the metadata (at minimum a digest of the snapshot
  bytes and the metadata version). A version or digest mismatch is a
  recoverable load error, not a default.
- Load in this order: primary + metadata, then backup + metadata, then the
  legacy unpaired `session.loro`/`session.json` shape. Return a typed result:
  `Loaded { snapshot, preferences, source }`, `Missing`, or
  `Unrecoverable { quarantined_paths }`.
- On `Unrecoverable`, preserve the files, surface the reason through the status
  path from D7 (or a warning until D7 lands), and **do not autosave over them**.
  A new blank document may be created only after an explicit new/open action.
- Keep one owed autosave if the write is moved off the event loop: an in-flight
  write plus one latest request, never a growing queue. Until then, a
  synchronous write is acceptable, but its failure must be state, not only a
  `tracing::warn!`.
- Migrate the legacy files only after a successful verified load. A migration
  failure must not delete the legacy files.

**Read first.** `crates/emthin/src/session.rs`, `crates/emthin/src/docui/mod.rs`
(`save`, `load`, `open_document`, `tick`), `crates/emthin/src/main.rs` (session
path setup), and the Android `RecoveryCodec`/`RecoveryStore` for the fault cases
already handled there.

**Acceptance.**

- Unit tests cover: round trip; missing file; a future metadata version; a
  truncated snapshot; a checksum mismatch; a failed temporary write; a failed
  rename; a valid backup after a torn primary; legacy migration; and an
  unrecoverable pair that is not overwritten by a later autosave.
- A process-kill test (or a deterministic fault injection at the write
  boundary) leaves at least one valid snapshot after a kill during save.
- Preferences for another document survive a failed save of the current one.
- `cargo test -p emthin` and the existing session tests pass.

**Result (2026-11-18).** Eleven store tests cover the listed recovery cases,
including blocked temporary/backup paths and a torn primary. Two `DocUi` tests
prove an unrecoverable load is not overwritten by `tick` and a failed save is
state. `cargo test --locked --workspace` passes 471 tests with 2 ignored;
`cargo clippy --locked -p emthin --all-targets -- -D warnings` passes. This is
deterministic write-boundary fault coverage, not an OS process-kill harness.

**Do not.** Change the Loro snapshot format, add a database, add encryption,
or make `SessionFile` a general-purpose configuration store.

### D2 — Make document files truthful

**Current.** `ipc_save_doc` writes only `model.text()`; `open_document` only
reads UTF-8 source. Marks are lost on an explicit save, a Loro file cannot be
opened as a document, and overwriting an externally edited file is
unconditional.

**Required behavior.**

- Add a document-file abstraction owned by `DocUi` (or `docui/file.rs`) with a
  detected `FileKind`: `Source` (UTF-8 Typst/mathed text) or `Snapshot` (Loro
  bytes). Detect by successful `MathDoc::from_snapshot` first, then UTF-8;
  extension is a hint, not the only signal.
- `open_document` loads either kind. Opening a snapshot preserves marks and
  history; opening source creates a fresh model. Keep the current source-load
  behavior for `.typ`/text files.
- `save_doc` keeps its current source-writing behavior for source paths, and
  writes a snapshot for `.loro`/`.mathed` paths or when the IPC request asks
  for `format: "snapshot"`. Add the format selector as an optional,
  backward-compatible field; a client that omits it keeps today's behavior.
- Use the same atomic writer as D1 for both formats. A source save must not
  claim to preserve marks; a snapshot save must round-trip them.
- Record `FileStamp { canonical_path, bytes, modified, sha256 }` when a file is
  opened or saved. Before replacing a file, re-read and compare. On a mismatch,
  fail with a new typed `Conflict` error unless the request explicitly says
  `force: true`; the caller can then use a new path (save-copy).
- Add `Conflict` to `IpcErrorCode` only if clients need to branch. Otherwise use
  `IoFailed` with a non-retryable detail, but update `docs/error-contract.md`
  either way. This is a protocol change: update `docs/ipc.md`,
  `scripts/check-ipc-docsync`, and the message tests in the same change.
- Emit `DocSaved` with the path, format and digest. Do not report success until
  the rename has completed.

**Read first.** `crates/emthin/src/ipc/dispatch.rs` (`ipc_open_doc`,
`ipc_save_doc`), `crates/emthin/src/ipc/messages.rs`, `docs/ipc.md`,
`docs/error-contract.md`, `crates/emthin/src/docui/mod.rs` (`open_document`),
and `crates/emthin/src/docui/ascii_interchange.rs` for the existing source
projection.

**Acceptance.**

- A snapshot save followed by open restores text **and** marks.
- A source save followed by open restores text and explicitly does not claim
  marks.
- An outside modification between open and save is refused; `force` replaces
  it and updates the stamp.
- An interrupted save leaves the old file valid.
- IPC docs and error contract pass their checks; malformed `format`/`force`
  fields are rejected rather than ignored.

**Do not.** Add a native file picker, a recent-files database, cloud sync, or a
second serialization format for the document.

### D3a — Separate caret/selection repaint from layout

**Current.** `DocModel::set_caret` and `set_selection` set `dirty` whenever the
document has markers, statements, repeated spaces or math spans. `relayout`
then reruns the full Typst layout. That is correct when the caret actually
changes a revealed hidden span, but wasteful when it moves through ordinary
prose.

**Required behavior.**

- Compute a `RevealKey` from the caret/selection, the resolved marker/statement
  spans, and the transform options that actually change revealed source.
- `DocModel` keeps a content generation separate from caret/selection state.
  Bump it on every mutating API, including `math_doc_mut`, `replace`,
  `replace_many`, undo/redo and external imports. Do not rely on
  `MathDoc::revision` alone: the Android work already records that it changes
  for text but not marks.
- In `DocUi::relayout`, rebuild layout only when content generation or
  `RevealKey` changed. A caret move that does not change either sets a repaint
  flag and returns no new layout.
- Caret and selection overlays read the new positions from `DocLayoutCache`
  without a compile. Preserve the existing fallback: if no layout exists yet,
  the first edit still lays out.
- Add a unit-test counter at the layout seam (or an injectable rebuild hook) so
  a test can prove "ten caret moves in prose: zero rebuilds" and "caret enters
  a hidden statement: one rebuild".

**Read first.** `crates/emthin/src/docui/model.rs` (`set_caret`,
`set_selection`, `has_revealable_content`, `transform_options`),
`crates/emthin/src/docui/mod.rs` (`relayout`), `crates/emthin/src/docui/layout.rs`,
and `mathed_core::transform::TransformOptions`.

**Acceptance.** The compile-count test above; the existing caret/selection and
render tests still pass; a visible caret still follows a hidden marker reveal.

**Do not.** Remove the reveal behavior, cache whole transformed documents by
caret, or add a second layout engine.

### D3b — Move layout/raster to a latest-wins worker

**Current.** `DocLayoutCache::rebuild` runs `layout_doc_paged_with_text_size`
synchronously from the tick. `PageLayout` owns page rasters and glyph indexes;
a long document therefore blocks input and frame callbacks for the whole
compile. There is no stale-result path because there is no worker.

**Required behavior.**

- Add a layout worker with one active job and at most one pending job. A new
  request while one is active replaces the pending request; it does not queue
  an unbounded series. This is the Kyneta `owed` shape and the Infinity
  in-flight/cancel shape, applied to layout.
- The worker owns a `LayoutRequest { generation, text, options, text_size }`
  and returns `LayoutResult { generation, pages, diagnostics, compile_time,
  raster_time }`. The UI thread owns `DocLayoutCache` and applies a result only
  if its generation is still the newest requested generation.
- Keep the previously applied pages while a replacement is pending. Do not
  remove all pages at request time; a pending layout must not blank the
  document. A failed request keeps the old pages and records an error.
- Rasterise the visible page first. If `PageLayout` is not `Send` in the
  current dependency versions, the worker should return the page bytes plus
  glyph/index data and construct `PageLayout` on the UI side; verify this with
  a compile-time probe before designing around it.
- Cancel or discard an in-flight result when the document is replaced (New/
  Open/import). Do not apply a result from a previous document to the new one.
- Preserve the existing default: a failed layout leaves the last good raster
  visible and records `last_error` for D7. A successful layout clears it.
- Log `layout_compile_ms`, `layout_raster_ms` and `layout_apply_ms` at debug
  level. Do not add a metrics dependency.

**Read first.** `crates/emthin/src/docui/layout.rs`, `crates/emthin/src/docui/mod.rs`
(`relayout`), `crates/emthin/src/tick.rs`, `crates/emthin/src/state/mod.rs`
(`relayout_document`), and the voice worker in `crates/emthin/src/docui/voice.rs`
for the existing std-thread/channel pattern.

**Acceptance.**

- A test with an injected slow/fake layout proves that a newer request wins and
  an older result is discarded.
- The visible page stays rendered while a new request is pending.
- A failed compile keeps the old pages and surfaces `last_error`.
- A document replacement discards an in-flight result.
- A measured long-document edit does not block the event loop for the full
  compile; report the actual before/after timings from the D0 instrumentation.

**Do not.** Add Tokio, a thread pool, a second renderer, a persistent render
cache, or a queue of every keystroke. One active plus one pending is the
contract.

### D4 — Restore the reader's place with document anchors

**Current.** `DocumentPreferences` persists page, zoom, pan, reading size and
citation format. It does not persist caret, selection or a source-relative
reading anchor. After relaunch the page is right but the caret is at zero.

**Required behavior.**

- Extend the document's saved preferences with optional Loro cursor anchors:
  a caret anchor, two selection anchors (fixed end and caret end), and a
  reading anchor for the top-visible source position. Use
  `MathDoc::create_anchor(byte, side)` and `resolve_anchor(encoded, fallback)`;
  do not store raw byte offsets as the durable identity.
- Keep the existing page/zoom/pan/text-size fields for compatibility and for
  fallback. Restore preferences after the first successful layout, as today,
  then resolve anchors, clamp them to the new text, and set the caret/selection
  without triggering a wasteful reflow when the reveal key is unchanged.
- If an anchor no longer resolves, use its stored fallback byte, then the page
  start, then zero. A deleted selection anchor collapses to the caret; never
  apply half a selection.
- Save anchors only for the current document key. New/Open clears the previous
  document's transient anchor state; switching back uses the saved entry.
- Do not enter undo history for a restore. Restoring a caret is view state, not
  an edit.

**Read first.** `crates/emthin/src/session.rs` (`DocumentPreferences`),
`crates/emthin/src/docui/mod.rs` (`apply_document_preferences`, `save`),
`crates/emthin/src/docui/model.rs` (caret/selection), and
`velysterm/crates/mathed_core/src/doc.rs` (`create_anchor`, `resolve_anchor`,
tests). The Android `StableAnchor`/A7 work is the behavioral reference.

**Acceptance.**

- Insert text above the caret, relaunch, and restore the same logical caret.
- Restore a non-empty selection with the correct direction-independent ends.
- Restore a reading anchor on page 2 after an edit that adds a page before it.
- A deleted anchor falls back safely; an empty session restores zero/default.
- No restore creates an undo entry or marks the document dirty.

**Do not.** Add a parallel offset map, persist pixel scroll only, or make
presence cursors durable.

### D5 — Make find cancel and jumps returnable

**Current.** `FindBar` keeps the query and matches after `close`, but not the
pre-search caret, selection, page or pan. Escape only closes the bar. Citation
and page jumps have no return history. The Android A11 implementation records
full origin restoration; the desktop FindBar does not.

**Required behavior.**

- On `open_at`, capture an `Origin { caret, selection, page, pan }`. On cancel
  (`Escape`/close without acceptance), restore all of it: page, pan, caret and
  selection. On accept, keep the selected match and discard the origin.
- `F3`/`Shift+F3` reuse the last accepted query, as today. Reopening starts a
  fresh query and a fresh origin.
- Add a bounded return history (30 entries) for significant jumps: an accepted
  Find match, a citation go-to, a page jump, and an IPC navigation request.
  Do not push on ordinary caret movement, typing, or every F3 step within one
  Find session (push once when the Find session is accepted).
- Entries store the D4 anchors plus page/pan, not raw byte offsets. Adjacent
  duplicates collapse; a new jump truncates the forward branch; New/Open clears
  the list. Back/Forward restore through the same path as D4.
- Add `Alt+Left`/`Alt+Right` actions and, if a control-plane client needs them,
  `navigate_back`/`navigate_forward` IPC methods. Keep the protocol catalog and
  `docs/ipc.md` in lockstep if added.
- Preserve the existing Find semantics: case-insensitive only for all-lowercase
  queries, inclusive match count/status, and F3 after Escape.

**Read first.** `crates/emthin/src/docui/search.rs`, `crates/emthin/src/docui/mod.rs`
(`open_find`, `close_find`, `find_step`, `goto_citation`), the input handling in
`crates/emthin/src/input.rs`, and `crates/emthin/src/docui/keymap.rs`. Use the
Android `FindController`/`NavigationController` only as behavior references; do
not port its UI.

**Acceptance.**

- Escape after a search restores the exact pre-search page, pan, caret and
  selection; accepting a match keeps it.
- F3 still steps after Escape; a new Find starts at the current caret.
- Find accept, citation go-to and page jump each add one history entry; Back
  returns and Forward repeats; a new jump after Back truncates Forward.
- History is cleared by New/Open and survives an edit above the stored anchors.
- Unit tests cover deduplication, truncation, anchor fallback and origin
  restoration; the E2E adds one Find-cancel check if the harness can drive it.

**Do not.** Add a general command palette, a persistent file history, or a
second navigation stack owned by the compositor.

### D6 — Wire collaboration presence and peer status

**Current.** Implemented 2026-11-18. `emthin-collab::Session` already had
`set_cursor`, `presence_blob`, `apply_presence`, `present` and
`remove_outdated_presence`; emthin now publishes on a one-second heartbeat, on
caret/focus changes and on join/name changes, applies inbound
`collab_presence` blobs, exposes `peers` in `list_state` and
`collab_presence`, and draws remote carets through the existing page glyph
index. Expired peers are pruned, and a reconnect reuses the same peer entry.

**Required behavior.**

- Publish presence on a heartbeat (about once per second) and when the caret
  changes. Publish `None` when the document loses focus or the peer leaves.
- Apply inbound presence blobs through the existing IPC bridge. Presence is
  advisory: clamp a remote cursor to the current text length, hide it when the
  peer expires, and never let it edit or dirty the document.
- Add a peer snapshot to `list_state` and a `collab_presence` notification:
  peer id, display name, optional cursor, connected state and last-seen age.
  Prune with `remove_outdated_presence`; do not grow the peer list on repeated
  joins.
- Draw remote carets as thin overlays on the visible page, using the same
  glyph index and page transform as the local caret. Do not add a second
  coordinate system. A remote cursor from another page is hidden, not moved to
  a guessed page.
- Expose the existing `reconnect` path in the session status. The transport
  that owns the socket remains external; emthin does not open a network
  connection or add AT Protocol.
- Keep the local peer id stable for a process/session. A display name defaults
  to a short peer id and can be set over IPC; no account system.

**Read first.** `crates/emthin-collab/src/session.rs` (`join`, `reconnect`,
`set_cursor`, `present`, `remove_outdated_presence`),
`crates/emthin/src/state/mod.rs` (`collab`, `collab_local_edit`),
`crates/emthin/src/tick.rs`, `crates/emthin/src/ipc/dispatch.rs`,
`crates/emthin/tests/collab_ipc.rs`, and `docs/collaboration.md`.

**Acceptance.**

- Two headless instances joined over the test harness see each other's
  display name and cursor; moving one caret updates the other on the next
  heartbeat.
- Presence never changes document text, revision, undo history or save state.
- A stopped peer disappears after the configured timeout; a reconnect reports
  the peer again without duplicating it.
- `list_state` and the notification remain valid when no peer is connected.
- The existing delta/ack tests still pass, including gap buffering and replay.

**Result (2026-11-18).** `crates/emthin/tests/collab_ipc.rs` now has
`presence_is_visible_expires_and_reconnects_without_editing`, which drives two
real IPC peers through names, a remote caret, `list_state`, expiry, a fresh
publish and a reconnect; it asserts the receiving document text and revision
are unchanged by presence. The existing delta test still passes. `python3
scripts/check-ipc-docsync` reports 24 client→server and 19 server→client
methods in sync; `cargo clippy -p emthin --all-targets -- -D warnings` passes.

**Do not.** Add a WebSocket server, PDS client, account/DID flow, durable
presence, remote cursors that edit, or a second collaboration protocol.

### D7 — Add one action/status surface

**Current.** Implemented 2026-11-18. `docui::keymap::ACTION_CATALOG` now names
every global action with a stable id, label, binding and availability predicate;
the launcher wording describes relaunch, and `DOCUMENTED_KEYMAP` is checked
against the catalog. `docui::status::StatusModel` projects only applicable
state, and a quiet document draws nothing. The compact line is rendered by
`status_bar` through the same raster/overlay path as the find bar; full layout
and save errors remain available from `list_state`.

**Required behavior.**

- Introduce an `ActionCatalog` table (id, human label, binding, availability
  predicate) and route the existing global actions through it. Keep plain text
  editing in `DocModel`; this table is only for document-level actions. Fix
  the launcher label to describe relaunch rather than a prompt.
- Make availability explicit where behavior is already contextual: Escape
  closes a completion, then Find, then returns focus; F3 is available only
  with a prior query; Back/Forward only with history; citation actions only
  with an open citation. First matching rule wins, as in Atuin's keymap.
- Add a compact `StatusModel` that composes only applicable state: document
  name (or Untitled), save state, layout error, Find count, peer count and
  voice/formal activity. Quiet state renders nothing. This is a projection of
  existing state, not a new source of truth.
- Render the status as one small bottom overlay using the existing
  `find_bar` raster/cache pattern. Show the full layout error on demand (for
  example via IPC `list_state` or the status's error detail); do not invent a
  diagnostics panel.
- `DocUi::save` and the new D1 store must update the status on failure, not
  only log. A recovered backup is reported as a warning, not a success.
- Update `DOCUMENTED_KEYMAP` from the catalog and make its test compare the
  catalog, so documentation cannot drift again.

**Read first.** `crates/emthin/src/docui/keymap.rs`,
`crates/emthin/src/input.rs`, `crates/emthin/src/find_bar.rs`,
`crates/emthin/src/figure_render.rs` (overlay placement),
`crates/emthin/src/docui/mod.rs` (`last_error`, save),
`crates/emthin/src/ipc/messages.rs` and `docs/ipc.md`.

**Acceptance.**

- A layout failure is visible without reading a log; a save failure is visible
  and remains until a successful save.
- The status names peers only when present and Find only while open.
- Every documented binding resolves to a catalog entry, and catalog labels
  match the keymap table test.
- Escape/F3/Back/Forward behavior matches the predicates in unit tests.
- Existing find-bar rendering tests still pass.

**Result (2026-11-18).** `cargo test -p emthin --lib` passes 380 tests (2
ignored), including catalog/availability, status precedence/quiet-state,
status-bar raster and the existing find-bar tests. `cargo clippy -p emthin
--all-targets -- -D warnings` and `python3 scripts/check-ipc-docsync` pass.
`list_state` now reports `status`, `layout_error` and `save_error` alongside
the peer snapshot.

**Do not.** Add a command palette, configurable user keymap DSL, a diagnostics
editor, a settings framework, or arbitrary status commands.

### D8 — Extend verification with each card

**Current.** The cargo suite and E2E are healthy gates for their existing
surfaces, but none of them can see the failure modes this plan introduces.

**Work.**

- D1: deterministic write-boundary and recovery tests now assert the old bytes
  are never replaced by a default on an unrecoverable load. Add a process-kill/
  restart check when the gate can own process lifecycle.
- D2: round-trip marks, source/snapshot detection, outside-change refusal,
  forced replacement and interrupted-write recovery.
- D3a: a rebuild-count test that distinguishes caret movement from a reveal
  change.
- D3b: a deterministic fake-worker test for latest-wins, stale rejection,
  replacement cancellation and last-good-page retention. Add one measured
  long-document timing to the D0 notes.
- D4: an edit-above-anchor relaunch test and a deleted-anchor fallback test.
- D5: Find cancel/accept origin tests, history truncation/dedup tests, and one
  E2E Find-cancel check if the harness can drive Escape reliably.
- D6: extend `crates/emthin/tests/collab_ipc.rs` with presence, expiry and
  reconnect; assert the document is unchanged by presence.
- D7: status precedence tests for error > save > find > peers; catalog/keymap
  consistency test.
- Keep the existing gates: `cargo fmt --check`, clippy with `-D warnings`,
  rustdoc, `cargo test --locked --workspace`, `scripts/verify-invariants`,
  `check-links`, `check-ipc-docsync`, and the desktop E2E when available.
- Name each test after the requirement it proves. If a lane cannot run on a
  machine, report the missing requirement in the result; do not silently skip
  it or count it as a pass.

**Acceptance.** Each checkpoint has a command log and a short evidence note.
The plan's status table is updated only from those logs. Do not copy a count
from an earlier document.

**Result (2026-11-18).**

- `nix develop --command cargo test --locked --workspace` exits 0: **511
  passed, 2 ignored**. Per binary: 380 lib (`emthin`), 1
  `docui_async_restore`, 2 `collab_ipc`, 18 `xwayland_satellite`, 5
  `xwayland_satellite_watch`, 14 `emthin-clipboard`, 34 `emthin-collab`, 17
  `emthin-core-client`, 40 `emthin-dbus`; doc-tests 0.
- `nix develop --command cargo clippy --locked --workspace --all-targets --
  -D warnings` exits 0.
- `bash scripts/verify-invariants` exits 0: 7 passed, 0 failed, 0 gaps.
- `python3 scripts/check-ipc-docsync` exits 0: 24 client→server, 19
  server→client in sync.
- `python3 scripts/check-links` over the touched documents
  (`README.md`, `README_cn.md`, `docs/ipc.md`, `docs/collaboration.md`,
  `DEVELOPMENT_PLAN.md`, `docs/measurement-surfaces.md`, `AGENTS.md`) exits 0:
  19 relative links resolve. The whole-repository run exits 1 on one
  pre-existing workspace link: `ATTRIBUTION.md:5` points at
  `../ATTRIBUTION.md`, which is outside this checkout and absent on this
  machine. The link is correct for the workspace root, so it was not rewritten
  or skipped to force a pass.
- `cargo fmt --all -- --check` was not run wholesale: the worktree already has
  pre-existing formatting drift in unrelated modified files (D0 baseline).
- Desktop E2E ran on 2026-11-18 in both configurations: `wtype` (headless
  sway) ran 28 checks and `xdotool` (sway's X11 backend on Xvfb) ran 36, with 0
  skipped and all passed; the XTEST extras assert Find cancel, then the fcitx5
  preedit and commit.
  The flake now exports `GBM_BACKENDS_PATH` and `LIBGL_DRIVERS_PATH` from its
  Mesa so sway does not look for
  `/run/opengl-driver/lib/gbm/dri_gbm.so`; that was an environment gap, not a
  product failure.

## Dependency and delivery order

```text
D0 baseline
  └─ D1 session envelope
       ├─ D2 document files
       ├─ D3a repaint separation
       │    └─ D3b latest-wins layout worker
       ├─ D4 anchored reader state
       │    ├─ D5 find cancel + return history
       │    └─ D6 presence (with D7 status hook)
       └─ D7 action/status surface
D8 verification runs with every card
```

Land in four reviewable checkpoints:

1. **E0 (D0–D2).** Recount the baseline, then make the session and explicit
   document files atomic and truthful. No UI work is required to prove this.
2. **E1 (D3a–D3b).** Remove unnecessary recompiles, then move the remaining
   compile/raster work to one owned worker. D3a should land before the worker
   so a thread is not used to hide a needless recompile.
3. **E2 (D4–D5).** Restore the reader's place and make Find/jumps returnable.
4. **E3 (D6–D7).** Show peers and state; the status surface is also the error
   outlet for D1 and D3.

D6's presence overlay should reuse D7's status/overlay infrastructure where
possible. If D7 is delayed, D6 may expose presence over IPC without drawing
carets, but must not invent a second notification system.

## Cheap-model handoff

Implement one card at a time. A cheaper model should not be asked to redesign
the document model, choose a network transport, or land two checkpoints in one
change.

Each handoff prompt should contain:

1. The card id and its **Required behavior** and **Acceptance** sections.
2. The exact files in **Read first**, plus `AGENTS.md` and the relevant existing
   tests.
3. The rule that unrelated working-tree changes are preserved and nothing is
   committed or pushed.
4. The verification commands for that card, and the instruction to report
   skipped lanes with the missing requirement.
5. The explicit **Do not** list from the card. Those are scope fences, not
   suggestions.

Suggested first prompt:

```text
Implement D2 (truthful document files) in DEVELOPMENT_PLAN.md.
Read the D2 card, then crates/emthin/src/ipc/dispatch.rs,
crates/emthin/src/ipc/messages.rs, crates/emthin/src/docui/mod.rs,
 docs/ipc.md, docs/error-contract.md, and the existing session/store code.
Preserve unrelated working-tree changes. Reuse the D1 atomic writer; do not
change the Loro format or add a database. Run the relevant tests and report the
commands, results, and any unavailable lane. Do not commit or push.
```

D3a and D4 can proceed independently after D1; D3b waits for D3a's
compile-count result.

## Parked work

These are real needs, but they are not selected because they either require a
new subsystem or can be obtained by finishing an existing path. Revisit them
only with a separate scope decision.

| Item | Why parked | What would reopen it |
|---|---|---|
| Interactive app launcher / command palette | The document already owns `launch:` and dormant figures already relaunch by click/Return. A prompt adds a second command surface without improving a broken path. | A concrete workflow that needs discovery of commands not present in a document, plus a decision on where the prompt lives. |
| Free 2D figure placement, workspaces, tabs | Pages and document flow are the design. `ext-workspace-v1` already maps pages. | A user-facing need that cannot be expressed by page flow; would reopen the locked rewrite decision. |
| Semantic outline, definition/use navigation, rename | `mathed_core` has the semantic index and the Android pad has a controller, but desktop needs a selection/navigation UI and shared-core review. | D5's return history landing first, then a separate card that reuses `SemanticIndex` without adding LSP. |
| Rendered-text accessibility | Android has a node provider; desktop would need an AT-SPI/accesskit strategy, not a sibling borrow. | A concrete accessibility target and a testable desktop authority. |
| Notebook/kernel execution, templates, execution output | `\formal` is a verification seam, not a notebook. The pad explicitly excludes kernel workflows; desktop would need a worker/output protocol. | A separate notebook design; do not smuggle execution into D7 or the status line. |
| AT Protocol client / remote transport | `docs/collaboration.md` and `docs/atproto-session.md` deliberately keep live deltas on `emthin-collab`; no client or PDS exists in the workspace. | A deployment decision and credentials, plus an adapter crate outside `emthin-collab`. |
| New plugin framework | `\voice` and `\formal` already establish helper-process seams that degrade to no-ops. | A third concrete plugin with a lifecycle need that the existing seams cannot express. |

## Definition of done

- A kill during autosave never replaces a valid document with an empty one.
- Explicit source/snapshot saves are atomic, preserve the chosen representation,
  and refuse an unnoticed outside write unless forced.
- Caret movement through ordinary prose does not recompile the document; a
  compile that does run cannot blank or overwrite a newer visible page.
- Relaunch restores the logical caret, selection and reading anchor; Find
  cancel restores its origin; significant jumps have Back/Forward.
- Collaboration presence is visible and expires; it never edits the document.
- Layout/save/peer failures are visible in one status projection, and the
  documented keymap matches the action catalog.
- Every checkpoint has fresh command evidence; shared-core changes keep the
  Android host lane green, and a pad-visible change names its device result or
  its missing requirement.

D0–D8 are implemented in this worktree. The desktop E2E ran on 2026-11-18 in
the `wtype` lane (28 checks) and the `xdotool` lane (36 checks, including Find
cancel and the fcitx5 preedit/commit), 0 skipped, all passed, after the flake began
exporting Mesa's GBM/DRI search paths. The workspace-root `ATTRIBUTION.md` link
also cannot resolve inside this checkout, so `check-links` reports that one
pre-existing path rather than a pass.
