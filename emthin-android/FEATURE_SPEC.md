# Selected Android feature specification

Decision date: 2026-11-15. This is the normative behavior and engineering scope
for the [development plan](DEVELOPMENT_PLAN.md). The implementation study and
rejected alternatives are in [SIBLING_STUDY.md](SIBLING_STUDY.md). Nothing in
this specification is a claim that the proposed behavior already works.

The [native terminal figure specification](../docs/HERDR_INTEGRATION_SPEC.md)
defines the later Android integration with Herdr Core and OpenShell.

## Product boundary and selected scope

One Android Activity edits one Loro-backed mathed document. The rendered
document stays the main surface. Source, relationships, references, search and
diagnostics are views of that document. Typst owns pages and figure geometry.
The pad renders `\app` placeholders; it does not host Wayland applications.

Selected extensions complete those existing paths. Search gains literal
replacement; navigation gains return history and a section/definition index;
modifier inspection gains target navigation and assistance; file operations
gain truthful save/recovery state. None needs a second document model, a
terminal, an agent runtime or a new notebook execution architecture.

| ID | Exact feature | Existing path extended | Main task |
|---|---|---|---|
| C00 | Buildable Android adapter and reproducible dependency lane | Citation JNI/shared helper, standalone workspace and CI | A0, A1 |
| C01 | Whole-document recovery with explicit corrupt-data handling | Autosave and cold-launch restoration | A3 |
| C02 | Save, Save copy, safe Open and outside-change detection | Android document picker and `.mathed` persistence | A4 |
| C03 | Grapheme-safe editing and composition-sized undo | Shared model, native text fields, completion | A5 |
| C04 | Touch/keyboard selection and structure-preserving clipboard | Existing selection/copy/cut/paste | A5, A8 |
| C05 | Contextual literal search with complete cancel restoration | Mathed incremental search | A11 |
| C06 | Replace next/all with protected structure and one undo | Search plus `MathDoc::replace_many` | A11 |
| C07 | Reading size, fit-width zoom, stable reading anchors | Current page scale, view offsets and glyph mapping | A7 |
| C08 | Back/forward, sections, definitions, uses and safe rename | Caret navigation, citations list and semantic index | A12 |
| C09 | Navigable citation details with local style/locale choices | Existing nested citation dialog and shared bibliography formatter | A13 |
| C10 | Navigable, validated modifier relationships and comparisons | Marker-pair field and comparison columns | A14 |
| C11 | Adaptive source inspector and local folds | Existing source/modifier panel and explicit expansion | A7, A9, A14 |
| C12 | Actionable Typst, semantic and relationship diagnostics | Render errors, semantic index and modifier isolation | A15 |
| C13 | Latest-work rendering, reused layout and bounded page pixels | Existing background render/chunk stream | A2, A6, A7 |
| C14 | One action table, useful status and consistent Back/focus | Shortcut strip, key dispatch and status | A9 |
| C15 | Equivalent accessible reading, navigation and editing | Native source fields and rendered page | A8 |
| C16 | Separate host, Java, APK and device evidence | Existing test suite, build scripts and device lane | A1, A10 |

## Shared contracts

### Identity and state ownership

Use these identities; the raw `MathDoc::revision()` is not a document identity
or a marks revision:

| Value | Definition and lifetime |
|---|---|
| `documentId` | Random local UUID, allocated at New or successful Open; persisted with recovery, independent of title and URI |
| `instanceId` | Fresh Activity/model incarnation token; never persisted as an identity for accepting work |
| `mutationSeq` | Monotonic UI-thread counter for successful text/mark edits, undo and redo; no increment for movement or preferences |
| `captureId` | `(documentId, instanceId, mutationSeq)` for one immutable captured snapshot |
| `presentationKey` | Reading size, explicit folds, marker visibility and theme version; source caret is excluded from the rendered-only layout |
| `layoutId` | Capture identity plus presentation key, assigned to one compiled document |
| `pageKey` | Layout ID, page index, raster pixel size; excludes cursor/selection/search overlays |
| `logicalFingerprint` | SHA-256 of canonical current rich-text content, including all attribute names/values, not just recognized modifiers; excludes CRDT operation history and view state |

Canonical fingerprint material is UTF-8 text/delta content with attribute maps
sorted by key and adjacent equal-attribute text runs coalesced. Expose this
through one shared `MathDoc` helper, since `segment_marks()` alone filters
`prop:*` and discards values. Hash on a worker from the immutable snapshot;
cache the result per capture. Snapshot checksum is separate: it validates the
actual stored bytes. Undo back to the saved logical content clears the dirty
indicator once its fingerprint has been checked, even though Loro history grew.

The live `DocModel` handle stays UI-thread-only. A worker can import a captured
snapshot into its own model; it never accesses the live handle. Content lives
in Loro. URI/save acknowledgements and document title are local metadata.
Reading anchors, folds, marker visibility, citation choices, query and inspector
state are local view metadata. Temporary preedit, pending picker requests,
dialogs, active jobs and navigation history are not shared document content.

### Source and position rules

Rust model APIs use UTF-8 bytes; Android uses UTF-16 code units. Every bridge
range states its unit, uses checked conversion, and snaps editable boundaries
to graphemes. Geometry is in Typst page points until `PageTransform` converts
to pixels. Reuse `GlyphIndex`, `OffsetMap`, marker scan and `SemanticIndex`.

