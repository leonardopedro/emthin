# Android review and development plan

Reviewed and expanded 2026-11-15. Scope: `emthin-android`, its shared document/rendering
dependencies, and the Android build and verification lanes. This is a plan;
the tasks below have not all been implemented by this review. Desktop and
shared-core work is tracked separately in [../DEVELOPMENT_PLAN.md](../DEVELOPMENT_PLAN.md).
The [MM0 proof workflow fixture](../DEVELOPMENT_PLAN.md#mm0-proof-workflow-fixture)
adds a cross-app Android acceptance run to the existing disposable lane.
The [native Kyneta plan](../docs/KYNETA_NATIVE_PLAN.md) adds offline
collaboration, peer readiness, history and proof-note views through the
shared Rust model and existing JNI bridge. It also specifies an AustralVM
rule candidate and the device acceptance run.

The [native terminal figure plan](../docs/HERDR_INTEGRATION_PLAN.md) defines
the later Herdr Core and OpenShell work. The scope exclusions below apply
to this editor milestone.

## Preview release check

Run `./device-lane.sh math-preview` against the disposable APK with Python
Pillow installed. It captures a note with a displayed equation and verifies
visible ink inside the preview's accessibility bounds. It then taps the
equation, edits its source, and checks that Undo restores both text and
source focus. The 2026-11-22
Waydroid trial found a blank preview even though the bitmap and hit map
arrived. The page now draws its bitmap at the same explicit dimensions
used by the caret, touch and terminal figures. Keep this check alongside
the scroll, IME and reflow lanes. Repeat the phone acceptance steps in
the [editor release checks](../DEVELOPMENT_PLAN.md#editor-release-checks).

Open, Save, Undo, Find and Edit stay in the toolbar. Edit opens the existing
source editor at the caret and requests the keyboard. More groups document,
navigation, view and session actions. Compact windows and large fonts use
two toolbar rows. At 360 dp on a phone, verify that a user can start editing
an equation, undo and save through Android's file picker.

Ordinary free variables stay available as source-linked notes under More,
View, Diagnostics. The global banner counts actionable errors and warnings.
The math fixture verifies that valid equations keep the banner quiet, and
the diagnostics fixture verifies that real warnings and modifier errors
still appear and lead to their source.

## Execution status

Updated 2026-11-17. Checkpoint M0 and cards A2–A7 are implemented; A8 is partially implemented (touch selection, handles and virtual reading nodes; TalkBack binding, an automated spoken-page utterance check through `TALKBACK_APK`, and CJK/emoji InputConnection composing are covered, while a full human spoken walkthrough remains); A9 and A11–A15 are implemented; A10 is complete with a separate disposable test package, automated TalkBack coverage and a real composing-IME path.

| Card | Status | Evidence / boundary |
|---|---|---|
| A0 | done | `citation_detail` now passes `None` and `citations::DEFAULT_CITATION_LOCALE`; the Android host suite compiles and passes. |
| A1 | done (host lane) | `Cargo.lock` gained only the `sha2` direct dependency from `austral_cranelift_bridge`; the Android CI job now checks out `australVM` and logs the three sibling commit ids. `cargo test --locked` passes 75 tests. |
| A2 | done (host lane) | `PadSession` owns document/instance/mutation/request identities; `RenderController` keeps one active job and one replaceable newest pending job per lane. Page, inspector, citation, Open and Save callbacks accept only their original owner. Startup JIT moved behind the `emthin.jitSmoke` intent extra used by `device-lane.sh`. `test-java.sh` passes 18 ownership checks; Java compiles against the real Android platform jar with `-Xlint:all`. |
| A3 | done (host lane) | `RecoveryCodec`/`RecoveryStore` implement the two-slot `EMPADRC1` envelope with AtomicFile writes, SHA-256 checksums, bounds checks, quarantine of rejected bytes, and a view sidecar. Startup inspects both slots on the owned recovery worker, migrates legacy `docSnapshot`/`docText` once, and offers export/open/start-new or text-without-marks choices instead of seeding over unreadable data. Recovery captures coalesce to one in-flight plus one owed latest; New/Open wait for a checkpoint. `snapshotsBytes`/`logicalFingerprint` bridge canonical rich text without Loro history. Host tests add 29 codec checks and one Rust mark-only fingerprint test. |
| A4 | done (host lane) | Save and Save copy use the actual immutable snapshot written; `savedFingerprint` is separate from recovery's durable fingerprint, so an older completion cannot clear newer edits. Picker launches bind document/instance identity, recreated Activities discard them, and New/Open reject stale completions. Open remembers provider content SHA-256, size and last-modified hint in prefs; Save re-hashes on the provider worker and offers Save copy/Replace/Cancel on an outside change, with a recheck before replacement. Provider reads/writes use strict UTF-8, the 32 MiB bound, `.mathed`/Loro-header validation, one active operation plus one owed same-URI save, a 1 s Cancel action, and separate recovery worker. `ProviderIoTest` adds 16 stream/hash/save-identity checks; Java compiles with `-Xlint:all`. |
| A5 | done (host lane) | `sync_text` expands minimal edits to whole grapheme clusters using the same UAX #29 rules as `DocModel`; the bridge exposes `graphemeBoundary` for rendered movement, deletion and selection. Native text fields keep IME composing candidates local, suppress marker/completion rewrites and model/recovery sync until commit, flush before Save/New/Open/inspector close, and preserve composition across deferred panel refreshes. Typing/paste over a selection and cut/paste are one marker-safe replacement; rendered deletion uses grapheme boundaries. Rust tests cover ZWJ families, combining accents, surrogate landings, RTL and an A5 wiring guard; Rust passes 79 tests, Java compiles with `-Xlint:all`. Stable Loro cursor anchors and touch-handle selection remain assigned to A7/A8; the real composing-IME path is covered by `device-lane.sh composing-ime`. |
| A6 | done (host lane) | The render worker prepares one retained Typst layout from raw snapshot bytes and lazily rebuilds per-page glyph indexes; later chunks rasterize without importing or compiling again. The layout identity covers the snapshot digest, marks, marker visibility, explicit folds and presentation width; caret/selection/search overlays repaint pages without a layout request. The app no longer uses the base64 snapshot render call. PNG/base64 decode and pixel sampling moved to the worker, and pages publish only from the newest owner. Old pages stay visible while a replacement is pending but their hit positions cannot edit the newer document. Rust tests prove one compile across chunks, cached/independent raster and hit agreement, presentation/mark invalidation, and range location; layout, raster, decode and UI-apply costs are logged separately. Rust passes 84 tests; Java compiles with `-Xlint:all` and host tests pass 18 ownership, 30 recovery and 16 provider checks. |
| A7 | done (device lane) | Stable selection and reading anchors use Loro's own cursors through `MathDoc::create_anchor`/`resolve_anchor`, not a parallel offset map. The selection's fixed end and the top-visible rendered text offset resolve after each accepted mutation; the reading anchor and fallback are persisted in recovery metadata and restored after relaunch/page replacement. Anchor records are document-bound and reject another document. Page streaming now builds one placeholder per page and materializes only the visible page plus one neighbour; Java bitmaps are trimmed by a memory-class byte budget with `onTrimMemory`/`onLowMemory` response, and the native glyph index keeps the same page window. Rendering returns point page sizes, and taps/caret/selection use each page's own transform. Reading size reflows through the retained Typst layout (`theme_prelude(text_size)`) and zoom is a 1–4× horizontal-panning view transform; both persist. A late-page anchor/range is resolved through `locate_range` and only its target page is rasterized. Rust passes 84 tests, shared mathed_core/mathed_mini pass, Java compiles with `-Xlint:all`, host tests pass 18 ownership, 30 recovery and 16 provider checks, and the device lane passes persistence, New/Undo and the page-2 collapse/undo reading anchor. |
| A8 | partial (device lane) | Rendered pages use the platform long-press timeout to select a word through mathed's `wordBoundary`, and a drag extends the same anchored selection by grapheme; a contextual Copy/Cut/Paste/Select-all bar calls the existing marker-safe rendered clipboard path. Start/end selection handles are drawn from the same hit transform and drag either endpoint by grapheme without entering history. Page scroll ownership still wins a drag before the timeout, and the lane's swipes remain separate from taps. Each loaded page exposes source-backed paragraph nodes through an `AccessibilityNodeProvider`: text and bounds come from page hits/`PageTransform`, clicking a node opens its citation, fold or source inspector, accessibility focus follows a source offset, evicted pages expose no children, and focusing a placeholder requests only that page. Rust passes 95 tests; Java compiles with `-Xlint:all`; host tests pass 18 ownership, 30 recovery, 16 provider, 17 Find-controller and 27 navigation-controller checks; the full device lane passes. Manual device checks selected a word, rendered and dragged the handles (`caretMove` through the selected range), showed the contextual bar, cut Select-all and restored with Undo. TalkBack 15.1 (`com.android.talkback`, F-Droid build `370044211`, minSdk 26, targetSdk 30) is exercised by `device-lane.sh accessibility`. With `TALKBACK_APK` pointing at an APK it installs when absent, dismisses the fresh install's first-run tutorial with keyboard focus plus Enter, pauses the TTS engine after the tutorial so TalkBack logs the exact utterance, launches a `Spoken reading` fixture, and asserts the secure-settings component, a bound TalkBack service, `[com.emthin.pad.test]` in the client list, the rendered `Page 1 of 1` and text nodes, and TalkBack's `fragments:[{text:Page 1 of 1` utterance. It then restores `accessibility_enabled`, `enabled_accessibility_services`, `touch_exploration_enabled`, the TTS engine and the two package-verifier settings, and uninstalls TalkBack only when it installed it. The verified end state is `accessibility_enabled=0`, `enabled_accessibility_services=null`, `touch_exploration_enabled=0`, no TalkBack package, TTS enabled and verifier settings `null`; without an APK or existing install the step skips explicitly. Full e2e passes with the install/uninstall in the middle of the lane, and the byte-identity check still passes. Waydroid flagged the older target SDK; it was not a personal-device recommendation. CJK and emoji composing now run through the test IME in A10. A human spoken walkthrough remains outstanding. |
| A9 | done (host + device) | `ActionCatalog` is the one descriptor table for toolbar order, visible labels and Ctrl+S context: toolbar buttons and their key bindings call the same `runAction`. The Save label reads `^S Save (Find: next)` normally and flips to `^S Find next (Save: close Find)` while Find is open; the Find row says `^S Next in Find`. `ActionCatalogTest` covers 46 assertions for unique toolbar ids, labels, marker state and Ctrl+S resolution. The status line composes the document name (`Untitled` before Save/Open), durable state (`restoring`, `saved`, `unsaved`, `saving`, `saving copy`, `opening`), current detail, Find match count while Find is open and diagnostic count, with revision/segment/property data moved to the `status` logcat trace and copy/save/open failures updating the visible line. Source-only Open preserves the file name and persists it; `device-lane.sh named-document` opens a real 21-byte `named-source.txt` and asserts the status name. Focus audit: search Back now recognizes the replacement field as well as the query, an inspector Back with an active IME dismisses the keyboard before closing the sheet, and dialogs retain their own Back. `device-lane.sh focus-stack` proves Find+IME Escape, dialog Back and inspector Escape; Waydroid's IME can consume the first synthetic Escape, so that check accepts a bounded retry. Java compiles with `-Xlint:all`; host checks pass 18+30+16+17+27+22+9+12+46; full e2e passes (normal APK md5 `a6a6e7e3d63911ecb74bc8e659b3d8ef`; test APK `627f4bb417d7a6772d386ec7468187fe`). |
| A10 | done (lane) | The device lane no longer looks for the first `EditText` or assumes the 89/90-byte starter. Toolbar actions are selected by `ActionCatalog`’s visible labels, text fields by content descriptions, and deterministic content comes from explicit intent/file fixtures. Edit expectations now come from the observed `textLen` trace, so persistence, New/Undo, collapse/Undo and Replace/Undo derive their lengths instead of hard-coding them; the named-source fixture derives its expected size from the pushed file. `build-apk.sh` can build a second `com.emthin.pad.test` artifact (`build/test/emthin-pad-test.apk`) from the same sources via `aapt2 --rename-manifest-package`; the lane defaults to that package and its own app data, so `pm clear` cannot touch the normal install. A non-test package still requires `EMTHIN_LANE_DISPOSABLE=1`. Existing coverage keeps late-result ownership and corrupt-recovery checks in host tests (`OwnershipTest`, `RecoveryCodecTest`), while the device lane covers persistence, New/Undo, page collapse, Find/Replace, named SAF source, focus/IME and diagnostics/modifier rendering. Full e2e passes against the test package (APK md5 `627f4bb417d7a6772d386ec7468187fe`); the normal APK is `a6a6e7e3d63911ecb74bc8e659b3d8ef`; Java host checks pass. The device lane also installs, exercises and restores TalkBack when `TALKBACK_APK` is supplied. A test-only IME in the disposable APK runs `device-lane.sh composing-ime`: it opens the visible source field, calls `setComposingText("你好")`, asserts the uncommitted field text, calls `finishComposingText`, asserts the model's `applyState`, then refocuses and calls `commitText("😀")`, asserting both the field entity `&#128512;` and the model update. Previous IME selection is restored. F10 is resolved. |
| A12 | done (device lane) | `semantics.rs` projects mathed's existing `SemanticIndex`/`DocumentContext` into definitions, resolved use sites, sections, unresolved names and references. `NavigationController` owns a 30-entry per-document return list with adjacent deduplication, Forward truncation, stable selection/reading anchors and fold context; New/Open reset it. Navigate has Sections/Definitions/References tabs (`Ctrl+G`, `Ctrl+K`), source destinations and resolved-use lists. Accepted search/section/definition/use/reference jumps push history; Back/Forward are toolbar actions and Alt+Left/Right when a text field is not focused. Rename uses a separate checked native plan: one math identifier, concrete name range, no duplicate definition/lookalike outside the plan, and a simulated rebuilt index before one grouped undo. Rust passes 92 tests (four semantic/rename checks); Java compiles with `-Xlint:all`; host checks pass 24 navigation-controller assertions; the full device lane passes Accept/Back/Forward and Replace-current/Undo (final APK md5 `e8d421506c6fc962b64c8ec486346fa0`). Manual device checks opened Navigate and switched Sections/Definitions/References. Rename planning, shadowed-name exclusion and grouped undo are covered by native tests; a full device rename was not completed because the Waydroid image's rendered starter has no editable definition fixture. |
| A13 | done (device + native) | Citation index rows now carry key, source span, short source snippet, resolved status and status error. `citationDetail` accepts presentation-only style/locale overrides through the shared bibliography formatter and returns number/label/keys/source metadata plus the full citation and entries. The dialog uses the shared `CITATION_STYLES`/`CITATION_LOCALES` arrays from Rust, adds style/locale selectors and Reset, persists overrides per recovered document id, restores them after a cold launch, and reports source/Go to source/Edit source. A host-testable `CitationController` owns paths, breadcrumb labels, reference-identity cycle detection, the 16-level limit, formatting generations and filter matching. Nested details carry Back, and closing without navigation restores the invoking caret and reading position. The References tab resolves status and filters by number, label, key, snippet or status. Rust passes 92 tests including style/locale metadata and unresolved-index status; Java compiles with `-Xlint:all`; host checks pass 18 ownership, 30 recovery, 16 provider, 17 Find, 24 navigation and 22 citation checks. Manual device checks constructed a document-reference citation, opened the resolved `[1]` reference row with its `\cite(#3,#4)` snippet, opened the detail with formatted source, selected `vancouver`, reset to Document default, and confirmed the selected style survived force-stop/relaunch. Nested cycles/depth and unresolved-key suggestions are covered by controller/native tests; a multi-entry Hayagriva bibliography was not exercised on the device. Full device lane passes (final APK md5 `0d299680a1710c519d3752eab16ca8fd`). |
| A14 | done (host + native) | Marker-pair commits now preserve the active partial pair, accept optional `#` prefixes, validate complete pairs, and collapse duplicates instead of failing. The target field suggests the markers that can complete the current pair, with snippets. The inspector lists incoming and outgoing relationships with marker ids, snippets, resolved state, Go to modifier/target and one-step removal; legacy links remain removable. Comparison rows expand individually, reuse a six-result access-ordered cache, and the panel switches to aligned columns at 600 dp with a Show document escape. A14 extraction `InspectorView` is host-tested: nine checks cover the 599/600 dp boundary, density, LRU eviction and density in cache keys. Rust passes 92 tests including incoming/outgoing relationship metadata and row rendering; Java compiles with `-Xlint:all`; host checks pass 18 ownership, 30 recovery, 16 provider, 17 Find, 24 navigation, 22 citation and 9 inspector checks. The full device lane passes on the A14 APK (md5 `8842655d1ef07fffbbb0c8a90fd97084`). The A14 inspector interaction itself was not manually exercised on this image; below/above-breakpoint resizing with a live target composition remains a manual check. |
| A15 | done (host + native + device) | `diagnostics.rs` reuses the scanner, `SemanticIndex`, modifier marks and Typst diagnostics; it creates no second parser or lint engine. Every record carries `id`, severity, code, message, capture id, UTF-16 span, marker pair and `exact`/`owner`/`document` provenance. `layout.rs` retains diagnostics in layout metadata, captures Typst warnings instead of discarding them, returns `{error, diagnostics}` for fatal layout failures, and attributes isolated modifier failures to their source markers. Kernel/execution statements report “Execution is unavailable on this Android frontend” while preserving their source. The status error/warning count opens the diagnostic list; selecting an exact or owner record navigates to the indicated source through the existing history, while detached/generated spans are never presented as exact neighboring text. Render start marks old pages as out of date until the newest accepted result publishes its diagnostics. Rust passes 95 tests, including exact versus detached Typst spans, static semantic/scan diagnostics and modifier-owner attribution. A repeatable `device-lane.sh modifier-inspector` fixture supplies text, two links, targets and a caret marker through intent extras (base64 for spaces/`#`); it asserts incoming and outgoing rows and the expanded rendered comparison. A separate `device-lane.sh diagnostics` step exercises the exact and owner paths: the banner and status count open the list; an execution warning and unresolved name are marked `exact`; selecting the unresolved name navigates to offset 71 and opens the whole-document source; Back returns to the rendered page and closes that panel. An invalid `#3 nope #4` modifier produces `typst` and `modifier-application` rows marked `owner · #3 #4`; selecting one navigates to the marker range at offset 15, and Back restores the modifier-at-caret panel and its `1 2` target list. Device testing found two history-restoration bugs, both fixed: `NavigationController.Entry` now records `wholeDocumentEditing`, so Back no longer reopens a diagnostic origin as whole-document source; and a non-inspector destination now closes the source panel that its predecessor opened. Both steps pause TalkBack only for synthetic taps and restore it afterwards. Java host checks pass 27 navigation assertions and 12 status-model checks; the full e2e lane passes (final APK md5 `a6a6e7e3d63911ecb74bc8e659b3d8ef`). |
| A11 | done (device lane) | `mathed_core::search::find_matches_in` adds a range-limited literal query without changing existing callers. Android's `searchScoped` runs smart-case scan/filter/count off the main looper and returns only a bounded 200-result page; Text excludes marker/property ranges, Source includes them with a protected flag, and Selection freezes its non-empty endpoints. `FindController` owns scope, current match, wrap cue, stale-request rejection and plan requests; the Find panel adds scope, Next/Previous/Accept/Cancel, source snippets with marked matches, a results list, and full origin restoration (selection, reading anchor, viewport, folds, inspector). Replace/Replace all use one checked native `replace_many` batch, revalidate the expected text and protected ranges, present replaceable/protected counts with up to 20 preview snippets, refuse plans over 10,000 edits, and clear stale previews. Manual device checks covered 1/1 results, Source `#3` (`1 / 2 · Accept opens source`), frozen Selection scope, Replace current (89-byte source updated), Replace all preview/apply of 2 occurrences, and one Undo restoring the 89-byte original and its pre-replacement selection; Escape, F3 reuse, two-step Back/IME dismissal, and exact Cancel restoration of the page bounds on a clean launch were also exercised. Rust passes 88 tests; Java compiles with `-Xlint:all`; host checks pass 18 ownership, 30 recovery, 16 provider and 17 Find-controller; the full device lane passes (final APK md5 `e8d421506c6fc962b64c8ec486346fa0`). |

Local sibling revisions for this run: velysterm `423a8c453128`, unfer `8da328b18c91`, australVM `95eb3bafb52d`. The APK/device lane was
run on Waydroid: a fresh arm64 APK built and installed, and install, launch,
JIT, render-chunk, caption-scroll, swipe and installed-byte identity pass. The
persistence, New/Undo and scroll-anchor checks also pass on the rendered page
surface: relaunch restores document, caret and reading y; New resets and Undo
restores; a page-2 scroll clamps on a one-page collapse and Undo returns to the
retained y. The lane's two-page assertion matches A6/A7's one-page demand
chunks. Manual device checks pasted `中文👨‍👩‍👧‍👦` into the real source field
and cut/pasted it unchanged, exercised the A+/A− and Zoom+/Zoom− controls
(size, reflow, 1.5× page width and scaled reading position), and exercised
long-press selection, the contextual bar, Select all/Cut and Undo. CJK/emoji
composing now goes through a test-only IME in the disposable APK, so it does
not depend on Waydroid exposing third-party IME windows. The
`composing-ime` e2e step opens the source field, composes `你好`, finishes it,
commits `😀`, and asserts the field and model updates. The `accessibility` e2e step
installs TalkBack 15.1 from `TALKBACK_APK` when needed, dismisses its first-run
tutorial (keyboard focus plus Enter), pauses TTS to capture the exact spoken page
caption, then restores the accessibility, TTS and verifier settings and
uninstalls only its own install. It ends with
`accessibility_enabled=0`, no enabled services, no touch exploration and no
TalkBack package. A human spoken walkthrough remains outstanding. No implementation card remains queued; only
A8 retains a device-only acceptance gap. The GAPPS image is downloaded and hash-verified but the
system-image swap still requires the unavailable root password.

The selected scope is a reliable, responsive Android document editor with
complete search/replace, return navigation, semantic source editing, references
and modifier inspection. These extend the rendered editor already present.

Read these three documents as one handoff:

1. [FEATURE_SPEC.md](FEATURE_SPEC.md) defines the exact selected behavior,
   controls, state ownership, bridge boundaries, limits and test scenarios.
2. [SIBLING_STUDY.md](SIBLING_STUDY.md) records source-level findings from all
   nine requested projects, adopted mechanisms and rejected alternatives.
3. This document supplies the current findings, dependency order, task cards
   and cheaper-model execution instructions.

The feature specification resolves choices left general in the initial review.
Its C00–C16 contracts govern implementation details; the cards below divide
that scope into reviewable changes. No sibling runtime is imported wholesale.

## Review baseline

The reviewed emthin HEAD was `a49c822e9c52d9d8e71c631cf011167395c95d6d`, with
existing desktop/IPC/collaboration changes in the working tree. Android had
no source changes. Preserve those unrelated changes. Dependency checkouts
were velysterm `423a8c4`, australVM `f763adfd`, and unfer `d5123ae`.
These identify local observations, not a verified release dependency set.

| Check | Result observed in this review |
|---|---|
| `bash scripts/verify-invariants`, repository root | 7 passed, 0 failed, 0 gaps |
| `cargo test --locked`, Android directory | Failed before compilation: lockfile needs updating |
| Offline dependency resolution in a temporary Android copy | Lockfile change adds `sha2` to `austral_cranelift_bridge` dependencies; original lockfile untouched |
| `cargo test --offline` in that copy, with the installed native GCC on PATH | Compilation fails with E0061 at `src/lib.rs:382`: bibliography helper now requires four arguments; Android supplies two. No test verdict |
| Both Java sources, installed JDK 21, `javac --release 8`, Android 37 platform jar | Compile successfully; three warnings about obsolete source/target 8 options |
| Full APK build, installation, device/IME/visual checks | Not run; no device result claimed |

Diagnostic logs are in `/tmp/emthin-pad-review-{tests,offline-tests,javac}-2026-11-15.log`.
They are temporary local evidence. The implementation must attach fresh logs
to its own results. Old README/AGENTS counts are historical, not today's test
counts. The offline check initially lacked `cc`; supplying the installed GCC
removed that environment problem and exposed the actual Rust compile error.

### Existing work to continue

The [Android README](README.md), [feature inventory](../docs/MATHED_FOOT_PARITY.md)
and current source supersede the earlier description of a plain source editor
with a preview. The [October Android plan](../workspace-backup/PLAN_EMTHIN_ANDROID_MATHED_2026-10-18.md)
records the rendering integration and subsequent fixes; do not repeat them.

Already present:

- The desktop `DocModel` included directly, Loro snapshots and modifier marks,
  undo/redo, autosave, caret/reading-position restoration, and confirmed New.
- Rendered pages as the main surface; source/modifier comparison columns;
  glyph hit positions; keyboard navigation, selection and clipboard.
- Shared mathed incremental search and word navigation, math completion,
  marker naming/visibility, folding, and expandable citations.
- Android document-picker open/save, `.mathed` snapshots and UTF-8 import.
- Background rendering, debounce and stale-generation checks, page chunks,
  PNG transport on the active snapshot-render path, and bitmap release on rebuild.
- A host-test lane and an adb lane with persistence, New/Undo and page-collapse
  scroll checks. A constant-return Cranelift startup smoke also exists.

These are implemented paths, not a claim that all are currently verified.
The shared citation API change currently prevents the Rust test binary from
building. The JIT smoke is not a notebook execution frontend.

## Findings and priority

Source references below use symbol names so tasks survive line-number drift.
“Observed” means a command was run; “source” means a code-path finding;
“device check” marks behavior that still needs reproduction on Android.

| ID | Priority / evidence | Finding and consequence |
|---|---|---|
| F1 | P0, observed | [lib.rs](src/lib.rs), `citation_detail`, calls `citations::bibliography_detail(text, number)`. The [shared helper](../crates/emthin/src/docui/citations.rs), included by path, now takes style and locale too. Current Android compilation fails. |
| F2 | P0, observed/source | Android's lockfile is stale against the current australVM manifest. The [CI Android job](../.github/workflows/ci.yml) clones velysterm and unfer, but not australVM, despite the direct path dependency in [Cargo.toml](Cargo.toml). The missing checkout is a clean-runner defect inferred from source; hosted CI was not run. |
| F3 | P1, source | [MainActivity](android/src/com/emthin/pad/MainActivity.java), `onCreate`, silently creates a text-only model when `createFromSnapshot` returns zero. `applyState` then calls `saveDocument`, replacing the recovery snapshot. An unreadable or mismatched snapshot can therefore discard marks while retaining apparently correct text. |
| F4 | P1, source | `applyState` snapshots and saves on every update; `startRender` snapshots again. Full snapshot encoding occurs on the UI thread, even though the preference write is asynchronous. Recovery data and view metadata have no shared versioned identity. Large-document latency and lifecycle races need measurement. |
| F5 | P1, source | `layout_page_chunk` creates a new `MiniWorld` and compiles the whole document for each chunk. Chunking limits raster count per response, not layout work or total resident memory. `applyChunk` decodes PNGs and scans bitmap pixels on the main thread; all loaded page views/bitmaps remain until `releasePageBitmaps` on rebuild. |
| F6 | P1, source | Unguarded `CaretEditText.onSelectionChanged` schedules rendering, while the active `rendered_only` path suppresses caret-driven expansion. `setDocumentCaret` already avoids this through `syncingCaret`; preserve that working path and extend the distinction. `startRender` removes every page before replacement output arrives. |
| F7 | P1, source/device check | `moveRenderedCaret` advances by code point at glyph ends, unlike the shared model's grapheme-aware editing. Source `TextWatcher`s perform marker/completion rewrites without explicitly checking Android composing spans. Combining characters, emoji sequences, CJK composition and cancellation need real input-connection tests. Existing 400 ms undo coalescing must be preserved. |
| F8 | P1, source | Workers have separate partial generation guards. `onDestroy` calls `shutdown`, which allows queued work to finish, without invalidating render/inspector generations. Picker requests are not bound to the document that launched them. Save completion tracks document generation, not whether later edits remain unsaved. |
| F9 | P2, source/device check | `RenderedPage` is an `ImageView` with custom drawing/touch handling, without a rendered-text accessibility provider. Rendered selection is exposed mainly through keyboard commands. Native source fields help, but do not give equivalent document reading, citation and selection access. |
| F10 | resolved by A10 | The device lane now uses toolbar labels, content descriptions and explicit fixtures, derives starter/edit lengths from `textLen` traces, and guards `pm clear` behind `EMTHIN_LANE_DISPOSABLE=1`. Separate disposable app data, optional TalkBack install/restore and the real CJK/emoji composing path are now in the lane. |
| F11 | P2, source | Geometry assumes `PAGE_WIDTH_PT = 595.28` in hit conversion and caret drawing, while Typst can produce other page sizes. Raster height follows the actual dimensions but hit conversion assumes A4. Existing scaling needs one consistent transform and readable phone-sized presentation. |

An important cache constraint: `MathDoc::revision` in
`../../velysterm/crates/mathed_core/src/doc.rs`
changes for text, **not marks**, and resets for a new snapshot-backed instance.
It is insufficient by itself for render, recovery or saved-state identity.
Modifier-only changes must invalidate rendering and count as unsaved changes.

## Inspiration mapped to existing Android work

These are local source observations at the listed revisions. No upstream
feature or performance claim is being inferred from the project name.
Sibling source paths are relative to this document and written as code so
the repository's link gate also works without those review-only checkouts.

| Project / local revision | Specific source inspected | Transfer to the pad | Boundary |
|---|---|---|---|
| kyneta `83d6e0d0` | `../../kyneta/ARCHITECTURE.md`, `../../kyneta/packages/machine/README.md`, `../../kyneta/packages/transport/src/task-scope.ts` | Explicit state transitions, effects outside state, lifecycle-owned jobs, and change-specific invalidation; tasks A2, A3, A6 | Keep Loro and the existing JNI seam; no TypeScript runtime or generic substrate layer |
| pinetrie `0f4483f4` | `../../pinetrie/docs/adrs/0027-column-layout.md`, `../../pinetrie/docs/adrs/0213-tests-name-their-requirements.md` | Keep fold/inspector/reading state local, version recovery metadata, name verification requirements and report gaps; A3, A10 | No conversation tree, containers or agent-management UI |
| desktop (Atuin Desktop) `06102dc` | `../../desktop/README.md`, `../../desktop/src/state/runbooks/snapshot.ts`, `../../desktop/src/state/runbooks/operation_processor.ts` | Separate document identity, durable content and derived UI; explicit saved revision; A3, A4 | Executable runbooks are outside this Android editor milestone |
| fresh `4a5940ed6` | `../../fresh/docs/internal/buffers-splits-undo.md`, `../../fresh/crates/fresh-editor/src/view/viewport.rs`, `../../fresh/crates/fresh-editor/src/app/async_dispatch.rs` | Document-relative reading anchors, bounded UI work, late-result rejection and recoverable edits; A2, A3, A7 | Reuse techniques, not Fresh's buffer/split/plugin system |
| foot `152487e4` | `../../foot/render.c`, `../../foot/search.c`, `../../foot/ime.c` | Separate overlay repaint from content work; preserve search cancel/accept semantics and composing input; A5, A6, A9 | Terminal protocols and PTYs remain terminal-client concerns |
| helix `a714a818a` | `../../helix/helix-core/src/transaction.rs`, `../../helix/helix-view/src/document.rs` | Intent-based edits, selection mapping and coherent undo around IME/replace operations; A5, A8 | No modal editor fork or replacement document model |
| atuin `6e57c7e3d` | `../../atuin/crates/atuin/src/command/client/search/keybindings/keymap.rs`, `../../atuin/crates/atuin/src/command/client/search/engines/db.rs` | Make existing actions context-aware and search predictable; A9 | Do not add shell-history storage or replace mathed search with fuzzy search |
| starship `e3b64da0` | `../../starship/src/context/mod.rs`, `../../starship/src/configs/starship_root.rs` | Cache work by inputs; show compact, relevant save/render/error state; A6, A9 | No shell prompt, arbitrary status commands or settings framework |
| Infinity `e1a3e4b` | `../../Infinity/crates/infinity-agent-core/src/system/local/driver.rs`, `../../Infinity/crates/infinity-agent-core/src/system/defer.rs` | Give work one owner, reject late completions, and stay idle without polling; A2 | No embedded agent runtime, network protocol or durable job replay for page renders |

The foot/mathed behaviors already adopted are recorded in the existing
[inventory](../docs/MATHED_FOOT_PARITY.md). The desktop collaboration work in
[collaboration.md](../docs/collaboration.md) is also already underway in this
working tree. Neither is a reason to reimplement its primitives in Android.

## Implementation sequence and task cards

All selected features improve the existing editing, file, navigation, search,
reference, relationship or rendering paths. They include explicit extensions
such as literal replacement and return history; their complete scope is fixed
in C00–C16. Independent agent/terminal/notebook subsystems remain excluded.

Sizes are implementation scope, not elapsed-time promises: S = one focused
change; M = two or three focused changes. Each change should leave a buildable
tree and carry the evidence appropriate to its boundary.

| Order | Task | Depends on | Size | Implementer |
|---|---|---|---|---|
| 1 | A0 Repair the shared citation seam | — | S | Cheaper LLM |
| 2 | A1 Repair the reproducible Android verification baseline | A0 | S | Cheaper LLM |
| 3 | A2 Own asynchronous work across document/activity lifetimes | A1 | M | Cheaper LLM |
| 4 | A3 Make recovery preserve the whole document | A2 | M | Cheaper LLM; review storage contract once |
| 5 | A4 Make document-picker saves and switches truthful | A2, A3 | S | Cheaper LLM |
| 6 | A5 Complete Unicode and composing-input correctness | A1, A2 | M | Cheaper LLM; review shared editing API changes |
| 7 | A6 Separate overlays, layout and raster work | A2, A5 | M | Cheaper LLM; review cache contract once |
| 8 | A7 Bound page memory; implement reading size, zoom and anchors | A3, A5, A6 | M | Cheaper LLM |
| 9 | A11 Finish scoped search and literal replacement | A5, A6, A7 | M | Cheaper LLM |
| 10 | A12 Add return history and semantic navigation/rename | A5, A7 | M | Cheaper LLM |
| 11 | A13 Complete citation navigation and presentation choices | A0, A2, A12 | M | Cheaper LLM |
| 12 | A14 Complete modifier inspection and adaptive source controls | A5, A6, A7, A12 | M | Cheaper LLM |
| 13 | A15 Add structured source-linked diagnostics | A6, A12, A14 | M | Cheaper LLM |
| 14 | A8 Complete touch and accessible rendered editing | A5, A7, A11–A15 | M | Cheaper LLM |
| 15 | A9 Unify actions, focus and useful status | A4, A8, A11–A15 | S | Cheaper LLM |
| throughout / final | A10 Repair and extend the existing device lane | Start after A1; extend with every card | M | Cheaper LLM; owner supplies device access if unavailable |

Land four checkpoints: M0 (A0–A1) restores the build; M1 (A2–A5) establishes
recovery, files and editing correctness; M2 (A6–A7) establishes responsive,
bounded rendering and readable pages; M3 (A11–A15, then A8–A9) completes the
selected navigation/search/relationship workflows and their touch/accessibility
paths. Extend A10 at every checkpoint. Each checkpoint needs its own relevant
boundary evidence; M3 is not a reason to postpone basic device input checks.

### A0 — Restore the shared citation call

Entry points: `citation_detail` and citation tests in [lib.rs](src/lib.rs);
the included [citations.rs](../crates/emthin/src/docui/citations.rs).

Pass `None` and `citations::DEFAULT_CITATION_LOCALE` explicitly, as C00 states.
This is an adapter fix;
do not add a citation-settings UI, duplicate bibliography resolution or change
the shared helper merely to preserve the old Android arity.

Acceptance: Android reaches test execution after A1's lock resolution;
bibliography, document references, unresolved keys and nested citation tests
retain their meaning. Include a citation render/open device check in A10.

### A1 — Establish the real Android baseline

Entry points: [Cargo.lock](Cargo.lock), [Cargo.toml](Cargo.toml), Android job in
[ci.yml](../.github/workflows/ci.yml), [AGENTS.md](../AGENTS.md).

Update the Android lockfile against the intended current dependency checkouts;
review the diff rather than updating unrelated dependencies. Add australVM and
any transitive path checkouts actually required by its selected features to
the Android CI job. Record exact sibling revisions in verification output and
use an explicit compatible revision set for repeatability; shallow `main`
alone is not a compatibility contract. Keep Android's separate workspace and
Rust toolchain. Correct stale Android descriptions/counts only from fresh logs.

Acceptance: a clean checkout with the recorded siblings runs Android
`cargo test --locked`; source scans cannot substitute for this. Compile Java
with the real platform jar. A missing native compiler/SDK/device is reported
as that specific requirement, with no test-pass claim. Do not force a Gradle
migration or an APK build on every host-only change.

### A2 — Make asynchronous ownership explicit

Normative scope: C13 and the identity/owner tables. Capture native JIT smoke
only through the debug lane; remove it from normal startup. Use one render
worker and separate recovery/provider workers with C13's explicit ownership.

Entry points: `onDestroy`, `requestChunk`, `requestModifierComparisons`,
citation dialog callbacks, `readDocument`, `writeDocument` in
[MainActivity.java](android/src/com/emthin/pad/MainActivity.java).

Introduce a small testable coordinator for document-instance identity,
content/mark change identity, presentation changes, request ID and disposed
state. Keep `DocModel` ownership on the UI thread; workers receive immutable
inputs. Invalidate the relevant identities on New/Open, destruction, changed
inspector target and dialog dismissal. Accept results only for their original
owner. A saved completion for an older edit may be recorded as such, but must
not mark newer edits saved.

Keep at most one active expensive render and one replaceable pending render;
prioritize the latest request. Drop obsolete queued work before it starts.
Do not pretend interrupting a Java future forcibly cancels a native Typst
compile. Let active native work finish safely and discard its result. Clear
callbacks and release bitmaps/cache state at destruction. Extract helpers as
needed, without splitting the whole Activity into a framework.

Acceptance: controllable-worker tests deliver results out of order, after
New/Open and after disposal. None changes the wrong document/view or leaks a
page. A marks-only edit invalidates content work even when text revision stays
equal. An idle editor schedules no recurring background job.

### A3 — Preserve snapshots and recover without silent mark loss

Normative scope: C01 and the identity/fingerprint contracts in FEATURE_SPEC.
Use its two-slot binary envelope, AtomicFile writes, rejected-data preservation,
view sidecar and fixed capture timing. Add the shared canonical-content helper
needed to distinguish logical edits from Loro history. This contract is decided;
the implementer need not choose a database, record format or autosave policy.

Entry points: `onCreate`, `saveDocument`, `savePosition`,
`onSaveInstanceState`, `model_from_snapshot`, snapshot round-trip tests.

Replace document-sized preferences with an app-private atomic recovery record
containing the Loro snapshot, format version, document identity and associated
view metadata. The snapshot owns text and marks; a text copy is an explicitly
labelled recovery/export fallback. Reuse an existing suitable atomic-file
primitive, or Android `AtomicFile`, rather than building a database. Keep the
last valid recovery generation until its replacement succeeds.

Capture model-owned snapshot bytes on the UI thread only when content or marks
change, coalesce repeated captures, and do encoding/file I/O on an owned
worker. Reuse the immutable captured snapshot for matching render work. A
background write must not replace a newer generation. Lifecycle flush must
have a defined durability outcome; do not rely on `onDestroy` being called.
Measure capture cost before considering any change to model thread ownership.

Migrate the current preferences once, preserving their original raw snapshot
until verified. Corrupt/mismatched/future-version data must produce an explicit
recovery choice and retain the original bytes; opening text-only recovery must
never silently overwrite the only snapshot. Rotation and process recreation
must refer to the same recovery identity. Keep folds, open inspector, reading
position and marker visibility as local view state, outside shared Loro content.

Acceptance: text plus overlapping/modifier-only marks survive migration,
rotation and cold restart; truncated and mismatched records retain recoverable
bytes; simulated write failure leaves the previous record usable. Report what
can be lost before a pending autosave becomes durable. Existing New/Undo and
page-collapse reading-position cases continue to pass.

### A4 — Finish existing open/save behavior

Normative scope: C02, including Save copy, destination hashing, exact picker
ownership and separate recovery/file acknowledgements.

Entry points: `chooseDocument`, `saveToDocument`, `onActivityResult`,
`rememberDocument`, `readDocument`, `writeDocument`.

Bind each picker request and completion to its originating document identity.
Track the content/mark identity captured by an explicit save, separately from
the latest edit. A save of revision N must leave N+1 visibly unsaved. Keep
plain-text import's current “save as `.mathed`” behavior so marks survive.

Retain the current document/recovery until an open validates successfully;
handle cancelled pickers, revoked grants, malformed snapshots, invalid UTF-8
and provider failures. On failed save, retain recovery and offer the existing
Save flow to another destination. SAF providers do not promise atomic replace:
do not claim external-file crash atomicity after opening a URI with `wt`.
Use the local recovery copy to bound the loss.

Acceptance: fake-stream tests cover read/write failures and edits during saves;
New while a picker is open cannot retarget its callback. A document-provider
device scenario opens, changes a modifier, saves, reopens, and preserves it.

### A5 — Respect graphemes and Android composition

Normative scope: C03–C04 and the shared editing/anchor contracts. Candidates
remain in the native field until commit, rather than entering Loro recovery.

Entry points: UTF-16 conversions and `sync_text` in Rust;
`SourceEditText`, marker/completion `TextWatcher`s, `moveRenderedCaret`,
rendered selection and paste methods in Java.

Use the existing mathed grapheme/word rules for rendered movement and selection;
avoid a second Java definition. Inspect composing spans through native Android
editing APIs. Keep preedit usable, defer marker auto-naming/completion rewrites
until a committed boundary, and preserve the composition when panels refresh.
Wrap the platform input connection only where a demonstrated gap requires it.

Represent replace-selection/paste/completion as one editing intent. Preserve
the engine's existing rapid-edit undo grouping and explicitly test composition
lasting longer than its 400 ms merge window; timer-based grouping alone is not
a composition transaction. Do not replace Loro's undo manager. Add the
`DocModel` batch wrapper and narrow stable-anchor helpers specified in the
feature contract as separate reviewed changes, verifying both workspaces for
velysterm changes. Reuse `MathDoc::replace_many` grouping; do not nest another
native group around it or mutate the model for temporary composing candidates.

Acceptance: emoji ZWJ sequences, combining accents, surrogate boundaries, RTL
text and CJK composition commit/cancel remain intact. One undo restores the
pre-intent content and marks. Hardware keys and software IME exercise the same
intent, with at least one real composing-IME device check.

### A6 — Reuse layout and keep pixels off the UI thread

Normative scope: C13 and the bridge/invalidation tables. Implement the explicit
prepare-layout/render-page/locate-range worker seam and snapshot byte capture.

Entry points: `layout_page_chunk`, `render_pages_with_options_expanded`,
`renderPagesWithSnapshot`, `scheduleRender`, `startRender`, `applyChunk`.

First separate overlay-only changes from layout inputs. Caret/selection/search
highlights should repaint overlays when the rendered-only document, explicit
folds and marker visibility did not change. Retain the old valid pages while
replacement output is pending; prevent editing through stale hit positions
or explicitly map them to the new document.

Next retain one worker-owned compiled layout and its source-map/glyph context
for an immutable document snapshot and presentation key. Later chunks rasterize
that layout without recompile. Include marks, expanded segments, marker mode,
theme/text settings and actual layout width in the key; scale belongs in raster
identity when it does not alter layout. Never share the UI model handle with
the render worker. Cache failed-modifier isolation for that exact snapshot too.

Decode PNG/base64 and compute diagnostic pixel samples away from the main
looper; publish only a validated current bitmap/hit result. Preserve the
stateless text/snapshot entry point for independent callers and host tests.
Use raw snapshot byte arrays across JNI as C13 specifies. Keep PNG pixel
transport and exclude a shared-pointer/zero-copy rewrite from this milestone.

Acceptance: for a normal valid fixture, an initial chunk and a later chunk of
one layout identity compile once; ordinary caret and search-overlay changes
compile zero times; a modifier-only change recompiles and changes its output.
Compare cached and independent outputs/hits. A superseded/error result cannot
erase the newer frame. Log layout, raster, decode and UI-apply costs separately.

### A7 — Bound memory and use one page transform

Normative scope: C07, C11 and C13. Split into pixel/metadata residency; actual
page transforms plus reading-size/zoom controls; stable reading/focus anchors.
Use the stated constants, gestures and persistence, not a second design pass.

Entry points: page streaming/view creation, `releasePageBitmaps`,
`restorePreviewScroll`, `caretSpot`, hit conversion, page JSON metadata.

Keep page-size placeholders and lightweight navigation metadata, but materialize
bitmaps only for the visible region and a small neighbor window. Bound both
Java and native raster caches by bytes, including an explicit allowance for
the in-flight decode/raster. Respond to memory pressure and rehydrate evicted
pages from the retained layout. Use C13's fixed memory-class budget and working
page limit, and measure peak use; a chunk-page cap alone is insufficient. For scale, one
1489 x 2105 RGBA bitmap is about 12 MiB before transport/decoder overhead.

Return actual page dimensions in points. Use one transform for drawing, taps,
caret, selection and accessibility bounds; remove the global A4 hit-test
assumption. Improve the existing reading-size/scale behavior for narrow phones
through presentation options on the same Typst layout path. Do not rewrite
`\app` geometry or store a second layout policy in Java.

Retain a document-relative top-visible text anchor plus an intra-line offset,
with page/pixel fallback for pages without text. Map anchors through edits or
use existing stable Loro positions where available; clamp if their text is
deleted. Preserve the existing “collapse then Undo returns to the old place”
and “user touch takes over the position” semantics. Navigation/search must
request the target page rather than walk every intervening bitmap into memory.

Acceptance: a 100-page fixture scrolls/searches forward and back with bounded
bitmap residency and unchanged content; a custom-width Typst page has correct
taps and caret. Rotation, IME resize and text above the viewport preserve the
reading anchor. Include a narrow-phone visual check and memory-pressure check.

### A8 — Complete touch and accessible access to existing content

Normative scope: C04 and C15 across every selected navigator, inspector and
diagnostic action, including source fallback when layout fails.

Entry points: `RenderedPage`, glyph/line navigation metadata, rendered clipboard
methods, source/modifier controls.

Add long-press/drag selection and a contextual copy/cut/paste action path using
the same selection/intents as the keyboard. Respect ScrollView drag ownership;
a scrolling gesture must not become an edit tap. Reuse hidden-marker copy/cut
rules and preserve Loro marks through edits.

Expose meaningful rendered text blocks and citations through Android virtual
accessibility nodes as C15 specifies. Derive nodes/bounds
from the same document and page transform, not OCR. Provide readable text,
focus order, activate/edit/select actions, and equivalent access to the source
and modifier panel. The source field remains an editing fallback when layout
fails. Route activation through `performClick` where appropriate.

Acceptance: touchscreen-only reading, selection, copy/paste, citation opening,
source editing and undo work without hardware shortcuts. TalkBack can read a
document and open/edit the same content; nodes follow layout and page eviction.
Check both hidden and visible markers and malformed-document recovery.

### A9 — Make existing actions and status consistent

Normative scope: C14. Other tasks use action IDs as they land; finish their
common predicates, toolbar, Back/Escape stack and file/recovery status here.

Entry points: shortcut bar, `dispatchKeyEvent`, search bar, `refreshSegments`,
save/render error reporting.

Use a small action descriptor table for current commands, labels, enabled
conditions and keyboard/touch dispatch. Keep source fields' native editing
behavior. Make the existing Ctrl+S search-next versus document-save context
explicit in the visible label/help; preserve foot's accepted search aliases.
No command-palette framework or user-programmable keymap is needed.

Improve existing status to show the document name, saved/unsaved/saving state,
search count and actionable render/recovery failure. Retain revision/segment
diagnostics in logs or an optional diagnostic view. A render error must preserve
editable source and useful last-valid content, clearly marked if stale. Avoid
announcing background progress repeatedly to accessibility services.

Acceptance: every advertised touch command and hardware binding invokes the
same action under the same context; search accepts/cancels to the expected
caret, empty/no-match queries are clear, and an old save completion never
claims a newer edit is saved. Status queries run no external commands.

### A10 — Make the existing device lane reflect the rendered editor

Entry points: [device-lane.sh](device-lane.sh), host tests, Android CI job.

Start by replacing first-`EditText` selectors and fixed starter lengths with
stable view IDs/content descriptions and explicit document fixtures. Open the
current source inspector through the actual rendered UI before typing. Keep
the old recovery/New/Undo/page-collapse assertions as behavior, not as obsolete
widget coordinates or source-string presence checks.

Add small scenarios alongside each task: lifecycle late results, corrupt
recovery, SAF open/save with marks, composition, search to an unloaded page,
touch selection, citation/fold/modifier rendering and narrow-screen reading.
Use controllable executors and fake document streams for cheap host checks;
use Android/device tests for IME, provider grants, view measurement and
accessibility. Keep source-literal tests only for narrow invariants they can
actually prove. Do not treat a `contains("saveDocument")` assertion as a
persistence test. Test stale generations with delayed completion, not sleeps.

Require a dedicated disposable app-data target for destructive e2e setup.
The current lane calls `pm clear`; never run it against someone's working
document without preserving that data or using an isolated test profile.
Retain installed-versus-built APK identity checks and before/after artifacts.
For the late-result and recovery cases, demonstrate once that disabling the
guard/atomic replacement makes the relevant test fail, then restore it.

Acceptance: report separate host, Java, APK and device verdicts and explain
missing prerequisites. A device-free run cannot close IME, visual or TalkBack
acceptance. Extend this lane; do not create another agent/E2E framework.

### A11 — Finish scoped search and literal replacement

Normative scope: C05–C06. Entry points: `openSearch`, `updateSearch`,
`showSearchMatch`, `scrollSearchMatch`, `closeSearch`, Rust `search_ranges`,
mathed `search.rs`, and `MathDoc::replace_many`.

Split into three patches: bounded range-limited matching and search state;
Text/Source/Selection UI with full origin restoration and snippets; checked
Replace/Replace all using grouped edits. Create `FindController` and reuse
the current bar, not another search overlay. Build protected intervals from
the existing marker scan. Keep incomplete query work off the main thread.

Acceptance: run C16's search/replace fixtures against real model marks. An
unloaded-page result requests its actual page, a hidden source result opens
the correct editor, Cancel restores scroll/selection/folds/focus, and one Undo
restores the entire replace batch. More than 10,000 eligible edits refuses the
plan without partial application. Preserve mathed smart-case behavior.

### A12 — Return navigation and semantic source editing

Normative scope: C08 and the shared anchor contract. Entry points:
`setDocumentCaret`, source inspector commands, the current citation list,
mathed `SemanticIndex`, `DocumentContext`, `plan_rename` and Loro cursor wrappers.

Split into anchor/history controller; Sections/Definitions/References navigator;
definition go-to/uses/rename. History is exactly the 30-entry local list in C08.
Build sections and use sites from existing semantic data even when rendering
fails. Rename is a planned grouped edit, never a regex across the whole source.
Keep navigation separate from Undo and native text-field caret movement.

Acceptance: search/reference/diagnostic navigation can return after intervening
edits; a new destination after Back truncates Forward; duplicate and ordinary
caret movements do not fill history. A shadowed or unresolved same-spelling
name remains unchanged by rename. All destinations retain a source fallback
when their rendered page/body is hidden or unavailable.

### A13 — Complete citation details and local choices

Normative scope: C09. Entry points: `citation_detail`, `citationIndex`,
`showCitations`, `appendCitationRows`, shared bibliography formatting/constants,
and local view metadata.

First add source/number/key/snippet metadata and a breadcrumb path with cycle
protection; then add style/locale controls from the shared arrays and their
per-document persistence. Cache detail requests by capture and presentation
choices. Old dialog/path requests cannot replace newly selected details.

Acceptance: document and bibliography cases, unresolved-key suggestions,
16-level limit/cycle source access, go-to-source/return, choice/reset and cold
view restoration. Choosing a citation style changes neither document text,
numbering nor its dirty state and does not compile the whole page again.

### A14 — Complete modifier links and adaptive inspection

Normative scope: C10–C11. Entry points: `TargetEditText`, `commitTargetPairs`,
`updateModifierPanel`, `requestModifierComparisons`, `showModifierComparisons`,
`modifiers::{links,source_range,set_marker_targets}` and `setModifier`.

First preserve inspector drafts/composition, implement name suggestions and
validated complete-pair commits; then add incoming/outgoing source/target
navigation and removal, adaptive source/comparison layout and per-row rendering.
Extract `InspectorView` during these changes. Preserve both existing mark
representations; opening an old snapshot does not migrate links by accident.

Acceptance: missing/reversed/duplicate pairs, active partial pair, Done and
focus-leave behavior, removed markers, legacy removal, navigation return and
six-result comparison cache. Overlap/marks-only edits survive save/undo and
update rendering. Test below/above 600 dp and resizing with active composition.

### A15 — Structured diagnostics and honest unsupported behavior

Normative scope: C12. Entry points: `layout_page_chunk`'s discarded Typst error,
modifier-isolation warnings, `SemanticIndex`, source-map copy spans and
`previewError`/status presentation.

Retain diagnostic provenance and producing-owner information in native layout
results, then add the diagnostics list and source navigation. Reuse existing
semantic resolution; no new lint/parser/execution subsystem. Formatting/storage
errors use typed records rather than successful empty content. Preserve the
source and clearly label old pixels when no current preview can be rendered.

Acceptance: exact versus generated/unlocated errors, malformed modifier owner,
unresolved name/pair, unimplemented execution statement and stale-result
rejection. Selecting an exact/owner diagnostic opens the appropriate source
and returns through navigation history; generated text is never falsely
reported as an exact error in its neighboring source character.

## Scope decisions

- **Notebook/kernel execution: defer.** The pad deliberately excludes the
  non-Android kernel dependency tree. Its startup JIT smoke does not provide
  worker isolation, cancellation, grants, outputs or notebook persistence.
  Move the smoke off the UI startup path or gate it as a diagnostic under A2;
  update its device proof accordingly. A future notebook proposal must reuse
  mathed's existing block/worker model and justify an Android-compatible seam.
- **Live collaboration: defer.** Loro sync/presence and the in-progress desktop
  `emthin-collab`/IPC work already supply primitives. A mobile network transport
  and lifecycle are separate unverified boundaries. Do not build a second CRDT,
  introduce Kyneta, or store presence/view state as document content.
- **Agent chat/tree and embedded Infinity: exclude.** They do not repair an
  Android editing gap. Their lifecycle patterns are sufficient inspiration.
- **History/home dashboard: exclude.** Existing Android open/save and recovery
  should become dependable first; the system picker already supplies discovery.
- **PTY/terminal, shell prompt, split/workspace manager: exclude.** They add
  independent subsystems and conflict with the document's layout authority.
- **Generic plugins, fuzzy search, LSP, cloud sync and UI-toolkit rewrite:
  exclude.** No demonstrated requirement here survives improvement of the
  existing search, document model, Android adapters and renderer.

Reopen a deferred feature only with a concrete user workflow, evidence that
an existing path cannot meet it, the smallest extension to that path, and an
independent acceptance test. A sibling implementing it is not sufficient.

## Cheap-model handoff and release evidence

Use this plan as a task queue. Give a cheaper coding LLM one card, its entry
files and acceptance conditions at a time; it need not reread all nine sibling
repositories. Recheck that the finding still exists before editing. Implement
ordinary adapters, helpers, fixtures and scripts with that model. Reserve a
stronger reviewer for the storage identity, IME transaction or layout-cache
contract only if the task changes that contract or a concrete failure remains.
No autonomous agent orchestration is needed in the product or this handoff.

The product/architecture choices are made in FEATURE_SPEC. Implementation
should not reopen them as broad design questions or restudy the nine siblings.
If a pinned API contradicts a contract, isolate that mismatch and propose the
smallest compatible adjustment with evidence. Keep implementation on the
cheaper model unless that concrete problem needs a stronger review.

Every assignment contains: its A-card, the mapped C-contracts, the actual
current entry files and the last checkpoint's evidence. After each card,
return a compact handoff: implemented contracts, files changed, exact checks,
unverified boundaries and next ready card. Do not carry a second feature
backlog in agent messages. The four milestones and C16 fixtures are the
completion checklist.

Suggested first assignment (completed 2026-11-16; see *Execution status*):

> Implement A0, then A1, in `emthin-android/DEVELOPMENT_PLAN.md`.
> Read C00 and the shared contracts in `emthin-android/FEATURE_SPEC.md` first.
> Preserve unrelated working-tree changes. Repair the Android citation adapter using
> current shared defaults, resolve only the necessary lockfile drift, and make
> the Android CI path-dependency checkout complete/repeatable. Do not add UI
> features. Run `bash scripts/verify-invariants` first, then the Android locked
> host suite and Java compile. Report exact commands, sibling revisions, test
> results and any unavailable boundary. Stop after the two scoped changes;
> leave A2 onward for subsequent assignments.

For each completed card, record files changed, behavior before/after, commands
and results, remaining boundary gaps and the next card. Keep the first failing
case; avoid repeated broad runs without a new failure or change. Add tests for
the meaningful state/IO boundary, not for incidental helper shape.

Commands are run from the stated directories:

```sh
# Repository root, before relying on another gate
bash scripts/verify-invariants
python3 scripts/check-links

# Android standalone workspace; supply the native compiler/toolchain environment
cd emthin-android
cargo test --locked
cargo fmt --all --check
```

Compile Java with the platform jar when Java changes. Run `build-apk.sh` for
JNI/build changes and before final device verification, using the existing
toolchain setup and the target device's ABI. Run `device-lane.sh e2e` against
its default `com.emthin.pad.test` package; targeting the normal package
additionally requires `EMTHIN_LANE_DISPOSABLE=1` because the lane clears app
data. Shared document/model changes need
the relevant mathed tests in velysterm and emthin tests inside their own flake
devshells. Before a PR, run the repository-required fmt, locked workspace
clippy with warnings denied, and private-item rustdoc gates from AGENTS.md.
Android remains a separate lane; desktop workspace success does not cover it.

The milestone is complete when the build baseline is reproducible; recovery
retains marks; current edits survive the tested lifecycle boundaries; rendering
and bitmap residency are bounded; phone, touch and accessible editing work;
and those claims have the corresponding host and device evidence. Do not mark
the milestone complete while any required device boundary is unverified.
