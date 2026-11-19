# Sibling implementation study for the Android pad

2026-11-15. This study supports the selected scope in
[FEATURE_SPEC.md](FEATURE_SPEC.md) and the execution queue in
[DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md). It extends the first review beyond
general architecture into user behavior, implementation mechanisms and failure
cases. These are observations of local checkouts; sibling applications and
their test suites were not executed. Test source was inspected where named.

Paths in code below are relative to this file. They are evidence locators,
not markdown links requiring nine additional clones in emthin's link-check CI.
The Android design decisions are adaptations, not claims that each sibling
implements the proposed Android feature verbatim.

## Kyneta — acknowledgement, identity and stable positions

Revision: `83d6e0d06f4be678f696c5f9e25d7b145a63c0b9`.

Read `../../kyneta/ARCHITECTURE.md`, `TECHNICAL.md`, and these implementation
paths under `../../kyneta/`:

- `packages/exchange/src/store/store-program.ts` and its
  `store/__tests__/store-program.test.ts`.
- `packages/exchange/src/store/store-format.ts`, `doc-status.ts` and
  `undo/undo-program.ts`.
- `packages/schema/src/position.ts`, `changefeed.ts` and
  `packages/transport/src/task-scope.ts`.

The store coordinator has an acknowledged version and a single owed write.
Repeated mutations during a write collapse into one follow-up; successful
persistence and release are explicit effects. `DocStatus` distinguishes
pending from settled-empty, preventing defaults from overwriting data still
loading. Store-format refusal distinguishes incompatible and malformed data.
Positions carry affinity and can resolve after edits. Undo distinguishes a
gesture from time-based typing groups. A task scope cancels one owned period,
joins tasks, and reports errors not caused by that cancellation.

Selected: acknowledged recovery/save versions, loading before seeding the
starter, versioned recovery, stable navigation anchors, explicit edit intents,
and one owned pending render/write (C01, C02, C03, C08, C13). Keep Loro and
JNI. Reject Kyneta's schema/substrate framework and multi-document undo stack:
the pad already owns one document and its native undo manager.

## Pinetrie — context without stealing the reader's place

Revision: `0f4483f4aa86058347448f616848b475cb8c0fa5`.

Read `../../pinetrie/docs/06-interface.md`, ADRs `0027-column-layout`,
`0203-column-dock-changes`, `0213-tests-name-their-requirements`, and these paths
under `../../pinetrie/`:

- `packages/interface/search.ts`, `search.test.ts`, `column-motion.ts`,
  `column-view.ts` and `attention.ts`.
- `packages/web-core/edge-scroll.ts`, `edge-scroll.test.ts`, `fold-motion.ts`.

Search belongs to a particular canvas; changing context cannot reuse another
canvas's search state. Its corpus is explicit, and snippets preserve offsets
while marking matches. The UI specification follows arriving content only
while the reader is at the end; scrolling away suspends following. Folds are
reader-local, distinct from shared card order. Actions live near the work,
while status is a mark with words when needed. Edge scrolling increases toward
the edge. Fold motion has an explicit reduced-motion path. Verification names
requirements instead of hiding missing coverage in arbitrary test skips.

Selected: document-bound search, snippets, reader-owned viewport, local fold
state, adaptive modifier inspection, selection edge scrolling, meaningful
status and separate device requirements (C04, C05, C07, C10, C11, C14, C16).
Do not copy its word-prefix matcher: mathed's existing literal smart-case
semantics remain authoritative. Do not copy conversation branches, agent lanes,
canvas boards or the elaborate column animation system.

## Atuin Desktop (`desktop`) — document versus runtime state

Revision: `06102dcd77f5564a08fbba2ac4a29c1b7deb27d0`.

Read the README and these paths under `../../desktop/`:

- `src/state/runbooks/snapshot.ts`, `operation_processor.ts`, `search.ts`.
- `src/lib/sync/runbook_synchronizer.ts`, `src/lib/std/async_queue.ts`.
- `src/lib/workflow/blocks/block.ts`, `blocks/editor.ts`, `workflow_runtime.ts`.
- `docs/docs/blocks/executable/variable.md`, `sub-runbook.md`.

Snapshots have runbook identity and tags, while runtime running state is kept
separately. Shared template variables and local variables have deliberately
different persistence/synchronization rules. Search ranks document title and
content separately. The priority queue bounds simultaneous work, not the
number of queued requests. Execution blocks have identities, dependencies
and output/runtime concerns beyond ordinary document rendering.