Add a small shared wrapper over Loro's pinned-version cursor APIs:
`create_anchor(byte, affinity)` / `resolve_anchor(encoded)`. Convert explicitly
between Loro's required index unit and mathed bytes. An anchor record contains
its document ID, encoded cursor, left/right affinity and fallback byte. It
cannot be applied to another document. Resolve deleted anchors to the biased
neighbor; if no position resolves, clamp the fallback to a grapheme boundary.
Use these anchors for navigation, reading position, search origin and ranges
retained across a document edit. Do not build a parallel edit-history mapper.

### Editing intents

All replacements validate ranges against the expected document/capture before
the first mutation. Reject overlapping, stale or invalid operations as one
error. Use `MathDoc::replace_many` and a `DocModel` wrapper that rescans and
updates selection once. Named intents are typing, composition, paste, cut,
replacement, definition rename, modifier change and New. A multi-range intent
is one undo group. The existing 400 ms typing merge interval remains in place;
explicit intents are bounded independently of that timer. Failed validation
creates no undo entry.

### Implementation boundaries and bridge changes

Extract these owners as their tasks land; do not first perform a wholesale
file move. `MainActivity` retains lifecycle wiring and view assembly. Use
plain Java, with Android imports only where needed.

| Owner | Responsibility; task |
|---|---|
| `PadSession.java` | Document/capture identity, edit/save/recovery state and lifecycle decisions; A2–A4 |
| `RecoveryCodec.java` / `RecoveryStore.java` | Pure bounded envelope codec / AtomicFile shell; A3 |
| `DocumentFiles.java` | Picker ownership, grants, streams, destination hash and file acknowledgements; A4 |
| `RenderController.java` | Active/latest-pending work, immutable inputs, priorities and accepted publications; A2, A6 |
| `PageTransform.java` | All page-point/screen conversions; no Android view or compositor policy; A7 |
| `RenderedDocumentView.java` | Page materialization, draw/selection/touch and virtual accessibility nodes; A7, A8 |
| `FindController.java` / `NavigationController.java` | Search/replace context / stable location history and destinations; A11, A12 |
| `InspectorView.java` | Native source/target fields, drafts, comparison rows and focus preservation; A14 |
| `PadActions.java` | Current commands, enabled predicates and status projection; A9 |

Native JNI entry points stay in `lib.rs` until moving an implemented owner is
useful. New pure code belongs in `editing.rs` (intents/conversions),
`analysis.rs` (semantic/search/diagnostic data) and `layout.rs` (retained worker
layout). Keep the existing `modifiers` module, shared `model`/`citations` path
imports, and `jit` diagnostic module. Do not create another document engine.

Planned bridge contract:

| Operation | Thread / result |
|---|---|
| `snapshotBytes(handle)` / `openSnapshotBytes(bytes)` | UI thread capture/import; raw bytes / model handle, with invalid input reported explicitly |
| `applyEdits(handle, expectedIdentity, edits, selection)` | UI thread; checked UTF-16 batch converted to bytes before mutation; one returned editor state |
| Existing typing sync / committed `applyEdits` | Preserve timed typing coalescing; committed composition and multi-range intents use the existing grouped batch primitive, with no nested or long-lived native undo group |
| `createAnchor` / `resolveAnchor` | UI thread wrappers over the live model's stable positions; no worker accesses its handle |
| `prepareLayout(snapshotBytes, layoutId, presentationOptions)` | Render worker; retains one thread-confined layout; returns page metadata, location summaries and diagnostics |
| `renderPage(layoutId, pageIndex, scale, pixelLimit)` | Same render worker; one PNG plus bounded hit/line metadata, actual page size and owner ID |
| `locateRange(layoutId, startUtf16, endUtf16)` | Same worker; page and source visibility, without rasterizing intervening pages |
| `analyze(text)` / pageable `findMatches(text, query, scope, continuation)` | Immutable worker inputs; semantic/range JSON, continuation valid only for those exact inputs |
| Existing citation/modifier functions | Immutable worker inputs; add choices/owner context and row-level comparison parameters |

Add a small declaration-conformance check to keep Java/JNI names and arity
aligned; no second protocol catalog. Preserve the independent stateless
`renderPages` test seam as a wrapper over the same layout primitives, not a
second pipeline. Mutation responses retain existing state fields and add
explicit errors; do not interpret them with substring checks.

| Event | Required invalidation |
|---|---|
| Caret/selection/search highlight | Overlay only; local view save may be owed |
| Source edit, mark change, undo/redo | New capture and analysis/search identity; new layout as required |
| Fold/marker/reading-size change | New presentation/layout, no document dirty mark |
| Zoom/pan/viewport movement | Page transforms and raster requests; no compilation |
| Citation style/locale | Citation detail cache only, no full-page compile |
| New/Open | New document-bound requests; abandon old picker/render publications |
| Activity destruction | Dispose view publications; preserve already captured storage work |

File and recovery states are independent. File state is Unbound, Dirty,
Saving(capture) or Saved(fingerprint), with an optional failure that does not
erase the last acknowledgement. Recovery state is Loading, Ready(generation),
Writing(capture, owedLatest), Failed or NeedsRecoveryChoice. The sole writer
serializes acknowledgements; an old acknowledgement cannot lower the durable
generation or mark a newer content fingerprint saved.

