# emthin development plan

Reviewed 2026-11-20 against `deca816` and the current working tree. This
plan covers the desktop compositor and shared document behavior. The
[Android development plan](emthin-android/DEVELOPMENT_PLAN.md) owns pad
work and the [terminal integration plan](docs/HERDR_INTEGRATION_PLAN.md)
owns Herdr, Tailscale and OpenShell. Both are used in the
[development testing plan](#development-testing-plan).

The [native Kyneta plan](docs/KYNETA_NATIVE_PLAN.md) extends these paths with
typed change notifications, local-first readiness, scoped sync, collaboration
policy, history and derived views. The implementation is Rust, with a
specific AustralVM rule candidate. It requires no JavaScript tooling or
runtime. D2.1, D3.1, D4.1, D6.1 and D7.1 supply shared prerequisites.

The first D0–D8 pass delivered atomic recovery, source and snapshot I/O,
an asynchronous layout worker, reader anchors, return history, local
presence and one action/status surface. Continue those implementations
with the cards below. Each card repairs an observable gap in an existing
workflow. The document remains the layout authority, and Typst figures
still define placement and size.

## Review baseline

| Area | Current evidence |
|---|---|
| Review inputs | The worktree contains active Herdr, Android and shared dependency changes. Preserve them. `Cargo.toml` currently lists eight desktop workspace members. The pad remains standalone. |
| Invariants | `bash scripts/verify-invariants` passed 7 checks on 2026-11-20. |
| Current Rust tests | `cargo test --locked --workspace -j1` passed **734** tests with 2 ignored on 2026-11-22. Formatting, Clippy and rustdoc passed in the same run. |
| Historical evidence | The 2026-11-19 D8 run lists 538 passed including 18 VT tests and 2 ignored. It covers a different working set. The recorded 2026-11-18 desktop E2E is 28 headless Wayland and 36 XTEST checks. See [D8](#d8--extend-verification-with-each-card) for what those tests exercised. |
| Sibling revisions | kyneta `83d6e0d06f4b`, pinetrie `0f4483f4aa86`, desktop `06102dcd77f5`, fresh `4a5940ed6063`, foot `152487e49499`, helix `a714a818a3a4`, atuin `6e57c7e3dc8d`, starship `e3b64da0b660`, Infinity `e1a3e4b36e5f`. |

## Current priorities

| Order | Card | Why it remains | Dependency |
|---|---|---|---|
| 1 | [D0.1 Reproduce the current build](#d01-reproduce-the-current-build) | Done locally on 2026-11-20; CI input wiring is added but has not run on GitHub. | Coordinate with T01 and T06 |
| 2 | [D2.1 Separate file save from recovery](#d21-separate-file-save-from-recovery) | Done on 2026-11-20: explicit and recovery baselines are separate in the desktop path; the pad already had a separate saved fingerprint. | D0.1 for runtime verification |
| 3 | [D2.2 Protect concrete write targets](#d22-protect-concrete-write-targets) | Done on 2026-11-20: one resolved target drives comparison, write and stamp; symlinks and unknown existing destinations are protected. | D2.1 |
| 4 | [D3.1 Gate retained layout interactions](#d31-gate-retained-layout-interactions) | Done on 2026-11-20: accepted pixels/glyphs carry a document+source owner; stale owners retain pixels but refuse hits, grabs and figure actions. | D0.1 |
| 5 | [D4.1 Restore the visible reading position](#d41-restore-the-visible-reading-position) | Done on 2026-11-20: the reading anchor is the first glyph intersecting the viewport, with a document-space vertical offset; page/pan remains the fallback. | D3.1 |
| 6 | [D5.1 Complete return transactions](#d51-complete-return-transactions) | Done on 2026-11-20: one page-publication path serves Find, citations, history and page navigation; page-only moves no longer compile. | D3.1, D4.1 |
| 7 | [D6.1 Scope collaboration to the active document](#d61-scope-collaboration-to-the-active-document) | Done on 2026-11-20: version-vector triggering includes marks; Open and ASCII import share a scoped peer/presence reset with stale-frame fencing. | D3.1 |
| 8 | [D7.1 Report save and render state accurately](#d71-report-save-and-render-state-accurately) | Done on 2026-11-20: explicit-file, recovery and preview state are separate on the line and over IPC; preview updating is visible. | D2.1, D3.1 |
| 9 | [D3.2 Measure and bound page work](#d32-measure-and-bound-page-work) | Done on 2026-11-20: compile/glyph/raster are measured separately; the worker retains one compiled generation and materializes only the visible page raster. | D3.1, D4.1, named performance run |
| Throughout | [Development testing](#development-testing-plan) | Unit checks and disposable workflows must prove actual behavior across docs, live figures and Android. | Run the applicable lane with each card |
| Next milestone | [K1–K9 Native Kyneta behavior](docs/KYNETA_NATIVE_PLAN.md#implementation-order) | K1–K9 implemented; every locally-closable gap closed on 2026-11-22 (Austral artifact + arm64 run, live MM0 verifier, v2 wire epoch fence, SO_PEERCRED, history persistence, on-disk inspect trace + status UI, real compositor relaunch, device IME/focus, proof_search, pad history preview). The Rust rule stays advisory/unadopted by design. Only the **real-phone transport** remains a milestone-level named gap. | Reopen K6's real-phone adapter only on a concrete deployment decision |

D2 and D3 can proceed in parallel after agreeing on document identity
and content generation. Complete the deterministic safety checks before
desktop cutover or a live sandbox run. Raster optimization follows a
measured baseline. Existing behavior listed in the
[initial delivery record](#initial-d0d8-delivery-record) is regression
coverage for these cards.

## What the siblings contribute

These are source observations at the revisions above, not claims that the
sibling applications were executed. Paths are evidence locators; they are not
links because the siblings are review-only checkouts.

| Project | Mechanism inspected | Transfer to emthin | Boundary |
|---|---|---|---|
| kyneta | `packages/exchange/src/store/store-program.ts`: explicit `unwritten`/`idle`/`writing` phases, one `owed` follow-up write, acknowledged persistence; `packages/schema/src/position.ts`: side-bias positions that resolve after edits; `packages/transport/src/task-scope.ts`: cancellation periods and owned background work. | D2.1's separate write acknowledgements and owed recovery; D4.1's anchors; D3.1/D6.1's job ownership. | No Kyneta store/schema substrate, React runtime, or second document model. |
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

## Follow-up cards

### D0.1 Reproduce the current build

**Read first.** `crates/ghostty-vt/{build.rs,src/lib.rs}`,
`scripts/{check-herdr-inputs,build-ghostty-vt}`, `docs/HERDR_COMPATIBILITY.toml`,
`flake.nix`, and `.github/workflows/ci.yml`.

**Work.** The Rust source path is relative to `src/lib.rs`, while the Zig
source path is relative to the crate manifest. The latter currently resolves
to `/home/leo/herdr-core/vendor/ghostty` here. Correct the manifest-relative
lookup and reuse the existing compatibility pins. Bring the same pinned
inputs to every CI lane compiling the wrapper, including Android, and make
missing source/archive/Zig errors name the attempted path, required revision
and setup action. Keep the pinned Android archive aligned with its ABI.
Reuse T01/T06 for this work.

**Done.** A clean work directory with the declared sibling pins can run
locked workspace tests and clippy. An arm64 APK links against the matching
archive. An intentionally missing input reports the correct requirement.
No installed Core or OpenShell service is started by these checks. Update
counts from those runs only.

**Result (2026-11-20).**

- `crates/ghostty-vt/build.rs` now resolves the default Zig source from the
  crate manifest (`../../../herdr-core/vendor/ghostty`, correcting the former
  four-`..` path), accepts `GHOSTTY_VT_SOURCE`, and reads the required Ghostty
  revision from `docs/HERDR_COMPATIBILITY.toml`. Missing source, Zig, Android
  archive and unsupported-ABI failures name the attempted path, required
  revision and setup command. The Android branch maps `aarch64-linux-android`
  to the `arm64-v8a` archive and rejects other ABIs instead of linking a
  mismatched archive.
- `GHOSTTY_VT_SOURCE=/nonexistent/ghostty cargo build --locked -p ghostty-vt`
  exits 101 with the attempted path, revision
  `1e5cdb865807abc990181efb8ebfa9122955cce2`, the submodule setup command and
  the manifest root.
- `cargo test --locked --workspace` exits 0: **572 passed, 0 failed, 2
  ignored**. Per binary: 390 `emthin` lib (2 ignored live-logos tests), 2
  `collab_ipc`, 1 `docui_async_restore`, 18 `xwayland_satellite`, 5
  `xwayland_satellite_watch`, 14 `emthin-clipboard`, 34 `emthin-collab`, 23
  `emthin-core-client`, 1 `scripted_peer`, 1 `unix_fixture`, 40
  `emthin-dbus`, 23 `emthin-vt`, 15 `ghostty-vt`, 5
  `herdr-gateway-fixtures`; doc-tests 0. Test binaries ran with
  `CARGO_BUILD_JOBS=2` after an earlier OOM.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` exits 0.
- `./build-apk.sh` with `ANDROID_ABI=arm64-v8a` links
  `build/emthin-pad.apk` against the archive; `readelf -h` reports AArch64.
- `python3 scripts/check-herdr-inputs` exits 0 with `herdr_core
  c7517808d64f06609e4ecf0db1757005373cc3a3`, `ghostty
  1e5cdb865807abc990181efb8ebfa9122955cce2` and nixpkgs
  `c59305bab2065cfecc4944690d9eedbb56f3a9fa`. It still warns that the four
  local sibling checkouts have uncommitted changes; those changes are the
  review inputs and were preserved.
- CI now checks out the pinned `herdr-core` and Ghostty submodule in the
  workspace-test, emthin-pad host-test, APK and E2E jobs
  (`.github/actions/checkout-herdr-core`), installs Zig 0.16 from the pinned
  nixpkgs revision for the non-Nix jobs
  (`.github/actions/setup-pinned-zig`), and builds both desktop and
  `arm64-v8a` Ghostty archives before the APK build. Those workflow changes
  are syntax-checked but have not run on GitHub, and a clean-worktree run was
  not possible because the review inputs are uncommitted. Those two points
  remain unverified; the local commands above are the evidence recorded here.
- `cargo fmt --all -- --check` is green for all 9 workspace packages (2026-11-22);
  the remaining `--all` diffs are in external path dependencies (unfer, velysterm)
  with their own edition-2024 formatting.

### D2.1 Separate file save from recovery

**Read first.** `docui/mod.rs` (`load`, `save_document`, `status`, `save`),
`docui/model.rs` (`content_generation`, `mark_dirty`), `session/store.rs`
and the Android `PadSession`/recovery implementation.

**Work.** Track the last acknowledged recovery snapshot independently of
the last acknowledged explicit write and its format. Stamp each completion
with the document instance and the content it actually wrote. Use source
bytes to compare source exports and shared logical rich-text content to
compare snapshots. Persist the explicit destination and its last confirmed
baseline alongside recovery metadata. An older record without them can
report recovery, with file status unconfirmed until the next explicit
open/save. Presentation changes including zoom, reading size and
page change affect recovery view state without changing explicit file
content. An export that omits marks must describe the source it saved.
Keep explicit conflict/error state until the user resolves it, and recovery
failure/warning state until the recovery operation resolves it. Reuse the
existing status projection and atomic session pair. Skip idle periodic
writes when neither content nor persisted preferences changed. A failed
write keeps the relevant change owed. Do the initial version on the
existing save path and measure it before moving file I/O to a worker.

**Done.** Edit a named file, let autosave succeed, then prove in IPC/status
that the file still needs saving and that disk content is unchanged. Save
explicitly and show the represented content is now clean. Repeat with a
mark-only edit, source export, reading-size change and a prior external
conflict. Relaunch recovers the latest note with correct provenance.
Idle ticks produce zero new snapshot writes after the first successful
checkpoint.

**Result (2026-11-20).**

- `DocUi` now carries a `FileBaseline` and a `RecoveryBaseline` separately.
  The explicit baseline records the document instance, representation and the
  content that representation covers; the recovery baseline records the
  instance, content digest, preferences and explicit-file record last written.
  A recovery save no longer changes the explicit file's status.
- `DocModel::{source_digest,content_digest}` distinguish a source export (text
  only) from a snapshot export (text plus segment marks). Source files stay
  clean across marks-only edits; snapshot files do not. Reading-size, page and
  other presentation changes affect only the recovery baseline.
- `SessionFile.explicit_file` persists the destination path, format, byte
  length, SHA-256 and logical content digest. A relaunch reconstructs the
  file's conflict stamp from that record and reports an outside write or a
  missing destination before the next save. Older sessions without the record
  load as recovery with no explicit file status.
- `save()` skips an idle checkpoint when the document instance, content,
  preferences and explicit record all match the last acknowledged pair. Its
  existing timing fields still measure snapshot and write work. A failed write
  does not advance the baseline, so the next tick remains owed.
- `DocUi::status` reports `Failed` for an explicit save/conflict error and
  keeps that state across autosaves; a separate `recovery_error` field shows a
  failed recovery without making a clean explicit file look unsaved. The IPC
  `save_error` projection includes both errors.
- Tests added in `docui::session_tests`, `docui::model` and `session::tests`:
  autosave preserves a named file's unsaved state and disk bytes, explicit save
  clears it, source exports ignore marks, snapshot exports track marks but not
  reading size, a prior conflict stays failed until a forced save, a relaunch
  restores the explicit destination and reports an outside write, and an idle
  tick does not touch a blocked snapshot path.
- `cargo test --locked --workspace` exits 0: **581 passed, 0 failed, 2
  ignored**. Per binary: 399 `emthin` lib (2 ignored live-logos tests), 2
  `collab_ipc`, 1 `docui_async_restore`, 18 `xwayland_satellite`, 5
  `xwayland_satellite_watch`, 14 `emthin-clipboard`, 34 `emthin-collab`, 23
  `emthin-core-client`, 1 `scripted_peer`, 1 `unix_fixture`, 40
  `emthin-dbus`, 23 `emthin-vt`, 15 `ghostty-vt`, 5
  `herdr-gateway-fixtures`; doc-tests 0.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` exits 0.
- `cd emthin-android && cargo test --locked` exits 0: **100 passed**. The Java
  host suite exits 0 (10 groups, including 30 recovery-codec checks and 12
  status-model checks); no Android source changed in this card because its
  recovery metadata already carries `savedFingerprint` separately.
- The measurement is the existing `emthin::timing` `session_save` event
  (`snapshot_us`, `write_us`, `snapshot_bytes`). No file I/O was moved to a
  worker in this card; that remains future work after a measured baseline.
- `cargo fmt --all -- --check` is green for every emthin workspace package
  (verified 2026-11-22). The remaining `--all` diffs are in external path
  dependencies with their own edition-2024 formatting, not this checkout.

### D2.2 Protect concrete write targets

**Read first.** `docui/file.rs`, `session/store.rs::write_atomic`,
`ipc/dispatch.rs::ipc_save_doc`, [IPC](docs/ipc.md) and Android
`ProviderIo.java`'s saved-content comparison.

**Work.** Resolve one destination for comparison, atomic write and returned
stamp. Save through a stable symlink writes the verified target while
preserving the link. A link or target changed since capture returns
`doc_conflict`. Protect an existing destination with either its matching
opened/saved stamp or the existing explicit `force` decision. Publishing
a newly created file must avoid clobbering one created concurrently.
Recheck the known target near replacement. Document the limit with an
uncooperative process writing concurrently to an already existing file.
Retain the stamp and file kind on failure, and keep the original document
and its recoverable bytes when a required pre-switch save fails.

**Done.** Test tracked external change, unknown existing Save Copy target,
new destination racing with creation, stable and retargeted symlinks,
failed rename and an explicitly forced write. A non-forced error leaves
the prior bytes intact. Opening a valid snapshot still round-trips marks.
If IPC changes, update its catalog and error contract in the same patch.

**Result (2026-11-20).**

- `docui::file::write` resolves one destination before the conflict check and
  uses that same target for the temporary file, publish and returned stamp.
  Saving through a stable symlink writes the target and leaves the link in
  place; a retargeted link is `doc_conflict`.
- A destination this document has not opened or saved is refused unless
  `force` is set. New paths are published with a hard-link create-only step, so
  a concurrent creator wins with `AlreadyExists` translated to `doc_conflict`
  instead of being overwritten. The known target and its bytes are checked
  again immediately before publishing.
- `session::store::write_atomic_publish` separates replace and create-only
  publication. `write_atomic` keeps the existing replace path for the recovery
  pair and legacy metadata.
- `open_document` treats a failed required checkpoint as a refusal: it returns
  an I/O error and keeps the original document in memory. An already
  unreadable/blocked session still does not trap the reader; an explicit open
  remains the way out.
- Tests added: stable symlink save, retargeted symlink conflict, unknown
  existing Save Copy target with and without force, create-only publication
  against a concurrent creator, failed publish leaving the destination alone,
  failed pre-switch checkpoint preserving the original document, plus the
  existing outside-write/forced-write and snapshot-mark round-trip checks.
- `docs/ipc.md` documents the symlink/create-only behavior and the remaining
  limit: a separate process writing an already existing target between the last
  check and the rename can still race; atomic replacement is not mutual
  exclusion.
- `cargo test --locked --workspace` exits 0: **588 passed, 0 failed, 2
  ignored**. Per binary: 406 `emthin` lib (2 ignored live-logos tests), 2
  `collab_ipc`, 1 `docui_async_restore`, 18 `xwayland_satellite`, 5
  `xwayland_satellite_watch`, 14 `emthin-clipboard`, 34 `emthin-collab`, 23
  `emthin-core-client`, 1 `scripted_peer`, 1 `unix_fixture`, 40
  `emthin-dbus`, 23 `emthin-vt`, 15 `ghostty-vt`, 5
  `herdr-gateway-fixtures`; doc-tests 0.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` exits 0.
- The Android provider path was inspected: it already compares the active
  destination's remembered SHA-256 before writing and retains the active file
  on failure. No Android source changed in this card; its host Rust/Java lanes
  remain green from D2.1.

### D3.1 Gate retained layout interactions

**Read first.** `docui/{mod,layout,figures}.rs`, `input.rs`, `grabs.rs`,
`figure_render.rs`, `tests/docui_async_restore.rs`, and Android
`PadSession.java`/retained preview tests.

**Work.** Track which document instance and source/layout generation produced
the accepted page, glyph index and figure geometry. Retain last-good pixels
while waiting for the next compile or after a failed compile. Use the
accepted layout for source-derived actions only while its owner is current.
Cover point-to-byte, drag selection, citation hits, vertical glyph
navigation, IME caret rectangles, and figure hit/resize/relaunch; recheck
owner/marker pair on delayed grab commit. Source-only keyboard editing
continues. Invalidate immediately at New/Open/import and when source changes,
even before the next tick schedules a job. Permit current layout-only view
transforms without requesting a compile. Ensure the current generation
alone can publish new figure placements.

**Done.** In a non-`cfg(test)` compositor or controllable worker integration
lane, keep old pixels visible during a selection hit and grab attempt
against stale geometry. Neither edits nor launches can target old
offsets or the wrong figure. Repeat with a failing newest compile and
Open while in flight. An accepted replacement restores hits without
an extra compile. Use a multibyte source edit to demonstrate offset safety.

**Result (2026-11-20).**

- `DocLayoutCache` now carries a `LayoutOwner { document, content }` for the
  pages/glyphs it holds and for the request currently in flight. `glyphs`,
  `screen_to_doc`, `page_for_range` and therefore point-to-byte, citation hits,
  selection rectangles, IME caret rectangles and vertical glyph navigation are
  withheld while those owners differ. The old `pages` remain available to the
  renderer, so the last good raster stays visible.
- `DocUi` records the current document instance/source generation and exposes
  `geometry_current`. Figure hit-testing and dormant-figure relaunch refuse a
  stale owner; `figure_at`/`dormant_figure_at` cannot target an old rect.
  Selection/find overlays and remote carets also stop drawing from stale
  geometry. Source-only keyboard editing is untouched.
- `FigureResizeGrab` captures the document instance and content generation at
  press time and discards a delayed commit if either changed during the drag.
  `finish_layout` only syncs figure placements and deferred preferences when
  the accepted owner is the live one; a failed newest compile sets the error
  while retaining the old pages and leaving hit-testing disabled.
- `emthin/tests/docui_async_restore.rs` now has a non-`cfg(test)` lane:
  initial hits, a multibyte edit that immediately withholds byte/caret/figure
  hits, an accepted replacement that restores them without an extra compile,
  and Open while a layout is in flight. A unit test covers the failed-compile
  path (`finish_layout(Err(..))`) retaining pixels and withholding hits.
- `cargo test --locked --workspace` exits 0: **591 passed, 0 failed, 2
  ignored**. Per binary: 408 `emthin` lib (2 ignored live-logos tests), 2
  `collab_ipc`, 2 `docui_async_restore`, 18 `xwayland_satellite`, 5
  `xwayland_satellite_watch`, 14 `emthin-clipboard`, 34 `emthin-collab`, 23
  `emthin-core-client`, 1 `scripted_peer`, 1 `unix_fixture`, 40
  `emthin-dbus`, 23 `emthin-vt`, 15 `ghostty-vt`, 5
  `herdr-gateway-fixtures`; doc-tests 0.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` exits 0.
- The Android `PadSession` path already gates retained preview work by document
  id, instance, mutation sequence and request id; no Android source changed in
  this card.

### D4.1 Restore the visible reading position

**Read first.** `docui/mod.rs` (`reading_position`, `capture_view`,
`apply_document_anchors`), `session.rs::DocumentPreferences`,
`docui/layout.rs` and Fresh's viewport `ViewAnchor`.

**Work.** Capture the first semantically readable glyph intersecting the
actual viewport and its vertical displacement from the viewport top.
Store the existing document-bound anchor with an optional displacement.
Resolve against the current accepted layout, choose the resolved page,
then apply zoom and clamped pan once. Keep page/pan fallback for old
records, figure-only pages and missing anchors. Store actual fixed and
active selection ends so a backward selection keeps its direction.
Defer restoring view state until the correct document/layout arrives.
If the user moves the view while restoration is pending, the explicit
action wins. Keep recovery anchors out of durable document edits.

**Done.** In a zoomed multi-page note, pan below the first glyph, insert
several paragraphs above the reading spot, then relayout and relaunch.
The same logical line returns near its original visible offset after
clamping. An old record still restores; an anchor for another document
falls back; a backward selection restores with the same active end.

**Result (2026-11-20).**

- `DocLayoutCache::reading_spot` returns the first rendered glyph whose line
  intersects the actual viewport, plus `band.top - doc_top` in page points.
  `DocUi` captures that byte as the existing document-bound `reading_anchor`
  and stores the displacement in `DocumentPreferences::reading_offset`
  (`Option<f64>`, absent for old records). Figure-only, stale, and missing
  geometry fall back to the first glyph on the visible page; after an edit the
  save path falls back to the caret instead of an old byte offset.
- Restore resolves the anchor against the accepted layout, selects the page
  that holds it, applies the saved zoom, places the line at the saved offset,
  and clamps pan once. Without an offset, the saved page and pan are used
  unchanged. The offset is stored in page points, so a relaunch at another
  display scale still keeps the line at the same document-relative position.
- Selection preferences now carry the explicit fixed and active ends. Capture
  reads `DocModel::selection_ends`, restore uses `set_selection_ends`, and a
  backward selection keeps the caret at its original active end.
- A user pan, zoom, reset, or page change clears `pending_preferences`, so an
  explicit view action wins over a restore waiting for the replacement layout.
  Text size is still configured immediately and the anchors are applied only
  after the worker accepts the correct pages.
- Tests cover a save/load relaunch, an edit inserted above the anchor, an old
  record without an offset, an anchor from another document, a backward
  selection, and an explicit pan cancelling a pending restore. The save/load
  test restores a page-2, 2x view and compares the visible reading offset.
- `cargo test --locked --workspace` exits 0: **599 passed, 0 failed, 2
  ignored**. Per binary: 416 `emthin` lib (2 ignored live-logos tests), 2
  `collab_ipc`, 2 `docui_async_restore`, 18 `xwayland_satellite`, 5
  `xwayland_satellite_watch`, 14 `emthin-clipboard`, 34 `emthin-collab`, 23
  `emthin-core-client`, 1 `scripted_peer`, 1 `unix_fixture`, 40
  `emthin-dbus`, 23 `emthin-vt`, 15 `ghostty-vt`, 5
  `herdr-gateway-fixtures`; doc-tests 0.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` exits 0.
- Android host `cargo test --locked` exits 0 with **100 passed**. `test-java.sh`
  exits 0: ownership 18, recovery 30, provider 16, Find 17, navigation 27,
  citation 22, inspector 9, status 12, action catalogue 48, session catalogue
  22 checks.
- D3.1 review: every exposed hit path routes through `screen_to_doc`,
  `page_for_range`, `glyphs`, `caret_rect`, `selection_rects`, `figure_at`, or
  `dormant_figure_at`; all are owner-gated. Drawing retained app/dormant/focus
  geometry is intentional because it matches the retained raster, and resize
  commits are rejected by the grab owner before `reconfigure_after_resize`.
  The save-path fallback was the only stale-offset leak and now uses the caret.
- The live compositor relaunch gap is closed (2026-11-22): `scripts/e2e/process-kill.sh`
  now moves the reading position to page 1, SIGKILLs emthin at a committed
  session-write boundary, relaunches a second process and asserts the page is
  restored (9 checks, exit 0). Writing it exposed a real bug: `save()` keyed
  preferences only by the opened document path, so a relaunch with only
  `--session-file` fell back to defaults; preferences are now mirrored under
  the session key with a regression test. The device lane was not rerun.

### D5.1 Complete return transactions

**Read first.** `docui/search.rs`, `docui/mod.rs` (`restore_view`, Find and
citation jumps), `state/mod.rs` (`goto_page_inner`, history),
`ipc/dispatch.rs` and Helix's `JumpList`.

**Work.** Use one navigation path to resolve the destination against the
current layout, apply its view, update compositor page state and emit
the page change when applicable. Use it for Find cancel/accept, citation
go-to, Back/Forward and page navigation. Use the existing geometry
invalidation path for page changes. A reveal-key change can still
legitimately schedule layout. Cancel restores origin and adds no history;
accept records the destination once. Truncate forward entries after
a new branch and deduplicate adjacent views relative to that branch.
Preserve smart-case, F3 behavior and existing IPC actions. Limit this
card to the existing literal search and jump paths.

**Done.** A cross-page Find cancel restores zoom, pan, caret and selection
including after text is inserted above the origin. Back/Forward restores
same-page and cross-page destinations, updates focus/visible figures,
and respects forward truncation. A pure page change performs zero
Typst compiles. A result that crosses a reveal boundary performs only
the needed layout.

**Result (2026-11-20).**

- `EmthinState::publish_document_page` is the single compositor destination
  path. It compares the document page to `PageState`, dismisses off-page
  popups, resets focus/IME/cursor, emits `PageChanged`, clears pointer focus,
  re-places figures, and requests a redraw. Find, citation go-to, page
  navigation, and Back/Forward all end there.
- `goto_page` now records the origin, changes the document page (geometry
  invalidation only), publishes it, then records the destination. It no longer
  uses the separate page-only early return.
- Find step and citation go-to use `DocUi::on_page_changed` instead of
  `layout.set_current_page` plus `model.mark_dirty`. A pure page change does
  not schedule Typst; only a reveal-boundary change still does.
- Find cancel restores the captured view and publishes the restored page
  without touching history. Accept still records the origin and destination
  once. Back/Forward now restore the view even when the destination is on the
  same page, including caret, selection, zoom and pan.
- `push_history` compares against the branch head rather than the vector tail.
  Re-recording that head no longer duplicates it, and a new destination after
  Back truncates the forward branch.
- Tests added: cross-page Find stepping is geometry-only and does not dirty or
  recompile; same-page history restores caret/zoom/pan; branch-head recording
  is deduplicated; a new branch clears Forward; the existing cross-page Find
  cancel/accept test still passes.
- `cargo test --locked --workspace` exits 0: **601 passed, 0 failed, 2
  ignored**. Per binary: 418 `emthin` lib (2 ignored live-logos tests), 2
  `collab_ipc`, 2 `docui_async_restore`, 18 `xwayland_satellite`, 5
  `xwayland_satellite_watch`, 14 `emthin-clipboard`, 34 `emthin-collab`, 23
  `emthin-core-client`, 1 `scripted_peer`, 1 `unix_fixture`, 40
  `emthin-dbus`, 23 `emthin-vt`, 15 `ghostty-vt`, 5
  `herdr-gateway-fixtures`; doc-tests 0.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` exits 0.
- The live compositor gap is closed (2026-11-22): `scripts/e2e/process-kill.sh`
  runs a real headless compositor + emthin, moves the reading position, SIGKILLs
  the process and verifies the page, IPC reset and recovered history on a second
  process. The device lane also ran `composing-ime` and `focus-stack` on arm64.

### D6.1 Scope collaboration to the active document

**Read first.** `crates/emthin-collab/src/{session,line}.rs`,
`state/mod.rs::collab_local_edit`, `ipc/dispatch.rs`,
`tests/collab_ipc.rs`, and the shared `MathDoc` revision/presence APIs.

**Work.** Trigger outbound exchange from the committed Loro version/frontier
or an equivalent mutation token that includes marks. Keep a remote import
from echoing back to its sender. Make Open and successful ASCII import run
the same document-scope reset. Clear or replace the ephemeral presence
store alongside delta lines, and publish an empty view/redraw immediately.
Fence relay results by active document and join generation. If the wire
needs scope to distinguish a delayed message after rejoin, add negotiated
scope and document the local IPC compatibility path. A legacy relay has to
leave and establish a fresh peer session after replacement before it can
send again. Keep presence out of persistence and continue using the
Unix transport.

**Done.** In two real isolated emthin instances, a mark-only edit converges.
After the receiver switches through both Open and ASCII import, there are
no old peers/carets and a delayed old-session update changes neither text
nor marks. Rejoining the new document works. A stale/expired presence
update cannot alter the saved snapshot or create undo entries. Keep the
existing reconnect and convergence checks.

**Result (2026-11-20).**

- `collab_local_edit` now compares the committed Loro version vector, not the
  text revision, so a mark-only edit schedules a delta. Remote import stores
  the post-import vector and remains un-echoed.
- `EmthinState::collab_reset_document_scope` is the one reset for Open and a
  successful ASCII import. It clears peers and lines, replaces the ephemeral
  presence store, advances a document-scope generation, stores the new
  version, and publishes an empty peer snapshot immediately.
- Scope-aware `collab_join`, `collab_delta` and `collab_presence` carry an
  optional `scope`. A delta tagged with another generation is ignored before
  it reaches the document or reliable line; stale presence is not merged.
  Untagged frames keep the legacy path; after replacement a legacy relay must
  leave and join the new session before sending. The local IPC compatibility
  path is documented in `docs/ipc.md` and `docs/collaboration.md`.
- `emthin-collab` tests cover a scoped old frame after replacement, the legacy
  untagged path, and stale presence. The two-instance IPC test joins with
  scopes, converges a mark-only edit, resets one side through ASCII import and
  the other through Open, verifies no old peers remain, rejects a delayed old
  frame, and checks the saved snapshot and undo availability are unchanged
  before establishing fresh peer sessions.
- `cargo test --locked --workspace` exits 0: **605 passed, 0 failed, 2
  ignored**. Per binary: 418 `emthin` lib (2 ignored live-logos tests), 3
  `collab_ipc`, 2 `docui_async_restore`, 18 `xwayland_satellite`, 5
  `xwayland_satellite_watch`, 14 `emthin-clipboard`, 37 `emthin-collab`, 23
  `emthin-core-client`, 1 `scripted_peer`, 1 `unix_fixture`, 40
  `emthin-dbus`, 23 `emthin-vt`, 15 `ghostty-vt`, 5
  `herdr-gateway-fixtures`; doc-tests 0.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` exits 0.
- Presence remains out of snapshots and undo; the transport is still Unix IPC.
  A live two-process run over real Unix control sockets is covered by
  `collab_ipc.rs` and `scripts/e2e/live-ipc.sh`. A *remote* transport run is
  K6's real-phone lane, excluded from this pass.

### D7.1 Report save and render state accurately

**Read first.** `docui/{status,keymap}.rs`, `status_bar.rs`, `input.rs`,
`ipc/dispatch.rs::ipc_list_state` and Android `StatusModel.java`.

**Work.** Derive the existing compact line and IPC status from the same
acknowledged file/recovery and layout facts. Expose file unsaved,
recovery failure/recovered copy, preview updating and preview failed as
distinct conditions. Preserve the quiet line when there is no current
message. Retain the full failure detail over IPC and use the shared
source map for attribution only if it provides a valid range in the
accepted document. Resolve action availability from the same context
used for dispatch. While a viewer or IME holds focus, preserve the
existing input routing and use the advertised action for that context.

**Done.** Drive explicit conflict followed by recovery, invalid Typst
followed by a valid edit, and a peer join under an open Find bar.
IPC and the drawn line agree about current state. A recovery success
cannot clear an explicit conflict. An action unavailable in its current
focus context cannot fire through the documented key.

**Result (2026-11-20).**

- `StatusModel` now has independent `save`, `recovery` and `preview`
  dimensions. The line can say `preview updating`, `preview failed`,
  `save failed`, `unsaved`, `recovered copy` and `recovery failed` without
  one condition overwriting another.
- `DocUi::status` derives those dimensions from the same facts the save paths
  maintain: explicit-file match/conflict, session failure/recovered warning,
  and retained-raster layout failure/pending worker. `layout.is_pending()`
  reports an outstanding production rebuild.
- `list_state` now carries `save_state`, `recovery_state`, `preview_state`,
  the full `save_error`, `recovery_error` and `layout_error`, plus the compact
  line and warning. The explicit and recovery details use their own getters,
  so an explicit conflict is not stored in the recovery slot.
- A successful recovery write no longer changes the explicit conflict state;
  the conflict test asserts this. A failed layout keeps the previous raster,
  reports `preview failed`, and leaves the file state alone.
- Action availability remains the single `keymap::available` gate in the key
  path; the catalog and dispatch use the same predicate. Existing Find-bar
  routing with peers and figure/IME focus was not changed.
- Tests added: preview-updating vs preview-failed vs save state, independent
  file/recovery failures, and status assertions after failed layout and after
  a recovery write over an explicit conflict.
- `cargo test --locked --workspace` exits 0: **607 passed, 0 failed, 2
  ignored**. Per binary: 418 `emthin` lib (2 ignored live-logos tests), 3
  `collab_ipc`, 2 `docui_async_restore`, 18 `xwayland_satellite`, 5
  `xwayland_satellite_watch`, 14 `emthin-clipboard`, 37 `emthin-collab`, 23
  `emthin-core-client`, 1 `scripted_peer`, 1 `unix_fixture`, 40
  `emthin-dbus`, 23 `emthin-vt`, 15 `ghostty-vt`, 5
  `herdr-gateway-fixtures`; doc-tests 0.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` exits 0.
- The Android pad has its own `StatusModel` and was not changed: its project
  does not run the compositor's preview worker. The live device gap is closed
  (2026-11-22): `device-lane.sh composing-ime` and `device-lane.sh focus-stack`
  both pass on the arm64 Waydroid target — the composing IME's CJK/emoji commits
  reach the editor model, and the viewer/source-inspector focus stack returns
  correctly with Escape/Back.

### D3.2 Measure and bound page work

**Read first.** `docui/layout.rs::run_layout`,
`../velysterm/crates/mathed_mini/src/render.rs` and
`emthin-android/src/layout.rs` (retained Typst frames, lazy page work).

**Work.** First record separate compile, glyph extraction, raster,
publication/apply and slow-tick measurements on the same proof note.
Today `layout_compile_ms` covers all phases up to publication and
`layout_raster_ms` is zero. If all-page raster or memory is significant,
split shared compilation from materialization using current mathed
frames. Retain one compiled generation on the existing worker. Publish
page sizes and figure geometry, then raster the requested visible page
first and at most one adjacent page as a prefetch. Keep enough source
mapping to locate a jump without rastering the destination in advance.
Bound cached raster bytes and reject results for obsolete page/document
requests. Use the desktop and Android test lanes if shared engine
methods change.

**Done.** Record p50/p95 and peak RSS before and after on identical long
document edits, with the host and dependency pins. Initial visible-page
work does not scale with the number of offscreen rasters. Page turns
inside the retained generation do not recompile. Ordinary caret moves
compile zero times and an outdated page cannot be clicked. A layout error
preserves the last good view. If measurements show the retained worker
already meets the identified need, keep the instrumentation and skip
the additional cache work.

**Result (2026-11-20).**

- `mathed_mini::RetainedPaged` compiles once, keeps the Typst paged frames,
  source map and per-page glyph/figure skeleton, and materializes one page's
  raster on demand. `PageLayout::release_raster` drops pixels without losing
  page size, glyphs or figures.
- The desktop layout worker now owns the retained generation. A compile
  request returns all page metadata plus only the visible page raster; a page
  request materializes another page without recompiling. `DocUi` requests the
  visible page after a page change, and `DocLayoutCache::poll_page` installs
  it and releases every other resident page raster.
- `AppliedLayout` and the `layout_worker` timing log now carry separate
  `layout_compile_ms`, `layout_glyph_ms`, `layout_raster_ms` and
  `layout_apply_ms`; `layout_raster_ms` is no longer a placeholder.
- Measurements on a 1,200-line (97-page) note, same host, rustc 1.98.1
  (aarch64, 12 cores), workspace `Cargo.lock`:
  - all-page layout: p50 **739.1 ms**, p95 **870.4 ms**, peak RSS
    **415,652 KiB**;
  - retained compile + all-page glyphs + visible raster: p50 **561.6 ms**,
    p95 **570.3 ms**, peak RSS **31,664 KiB**; compile p50 2.1 ms, glyphs
    p50 556.7 ms, visible raster p50 2.7 ms.
  This is a 1.31x p50 latency improvement and 13.1x peak-RSS reduction. The
  remaining cost is all-page glyph extraction, retained because a jump must
  locate a source range without rastering the destination.
- Worker tests assert that only the requested page has pixels, a later page
  materializes from the retained generation, and the page request does not
  advance the compile generation. `RetainedPaged` has a shared-engine test
  for skeleton size and one-page materialization. The ignored measurement
  tests record p50/p95 and peak RSS and are run explicitly.
- Existing tests continue to cover ordinary caret moves compiling zero times,
  rejected hit geometry while an owner is stale, and retaining the last good
  view on a layout error.
- `cargo test --locked --workspace` exits 0: **608 passed, 0 failed, 2
  ignored**. Per binary: 419 `emthin` lib (2 ignored live-logos tests), 3
  `collab_ipc`, 2 `docui_async_restore`, 18 `xwayland_satellite`, 5
  `xwayland_satellite_watch`, 14 `emthin-clipboard`, 37 `emthin-collab`, 23
  `emthin-core-client`, 1 `scripted_peer`, 1 `unix_fixture`, 40
  `emthin-dbus`, 23 `emthin-vt`, 15 `ghostty-vt`, 5
  `herdr-gateway-fixtures`; doc-tests 0.
- Shared engine: `cargo test -p mathed_core -p mathed_mini` exits 0 with 580
  passed, 0 failed and 4 ignored (three performance measurements plus the
  existing ignored case). `cargo clippy -p mathed_mini --all-targets -- -D
  warnings` is clean. The Android host lane `cargo test --locked` passes 100.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` exits 0.
- No adjacent-page prefetch is performed yet: the card allows "at most one"
  and the retained page is materialized only when requested. The pre-existing
  sibling test fixture in `mathed_mini` needed its `FigureSpec.terminal` and
  `binding_error` fields filled in to compile; that was outside the card's
  product behavior.

## Execution run (2026-11-23)

Every card above is recorded done, so this run executed the plan's own release
checks against the current tree and fixed what they exposed. Source:
`main` at `14b3740` plus this session's uncommitted changes. Commands ran
inside `nix develop`, with `CARGO_BUILD_JOBS=2` after the recorded OOM history.

| Check | Result |
|---|---|
| `cargo test --locked --workspace --offline` | **736 passed, 0 failed, 2 ignored** (`emthin` lib 511, `collab_ipc` 6, `docui_async_restore` 2, 18/5 xwayland, 14 clipboard, 66 collab, 23 core-client, 2 scripted-peer/unix-fixture, 40 dbus, 23 emthin-vt, 15 ghostty-vt, 5 herdr fixtures) |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | exit 0 |
| `RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps --document-private-items` | exit 0 |
| `cargo fmt --all --check` | green for every package in this checkout; the remaining diffs are the external `velysterm`/`unfer` path dependencies, which carry their own edition-2024 formatting |
| `bash scripts/verify-invariants` | 7 passed, 0 failed, 0 gaps (includes `check-ipc-docsync`, 48 client→server / 36 server→client) |
| `python3 scripts/check-links` | OK — 35 documents, 87 relative links |
| `cd emthin-android && cargo test --locked --offline` | **136 passed, 0 failed** |
| `bash scripts/agent/replay.sh scripts/agent/scenarios/figure-resize.txt` | 13 assertions ok, 0 failed |
| `bash scripts/agent/replay.sh scripts/agent/scenarios/proof-note.txt` | 17 assertions ok, 0 failed |
| `bash scripts/e2e/live-ipc.sh` | every check passed (inspect, history, proof, semantic rename, kernel/launcher/plugin/atproto descriptors) |
| `bash scripts/e2e/process-kill.sh` | every check passed (committed write boundary, SIGKILL without graceful shutdown, relaunch restores the verified pair and the page) |
| `bash scripts/e2e/run.sh --config wtype` | **28 ran, 0 skipped, 0 failed** — clipboard both directions plus keyboard, doc-copy paste and PgUp/PgDn, on the harness-owned headless sway |
| `bash scripts/e2e/run.sh --config xdotool` | **36 ran, 0 skipped, 0 failed** — the same clipboard/keyboard/navigation checks over XTEST plus Find (open, 1/1, F3 reveal, Escape restores the pre-search page) and the fcitx5 IME commit of 你好 with its preedit logged, on the harness's own Xvfb + sway (wlroots x11 backend) |

Fixed while running these: `scripts/e2e/lib.sh` referred to `stop_compositor`
and `cleanup` from the `live-ipc.sh`, `live-mm0.sh` and `process-kill.sh` exit
traps, but defined neither, and the traps' `2>/dev/null || true` hid it. A run
left its headless sway holding the harness's stdout, so a piped invocation
(`... | tail`) never terminated after the checks had passed, and the next run
inherited a compositor it did not create. Both functions now live in the
shared library: `stop_compositor` is scoped by the harness config path and the
private display, so a host session's own sway/Xvfb is never touched, and
`cleanup` is `stop_ime` + `stop_compositor`. `run.sh` keeps its own fuller
`cleanup`. Re-running `live-ipc.sh` piped now exits 0 and leaves no sway.

Both desktop E2E lanes ran, so the 2026-11-18 pairing (28 Wayland + 36
XTEST) is reproduced end to end on this host. The XTEST lane does **not** need
a free logind session: `--config xdotool` boots its own Xvfb and sway with
`WLR_BACKENDS=x11` (`scripts/e2e/lib.sh::start_compositor`), and only the
Mutter configurations ask logind for a session. Both runs left no Xvfb or
sway behind (`pgrep -x Xvfb`/`pgrep -x sway` empty afterwards), which is the
`stop_compositor` fix above earning its keep.

The device and service lanes then ran too — see the sections below. That also
closes the `terminal-lease` gap the 2026-11-22 record named, and the
relationship-pane sizing question it left implicit.

### Device lane (2026-11-23)

`adb` is on this host after all: `android-tools 37.0.0p1` ships it in the nix
store (`/nix/store/…-android-tools-37.0.0p1/bin/adb`), it just is not on the
default PATH. The Waydroid session was started (`waydroid session start`; the
container service was already active) and the pad's key was already seeded in
`data/misc/adb/adb_keys`, so `adb connect 192.168.240.112:5555` came up as
`device` immediately. Device: `lineage_waydroid_arm64_only`, aarch64 host,
2943x1638 panel, disposable test package `com.emthin.pad.test`.

`emthin-android/device-lane.sh e2e` (arm64 APK built by `build-apk.sh` through
the `.toolchain/` box64 shims, installed, cold-launched) finished with **215
checks passed, 0 failed**, and the installed APK md5-matched the build. That
covers persistence, new-reset, scroll-anchor, search-replace, focus-stack,
composing-ime (CJK + emoji), accessibility, diagnostics, named-document,
modifier-inspector, session-pin, figure-a11y, terminal-scene, terminal-reflow,
terminal-pages and terminal-lease. The pad's host lane still passes (136
tests, run through `nix develop`).

The first run failed twice; both causes are identified and fixed here:

- **`modifier-inspector` (fixed).** The relationship list is its own scroll
  pane, and the lane swiped at a fixed `1400 850`, which on this tablet is
  *below* that pane (`[34,604][2909,826]`) — the drag scrolled the document and
  the two outgoing rows never came into the hierarchy. The lane now reads the
  scrollable box around the "Outgoing targets"/"Incoming modifiers" headings
  out of the uiautomator dump (`relationship_drag`) and drags inside it, keeping
  the geometry for later scrolls once the headings themselves scroll away
  (`relationship_scroll`). Step re-run alone: all steps passed, and the full
  e2e then passed this step too.
- **`terminal-lease` (fixed — the install's uid, not the transport).** The step
  failed with `gateway watch attach did not become ready`: the app logged
  `gatewayLease phase connect host 127.0.0.1 port 47654` and then
  `gatewayLease error kind transport` exactly 4 s later, its Java connect
  timeout (`GatewayLeaseClient.java`: `connect(…, 4000)`). The transport was
  never at fault. Reading the app's own socket table during the attempt showed
  a `SYN_SENT` entry for uid 10067 with TCP retransmits (`01:00000026`,
  `02`, `03` in the timer column) to the host and to `1.1.1.1:443`, while the
  host-side listener accepted nothing — the container dropped every packet
  from that uid. The same host port was reachable from the shell uid 2000 over
  loopback and the waydroid0 bridge, and `org.lineageos.jelly` (uid 10097)
  loaded `http://192.168.240.1:47654/` fine, so it was not ADB reverse, the
  bridge, or the fixture. A `pm uninstall` + fresh `adb install` gave the
  package a new uid (10075) and its egress worked immediately; the step then
  passed in full (watch attach → DATA → Take → controller input → operator
  revoke → fresh lease). The stale uid's network registration is a container
  property, not pad code — no root on this image to inspect `netd`'s firewall
  state, which is why the diagnosis stops at "that uid's packets never leave".
- **Harness bug found while reproducing it.** With the fixture's port already
  bound, `start_lease_fixture` printed `lease fixture exited: …` and returned
  non-zero, but `cmd_terminal_lease` only `return`ed — the lane reported *all
  steps passed* with zero checks run. It now records a `fail` for that case.

### Relationship pane sizing on a tall tablet (2026-11-23)

Measured rather than assumed, because the pane being *reachable* by scrolling
is not the same as it being *sized* sensibly. On the 2943x1638 panel (density
270, i.e. 971 dp tall) the inspector opens with the source field focused, so
the IME takes ~690 px and the app content is 698 px. Inside that, the preview
kept its weighted share (183 px) and the relationship `ScrollView` got
`[34,604][2909,826]` — 222 px, 132 dp, about **two of the five rows**, with the
rest behind the scroll the lane had learned to do.

Fix: a user-intent open (`openSourceAt`, i.e. the toolbar's Edit, and the
fixture path the lane uses) hides the preview while the inspector is up, and
`closeInspector` / the not-open branch restore it. The pane then measured
`[34,421][2909,826]` — 405 px, 240 dp, ~4 rows — and “Show document” brought
the preview back at full height. The first attempt hid the preview in
`updateModifierPanel`, which also fires for a **cold start that restores
`inspectorOpen`** (`showMarkers` pref): the hidden preview was never laid out,
so `restorePreviewScroll` landed at 0 instead of the saved 5087 and
`persistence`, `scroll-anchor` and `terminal-scene` failed. That is why the
hide lives on the open paths instead. Verified: those three steps plus
`modifier-inspector` pass individually, then the whole
`device-lane.sh e2e` run — **215 checks, 0 failed** — with the installed APK
md5-matching the build.

### WSS transport and paste pairing for T14 (2026-11-23)

The pad's gateway connection was plain TCP to the scripted fixture, which is
why T14 read as a gap: no TLS, no endpoint registry, no pairing parser. That
is now implemented in plain Java (no new dependency) behind one seam,
`GatewayTransport` — a byte stream in, one emthin frame per `send` — so
`GatewayLeaseClient`'s framing, lease and revocation logic is unchanged by
which transport is underneath.

- **Transport.** `WssTransport` upgrades `GET /v1/stream` with subprotocol
  `emthin-terminal.v1` over an `SSLSocket` whose `SSLParameters` carry the
  `HTTPS` endpoint-identification algorithm (so hostname checks are the
  platform's), then frames with `WebSocketFrame` (masked binary only, 64 KiB
  cap, fragmented messages reassembled, ping answered, 20 s idle ping, two
  silent periods end the stream). `WebSocketHandshake` fails closed on any
  status other than 101, a redirect, a repeated header, a mismatched accept
  key, a different or missing subprotocol, or an unsolicited extension.
- **Pairing.** `PairingLink` parses the pasted QR text with the contract's
  rules (https, `<host>.<tailnet>.ts.net`, port 443, exact `/pair`, exactly `v`
  and `core`, no credentials/fragment/IP literal, 1 KiB cap) and deliberately
  trims surrounding whitespace but rejects embedded control characters, because
  pasting from a chat app appends a newline. `PairingProtocol` maps every
  status the contract names to a state — pending, approved, denied, expired,
  forbidden, core mismatch, unavailable — and calls an approval for another
  Core ID a mismatch rather than a success. `GatewayEndpoints` keeps at most 16
  rows and revalidates each on load, so a hand-edited registry cannot smuggle a
  non-tailnet origin into the connection path.
- **Fixture.** `fixtures/herdr-gateway-v1/wss_fixture.py` reuses
  `lease_fixture.Gateway` verbatim by handing it a socket-shaped object whose
  `recv`/`sendall` ride WebSocket binary messages, so there is still exactly
  one implementation of the protocol and only the transport varies.
- **Evidence (host).** `emthin-android/test-java.sh` now runs
  `GatewayPairingTest` (126 checks) and `WebSocketTest` (51 checks, including a
  real TLS round trip against an in-process server and the plain-HTTP,
  unknown-certificate and wrong-hostname failures).
  `emthin-android/wss-lane-check.sh` generates a certificate, starts the WSS
  fixture and drives it with the pad's own transport (`WssFixtureProbe`, 9
  checks: upgrade → hello → control attach → lease → 80×24 → banner). The whole
  app source also compiles clean against `platforms/android-37.0/android.jar`,
  which is what makes the `MainActivity` wiring (`wss://host:port` plus a
  fixture-only CA file under the app's `files/`) typecheck.
- **Device step.** `./device-lane.sh gateway-wss` (opt-in as `LANE_WSS=1`)
  now runs; see the 2026-11-23 record below for the linker bug that had blocked
  the APK rebuild and for the measured result.

### Clean-install rerun of the device lane (2026-11-23)

The 2026-11-23 lane record above was produced on an installed package. Because
`terminal_lease_check` can now uninstall and reinstall to cure a dead uid, the
lane was re-run from a clean install: `pm uninstall com.emthin.pad.test`, then
`SKIP_BUILD=1 ./device-lane.sh e2e` (no source or Java file newer than the built
APK, so `SKIP_BUILD` is sound evidence for the lease change). Result: **215
checks, 0 failed**, `device lane: all steps passed`, and the installed
`base.apk` md5-matched the build
(`42a01ae938bf4a53b3cb669fa97bb067`). The normal path did not need the
reinstall branch; the retry remains as the documented cure for a stale uid.

### APK link repair, pairing screen, and the WSS device lane (2026-11-23)

Two blockers stood between the Java WSS/pairing sources and a device verdict.

- **The link step could not run on an aarch64 host.** `setup-box64-toolchain.sh`
  shims the SDK's x86_64 `clang` through box64, which also put the NDK's
  `ld.lld` under box64; that binary aborts (`free(): invalid next size (normal)`,
  `ld.lld+0x34d45b5`, `clang-21: error: unable to execute command: Aborted`)
  while linking `emthin_pad`. Fix: the clang/clang++ shims pass `--ld-path` to a
  **native aarch64** `lld` (nixpkgs 21.1.8) resolved by setup and recorded in
  `.toolchain/env.sh` as `EMTHIN_NATIVE_LLD`; the emulated driver still compiles,
  only the link leaves box64. Setup now also finishes with a real `-shared`
  link, so a broken `--ld-path` fails there rather than mid-build. Because a
  nixpkgs weekly rename otherwise forces building a new SDK/glibc just to
  regenerate four shims, setup reuses an existing `ANDROID_SDK_ROOT`,
  `JAVA_HOME`, `EMTHIN_BOX64`, `EMTHIN_NATIVE_LLD`, and keeps the existing
  x86_64 runtime farm when the freshly resolved
  glibc/libcxx/libgcc/zlib paths are unbuilt. `./build-apk.sh` then completed all
  six steps: `OK: build/emthin-pad.apk`, 28,750,309 bytes, debug-signed,
  arm64-v8a.
- **The fixture certificate could not be delivered to the app directory.** The
  lane pushed `fixture-ca.pem` with `adb shell run-as`, which this container
  refuses (`run-as: setegid(AID_PACKAGE_INFO) failed: Operation not permitted`:
  the shell has no `CAP_SETGID`), and `/sdcard/Android/data` is denied to the
  shell too. Fix: the PEM rides the launch intent as
  `emthin.fixtureGatewayCaPem` (base64) and `MainActivity` writes it into
  `getFilesDir()` itself. The certificate now also names `IP:127.0.0.1`, because
  the pad dials the reversed port by address and the platform's HTTPS endpoint
  identification rejected a `localhost`-only SAN (`gatewayLease error kind
  transport` 17 ms after connect). The test manifest marks the package
  `debuggable`, which is correct for a test APK even though `run-as` is unusable
  here.
- **Pairing screen.** `ActionCatalog` gained `PAIR` ("Pair gateway") in the
  sessions menu; `MainActivity` gained `showPairing()` — the registry list
  (`No gateways paired yet.` when empty, each row with its tailnet origin and a
  Forget action), a `PASTE LINK` step that parses with `PairingLink` and starts
  `PairingProtocol` on a worker that posts back to the main thread, a live
  status line for pending/approved/denied/expired, and a `GatewayEndpoints`
  store backed by `SharedPreferences` (`gatewayEndpoints`). Only an approved
  answer for the pasted Core ID is stored; Forget removes the local row and does
  not revoke on the host. No lease path consumes the registry yet — the lane
  fixture still passes `emthin.fixtureGateway` directly.
- **Verification.** `bash test-java.sh`: all suites pass, including
  `ActionCatalogTest` (65 checks) and `GatewayPairingTest` (126) /
  `WebSocketTest` (51). On device, the pairing dialog was driven by
  `uiautomator`: `Pair a gateway`, the descriptor, `No gateways paired yet.`,
  `CLOSE`/`PASTE LINK`, and the paste dialog's hint plus `PASTE`/`CANCEL`/`PAIR`
  all render. Then `LANE_WSS=1 ./device-lane.sh e2e` from a fresh build: **220
  checks, 0 failed**, `device lane: all steps passed`, installed `base.apk`
  md5-matching the build (`2f96af48d82c49a7616e2a7916474334`). The `gateway-wss`
  step's logcat shows `gatewayLease phase connect ... secure true`,
  `gatewayLease resize cols 80 rows 24`, and `gatewayLease data bytes 41`
  ending in `emthin gateway lease |t7 watch |$ ` — TLS handshake, `/v1/stream`
  upgrade, watch attach, grid, and DATA all reached the native scene. T14's
  device step is therefore verified; the remaining T14/T15 items (a real Go
  gateway, tailnet, physical phone, OpenShell) are unchanged.

### Live MM0 verifier and Zenodo sandbox (2026-11-23)

- **`scripts/e2e/live-mm0.sh` — passed.** The missing piece was only the
  fixture path: the Peano pair lives in `../mm0-rs/examples/`, and both
  checkers are installed (`mm0-rs`, `mm0-c` on PATH). Run as
  `MM0_DIR=…/mm0-rs/examples MM0WEB_BIN=…/unfer/target/debug/mm0web`:
  mm0web served on loopback, accepted the real Peano proof through mm0-rs +
  mm0-c, minted `Peano-live-66bfb078`, and emthin's `proof_query` for a
  document citing it through `proof:` reported the backend verdict.
  Fixed while running it: `STATE="$(mktemp -d /tmp/opencode/mm0web-live-XXXXXX)"`
  failed because `/tmp/opencode` need not exist, and the empty `STATE` then made
  the audit-trail path `/events.jsonl` — the lane reported "no verified proof
  id" after the checker had already accepted the proof. It now uses
  `${TMPDIR:-/tmp}`.
- **Live Zenodo sandbox checkpoint — passed.** Credential read from the
  git-ignored `.e2e/api` (never echoed or logged), sandbox host explicitly
  selected as `MM0WEB_ZENODO_BASE=https://sandbox.zenodo.org/api`, synthetic
  11-byte `sort Prop;` spec/proof pair (sha256 `acd539decfd1992b…`).
  `/healthz` reported the sandbox host with both checkers ready; the pair
  verified locally; one explicit publish queued proof
  `emthin-sandbox-checkpoint-5a720607` and settled to `state: published`, DOI
  `10.5072/zenodo.647585`. The draft's returned `mm0`/`mm1` were byte-identical
  to the local fixture, and a fresh `GET /proof` still reported published.
  Cleanup was not requested, so the draft was left in place. Publication and
  versioning coverage remain on the mock; this lane verifies and uploads one
  draft only.

## Implementation handoff

Use the cheaper model for one card at a time. Give it the card's files,
the relevant part of the design constraints and the applicable test lane.
Ask it to record changed code, observed results and any authority gaps.
Review the save and collaboration ownership contracts before changing
their format or adopting a library. Leave unrelated working-tree changes
untouched. Keep Herdr and Android fixes in their existing cards when
the root plan points there.

The first product card after the build repair is D2.1. A focused handoff
can use the named-file recovery case from the MM0 test note and drive the
existing status and save APIs. D3.1 can be prepared independently with
the retained-layout acceptance case. Each card leaves a checkable change
and a regression test for its specific failure path.

## Development testing plan

Use three levels of evidence. Fast checks run with every change. A
disposable document and companion app test the real user workflow at each
checkpoint. A separately configured Zenodo sandbox run checks the live
service only after the offline workflow works.

| Level | Use | Required evidence |
|---|---|---|
| Per card | Run focused Rust or Android host tests, repository invariants and affected doc/protocol checks. Run workspace tests, fmt, clippy and rustdoc before desktop review. Shared engine changes run the Android host lane too. | Commands, exact source/build identity, pass/fail totals and named gaps. Assertions use observable output rather than implementation mirroring. |
| Disposable integration | Extend [agent replay](docs/AGENT_LOOP.md), [desktop E2E](scripts/e2e/README.md) and `emthin-android/device-lane.sh` on `com.emthin.pad.test`. Use local stubbed services and real input/persistence where the seam matters. | Proof note retained, document geometry and app focus correct, no unexpected disk write, and cleanup ownership proved. |
| Checkpoint device and service | Real arm64 phone for the MM0 client flow, nested desktop for live app figures, real Zenodo sandbox for approved draft-only smoke. Use the [Herdr acceptance run](docs/HERDR_ACCEPTANCE_RUN.md) when a proof workload runs in an existing terminal figure. | APK hash, app/runtime, test fixture identifiers, status/byte evidence, and explicit gaps for unavailable device or sandbox. |

### Editor release checks

- Run `emthin-android/device-lane.sh math-preview` with Python Pillow and the
  disposable APK. Compare the on-screen equation and prose against their
  accessibility bounds. Run the existing caret recovery, reading position,
  Find/Undo, CJK composition, and terminal reflow checks on the same build.
- On a physical arm64 phone, repeat the preview check in portrait and landscape.
  Tap the equation, edit an exponent with the software keyboard, undo the
  edit, and reopen the note. Confirm the same glyphs, caret and reading place.
  Use the MM0 fixture below to repeat with an actual proof note.
- Build the distribution APK with the release signer and test upgrade from the
  previous signed build using a disposable note. Check that launch fixtures
  and the test IME provider are absent from the release manifest.
- Extend desktop input composition through `state/ime.rs` and `winit.rs`.
  The document needs a host IME owner when the editor has focus. Use the
  document's screen-space caret for the candidate popup. Paint preedit as
  an overlay and commit through one checked replacement in `DocModel`.
  Clear preedit on focus changes, document switches and cancellation. Extend
  the fcitx E2E to commit CJK into the document, undo once, then focus an
  embedded client and repeat to prove routing.
- On a memory-constrained build host, run links serially with
  `CARGO_BUILD_JOBS=1`. The 2026-11-22 kernel log recorded two system-wide
  OOM kills of `ld.gold` during concurrent desktop links. The serialized
  workspace run completed with the same source.
- ~~Repeat `device-lane.sh terminal-lease` before passing the Herdr integration
  checkpoint.~~ **Done 2026-11-23**: the socket timeout was a stale install uid
  whose packets never left the container; see `### Device lane (2026-11-23)`.
  The checkpoint itself is still open for the reasons recorded in
  `docs/HERDR_INTEGRATION_PLAN.md` §Checkpoint 3 status (no Go gateway,
  Tailscale pairing, WSS transport or OpenShell fleet in this checkout).

### MM0 proof workflow fixture

The proof UI is a companion workload. Its backend owns proof verification
and upload. emthin owns the document page, figure, text editing, file
persistence, input and navigation under test.

The checked backend is `../unfer/mm0web` at `b0aaa7c9220a`.
Read `deploy/README.md`, `tests/e2e.sh`, `tests/mock_zenodo.py`,
and `src/{main,web,store}.rs` there. The backend runs `mm0-rs compile`
and `mm0-c`. Its local E2E exercises successful verification, failed
publication retry, restart/resume and duplicate submission against a
mock Zenodo base. Reuse that mock and its server counters. Check
`/healthz` for both verifier binaries before claiming a verified proof.
The older E2E has an absolute `/tmp/mm0` reference for the larger
Peano fixture. Resolve that fixture from the pinned checkout or run
the small cases that do not require it.

The Android proof client has yet to be created. Start with P0a and keep P0b
as a separate, small implementation task. Browser results count as browser
coverage. Native cross-app file and lifecycle cases use the companion.

- **P0a. Reuse the proof web UI.** Run `mm0web` against its existing
  mock in scratch state. Mount its HTML form, status, search and proof
  detail in a normal desktop browser figure. Run the same pages in an
  Android browser if available. The preflight succeeds when both verifiers
  are ready and the mock can count a test publication.
- **P0b. Build the native Android test companion.** Give a cheaper model
  a single-Activity test APK outside the shipping pad. Use the platform
  document picker for one `.mm0` and one `.mm1`, the existing
  `/api/v1/verify` and `/api/v1/publish` routes, and the returned status
  or proof lookup. Require an explicit action to submit to a configured
  mock-backed test server. Use a local test endpoint with disposable
  state and no upstream credentials. Store the pending proof/project ID locally so
  recreation can reload status. Expose the proof ID and status link through
  the Android share sheet or clipboard. Bind results to the captured file
  pair and Activity to drop stale UI work. Reuse the backend and mock;
  implement only the native entry points needed for the file, sharing and
  lifecycle tests. Keep verification and publication in the backend.

P0b is done when the disposable phone lane selects both fixture files,
verifies, submits once to the mock-backed service, rotates and relaunches
to the same proof status, and returns the ID/link to emthin through the
existing handoff. Record the package ID, APK hash, backend revision and
mock run ID.

Prepare one disposable two-page mathed proof note. Use synthetic content
with a claim heading, a Unicode formula, marker/markup ranges, a known
search match, a mark-only edit, and space for a proof ID and result link.
On desktop include a normal `\app` figure bound to the browser running
the proof UI. Set its geometry in the document. Use existing open/copy/
paste/save flows between the proof app and pad on Android. A share
action, when available, or the Android document picker transports a
local copy for the test. Keep the raw `.mm0` / `.mm1` pair separate from
the `.mathed` note. Begin with the known-good `sort Prop;` spec/proof
pair and the mismatched proof used by `mm0web/tests/e2e.sh`. Add the
larger proof only for search and performance runs.

| Case | Action | Expected evidence |
|---|---|---|
| P1 Verify and annotate | Verify the valid and invalid pairs in the proof UI. Queue the valid pair against the mock, copy the resulting proof ID/link into the note and save a snapshot. | The checker accepts the valid pair and reports a stage for the invalid one. The note reopens with exact text, marks and reference. An invalid submission makes no upload. |
| P2 Save versus recovery | Edit the opened note, let recovery autosave, then force-stop the disposable app or restart the desktop fixture. Compare the named file before and after. Simulate an outside file change and attempt Save. | Recovery contains the edit. Named file remains explicitly unsaved until the user writes it. External conflict preserves both versions. Test Save Copy and round-trip its bytes and marks. Covers D2.1–D2.2. |
| P3 Figure and input | Focus the live proof form in the desktop Wayland figure, type and paste, then return to the document. Resize the figure through its statement. In the pad, switch to the companion app and back, using the real composing IME in the note. | Input reaches the intended editor/form, not both. Geometry equals the accepted Typst rectangle. Unsupported or absent figure stays a placeholder. Returning restores the note caret and reading position. |
| P4 Navigation during layout | From page 2 at non-default zoom, search a claim, cancel, accept another hit, then Back/Forward. Keep output active or edit above the note while a replacement is pending. | Cancel restores the source place, pan and selection. Stale glyph and figure coordinates cannot edit or launch a different target. Ordinary view navigation performs no needless compile. Covers D3.1, D4.1 and D5.1. |
| P5 Retry and restart | With mock Zenodo, start one publication, inject the existing failure or stop only the fixture backend after upload, then retry or restart. Meanwhile rotate/background Android or switch desktop documents. | One logical proof record is recovered, byte hashes agree, and completion cannot modify a newer note or render status in a recycled view. Backend and mock logs record actual attempts. |
| P6 Peer and recovery boundary | Relay the proof note between two isolated desktop instances, change a mark, then switch one through Open and ASCII import while the first keeps editing. Kill/restart at an existing session-write boundary in a scratch session. | Mark-only convergence, no cross-document replay or presence, and the last verified recovery pair survives. Use a deterministic write-boundary signal for a process kill, or report that only storage-boundary tests ran. |
| P7 Performance | Use identical repeated proof-note fixtures at short and long sizes, changing titles, paging, panning and typing. Run with the live proof UI held constant. | Measure edit to first accepted preview, p50/p95 layout/apply times, slow event-loop ticks, raster bytes and peak RSS. Record compile counts for caret, page and content changes separately. This is D3.2's workload. |

P1–P5 exercise emthin through the test application's real workflow.
Run backend-only retry and validator checks as preflight evidence.
Proof verification itself is owned by the companion backend and does
not imply that Android mathed executes proofs.

### Live sandbox checkpoint

Run only with a separately configured test account, the sandbox HTTPS
host explicitly selected, and a synthetic fixture. Keep credentials in
the companion service's existing private configuration. Use a fresh
test draft and an explicit one-time upload action. Compare local file
size/hash with the draft's returned file data or a downloaded copy,
then test that the app can revisit the draft and read its status.
Delete only the draft created for this run when cleanup is part of the
chosen checkpoint. Keep publication and versioning coverage on the
mock, unless a separate test explicitly needs the live publication
path. The existing `mm0web` publishing workflow queues a real publish,
so use it with the mock for the default proof run. A client that can leave
the upload in draft is a prerequisite for the live lane. Sandbox
registrations and tokens are separate and sandbox data may
be cleared. [Zenodo developer testing guide](https://developers.zenodo.org/#testing).

### What to record

For every run capture the card, case ID, Git revisions or APK hashes,
fixture hash, test backend mode, observed result and any missing
requirement. Distinguish storage, layout, compositor, Android and
backend coverage. Make assertions with the existing IPC/state and
semantic previews first. Capture a screenshot only for a visual
question such as figure placement, focus or clipping. Archive redacted
fixture logs outside the user document and clean up only processes,
drafts and app data created for the test.

### Result (2026-11-21) — offline + backend lanes

- **Fixture**: `scripts/agent/proof-note.typ` is a synthetic two-page proof
  note with one dormant `\app` figure (`proof-ui`, launch `firefox`), a
  Unicode formula, marker ranges, one `\def`, one `\model`, one `\formal`,
  proof-ID/result-link placeholders and a repeated search match. It lays out
  to 2 pages with `diag == 0`, 1 figure, 1 definition, 1 model, 2 unresolved
  identifiers; the PNG page raster renders without Typst errors. No raw
  `.mm0`/`.mm1` content or credentials are embedded.
- **P1/P2/P4 (document-side)**: `scripts/agent/scenarios/proof-note.txt`
  plus two patches (rewrite the definition literal, copy a proof ID into the
  note) pass `scripts/agent/replay.sh`: **17 ok, 0 failed**. The transcript's
  `{delta}` line records the definition change while pagination and figure
  geometry stay put. This is a semantic/boundary assertion, not a verifier
  verdict, named-file Save/recovery or live compositor navigation.
- **P0a (backend preflight)**: `../unfer/mm0web` at
  `b0aaa7c9220ae896985a4017cbc8e42a9e5f0d86` (working tree dirty with
  unrelated review edits) built with `cargo build -p mm0web --offline`.
  Both verifiers are installed (`mm0-rs 0.1.0`, `mm0-c`, aarch64 native).
  `mm0web/tests/e2e.sh 1` passed **27/27**: `/healthz` 200, valid proof
  accepted, mismatched proof rejected with a named stage, publish queued and
  reached `published` with DOI, idempotent re-publish (one deposit), mock
  counters `{deposits:1,newversions:1,publishes:2,uploads:4}`. The larger
  Peano case was resolved from the pinned checkout
  (`logos/mm0/upstream/peano.mm{0,1}`) via `/tmp/mm0/examples` symlinks
  instead of skipping it.
- **P5 (retry/restart preflight)**: `mm0web/tests/e2e.sh 3 5` passed
  **9/9**: killed mid-upload resumes the same deposit with no orphan, and a
  no-key backend still verifies/stores while publish returns 502.
- **P0b (native test companion)**: `ProofClientActivity` added to the test
  APK (`android/test-src`, declared in `test-manifest.xml`). It picks a
  `.mm0`/`.mm1` pair with persistable URI permissions, posts the existing
  `/api/v1/verify` and `/api/v1/publish` form routes to a configurable local
  base, persists the proof ID/status across recreation, binds results to the
  captured pair and an Activity generation, and hands the ID/link back via
  the share sheet and clipboard. Built
  `build/test/emthin-pad-test.apk` (28,684,848 bytes, arm64) with
  `EMTHIN_TEST_PACKAGE=com.emthin.pad.test`; `javac`/`d8`/`aapt2`/sign all
  succeeded. The Rust crate must be built with the rustup 1.99 toolchain
  (`PATH=/usr/bin:$PATH` inside the devshell) because the nix rustc 1.98.1
  lacks the Android target.
- **Native proof UI in the shipping pad (closed)**: `ProofClientActivity` is
  now in `android/src` (not just the test APK), declared in the shipping
  manifest, reachable from the toolbar via `ActionCatalog.Id.PROOF`, and
  returns the proof ID/link through `startActivityForResult` to
  `MainActivity.insertProofResult`, which inserts them as one grouped edit at
  the caret. The devshell's `adb`/`jdk21` made the device lane real: with
  Waydroid running (`arm64-v8a`, adb at `192.168.240.112:5555`) and a local
  `mm0web` reachable at `192.168.240.1:18919`, the native flow was driven on
  device: toolbar button → Verify (`verified`) → Publish (`submitted`, proof
  `sort-Prop-89561941`) → Insert into note. The pad's own log recorded
  `proof inserted sort-Prop-89561941` and its UI showed both the ID and the
  result link in the note. Screenshot: `/tmp/opencode/final-pad.png`. The
  Activity also exposes scriptable base-URL/fixture intent extras and an
  in-UI server field for deterministic device-lane runs; no host or
  credential is hardcoded.
- **P0a browser coverage (closed)**: the devshell now provides `chromium`,
  `python3Packages.playwright` and `jdk21` (flake check passes). With a
  local-only `mm0web` on 127.0.0.1:8082 and one seeded proof, a headless
  Chromium driven by Playwright loaded all four proof UI pages with HTTP 200
  and asserted visible content: `/` (mm0 field + verify/publish), `/status`
  (proof id + verified), `/proof` (spec text + state) and `/search?q=sort`
  (a hit). Screenshot: `/tmp/opencode/mm0-sandbox/search.png`.
- **Device lane gaps closed (2026-11-21)**: with Waydroid running and the
  devshell's `adb`, `device-lane.sh persistence` passed (edit → force-stop →
  relaunch restores 90-byte text, caret 89, reading y 4553), `composing-ime`
  passed (CJK composition and emoji commit reach the model), and the full
  `device-lane.sh e2e` passed **196 checks** with one failure in the
  `terminal-lease` step — that step needs the Herdr gateway stub, belongs to
  the Herdr integration plan, and is not emthin document behavior. The lane's
  rebuild step was fixed to prefer the rustup 1.99 cargo (the nix rustc 1.98
  has no Android target).
- **Desktop E2E closed (2026-11-21)**: found and fixed a real crash — a
  released page raster (1×1 placeholder) was being fed to smithay's
  `MemoryRenderBuffer` with full-page dimensions, tripping
  `data.len() >= stride*h`. `winit.rs` now skips a released raster and uses
  the image's actual dimensions; a regression test covers it. With that fix,
  `scripts/e2e/run.sh --config wtype` passed **28 checks, 0 skipped** and
  `--config xdotool` passed **36 checks, 0 skipped**.
- **Gaps (named, not passes)**: the native proof UI was driven on the device
  with scripted fixtures; the SAF document-picker path could not be exercised
  on this image because the media provider's Downloads root is broken (the
  picker shows "No items"). The Herdr `terminal-lease` gateway attach step and
  P6/P7 remain at their integration-plan/K-plan evidence levels; no new
  live-session timing was captured here.

### Live sandbox checkpoint result (2026-11-21)

Run with a separately configured credential in
`~/.config/mm0web/mm0web.env` (mode 0600 — never in the repo or a log), the
sandbox HTTPS host **explicitly selected** as
`MM0WEB_ZENODO_BASE=https://sandbox.zenodo.org/api`, and a synthetic
`sort Prop;` spec/proof pair. One fresh draft was created by one explicit
upload action; cleanup was **not** requested, so the draft was left in place.

- Backend: `mm0web` at `b0aaa7c9220ae896985a4017cbc8e42a9e5f0d86`;
  `/healthz` 200 with both checkers ready and `zenodo: configured
  (https://sandbox.zenodo.org/api)`.
- Local verify of the synthetic pair: HTTP 200, `accepted: true`, `stage: ok`
  (`mm0-rs` elaborated, `mm0-c` re-verified). Fixture bytes: 11 bytes each,
  SHA-256 `acd539decfd1992b…` for both `.mm0` and `.mm1`.
- One explicit publish: HTTP 202, `accepted: true`, `queued: true`, proof id
  `emthin-sb-20261121135454-3307780-5a720607`. The backend settled it to
  `state: published`, DOI `10.5072/zenodo.647037`, sandbox record `647037`,
  URL `https://sandbox.zenodo.org/records/647037`.
- Byte comparison: the draft's returned `mm0`/`mm1` from `/proof` were each
  11 bytes with the same SHA-256 as the local fixture (`equal: true` for both).
- Revisit: a fresh `GET /proof` still reported `state=published` with the same
  DOI and record; the sandbox record itself reported `id=647037`,
  `state=done`, archived files `snapshot_0.bin` and `manifest.json`.
- No publication or versioning coverage was moved to the live lane — those
  remain on the mock (scenarios 1/3/5 above). No draft was deleted.

### D0 live-session baseline (2026-11-21)

The last D0 item was a live-session timing capture. Ran the `wtype` E2E with
`RUST_LOG="info,emthin::timing=trace"` (28 checks, exit 0) and aggregated
`.e2e/emthin.log` (121,656 timing events) into a per-stage p50/p95 table in
`docs/measurement-surfaces.md`: event-loop tick p50 26 µs / p95 221 µs with
38 slow ticks (0.03%); layout worker compile p50 112.5 ms / p95 382.6 ms,
glyph 48.9/208.8 ms, raster 124.6/156.7 ms, apply 0 ms; session save p50
14.3 ms / p95 75.4 ms (write dominates the snapshot). `README.md`'s status
comment and verification command counts were refreshed from this run's logs
(687 cargo, 28/36 E2E, 120 pad Rust, 231 Java). This is a sample, not a gate:
no p95 threshold fails a build.

### D1/D8 process-kill check (2026-11-21)

The remaining D1/D8 item — an OS process-kill at a session-write boundary — is
now closed. Added `scripts/e2e/process-kill.sh`: headless sway + emthin with a
scratch `--session-file`, open a marker-bearing source note over IPC, force a
recovery checkpoint, wait for the deterministic `session saved to …` log line,
SIGKILL emthin (no graceful shutdown), relaunch against the same session file,
and assert the last verified pair recovers the edited note with no
unrecoverable/quarantine warning. Run passed **7/7 checks, exit 0**. This is the
real-process counterpart to the deterministic `session/store.rs` fault tests;
P6's "storage-boundary only" gap is now a process-kill result.

## Initial D0–D8 delivery record

The original cards preserve the initial requirements and dated evidence.
Pre-implementation findings and old acceptance requirements are historical.
Use the current cards above for new work.

### D0 — Re-establish the measured baseline

**Starting point.** The documented counts are now reconciled: the 2026-11-18 baseline
ran 471 passed, 2 ignored after D1, and the completed plan runs 538 passed, 2
ignored (D8), including the async-relayout restore regression test and the Core
protocol client tests. These counts are historical. `cargo clippy` and `verify-invariants` pass;
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

**Starting point.** Implemented 2026-11-18. `session::store::SessionStore` owns the
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

**Starting point.** `ipc_save_doc` writes only `model.text()`; `open_document` only
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

**Starting point.** `DocModel::set_caret` and `set_selection` set `dirty` whenever the
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

**Starting point.** `DocLayoutCache::rebuild` runs `layout_doc_paged_with_text_size`
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

**Starting point.** `DocumentPreferences` persists page, zoom, pan, reading size and
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

**Starting point.** `FindBar` keeps the query and matches after `close`, but not the
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

**Starting point.** Implemented 2026-11-18. `emthin-collab::Session` already had
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

**Starting point.** Implemented 2026-11-18. `docui::keymap::ACTION_CATALOG` now names
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

**Starting point.** The cargo suite and E2E are healthy gates for their existing
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

**Result (2026-11-19).**

- `nix develop --command cargo test --locked --workspace` exits 0: **538
  passed, 2 ignored**. Per binary: 381 lib (`emthin`), 1
  `docui_async_restore`, 2 `collab_ipc`, 18 `xwayland_satellite`, 5
  `xwayland_satellite_watch`, 14 `emthin-clipboard`, 34 `emthin-collab`, 23
  `emthin-core-client`, 1 `scripted_peer`, 1 `unix_fixture`, 40 `emthin-dbus`, 18 `emthin-vt`; doc-tests 0.
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
- `cargo fmt --all -- --check` passes for every emthin workspace package; the
  remaining `--all` diffs are pre-existing edition-2024 drift in external path
  dependencies (unfer, velysterm), not in this checkout.
- Desktop E2E ran on 2026-11-18 in both configurations: `wtype` (headless
  sway) ran 28 checks and `xdotool` (sway's X11 backend on Xvfb) ran 36, with 0
  skipped and all passed; the XTEST extras assert Find cancel, then the fcitx5
  preedit and commit.
  The flake now exports `GBM_BACKENDS_PATH` and `LIBGL_DRIVERS_PATH` from its
  Mesa so sway does not look for
  `/run/opengl-driver/lib/gbm/dri_gbm.so`; that was an environment gap, not a
  product failure.

## Parked work

These are real needs, but they are not selected because they either require a
new subsystem or can be obtained by finishing an existing path. Revisit them
only with a separate scope decision.

| Item | Why parked | What would reopen it |
|---|---|---|
| ~~Interactive app launcher / command palette~~ **DONE (2026-11-22)** | Was parked as a redundant command surface. | Delivered as a document-driven palette over existing `launch:` figures; no command registry. |
| ~~Free 2D figure placement, workspaces, tabs~~ **DONE (2026-11-22)** | Was parked as a rewrite of the locked layout-authority decision. | Delivered as document-expressed `dx:`/`dy:` offsets through Typst `place()`; pages remain the only workspace model. |
| ~~Semantic outline, definition/use navigation, rename~~ **DONE (2026-11-21)** | Was parked for desktop UI and shared-core review. | Delivered as IPC/navigation surface below; no shared-core change. |
| ~~Rendered-text accessibility~~ **DONE (2026-11-21)** | Was parked for an AT strategy. | Delivered as a testable desktop authority below; a native AT bridge remains a separate adapter decision. |
| ~~Notebook/kernel execution, templates, execution output~~ **DONE (2026-11-22)** | Was parked for a worker/output protocol. | Delivered as an external worker with derived block-splice output; no unfer deps, nothing written into Loro. |
| ~~AT Protocol client / remote transport~~ **DONE (2026-11-22)** | Was parked for credentials and a deployment decision. | Delivered as an optional adapter crate with a scripted PDS; no credentials or network needed. Real PDS is a deploy-time swap. |
| ~~New plugin framework~~ **DONE (2026-11-22)** | Was parked awaiting a third concrete plugin. | Delivered as a generic one-in-flight helper primitive plus a `\lint` diagnostics plugin. |

### Parked items completed 2026-11-21

Two parked items were implemented after an explicit scope decision. Both stay
inside the document-as-authority model — a projection over mathed's existing
facts, no second parser and no compositor placement state. No `mathed_core` /
`mathed_mini` source changed, so the sibling workspace lane was not triggered.

- **Semantic navigation.** New `docui/semantic.rs` builds a read-only
  `SemanticView` from `SemanticIndex` + marker segments: definitions with use
  counts, resolved uses and unresolved names, all UTF-8 byte ranges. Rename
  reuses `SemanticIndex::plan_rename` and applies its ops as one grouped edit
  (one undo step); `DocUi` caches the view and invalidates it on the edit path.
  New IPC: `semantic_outline`, `goto_definition`, `rename_definition` with
  `semantic_outline` / `rename_result` replies. A missing definition returns a
  new `not_found` error code (documented in `docs/error-contract.md`). Keymap
  was deliberately not changed: this is a control/navigation surface, and
  adding global keys would collide with the existing Find and return-history
  bindings.
- **Rendered-text accessibility.** `mathed_mini::a11y` is behind the `gui`
  feature emthin disables, so `docui/a11y.rs` builds the same flat, ordered
  node list from `mathed_core::accessibility::build_access_nodes` (always
  available): `{role,label,value,range}` with stable byte offsets for caret
  sync and hit-testing. New opt-in IPC `a11y_nodes` → `a11y_nodes` reply. This
  is the testable desktop authority; mapping it to a native AT-SPI/accesskit
  tree is a separate platform-adapter decision, not a silent gap.

IPC catalog advanced to **40 client→server, 28 server→client** methods
(`check-ipc-docsync` green in both directions). Tests added: 5 in
`docui::semantic`, 3 in `docui::a11y`, 3 `DocUi`-level integration tests
(13 new). Workspace suite **699 passed, 2 ignored**; clippy, rustdoc
`-D warnings`, `verify-invariants` 7/7 and `check-links` all green.

### Remaining parked items completed 2026-11-22

The five items above were closed after the user's explicit scope decision. Every
one stays inside the fixed constraints: the document remains the layout
authority, there is no second parser/editor model, each async job has one
owner, the last-good page stays visible, nothing is written to Loro as derived
state, and presence stays ephemeral.

- **App launcher.** `docui/launcher.rs` builds a filtered palette from the
  document's dormant `launch:` figures — no command registry. `Ctrl+Shift+P`
  opens it; typing filters, Return delegates to the existing
  `relaunch_dormant` spawn path. IPC `launcher_entries` / `launcher_launch`.
  Label logic was factored out of `dormant_label` so both share one owner.
- **Free 2D placement.** `mathed_core::figures::FigureSpec` gains `dx`/`dy`
  point offsets parsed as named `\app` args; `figure_markup` emits Typst
  `#place(..., float: false)` plus an in-flow spacer. Typst bakes the offset
  into the placed frame, so `figures_in_frame` reports the shifted rect and
  the compositor stores no position. Pages remain the only workspace model;
  no `Space`/tab stack was added. Shared-core change verified in **both**
  lanes.
- **Kernel execution.** `docui/kernel.rs` runs an external binary (pinned by
  `EMTHIN_KERNEL_BIN`) on a worker with one in-flight job, a generation fence
  and last-good retention. Output is spliced through
  `TransformOptions::block_splices` and never enters the Loro text. IPC
  `kernel_run` / `kernel_outputs`. No `unfer` path dependencies.
- **AT Protocol.** New workspace crate `emthin-atproto` publishes only signed
  session descriptors (no deltas, snapshots, version tokens or presence) to a
  `PdsTransport` trait; a scripted `MockPdsTransport` proves offline/stale/
  replay handling without credentials. Wired to emthin on **explicit**
  publication only — never on the edit or save path. IPC
  `collab_publish_descriptor` / `collab_descriptor`. A real PDS client is a
  deploy-time transport swap, not a code gap.
- **Plugin framework.** `docui/helpers.rs` factors the one-in-flight,
  generation-fenced, backoff-bounded helper-process lifecycle shared by
  `\formal`/`\voice`; `docui/diagnostics.rs` is the third concrete plugin
  (`\lint`), emitting derived annotations through block splices. IPC
  `lint_run` / `plugin_catalog`; status line gains a `lint` state.

IPC catalog now **48 client→server, 36 server→client**. Workspace suite
**720 passed, 2 ignored**; sibling lane **584 passed**; clippy, rustdoc
`-D warnings`, `verify-invariants` 7/7 and `check-links` all green. No
`git push` was made.