Selected: distinguish recovery from an explicit file save, retain document
identity through file operations, label source versus preview, and prioritize
visible work without an unbounded request queue (C01, C02, C10, C13). Separate
presentation preferences from authored document properties.

Reject runbook execution and variable/environment subsystems for this
milestone: mathed already has statements, templates and a worker architecture.
Adding Desktop's execution model would duplicate those and would not make
them Android-compatible. Do not copy its priority queue without coalescing.

## Fresh — a reader and editor over one buffer

Revision: `4a5940ed606340a09e5fc5b88751ee56459f51b6`.

Read `../../fresh/docs/features/navigation.md`, and these paths under
`../../fresh/`:

- `docs/internal/buffers-splits-undo.md`, `search-and-diff.md`,
  `widget-controls-own-interaction.md`.
- `crates/fresh-editor/src/view/viewport.rs` and `app/async_dispatch.rs`.
- `crates/fresh-editor/src/services/recovery/storage.rs`, `store.rs`.
- `crates/fresh-editor/src/app/file_operations.rs::check_save_conflict`,
  `on_save_actions.rs`, `prompt_actions.rs`.

Viewport position is a document byte anchor plus a row displacement, not just
a pixel offset. Follow, Hold and HoldRows are explicit alternatives; pointer
placement does not immediately trigger an unrelated caret recenter. Recovery
retains unsaved content separately from window layout. Save-conflict checks
distinguish outside writes from the editor's own writes. Async dispatch has a
per-frame work budget. Search shares one matcher, supports selection scope,
caps results and paints visible overlays. Replace uses one transaction.
The interaction document's focused-control-first rule explains why global
handlers must not steal native text-field keys. These documents are checked
against the named source paths; their prose alone is not treated as a test.

Selected: anchored reading, explicit scroll ownership, atomic recovery,
pre-save outside-change detection, bounded search, literal replacement,
and focused-field key ownership (C01–C08, C11, C13, C14). The pad can use
its existing `MathDoc::replace_many`; it needs no piece-tree editor, QuickJS,
git review UI, project grep or split manager.

## Foot — cheap overlays, predictable search and input

Revision: `152487e494993ad7fb5633a27c7d7ad0f8810ea5`.

Read `../../foot/render.c`, `ime.c`, `search.c`, `selection.c`, `foot.ini`,
and the search, selection, font and touch sections of `doc/foot.ini.5.scd`.

Rendering has damage/refresh scheduling separate from content generation.
IME keeps pending preedit and commit state. Search cancel restores the
viewport; commit keeps the match. Search bindings have their own context,
including Ctrl+S/Ctrl+R. Font sizing is distinct from terminal resizing policy.
Touch long-press is a deliberate gesture, separate from tapping/scrolling.

Selected: overlay repaint without typesetting, composition-aware rewrites,
full search-origin restoration, compatible search aliases, reading-size
controls and native touch selection (C03–C07, C13). Preserve work already
recorded in [MATHED_FOOT_PARITY.md](../docs/MATHED_FOOT_PARITY.md).
Do not transfer terminal grid, PTY, escape-sequence, OSC or PRIMARY-selection
policy into Android's document model.

## Helix — navigation history and edits with position mapping

Revision: `a714a818a3a4f6dd5aad3b003a78692f90a1703f`.

Read `../../helix/book/src/jumplist.md`, and these paths under `../../helix/`:
`helix-view/src/view.rs`, `document.rs`, `helix-core/src/transaction.rs`,
`graphemes.rs`, `selection.rs`, plus movement's visual-column handling.

The jump list is bounded, deduplicates, truncates its forward branch after a
new jump, and maps stored selections from their original history revision.
Significant navigation adds a checkpoint; ordinary caret steps do not.
Transactions map positions and compose multiple edits. Grapheme utilities
distinguish an editable character from a Unicode code point and display cell.

Selected: 30-entry back/forward history, selection restoration across edits,
preferred visual x for vertical movement, one intent for replace/paste, and
grapheme boundaries (C03, C04, C06, C08). Use mathed's own semantic index for
definition/usage navigation and rename rather than introducing LSP. Do not
adopt Helix's modes, multi-cursor model, rope or history implementation.