## C00 — Repair the adapter before feature work

Pass `None` and `citations::DEFAULT_CITATION_LOCALE` to the current four-argument
bibliography helper. C09 subsequently supplies real presentation choices.
Keep formatting in that shared helper. Update only necessary Android lockfile
resolution and provision australVM's actual selected path-dependency closure
in CI. Record sibling SHAs in a checked-in dependency manifest read by the
Android job, not as floating `main` references. The first manifest names a
dependency set that has passed the Android suite; the reviewed SHAs are
starting inputs, not an assumed compatible set.

Keep the standalone workspace, its own toolchain and gradle-free APK build.
Add a host Java test command for pure controllers: compile with the installed
JDK and run an assertion-enabled test entry point, without Android classes or
JUnit downloads. Android view/input tests belong in the device lane.

## C01 — Recovery is a complete document, not a text backup

Use two app-private recovery slots, `recovery-0.bin` and `recovery-1.bin`,
written alternately with Android `AtomicFile` by one application-owned storage
worker. No database, background service or document-sized SharedPreferences.
The envelope is:

```text
8 bytes magic "EMPADRC1"
u32 little-endian metadata length (maximum 1 MiB)
UTF-8 JSON metadata
raw Loro snapshot bytes
32 bytes SHA-256 over every preceding envelope byte
```

Metadata includes format version 1, globally increasing recovery generation,
document ID, snapshot length, logical fingerprint, display name, optional URI
and last explicit-file acknowledgement, caret/selection anchors, reading
anchor/fallback, reading size, zoom/pan, marker visibility, folds and citation
choices. Store only the last accepted search query; never a pending edit or
picker callback. Validate lengths before allocating, checksum, version and
snapshot import before accepting a slot. Maximum snapshot input stays 32 MiB,
matching the current file-import limit. A larger document remains editable in
the current process but reports that recovery cannot be updated; never truncate.

On launch inspect both slots; select the highest fully valid generation. If
one newer slot is corrupt, open the last valid slot with “Recovered an earlier
checkpoint” and retain the bad bytes for export. If neither is valid, offer
“Export recovery data”, “Open another document” and “Start new”. Do not seed
the starter or overwrite unreadable data until that choice. During asynchronous
load display “Restoring document…” and disable content-changing commands.
An empty successfully loaded document is valid and stays empty.
Before reusing a rejected slot's path, quarantine its bytes under a unique
checksum-labelled `.rejected` name; deduplicate identical rejected bytes. If
preservation fails, suspend replacement of that slot and report the storage
failure. Never automatically delete unresolved recovery evidence.

Migrate old `docSnapshot`/`docText` preferences once: matching snapshot first;
plain text only when no snapshot ever existed. If an existing snapshot is
invalid or mismatched, offer explicitly labelled “Recover text without marks”
and retain the original. Remove legacy preferences only after a new valid slot
has been written and read back. `Bundle` carries recovery identity and small
view metadata, not the full document text.

Capture after 250 ms of content inactivity and at least once every 2 seconds
of continuing edits. Maintain one in-flight recovery write and one owed latest
capture. Capture again on pause and before New/Open replace the active document.
Opening/creating replacement content waits for the old content's required
checkpoint acknowledgement; failure keeps the old document and offers Retry
or an explicit discard. A paused Activity does not own the recovery worker's
lifetime; it may finish its already captured bytes without accessing views.

Do not block the main looper for fsync or claim survival of arbitrary immediate
force-stop. “Recovered” means the last acknowledged durable generation. A kill
before the next acknowledgement may lose that pending interval. Test both
boundaries. Persist view-only changes in a separate AtomicFile `view.json`,
bound to document ID and the last durable logical fingerprint. Coalesce writes
at 250 ms inactivity / 2 seconds continuous change; do not re-export Loro for
a scroll. The recovery envelope's view metadata is the fallback when that
sidecar is absent, mismatched or corrupt. Normal recovery does not promise
undo history across a cold launch; in-session undo/redo remains complete.

## C02 — Explicit files and recovery have different acknowledgements

`Save` writes the captured `.mathed` snapshot to the active writable URI or
launches `ACTION_CREATE_DOCUMENT` if none exists. `Save copy…` always launches
that picker and leaves the active URI and its saved fingerprint unchanged.
It reports “Copy saved”. Never announce “Saved” for merely updating recovery.
New/imported source has no saved URI and is unsaved until explicitly saved.

At Open or successful Save, remember the provider document's content hash,
size and last-modified hint when readable. Before overwriting an existing URI,
re-read/hash it on the file-provider worker; timestamps alone are insufficient. If
it differs from the remembered file, show “File changed elsewhere” with
“Save copy”, “Replace file” and “Cancel”. Recheck immediately before a chosen
replacement. SAF has no general compare-and-swap, so describe the remaining
read/write race honestly and retain local recovery. If read permission is
unavailable, default to Save copy; explicit replacement can acknowledge that
the destination cannot be checked. A read or write exception leaves dirty
state and recovery intact.

Bind picker request ID, document ID and instance ID when launched. A picker
returning after New, another Open or Activity destruction cannot save/import
into the new instance. Re-created Activities abandon an obsolete request and
let the user retry. Cancel changes nothing. Save acknowledgement includes the
actual capture/fingerprint written: edits made during the write remain dirty.
Only one explicit save for the same URI is active; later requests coalesce
to its newest requested capture. Do not coalesce writes to different URIs.
Provider I/O runs separately from app-private recovery, so a slow provider
cannot starve checkpoint writes. After one second expose a Cancel action for
an unfinished open/save; cancel invalidates publication immediately, signals
the platform cancellation mechanism and closes an obtained descriptor. It
does not claim to kill a provider that ignores cancellation. Keep at most one
active provider operation and one owed same-URI save. Other destination/open
actions wait until the active operation finishes or is cancelled; do not
silently replace one requested destination with another. A new request after
cancel may wait for the provider's actual return, with that state visible.
Never replay an abandoned save automatically. Report whether a cancelled write may already
have changed its destination, while retaining local recovery.

Keep UTF-8 source import and the 32 MiB read limit. A file named `.mathed` or
carrying a recognized Loro header that fails snapshot import is an error,
never a lossy text import. Decode other source candidates as UTF-8 strictly. A plain
source file is imported as a new `.mathed` document so marks cannot disappear
on Save. No PDF, notebook, ASCII projection or plaintext export is selected
for this milestone; finish lossless `.mathed` interchange first.

## C03 — Input and undo

Keep native Android text fields and their IME integration. Use composing-span
state and, where necessary, an input-connection wrapper for begin/end batch,
set/finish composing text and commit. Never replace the editable buffer during
active composition solely to refresh the inspector. Keep composing candidates
in the native source field, without syncing them into the live `DocModel` or
typesetting a candidate as committed content. Cancellation discards that draft;
commit applies its final anchored range once. Committed composition is one undo intent even if it
lasts longer than 400 ms. Persisted recovery is the latest committed state;
do not save an intermediate composing candidate as a completed document edit.
Save/New/Open first finish the focused field's composition through the native
input connection, then capture the resulting committed model. An explicit
cancel discards the candidate without adding a redo-able candidate edit.

Defer marker auto-naming and math delimiter completion until composition is
committed. Pasting never auto-creates markers or commits completions from text
that merely resembles a key sequence. Keep `\#` literal-hash behavior and RFC
word marker names. Completion applies only inside math, has an explicit hint,
and Escape cancels the pending hint before closing an editor.

Rendered left/right and deletion use graphemes; Ctrl+left/right uses existing
mathed word boundaries. Up/down uses visual line bands and preserves preferred
x through short/long lines; reset preferred x after horizontal motion, a tap
or an edit. At an unloaded page boundary request navigation metadata directly,
not a nearest glyph from an unrelated loaded page. Home/End go to the visual
line edges; Ctrl+Home/End go to document edges. Native text-field behavior
takes precedence while a field owns focus.

## C04 — Selection, clipboard and touch

Use one anchored selection for keyboard, touch, search acceptance and
accessibility. Shift movement extends it; unshifted horizontal motion collapses
to the corresponding edge. Ctrl+A/C/X/V retain their existing meanings.
Replacing selected content by typing or paste is one intent, including removal.

Single tap places the caret at the closest editable glyph boundary and opens
the source inspector only on text. Tap citation/fold titles retains their
specific action. A tap on empty document content opens source at the nearest
insertion point, including an empty document. Dragging past Android touch slop
scrolls and never becomes a tap. Long-press uses platform timeout and selects
a word using mathed boundaries; selection handles extend by grapheme. While
dragging a handle, scroll near the visible edge using frame callbacks; stop
callbacks on release/cancel. No always-running selection timer.

The contextual toolbar has Copy, Cut, Paste and Select all, with normal enabled
states. Copy uses the current document's source substring, omitting hidden
marker tokens when markers are hidden; it does not copy rendered pixels or
invent a rich clipboard format. Cut preserves complete hidden marker tokens
as the current UI promises. If a selection intersects only part of a marker,
snap to an allowed boundary instead of retaining a partial token. When markers
are explicitly shown, source selection can include their literal text. Pure
source fields retain Android's native selection/copy behavior. Unknown Loro
marks outside edited ranges must survive all operations.

## C05 — Search with an explicit corpus

Keep literal Unicode smart-case matching from `mathed_core::search`: lowercase
query ignores case; any uppercase makes it case-sensitive. No regex, fuzzy
ranking, SQL syntax or normalization that changes stored offsets. Extend the
shared search API with a range-limited/pageable query, keeping existing callers.

The existing Find bar gains a scope selector:

| Scope | Exact corpus and behavior |
|---|---|
| Text (default) | Raw source matches whose ranges do not intersect marker tokens or property-statement ranges from the scan; never concatenate across omitted ranges |
| Source | All canonical source, including markers and statements; a result lacking a rendered location opens the source inspector at that exact range |
| Selection | Freeze the non-empty selection as stable endpoints when chosen; search its source range. Disabled without a selection |

Text includes editable prose/math and collapsed bodies. It does not search
generated citation numbers, formatter output or modifier-generated glyphs as
if they were independent source. A match hidden in a fold opens its source
and temporarily reveals the containing fold; acceptance keeps that reveal,
cancellation restores the origin's folds. The bar labels the corpus, so a
Source result never appears to be a missing on-page highlight.