## Atuin CLI — context changes what an action means

Revision: `6e57c7e3dc8ddcbfdcbd65989ca86f81f5b2908f`.

Read these paths under `../../atuin/crates/atuin/src/command/client/search/`:
`engines.rs`, `engines/db.rs`, `keybindings/keymap.rs`, `conditions.rs`,
`defaults.rs`, and relevant interactive-search dispatch.

A search state includes its scope/context. Available filters depend on that
context. A key binding is ordered rules over an immutable context snapshot;
empty query, cursor position and list boundaries affect the result. Acceptance
can return a selection without executing it. Match-highlighting and query
parsing are separated from UI dispatch.

Selected: explicit Text/Source/Selection scope, preserve match identity on
updates, return versus cancel behavior, and an action table whose predicates
are tested (C05, C06, C14). Keep mathed's matcher, not Atuin's SQL/fuzzy query
language. Do not add a shell-history database, synchronization service,
executable search result or configurable key-expression language.

## Starship — useful context with bounded collection cost

Revision: `e3b64da0b660552377de8f7101a73a36b145791c`.

Read `../../starship/src/print.rs`, `module.rs`, `context/mod.rs`,
`configs/starship_root.rs`, and conditional execution in `modules/custom.rs`.

Prompt composition includes only applicable modules; context caches expensive
directory reads with `OnceLock` and separately bounds scans/commands. Its
grapheme-width test covers combining text and emoji sequences. Conditional
modules and time budgets are mechanisms, not permission to run arbitrary
status commands in the pad.

Selected: one compact status projection, delayed busy indication, cached
document-derived data, separate latency counters and text-aware truncation
(C13, C14). No prompt engine, shell probes, arbitrary status modules or user
configuration DSL. The mobile status reads existing state only.

## Infinity — owned work, interruption and genuine idleness

Revision: `e1a3e4b36e5f6143e03a59a7ad86e1abacfd22e8`.

Read these paths under `../../Infinity/`:

- `crates/infinity-agent-core/src/system/local/driver.rs`, `router.rs`.
- `crates/infinity-agent-core/src/system/defer.rs`, `model.rs`, `tests.rs`.
- `crates/infinity-daemon/src/session_store.rs`.

An in-flight step and its cancellation handle are one owned value. User input
can interrupt active work; unrelated synthetic events do not cancel the work
they are waiting for. Drivers become idle and wake when an input arrives.
Behavioral tests cover interruption, deferral and idling. Model selection is
captured for one round, so an in-flight operation does not change identity
halfway through. The inspected daemon session store uses direct file writes
and default-on-unreadable loading: those details are unsuitable for preserving
the pad's only document and are explicitly not adopted.

Selected: captured job inputs, one active/latest pending render, stale-result
rejection, separate non-droppable save work, and no polling while idle (C13).
The constant-return JIT smoke moves to an explicit diagnostic invocation.
No model provider, agent runtime, RAP/MCP server or durable rendering queue.

## Cross-check against emthin/mathed

The implementation targets in the feature specification were checked against
the pad's current `MainActivity`, `DocBridge`, `lib.rs` and `modifiers.rs`,
the included `DocModel`, shared bibliography helpers, and these paths under
`../../velysterm/crates/`:

- `mathed_core/src/doc.rs`: `replace_many` already groups edits; text
  revision explicitly excludes marks. `sync.rs` already exposes Loro versions.
- `mathed_core/src/glyphs.rs`: glyph/line geometry, point-to-source lookup,
  caret positioning and range rectangles already exist.
- `mathed_core/src/semantics.rs`: definitions, use sites, unresolved
  occurrences, block summaries and `plan_rename` already exist.
- `mathed_core/src/transform.rs`: copied-span source maps exist, but generated
  text can snap to a neighboring source position. Exact diagnostics must check
  mapping provenance instead of presenting that snap as an exact location.
- `mathed_mini/src/render.rs`: text-size reflow is distinct from raster scale;
  `rasterize_at` bounds dimensions to u16, which is not a memory-byte budget.
- `mathed_mini/src/world.rs`: font/library state is already shared; constructing
  a new `MiniWorld` does not reload all fonts. The avoidable work is repeated
  compilation, snapshot import, glyph construction and raster transport.

This leads to extensions of existing editing, navigation and rendering paths,
not a second editor, parser, CRDT, application layout engine or execution model.