Capture full origin once on opening: selection, reading anchor, focus target,
inspector and fold state. Query changes pick the first match at/after origin;
Next/Previous step from the current match and wrap with a “Wrapped” cue. After
edits retain the anchored current match if it still matches; otherwise choose
the next valid match at its mapped position. Query/scope changes invalidate
only search work and overlays. Empty query means no highlights, not an error.

Enter/Accept keeps and selects the current match and closes Find. Escape or
the explicit Cancel action restores the entire origin, including viewport;
Back first lets Android dismiss the IME, then cancels Find. Remember the last
accepted query per document. F3/Shift+F3 reuse it with Find closed. Ctrl+F and
Ctrl+Shift+R open Find; Ctrl+S/Ctrl+R mean next/previous only while Find is
active. Add normal touch Next/Previous/Accept/Cancel controls.

Show `n / total` with exact counts only after scanning completes. Page results
in blocks of 200 rather than retaining an unbounded JSON range array; before
completion show `n / 200+ · searching` when appropriate. Searches run off the
main looper with stale-input rejection. Offer a results list in the Find panel
with 80-grapheme source snippets, marked matches and source location; selecting
one performs the same match navigation. No global history or new search DB.

## C06 — Literal replacement

Expand a Replace row inside the existing Find panel; Ctrl+H opens that row.
Provide replacement text, “Replace” and “Replace all…”. Source scope remains
available for finding syntax, but replacements protect marker and property
statement ranges in every scope. The interface says “Structural matches are
not replaced”; counts distinguish replaceable and protected matches. Editing
structure remains possible in its source field or the semantic rename action.

Replace changes the current eligible match and selects the next; replacement
text is literal, with no capture groups or automatic marker/completion rewrite.
Empty replacement deletes eligible text. An empty search cannot replace.
Replace all shows a preview count and up to 20 before/after snippets in the
same panel, then “Replace N occurrences”. Apply the exact immutable match plan
only if its document/capture still matches; otherwise refresh the preview and
require a new action. Protected ranges are checked before any edit. One
`replace_many` operation, descending by byte position, preserves untouched
intervening marks; do not pass a fully rebuilt string through `sync_text`.
One Undo reverses all replacements and restores selection. Recompute search
after commit. Replacement errors leave the plan and document available.
Cap one replacement plan at 10,000 eligible operations. Beyond that show
“Narrow the selection to replace this many occurrences”; never silently apply
a prefix. Scanning/counting and preview preparation stay on a worker. Apply
through the grouped shared model API without a rescan per individual edit.

## C07 — Readable pages, view zoom and anchored scrolling

Add Reading size controls using the same mathed text-size semantics as the
desktop: default 17 pt for this pad, range 8–32 pt, 1 pt steps, Reset to 17.
Ctrl++/Ctrl+-/Ctrl+0 adjust/reset. Larger text reflows through Typst; it is a
local reading preference and never edits `\app` dimensions or authored text.
Existing explicit document text settings retain Typst's normal precedence.

Separately, view zoom defaults to Fit width (1.0), range 1–4, with
Ctrl+Shift++/Ctrl+Shift+-/Ctrl+Shift+0. Pinch changes only this view zoom and
keeps the focal document position beneath the pinch. A zoomed page can pan
horizontally; vertical document scrolling remains continuous. Gesture movement
reuses current pixels; reraster the visible page after 150 ms settling, when
requested scale differs by at least 12.5%, subject to the byte budget. Zoom
does not compile or repaginate. Font size, zoom and pan persist locally.

Each page response includes its actual width/height in points. One invertible
`PageTransform` accounts for fit scale, view zoom, page origin, centering and
pan. Drawing, taps, caret, selections, diagnostic highlights and accessibility
bounds all use it. Mixed/custom page sizes are supported; remove the A4
constant from geometry. A4 can remain a fallback before page metadata exists.

Maintain reader state as Follow caret, Hold after user scroll, or Hold at a
tap/selection head. Only explicit caret movement, edit, search acceptance or
navigation resumes following. Completing a background render or opening an
inspector does not steal a held viewport. Save top visible text anchor plus
its offset in page points; use page index/fraction for figure-only pages.
Retain the desired anchor across temporary page collapse so Undo can restore
it, until a user gesture establishes a new anchor. Rotation, IME resize and
font changes resolve the same anchor in the new layout.

## C08 — Navigation and semantic editing

Extend the existing citation-list surface into “Navigate” with Sections,
Definitions and References tabs. Ctrl+K still opens References. The toolbar
Navigate action opens the last used tab; Ctrl+G opens Sections. Sections use
mathed `DocumentContext.blocks` and their existing heading summaries; do not
write another Typst parser or promise a full rendered heading tree. Include
page destinations when known. Definitions show name and use count from
`SemanticIndex`; selecting goes to source, and “Uses” lists resolved occurrences.
Filters use the existing literal matcher; no fuzzy index.

Back/Forward are navigation actions in the toolbar overflow and Alt+Left/Right
when a native text field has not consumed them. Keep 30 entries per live
document, each holding stable selection/reading anchors and local fold context.
Push the source location for accepted search, section/definition/use jumps,
reference go-to-source and diagnostic navigation. Do not push every arrow,
scroll, intermediate search preview or fold toggle. Deduplicate adjacent equal
destinations; a new jump after Back truncates Forward. Navigation does not
undo edits. Clear history at New/Open; a cold launch restores only the current
reading position, not an old cross-document history.

The source inspector adds “Go to definition” and “Find uses” for a resolved
definition/use; “Rename definition…” additionally requires a concrete
`name_range` for that definition. Rename validates that the new name parses
as one mathed/Typst math identifier, then previews the definition
and exact resolved use count. Apply `SemanticIndex::plan_rename` as one checked
batch. Never do global string replacement and never rename unresolved lookalikes,
marker IDs or unrelated shadowed definitions. Reject a proposed name already
present as a definition or semantic occurrence outside the edit plan, so the
rename cannot capture another use. Rebuild/check the semantic index for the
preview before applying; a new binding error refuses the rename. An unchanged
name is a no-op. A stale index refreshes the
preview before applying. This is a mathed semantic command, not an LSP client.

## C09 — Citation details with source and presentation context

Keep numbered rendered labels and existing bibliography/document reference
resolution. Expand a detail with a readable label/snippet, formatted citation,
full bibliography entries and “Go to source” when a source span exists.
Nested document references show a breadcrumb and Back within the detail view;
closing returns focus and reading position to the invoking citation. Detect
cycles by reference identity on the current path and stop expansion with a
“Repeated reference — go to source” row. Limit nested detail depth to 16,
without hiding access to source. Expanding a branch does not expand its peers.

Style choices are “Document default” plus the shared `CITATION_STYLES`;
locale choices are the shared `CITATION_LOCALES`, initially
`DEFAULT_CITATION_LOCALE`. Wire these through `citationDetail` to the shared
formatter; do not maintain copies of the arrays in Java. Persist overrides
per document as view state. Reset returns to document style/default locale.
Choices affect formatted details, not numbered-reference identity. Do not
rewrite document statements or fetch CSL files over the network.

Unresolved keys show the existing explanatory suggestions and “Edit source”,
not an empty dialog. The References tab lists all citations with short source
snippets and resolved/unresolved status and filters by snippet/key/number.
Late detail formatting is bound to its document, dialog and breadcrumb path.

## C10 — Modifier relationships as an editable, navigable view

Keep the current marker-pair input and its text syntax. Suggest marker names
from the existing scan as the user types the current pair; choosing the start
then filters possible end markers to later positions. Show a short caption
snippet with each suggestion. Never generate all O(n²) pairs. Accept optional
`#` prefixes; normalize completed pairs to `start end`, separated by commas.

Validate missing/reversed/empty ranges and duplicate pairs before applying.
Duplicate pairs collapse to one. Preserve the active partial pair while it is
being edited; apply other complete valid pairs on caret-leave as today. Done
validates the entire list and applies its change as one undo intent. An invalid
or composing pair retains its draft and last valid relationships; leaving the
field cannot silently remove existing links. Show unresolved saved pairs as
repairable rows. Retain legacy selection links and their remove action; do not
rewrite their representation just by viewing them.

Incoming and outgoing rows show source/target snippets, marker IDs when known,
resolved/error state, “Go to modifier”, “Go to target” and “Remove link”. Removal
works for both pair-backed and legacy links through the appropriate existing
model operation. Pair-backed removal updates the normalized target field.
Navigation uses C08 return history. No new relationship graph or property
statement format is introduced.

Use the existing with/without rendering. Request a comparison only for expanded,
visible rows, one row at a time; retain at most six row results under the same
raster byte budget as pages. Key by capture, excluded link, range, width and
scale. Other rows show a text summary until expanded. Bad modifier expressions
still preserve valid modifiers and source; the warning links to that modifier.

## C11 — Source and folds on phones and larger screens

Keep one source buffer and one inspector identity. At less than 600 dp usable
width, show source and target controls in a bottom inspector with one
comparison card below another; at 600 dp or more use source/target columns
and aligned comparison columns. This changes shell controls only, never Typst
figure placement. Preserve the source selection and composing buffer while
comparison output refreshes. Keep controls reachable above the IME; if the
remaining height cannot show both preview and editor, prioritize the editor
and provide “Show document” to return. No fixed full-screen screenshot geometry.

Fold/unfold translator/template/base bodies using current segment identity,
with explicit local state. Selecting a diagnostic/search result may temporarily
reveal its body without rewriting content. If a fold changes geometry, preserve
the reading anchor. The title remains a readable, accessible activation target.
Collapse never drops source edits or draft marker pairs.

## C12 — Diagnostics that lead to the relevant source

Return a structured list with `id`, severity, code, message, capture ID,
optional source range/marker pair and provenance (`exact`, `owner`, `document`).
Retain Typst compile warnings/errors instead of discarding them as `Eval`.
Subtract the theme prefix and map copied spans through `OffsetMap`. A diagnostic
in generated markup is not an exact neighboring source error merely because
`render_to_doc` can snap: attach the producing modifier/statement when known,
otherwise show a document-level message. Runtime/memory/storage errors have
their own codes and Retry/Save copy/source actions as appropriate.

Also report unresolved marker pairs and `SemanticIndex` occurrences using
existing scanner/index results. No new grammar, regex lint engine or speculative
kernel validation. For unimplemented execution statements, show a concise
“Execution is unavailable on this Android frontend” diagnostic and preserve
their content. Do not run code to test a document.

The status error/warning count opens a diagnostics list. Selecting a located
item reveals/selects source and adds a return checkpoint. Source stays editable
when typesetting fails. The last valid page may remain visible with “Preview
out of date”; stale hits cannot mutate the current document. Refresh diagnoses
only the current capture and never applies a late error to a newer document.

## C13 — Exact work and cache policy

Use one document render worker, one serial app-private recovery worker, and
one serial file-provider worker. The last extends the current `fileWorker`;
provider waits cannot block recovery. Inspector,
citation and search work use immutable inputs; expensive native rendering is
serialized with page work rather than multiplying Typst compiles across three
executors. Priority is visible page, explicit citation/comparison request,
adjacent page prefetch. New content supersedes queued obsolete render work.
Recovery and explicit saves are not discarded just because a view render was
superseded. No persistent render-job queue or polling loop.

Keep one active native compile and one replaceable latest pending capture.
Once a compile finishes, reject its result if its owner no longer matches.
Typst cancellation is cooperative between phases/pages; Java interruption is
not a promise to stop an active native call. Capture presentation options at
job creation; do not read changing Activity fields halfway through a job.
Remove the unconditional startup JIT call; expose it only through the existing
debug/device verification invocation, keeping its DenyAll policy.

Retain one worker-owned compiled `PagedDocument`, `MiniWorld`, transformation
output and source-map context for the current layout identity. Already shared
fonts/library stay shared. Build page metadata/location indexes without
rasterizing every page. Locate a requested source range and rasterize its page
directly; scrolling/search does not import the snapshot or compile again.
Retain only visible glyph indexes plus neighboring/navigation-requested ones;
drop offscreen per-glyph JSON arrays, not just their bitmaps. Track metadata
bytes separately, and release all old-layout indexes when replaced.

Raster one page per in-flight request. Define retained-pixel budget
`B = min(48 MiB, max(8 MiB, memoryClassMiB / 8))`, converting the class to bytes
before arithmetic. This covers retained Java page/comparison bitmaps and native
cached pixel images. Do not retain a second native raster cache by default.
Materialize visible page(s) and at most one adjacent page each way, evicting
least-recently-visible pages first. Reserve a working-page pixel limit
`P = min(4_000_000, floor(B / 16))`: one RGBA working page is at most B/4.
Reduce raster density to satisfy P, keeping point geometry unchanged. Recycle
rejected/evicted results promptly; never recycle a bitmap still being drawn.

Retained pixels must stay within B. Separately account for native temporary
RGBA/straight-alpha conversion, PNG/base64, Java decode and queued publication;
allow at most one working-page pipeline. This is not a bound on the whole app,
Typst's compiled document, fonts or Android/GPU internal allocations. Record
those in measured peak RSS/heap evidence. Memory pressure drops prefetch and
comparison pixels first, then offscreen metadata; current visible content is
rehydratable from the retained layout. On severe pressure discard the layout
and rebuild on the next user request rather than crash-looping retries.

Keep PNG transport initially and decode/check dimensions away from the UI
thread. Capture snapshots as JNI `byte[]`, not base64 strings; each immutable
capture is reused by recovery, formatting and layout. Remove superseded binary
string call sites after migration. JSON remains for bounded state/metadata.
Do not introduce shared mutable direct-buffer pointers or zero-copy pixel JNI
in this milestone.

Instrument capture, import, compile, glyph build, raster, encode, decode,
UI apply, cache bytes and stale-result discard. Overlay changes must compile
zero times; later page requests for a valid layout compile zero times. A new
capture with changed marks recompiles even when text revision is unchanged.
Modifier-isolation retries are counted separately and cached for that layout.

## C14 — Actions, focus, Back and status

One static action table supplies label, key, enabled predicate and invocation.
It drives the touch toolbar/overflow and hardware dispatch. Keep primary
controls Undo, Redo, Find, Navigate and an overflow for file/view/diagnostic
actions, rather than a horizontally hidden row of every shortcut. Minimum
touch target is 48 dp. Shortcut hints appear in accessible labels/help, not
as the only readable action name. No configurable keymap DSL or command palette.

| Context | Required key behavior |
|---|---|
| Native editor/search/target field | Native text/selection/composition handling first; declared document shortcuts remain available |
| Find active | Ctrl+S next, Ctrl+R previous, Enter accept, Escape cancel; Save remains a labelled file action |
| Document | Ctrl+N/O/S, Ctrl+Shift+S Save copy, Ctrl+Z/Y, Ctrl+F, Ctrl+H, Ctrl+K, Ctrl+G, F3/Shift+F3 |
| Rendered document | Movement/selection/clipboard from C03/C04; Alt+Left/Right navigation history |
| No text field composing | Tab toggles markers as today; the touch Markers action always exists |

Do not insert a tab and then delete it from composing input to simulate the
marker action. In a native field Tab first serves its normal focus/editing
contract; use the explicit Markers action if the field consumes it.

Escape first dismisses completion/suggestions, then nested citation detail,
then cancels Find, then closes inspector, then clears selection. Each press
performs one action. Android Back first lets the platform dismiss an IME; a
subsequent Back uses the same local dismiss stack, then navigation history;
only with no local action does the Activity leave. Return focus to the invoking
rendered range/field and do not reopen the IME merely to restore a position.

Status is a pure projection: document display name and `Unsaved`/`Saving`/`Saved`
for explicit-file state, page `n / total`, current search count when active,
and error/warning indicator when present. Show busy text only after 150 ms;
avoid flicker for completed work. A local-recovery failure is a persistent
actionable message, not a success toast. Truncate by grapheme, keeping the
file name readable; details expose URI and timing counters. No shell commands,
network lookups, repeated screen-reader announcements or idle polling.

## C15 — Accessible equivalents

Use Android virtual accessibility nodes for visible logical text blocks,
citation/fold controls and selection actions, backed by the same source ranges
and `PageTransform`. Provide text, logical reading order, bounds, role and
activate/edit/copy actions. Source-only content and whole-document navigation
remain reachable through native accessible lists/fields, including when page
pixels are evicted or typesetting fails. Do not expose one node per glyph or
let screen-reader navigation depend on OCR.

When accessibility focus advances beyond a materialized page, request its
metadata/page without building every earlier bitmap. Maintain focus by stable
source anchor across relayout, never by recycled ImageView identity. Announce
explicit actions and errors; progress changes are not a stream of live-region
announcements. Test non-Latin text, larger Android font scaling, TalkBack,
keyboard-only operation, high-contrast selection/caret and reduced-motion
settings. Do not claim that native EditTexts alone make the rendered document
accessible.

## C16 — Verification contracts and fixtures

Tests are assigned to their actual boundary. Use deterministic fake workers,
streams and clock for host controller tests, real mathed for Rust model/layout
tests, and the existing adb lane for views, providers and input. A small native
Android `Instrumentation` test entry point may drive actual InputConnections
and a fixture document provider through `am instrument`; extend build/device
scripts for this test APK, without introducing Gradle or a second E2E framework.
At least one real composing IME and a TalkBack walkthrough remain device checks.

| Fixture / scenario | Required assertions |
|---|---|
| `empty`, `starter` | Empty stays empty; starter is seeded only when no recoverable content exists; tap opens editing |
| `unicode` | Combining accents, supplementary characters, ZWJ emoji, CJK and RTL; grapheme-safe edit/selection and no split surrogate |
| `composition` | Candidate changes, cancellation and commit lasting >400 ms; one undo, correct persisted committed state |
| `marks` | Overlapping links, pair and legacy links, unknown attribute values, marks-only edit, snapshot/copy/save/undo round trips |
| `file-races` | New during picker, edit during save, old completion after disposal, revoked grant, external file change, provider write failure |
| `recovery` | Truncated/checksum/future-version slots, old preference mismatch, fail during slot replace, kill before/after acknowledgement |
| `search` | Smart-case, source-only hit, protected match, frozen selection scope, query edit, unloaded-page match, cancel restores full origin |
| `replace` | Multiple non-adjacent matches with untouched marks between them, protected matches skipped, stale preview refused, one undo |
| `references` | Resolved/unresolved bibliography, nested/cyclic document reference, local style/locale reset, go-to-source/back |
| `semantics` | Section/definition/uses navigation; shadowed names and unresolved lookalikes excluded from rename |
| `folds-modifiers` | Panel resize/IME during composition, broken marker pair, per-row comparison, malformed modifier isolation |
| `mixed-pages` | A4 plus custom-width/landscape pages; common draw/tap/caret/accessibility transform |
| `long-document` | At least 100 pages; direct late-page search/jump; bounded retained pixels, no per-page recompilation, forward/back scroll |
| `reading` | Edit above viewport, user Hold, rotation, font reflow, pinch focus, page collapse then Undo |
| `diagnostics` | Exact copied-source error, generated-source owner error, unlocated error, stale diagnostic rejected |
| `accessibility` | Touch-only workflow, virtual reading order, source fallback, focus after eviction and TalkBack actions |

Initial latency targets, not measured results: on the recorded reference device,
five-page normal fixtures should have p95 input/controller handling <=32 ms and
first visible current page <=1 second after the render debounce. Main-thread
PNG decoding, Typst compilation, file I/O and JIT startup are forbidden regardless
of average latency. Record device/ABI/resolution and warm/cold runs. For the
100-page fixture report layout latency and total heap/RSS rather than inventing
a whole-app memory promise; structural compile/cache budgets still must pass.

Repair selectors with stable view IDs/content descriptions and explicit
fixtures. Device e2e may clear only an isolated test profile/package's data.
Keep installed-APK identity and screenshot/log evidence. Show the stale-result
and atomic-recovery tests catch a deliberately disabled guard once. No source
substring assertion substitutes for a lifecycle test. Missing IME/provider/
TalkBack/physical-device coverage is reported as unverified, never host-green.

## Explicitly unselected work

No execution notebook, remote collaboration transport, agent conversation
tree, terminal emulator, shell history, prompt engine, plugin/LSP system,
multi-document workspace, PDF export, import-time kernel execution or UI toolkit
rewrite is part of this specification. Existing statements/snapshots continue
to round-trip even where execution is unavailable. Those features require a
separate workflow showing why the existing document, worker, transport or file
path cannot meet the requirement. Their absence does not leave any selected
feature above unspecified.
