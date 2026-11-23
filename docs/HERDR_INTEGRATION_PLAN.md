# Implementation plan for native terminal figures

Use the [specification](HERDR_INTEGRATION_SPEC.md) as the behavioral contract
and the [review](HERDR_INTEGRATION_REVIEW.md) for source entry points. Each
card is sized for a cheaper implementation model. Give the model one card,
its cited H sections, and the actual files listed for that card. Have it
read the current repository instructions and stop after the card's
acceptance check. Keep commits reviewable per repository and record any
contract correction in the specification before dependent cards start.

## Sequence and checkpoints

| Checkpoint | Cards | Reviewable result |
|---|---|---|
| 1. Android contract | T01–T05 | A document with a terminal reference round-trips. The pad displays the right slot, and an isolated fixture drives a bounded observer state machine. |
| 2. Android terminal | T06–T10 | Live cell updates draw in an Android page. The phone test client types, reads, releases and restores its document. Desktop embeds the same session without nesting a TUI. |
| 3. Paired fleet observation | T11–T16 | A real phone pairs and attaches live existing terminals from two hosts with no SSH. The gateway controls device and terminal permissions. |
| 4. Fleet lifecycle and legacy | T17–T22 | An approved slot launches inside the existing fleet, all observed sharing is reported accurately, and old Herdr panes appear in desktop figures. |
| 5. Release evidence | T23–T24 | Matched display benchmarks, a real phone run, compatibility tests and a reversible deployment procedure. |

T01, T02 and T03 can proceed independently. T07 can start with fixture
cells while T06 is being finished. T11, T12 and T14 can start once their
wire fixtures and the isolated Core lane are ready. T17 begins with fake
binaries and can run in parallel with phone work. The remote relay and host
launch/control code need review before enabling the service on a real
tailnet. Other cards use fixtures first. Use one compatible revision
manifest for all integrated checkpoints.

## T01–T05. Model, wire and Android foundation

### T01. Pin and reproduce build inputs

- **Contract.** H1, H6 and H8. Repos: emthin, Herdr, Core, OpenShell and
  velysterm. Record full revisions, Ghostty submodule, Nix lock, needed
  toolchains, licences/notices and the tested architecture.
- **Work.** Create a CI-readable compatibility manifest and a script to
  print/validate local inputs. In OpenShell, resolve the flake's existing
  merge markers and keep both the release Core package and the Herdr
  compatibility package available. Reserve distinct attributes for source
  forks. Validate flake parsing/evaluation and establish hermetic Core and
  OpenShell test profiles. Initialize Ghostty and Zig only in the isolated
  build lane. Use the actual Android APK lane for its current pins.
- **Done.** A clean checkout reproduces the dependency map. The test lane
  cannot accidentally use the user's Core socket, change a NixOS profile,
  start production state or update fixtures from unrelated upstream code.

**Result (2026-11-19).** Added `docs/HERDR_COMPATIBILITY.toml` and the
read-only `scripts/check-herdr-inputs`. The validator pins emthin, velysterm,
Herdr, Core and OpenShell revisions, the Ghostty gitlink, nixpkgs revision and
narHash, licences, and local checkout state. `--require-clean` makes dirty
checkouts fatal for CI; the current execution worktree reports warnings. The
OpenShell flake is checked for merge markers. Blocked: OpenShell flake
evaluation/source builds and hermetic Core/OpenShell profiles were not run, so
the clean-checkout reproduction is only local-input validated, not built.

### T02. Add the shared figure binding

- **Contract.** H2 and H7. Repos: velysterm
  `crates/mathed_core/src/figures.rs`, transform and accessibility
  callers, and emthin `docui/{figures,edit,layout}.rs` and Wayland
  claim paths.
- **Work.** Add a typed terminal reference and distinguish Wayland, native
  terminal and invalid content while retaining shared placement. Parse
  canonical Core and legacy references and return a diagnostic for
  malformed or mixed bindings. Extend existing insert, clone, binding edit
  and resize operations. Wire dormant and Wayland selection to inspect
  the content type before binding or relaunch.
- **Done.** Headless sample has a Wayland figure, a Core figure and an
  invalid figure. Exact rectangles and stable pairs survive another
  property inserted above, page reflow and save/reopen. Resize edits only
  W/H. Bad input cannot become an executable Wayland launch. Existing
  document and app tests pass in both repositories.

**Result (2026-11-19).** `mathed_core::figures` now has typed `TerminalRef`
(`hc://` and `herdr-local://`), canonical validation, `FigureBinding`, and inert
`binding_error` diagnostics for malformed, duplicate or mixed bindings. Emthin
keeps Wayland claim/relaunch/first-dormant selection off terminal and invalid
figures, labels terminal/invalid dormant slots, preserves the reference through
clone, and refuses `set_figure_id` on a terminal figure. Tests cover canonical
and rejected references, mixed/duplicate bindings, and clone preservation.

### T03. Build the pure Core protocol client

- **Contract.** H3. Repos: emthin client crate, Core reference
  `schema/hc.openrpc.json`, `src/{client,wire,attach,viewers}.rs`.
- **Work.** Implement hello, bounded frame/JSON decoding, request
  correlation, subscribe, attach, validation, and credit. Build a
  scripted peer with one stream producing data and another withholding
  credit. Keep transport ownership outside the codec.
- **Done.** Test a hello reply coalesced with a frame, header and payload
  splits, coalesced/split event lines, attach reply/data ordering, RESIZE
  before DATA, 16 MiB ceiling, invalid control length, CREDIT after
  consumption, closure, capability downgrade and unstalled concurrent
  channels. Run without Core or Zig.

**Result (2026-11-18).** Added the transport-free
`crates/emthin-core-client` crate: bounded 9-byte frame decoding with arbitrary
header/payload splits, coalescing, unknown-kind rejection and the 16 MiB ceiling;
a bounded JSON-line decoder; hello, `terminal.attach` and `events.subscribe`
correlation and validation; per-channel attach/event streams; RESIZE, CREDIT and
CLOSE handling; automatic CREDIT after DATA; and tests for generation downgrade,
two concurrent channels, watch channels that cannot type or resize, split and
coalesced event lines, and a non-JSON control payload. The scripted-peer fixture
starves one stream's credit while the other stream and channel-zero control keep
making progress, with no Core or Zig process. `cargo test -p emthin-core-client`
passes 17 tests, and clippy is clean. T03 is complete.

### T04. Session and catalogue controller

- **Contract.** H2–H3. Depends on T03. Repos: emthin client and app
  controllers, Core isolated test harness.
- **Work.** Use fake and real Unix transports to resolve only canonical
  local addresses and own request, connection, program and viewer
  generations. Project atomic snapshots, missed replacements, status,
  finish/restart and endpoint mismatch. Define shared observer and
  controller state actions. Build a deterministic fixture program for
  line echo, resize, application state reports and exit.
- **Done.** Reattach to a fixture shell after dropping its view. No old
  typing after reconnect, endpoint mismatch or program replacement.
  Closing a figure leaves its holder running. Reconnect watches; an
  intentional new control action then starts an interactive channel.

**Result (2026-11-19, fixture complete).** Added
`emthin-core-client::controller` with an atomic catalogue (`created`, `lost`,
`restarted`, `finished`, `removed`, `status`, `turn`, `title`, `labels`, `size`,
`foreground`, `doc`, `missed`, heartbeat), program generations, and `Session`
viewer leases. Reconnect always starts watching; endpoint mismatch, restart,
release and detach invalidate old leases before input or resize. Five tests
cover replacement snapshots, stale events, finish/status clearing, reconnect
and endpoint mismatch. Added `unix::Connection`, which applies the existing
`Client` to a real `UnixStream`, rejects relative and parent-traversal socket
paths, and maps a truncated frame to an error rather than EOF. The
`unix_fixture` integration test runs a deterministic holder over a real socket:
dropping an attached view does not stop it; a second connection reattaches and
sees the prior lines; resize, status, exit, and attaching after exit are
reported correctly. Remaining: run the lane against a real Core endpoint and
verify OS-level holder ownership after process teardown.

### T05. Android figure geometry and picker with a fake host

- **Contract.** H2 and Android parts of H4. Depends on T02 and T04.
  Repos: Android `src/{layout,lib,diagnostics}.rs`, `DocBridge.java`,
  `MainActivity.java`, `PadSession.java`, `ActionCatalog.java`,
  `RenderController.java`, `NavigationController.java`.
- **Work.** Export figure metadata from retained Typst frames. Add a
  lightweight Sessions surface and PIN/Rebind/Go to figure/Detach
  actions with a fake catalogue. Add drawn unavailable, offline and
  watched states to existing figure rectangles. Persist only document
  and view data in recovery.
- **Done.** Disposable APK fixture pins, saves and reopens a reference,
  then removes/restores it with Undo/Redo. Font reflow, scrolling,
  rotation and background produce no new terminal. Stale result and
  recycled page checks reject mismatched geometry. Documents without a
  native figure still follow the existing renderer.

**Result (2026-11-19, Sessions surface).** Retained and chunked page JSON now
carries per-figure key/rect geometry plus the public terminal reference and any
binding error. The fake `SessionCatalog` provides live, watched, offline and
unavailable entries with Pin/Rebind/Go to figure/Detach labels. `SESSIONS`
opens a dialog listing those sessions; Pin appends a typed terminal figure as
one undo step and marks the catalogue entry pinned. Rebind lists the document's
public terminal figures and rewrites only the selected figure's `terminal:`
argument in one undo step; Detach transitions the live entry from watched to
live without removing it from the host. Go scrolls to a loaded figure. Terminal
figures draw watched, offline, unavailable and invalid-binding tags in
`RenderedPage.onDraw`. Each terminal figure also has an accessibility virtual
node whose description carries its caption, state and host, with Go to figure,
Rebind, Detach and Sessions activation actions wired to the same handlers as
the visual UI. Rust tests cover page metadata, reference-only rebind,
append/undo and stale-text refusal; 22 Java checks cover the catalogue states
and descriptions. The disposable test APK has a `device-lane.sh session-pin` fixture that pins a
public reference, proves Undo removes and Redo restores the figure, then
force-stops and reopens to assert the persisted `figureMetadata terminal 1`; it
passes on the arm64 Waydroid image. A second `figure-a11y` fixture builds a
watched figure, queries its virtual node, then activates accessibility focus,
Go, Detach and Sessions through the provider and checks the detached state; it
also passes there. Remaining: TalkBack traversal with a real accessibility
service enabled, and an actual remote host lease behind detach.

## T06–T10. VT, native scenes and input

**Result (2026-11-19, build gate passed).** Added Zig 0.16.0 to the emthin
devshell (the pinned Ghostty's exact minimum) and `scripts/build-ghostty-vt`.
The script builds the desktop and, with `ANDROID_NDK_HOME`, the arm64 Android
static/shared VM libraries from the pinned submodule. Verified: the desktop
archive and an AArch64 Android 29 ELF built with NDK r30-beta1. Remaining: the
emthin's `emthin-vt` wrapper, which now snapshots styled cells (including full
combining clusters and wide/spacer cells), cursor position/visibility/style,
DEC modes, row-run damage, generated replies, and mode-aware key/wheel encoding.
Nine VT/stream fixtures pass, including a `Pane` that turns Core attach
data, resize and close events into damage plus owed replies while ignoring
other channels and post-close data. Remaining: draw the snapshots and load the
result in a real arm64 APK. The NDK is
not currently in the default emthin devshell, so Android builds need
`ANDROID_NDK_HOME`.

### T06. Build the shared native VT client

- **Contract.** H3–H4. Depends on T01, T03. Repos: Core
  `crates/ghostty-vt` and emthin VT crate. Inspect the actual pinned C
  headers before binding.
- **Work.** Compile/link libghostty-vt for desktop and arm64 Android.
  Add small read-only styled-cell, cursor, rendering damage and mode
  APIs where needed. Expose mode-aware encoding of key, mouse and
  automatic replies. Keep it independent of the Core server and ownership
  state. Integrate the controller's stream into this parser worker.
- **Done.** Real APK loads on arm64, consumes VT fixtures and reads
  correct styles, combining characters and width. Compare live attach
  output, replacement paints, paste/mouse modes and generated replies
  to the isolated Core. If the pinned Android build fails, report the
  missing toolchain support and fix or update the common pin in a reviewed
  change before renderer integration.

### T07. Implement retained glyph batching

- **Contract.** H4. Repos: emthin shared scene/renderer and fixture
  corpus. Depends on T06's snapshot interface.
- **Work.** Batch cell background, glyph, decoration and selection draws.
  Use fixture cells to implement the texture atlas, damage, cursor blink
  and viewport clip and to test GL context recreation. Define bounds for
  glyph memory, pending updates, scrollback and zoom. Keep a debug text
  view for inspecting output parity.
- **Done.** Golden screen at multiple pixel densities and colors. Update
  one cell and observe partial geometry/damage with no page upload and
  no allocations per unchanged row after warmup. A hidden figure
  schedules no glyph work.

**Result (2026-11-19, batching layer).** Added `emthin-vt::render`: a bounded
shelf atlas keyed by glyph text, weight, italic and pixel size; retained
per-row solids/glyphs/decoration quads; damage-only row rebuilds; a separate
cursor quad; hidden-pane gating; and GL generations that preserve the CPU
atlas. Nine batching/font/GLES fixtures cover damage-only rebuilds with glyph reuse
across two densities, hidden panes doing no raster work, atlas budget
enforcement, cursor geometry, real DejaVu coverage, software composition,
byte-exact goldens at two densities and two backgrounds, real GLES 3 upload and
draw, one-row-only GPU re-upload, and a live context-loss recreation. The GL
path uses EGL surfaceless GLES 3, normalized atlas sampling, retained per-row
vertex buffers, and no atlas upload on an unchanged or cached-glyph update.
Scrollback is a clamped viewport over the parser's bounded history (the live
cursor is hidden in history), and zoom is clamped to 50–300%, rescaling cell,
baseline and glyph density. Two further fixtures cover oldest/live clamping,
close behavior and zoom limits. The crate now has 23 tests.

### T08. Android native scene

- **Contract.** H4. Depends on T05–T07. Repos: Android JNI and preview.
- **Work.** Integrate the native scene into the existing visible page
  host. Upload accepted page rasters, keep common page transforms and
  overlays, and attach bounded terminal streams only for the visible
  window. Retain snapshot-based layout on its existing worker. Route
  context loss, trimming, pause/resume and page recycling through
  document and connection generations.
- **Done.** The test APK shows an animated VT fixture within its exact
  page rectangle, without relayout or PNG work on a terminal update.
  Pan, scale and switch pages while streaming. Test context loss,
  eviction, and closed-document cleanup. Confirm the existing editor,
  Find, screen-reader and recovery lanes.
- **Result (2026-11-20).** `emthin-pad` now links the shared
  `emthin-vt` scene (`emthin-android/src/scene.rs`) and draws its RGBA
  buffer in the Typst figure rectangle. The rect is mapped through the
  page `ImageView` matrix, which is required because the bitmap is
  letterboxed inside the page view; using layout points directly put
  the scene above the visible viewport. `terminal-scene` on the arm64
  Waydroid device passes: placed rect 1134x709, frames 1..n without a
  page re-render, renderer context loss, resize/reflow, CJK/emoji
  commit-once, Enter/Backspace encoding, and Back releasing focus.
  Scenes close on page eviction and document replacement. A view zoom or
  font reflow that rebuilds the page stack now transfers the live scene
  by its public terminal reference, preserving parser, scrollback and
  input state; the next draw reconciles the new placed rectangle and
  resizes cols/rows instead of stretching one fixed raster. The
  `terminal-reflow` device lane proves a zoom from 102x30 to 113x39 cells
  across the page rebuild, continued animation after a pan, and the
  terminal accessibility Focus/Copy/Paste actions. The full `e2e` sweep
  also passes the existing editor, Find, composing-IME, screen-reader
  and recovery lanes. The `terminal-pages` lane starts an eight-page
  stream, scrolls to another page, returns, and proves the same parser
  keeps advancing. A page-figure `SurfaceView` now owns a real EGL14
  window surface: the CPU RGBA raster is uploaded and submitted by GLES,
  with the surface clipped and texture-cropped to the preview viewport so
  it cannot cover the toolbar or navigation bar. Returning to a page
  forces a fresh submission, and the lane asserts that repaint rather than
  only the parser frame. Not yet exercised on device: a real
  closed-document/Core stream (the fixture has no lease).

### T09. Android terminal focus and control

- **Contract.** H2–H4. Depends on T08. Repos: Android input layer and
  shared controller/encoder.
- **Work.** Add explicit Watch/Take control, terminal InputConnection,
  key row, hardware-key routing, mouse mode, selection/copy, paste and
  accessibility actions. Reuse Back/IME handling and restore anchors
  after terminal focus. Bind input to viewer generation. Show conflict
  and read-only fixture states before implementing real handoff.
- **Done.** Existing disposable composing-IME lane extended with
  terminal text and inactive previews. Verify CJK and emoji commit once,
  hardware Escape and arrows are delivered correctly, app mouse modes
  choose the intended target, reflow grid changes come from document
  size, and a late commit after release reaches neither the shell nor
  the document.
- **Result (2026-11-20).** Terminal focus, a named-key row (Esc, Tab,
  arrows, Ctrl-C, Watch), hardware key/modifier routing, an
  `InputConnection` with composition buffering, Back-before-IME release
  and late-commit rejection are implemented. The `terminal-scene` lane
  proves CJK `你好` and emoji `😀` commit once, Enter/Backspace reach the
  VT encoder, Back releases focus, long-press selects a word and copies
  it, and a tap in an app that enabled mouse tracking is delivered as an
  SGR press/release. Selection extends by dragging cell bounds; paste
  sends clipboard text to the selected or focused scene instead of the
  document; figure accessibility exposes Focus terminal, Copy terminal
  output and Paste into terminal actions, and the `terminal-reflow`
  fixture executes those provider actions. The key row now has explicit
  Watch and Take authority: a watched session is read-only at the input
  boundary, Take grants local input, and the fixture proves blocked text,
  a conflicting lease and a late composition after Back reach neither
  terminal nor document. Hardware Escape and arrows are encoded as
  terminal keys. Remaining: a
  real gateway/Core handoff for Watch/Take and viewer-generation
  revocation, a real shell lease, and TalkBack traversal (no
  accessibility service is installed on the test image). The z-ordered
  GL surface also occludes page-drawn WATCH/CONFLICT badges, focus
  borders and selection chrome inside the figure rectangle; those remain
  in the model and accessibility tree, but their visual overlay still
  needs to move above the scene surface.

### T10. Desktop render element and figure focus

- **Contract.** H2–H4. Depends on T02, T04, T06–T07. Repos: emthin
  `element.rs`, `figure_render.rs`, `winit.rs`, `input.rs`,
  `docui/`.
- **Work.** Register Core session sources, add element damage and GL
  state restoration, and route focus, mouse, key and paste through the
  existing figure-first checks. Use the headless preview and live
  compositor lanes for geometry and screen evidence.
- **Done.** Mixed terminal and Wayland page with one Smithay submission
  and independent damage. Resize updates `app` text. Page movement, zoom
  and selection do not seize a terminal's grid. Direct attach, release and
  reopen verify unchanged holder identity.

**Result (2026-11-19, desktop source).** `core_session.rs` registers desktop
Core session sources keyed by canonical `hc://` terminal reference. Each source
owns the Core viewer `Session`, a `Pane`, a retained VT renderer and a cached
GPU buffer; attach data/resize events produce RGBA frames, generated replies
are returned to Core, and a reference/endpoint Core mismatch is refused.
`figure_render` now collects terminal figures on the visible page and
composites their cached textures in the existing Smithay custom-element pass,
without a Wayland surface. `--core-socket PATH` connects each Core figure once
via hello/attach and registers a calloop reader; DATA, RESIZE and CLOSE feed
the matching source, CREDIT and generated replies go back to Core, and a
mismatched Core ID is refused. Clicking a terminal figure focuses it; keys are
encoded through the program's modes (ASCII plus common named keys and shifted
punctuation), clicks use SGR/X10 reporting, 1002/1003 modes receive held-button
and hover motion, and the wheel scrolls local history when the program did not
request mouse reports. Failed or closed endpoints retry with bounded
exponential backoff (250 ms doubling to an 8 s cap, five attempts); the figure
overlay names the next attempt and its backoff delay, or reports exhaustion.
Committed non-ASCII IME text is routed to the focused Core terminal while no
Wayland client owns the keyboard. Nine source tests cover render/revision,
ignored channels, resize/replacement, Core identity, key/mouse/drag encoding,
retry bounds and overlay text, a real Unix-socket attach stream, a live socket
that carries input, IME text and output through the figure state, and a
second attach after a closed socket and backoff. Still not done: the mixed
compositor acceptance run.

## T11–T16. Remote observation and pairing

### T11. Fixture the scoped gateway protocol

- **Contract.** H5 and the [gateway v1 wire contract](HERDR_GATEWAY_V1.md).
  Depends on T03. Repos: emthin gateway sources and common fixtures.
- **Work.** Check v1 request, reply, error and revision-hint fixtures
  into a location usable by Go and Rust tests. Cover contract limits,
  partial writes, channel remapping and expiry/lease races. Start with
  scripted auth identities and a Unix fixture Core.
- **Done.** Document every accepted method and frame, its authorization,
  and its state transition. Use fixtures to prove watch cannot send
  input and arbitrary Core methods have no passthrough. Include invalid
  Core ID, addresses, origins, channels, kinds, lengths and quotas.

**Result (2026-11-19, fixture corpus).** Added
`fixtures/herdr-gateway-v1/` (32 files) and the `herdr-gateway-fixtures`
workspace crate. The corpus covers hello, list, watch/control attach,
`control.request`/`respond`, stop, launch, `sessions.changed`, stable error
kinds, resize/data/credit/close/focus frames, and invalid methods, extra
properties, machine paths, versions, limits, duplicate keys, frame lengths,
control-channel stream frames and unknown error kinds. The Rust validator
rejects duplicate object keys before value decoding, enforces method/param
allowlists and 1–64 list limits, validates grid 20x5..300x120, and enforces
control 64 KiB, DATA 16 KiB, CLOSE/FACTS 1 KiB and CREDIT/FOCUS/RESIZE
shapes. Chunked fixtures prove a frame decodes across several transport reads,
and scenario fixtures cover channel reuse after CLOSE, watch-input rejection and
an expired viewer lease. `cargo test -p herdr-gateway-fixtures` and the
`validate-herdr-gateway-fixtures` binary pass. The Unix fixture Core itself and
real socket races are still not represented.

### T12. Scoped gateway attach and handoff

- **Contract.** H5. Depends on T04, T11. Repos: emthin Go gateway and
  local/remote client adapters.
- **Work.** Project explicit per-scope inventory, terminate control RPC,
  and implement bounded credit-preserving stream relay, writer leases,
  handoff and revocation. Sync effective rights at scope changes. Implement
  the local Unix route for desktop on a managed host. Keep all core peer
  and file operations outside the allowlist.
- **Done.** Two clients watch while one controls the fixture. Denied
  targets cannot be attached, queried or manipulated through channel
  reuse. Concurrent loss/revoke/handoff tests deliver no old input.
  A stalled client does not block a second terminal or control response.
  Changes from another direct Core client release grid authority as
  specified.

### T13. Host Tailscale identity and pairing

- **Contract.** H5. Depends on T11–T12. Repos: gateway CLI, private
  service state and deployment notes.
- **Work.** Package pinned tsnet, TLS, private provisioning and an
  approval CLI. Bind WhoIs peer facts to owner and node grants. Implement
  expiry, deny, forget/revoke and active teardown. Make the HTTPS
  endpoint available on a test tailnet after service authentication.
- **Done.** Same-owner known node can pair, verify expected Core ID and
  reconnect. Deny absent identity, unapproved node, revoked node, tagged
  node and wrong-owner cases. Certificate/hostname verification, endpoint
  visibility and registry mode are verified. QR and logs contain no
  credentials or terminal data.

### T14. Android secure transport and paste pairing

- **Contract.** H3–H5. Depends on T08–T09 and T11. Repos: Android
  manifest, JNI and shared networking client.
- **Work.** Add WSS and endpoint registry on an owned connection worker.
  Keep the existing Gradle-free build. Select a TLS/WebSocket stack after
  confirming Android arm64 linkage, public CA validation and production
  APK size. Test pairing through Paste link and request/approve UI before
  adding scanning. Distinguish disabled, offline, denied and mismatched.
- **Done.** Device fixture can pair, list, watch, control and detach
  through the checked protocol. HTTP, redirected destinations and bad
  certificates fail closed. Reconnect verifies identity, starts a new
  watcher and cannot replay terminal input.

### T15. Android QR and multi-host Sessions

- **Contract.** H5. Depends on T13–T14. Repos: Android app and build
  script. Use a small pinned QR decoder with the platform camera behind
  the Pair action, and keep Paste link as an accessible equivalent.
- **Work.** Accept only supported pairing URLs. Store endpoint identity
  locally, aggregate verified host catalogues, prioritize the visible
  figure host for reconnect, and provide per-host reconnect/forget.
  Keep Android network enrollment in the normal Tailscale app.
- **Done.** On a physical arm64 phone, pair two test hosts, reach both
  with SSH unavailable, then revoke one node grant. The second host stays
  usable. Network change/airplane mode and camera permission denial
  preserve the document and allow a clear recovery.

### T16. OpenShell enrollment of running sessions

- **Contract.** H5–H6. Depends on T12 and a verified existing fleet or
  disposable ready fixture. Repos: gateway host registry and
  `OpenShell/openshell-nixos`.
- **Work.** Use local authenticated OpenShell inventory and Core records
  for operator enrollment of existing `fb-*` and `inf-*` sessions.
  Add profile/sandbox provenance. Provide a read-only host command for
  listing proposed enrolments, and a separate explicit enrollment action.
- **Done.** A phone attaches an existing session with no `hc run` or
  VM creation and sees its status. A spoofed name/label or stale VM ID
  fails enrollment. Device revocation and normal app close leave the
  existing fleet and workspace available.

### Checkpoint 3 status (2026-11-23)

Checkpoint 3 is **not passed**. The fixture-verifiable half is green and the
remaining half needs the gateway/Tailscale/OpenShell repositories, credentials
and a physical phone that this checkout does not have. The
`terminal-lease` device step that gated this checkpoint (2026-11-22 record) now
passes, so the local evidence is recorded rather than deferred.

| Card | State here | Evidence or gap |
|---|---|---|
| T11 | Verified | `cargo test -p herdr-gateway-fixtures` (5 tests) and `validate-herdr-gateway-fixtures` print `validated 32 Herdr gateway v1 fixtures`. |
| T12 | Fixture-only | `fixtures/herdr-gateway-v1/lease_fixture.py` driven end to end on the Waydroid device: watch attach → authoritative grid → DATA → Take control → controller input round trip → operator revoke → fresh lease. `emthin-collab` (66 + 23 tests) and `emthin-core-client` (1 + 1) cover the local adapters. Gap: no Go gateway, so no credit-preserving relay, writer handoff, denied-target channel reuse or stalled-client isolation. |
| T13 | Gap | No Tailscale/tsnet identity, approval CLI or pairing service lives in this repository, and no test tailnet was exercised. |
| T14 | Partly verified (host only) | The WSS transport, pairing parser/state machine and endpoint registry exist and are exercised on the host JVM: `emthin-android/test-java.sh` adds `GatewayPairingTest` (126 checks) and `WebSocketTest` (51 checks — a real TLS round trip plus the plain-HTTP, unknown-certificate and wrong-hostname failures). `emthin-android/wss-lane-check.sh` runs `fixtures/herdr-gateway-v1/wss_fixture.py` (the same scripted Core behind TLS/WebSocket) against `WssTransport` via `WssFixtureProbe` (9 checks: upgrade, hello, control attach, lease, 80×24 grid, banner). Gap: `./device-lane.sh gateway-wss` has **not** run — the APK cannot be rebuilt on this host (the NDK `ld.lld` binary under box64 aborts while linking), so the installed APK predates these sources; and pairing is a parser + registry, not a Paste/approve screen (`MainActivity` still has no pairing UI). QR scanning is T15. |
| T15 | Gap | Needs T13–T14 and a physical arm64 phone; this run used the Waydroid container, which is not a pairing target. |
| T16 | Gap | No OpenShell fleet or gateway host registry is available here, so no real session could be enrolled. |

Run A of the [acceptance run](HERDR_ACCEPTANCE_RUN.md) is exercised by
`emthin-android/device-lane.sh e2e`: **215 checks, 0 failed** on 2026-11-23
(persistence, sessions pin, figure accessibility, terminal scene, reflow,
pages and lease), re-run the same day from a clean install (package
uninstalled first, `SKIP_BUILD=1`, installed APK md5-matched to the build).
Run B — two paired test hosts over Tailscale with no SSH — remains a hardware
and deployment gap and was not attempted.

The WSS device step is wired but opt-in: `LANE_WSS=1 ./device-lane.sh e2e`
(or `./device-lane.sh gateway-wss` alone). It is off by default because it needs
an APK built from the transport sources, and this host's NDK link currently
crashes under box64 (`free(): invalid next size` from `ld.lld` while linking),
so the installed APK would silently fall back to plain TCP. The transport's own
checks run on the host instead (`test-java.sh`, `wss-lane-check.sh`).

`scripts/check-herdr-inputs` **fails** here (0.2 s): `velysterm` is at
`00f24a7` against pinned `423a8c4` and `openshell` at `a0a1d2d` against pinned
`b98d464`; `herdr` and `herdr-core` match, with `herdr-core` dirty only by a
local `crates/ghostty-vt/src/lib.rs` edit. Those sibling checkouts are outside
this tree, so the drifts are recorded rather than reverted.

## T17–T22. OpenShell launch, sharing and old Herdr

### T17. Make the fleet launcher safe to plan and repeat

- **Contract.** H6. Depends on T01. Repos: OpenShell
  `openshell-nixos/{fb-fleet,fleet-policy.yaml}` and adjacent launch
  tests.
- **Work.** Put all staging behind apply. Replace the pre-DRY_RUN
  writes, directory deletion, store add and credential reads.
  Implement validated plan output, atomic per-slot reconciliation,
  Ready gating by sandbox ID, exact closure and mount/profile checks.
  Fix the case-sensitive Infinity checkout discovery and stage the
  Freebuff executable from the shared package rather than from each
  mutable HOME. Capture auth setup as a separate operator step.
- **Done.** Stub CLI/Nix tools and seed existing slot directories.
  Plan makes zero writes and reads no agent or provider secrets. Repeat apply reuses
  matching VM and active slots. A failed/timed-out create, wrong VM,
  stale policy or failed Ready check produces no launched terminal.
  Interrupted apply yields one running program or an explicit uncertain
  record, with no duplicate launch on retry.

**Result (2026-11-19, OpenShell launcher).** `fb-fleet` now defaults to a
write-free `--plan`; `--apply` stages the shared Freebuff executable and the
exact Infinity release binaries, holds a lock, records the VM ID, verifies
provider/policy hash/mounts, and starts no terminal until the exact sandbox ID
is Ready. Retries reconcile by terminal name and `launching`/`uncertain`
records instead of blind relaunch, and credentials are never copied. The stub
lane `tests/fb-fleet-test.sh` passes 21 checks: plan writes nothing and starts
no CLI, repeat apply reuses the VM and terminals, stale policy/wrong VM/failed
create/not-Ready start no terminal, and an interrupted launch is reconciled
without duplication. Not run against a real VM here.

### T18. Surface approved launch and stop in emthin

- **Contract.** H5–H6. Depends on T12, T16–T17. Repos: gateway profile
  adapter, Android/desktop Sessions, OpenShell launcher result contract.
- **Work.** Expose a list of approved slots and available actions.
  Launch by profile/slot/idempotency key and return the verified Core
  reference. Offer “Pin to document” after success. Confirm Stop using
  the session/program generation. Inventory refresh reflects Ready,
  launching, uncertain, live and stopped status.
- **Done.** From the phone start a fixture slot within the Ready VM.
  The guest reports the requested PTY resize. A repeated request returns
  the same result. No caller path/arg field can alter the launcher
  mapping. Closing the app detaches. Stopping that slot ends the guest
  workload and keeps adjacent sessions, VM and workspace. Verify the
  guest outcome as well as the host Core record.

### T19. Pin fork packages and preserve the shared closure

- **Contract.** H6. Depends on T01 and T17. Repos: OpenShell
  `openshell-nixos/{flake.nix,openshell-package.nix,herdr-core-package.nix,nixos-module.nix}`
  and available fork flakes.
- **Work.** Build pinned Core, legacy Herdr, gateway and DAX-enabled
  OpenShell/libkrun packages with separate compatibility attributes for
  prebuilt releases. Keep `/nix` RO and `/workspace` RW. Package
  read-only agent executables/runtime once and provide a closure root for
  live fleet. Keep the existing Core settings including notes suppression
  and screen restore. Add opt-in service units using private state.
- **Done.** Record package derivations and actual binary versions.
  Evaluate/compile target architectures without starting services.
  Verify expected flags reach the libkrun driver. Existing data and
  rollback retain their stores and private state.

**Result (2026-11-19, flake).** Removed leftover conflict markers from
`openshell-nixos/flake.nix`; `nix eval .#packages.x86_64-linux.fleet.name`
evaluates to `agent-fleet-0.1.0`. Building it needs x86_64 and fails here with
`platform mismatch` because this host is aarch64; no remote builder, VM start
or service activation was used.

### T20. Verify and report sharing

- **Contract.** H6, H8. Depends on T17, T19. Repos: OpenShell mount/cache
  probe and gateway fleet status.
- **Work.** Use disposable VMs and a read-only regular Nix store file.
  Compare effective mount options and guest cache before/after identical
  reads on control and DAX runs. Record file/path identity, closure
  size, CPU, available/mapped bytes and host PSS. Control for warm cache
  and avoid counting guest and host metrics as independent total RAM.
  Verify read-only `/nix`, writable fleet `/workspace`,
  and independent mutable slot writes.
- **Done.** Produce a reproducible artifact with driver revision,
  launch configuration, mount facts and measured data. App presents
  Unknown or Unavailable where measurements do not support Verified.
  Compare one vs several slots with one fleet VM. Distinguish reduced
  duplicated disk state from file-backed memory sharing and private
  process memory.

### T21. Correct the fleet status producer

- **Contract.** H3, H6. Depends on T16. Repos: OpenShell
  `herdr-core-agent` and optional Infinity core event source.
- **Work.** Report lifecycle from actual process outcomes and exit
  status. Project application-reported state and source in Sessions.
  Keep unknown until the agent sends a genuine turn event. If adding
  Infinity turn status, wire its existing driver event/cancellation
  seam through the wrapper with fixture events.
- **Done.** Fixture idle/working/interruption and failure signals match
  the actual source, or show Unknown. A failing process never shows
  success from a generic EXIT trap. No change to Core's status
  detection contract.

**Result (2026-11-19, runner status).** `herdr-core-agent` no longer reports
`done` from a generic EXIT trap. The runner emits `idle`, then `done` only for
a zero child status and `error` for failure or an interrupt, preserving the
child's exit code. `tests/herdr-core-agent-test.sh` passes seven fixture
checks for success, failure, signal interruption and the absence of a false
success report. The Sessions projection of these reports and an Infinity turn
event source are still pending.

### T22. Desktop Herdr 0.9.5 compatibility

- **Contract.** H7. Depends on T02, T07, T10. Repos: emthin legacy
  adapter and Herdr `src/protocol` plus native headless endpoint.
- **Work.** Read the stable snapshot/surface codecs, bind a selected
  pane, crop by published rect and use passive surface interest.
  Support boot/revision invalidation, missing/off-tab behavior and
  handoff. Add the opt-in read-only projection only if passive
  reuse requires it.
- **Done.** Fixture and actual isolated Herdr pane render through the
  native scene with the TUI client present. No change to that client's
  grid or focus. Restart/reorder never silently transfers the binding.
  Released wire fixtures and Herdr's existing test/bench lanes pass.

## T23–T24. Acceptance and handoff

### T23. Benchmark and profile

- **Contract.** H4 and H8. Depends on T08, T10, T12, T15, T22.
- **Work.** Use identical deterministic echo and full-screen redraw
  workloads for native Core and Herdr inside a host terminal. Trace
  input event, socket, parse, draw, submit and presentation. Use 1 and
  15 panes, visible and hidden cases. Record refresh, device, RTT,
  cold/warm state, memory, frame count and upload/damage statistics.
  Use observable presentation evidence, such as timing instrumentation
  or capture, for presentation claims.
- **Done.** Record p50/p95/p99 and candidate gate results, the first
  display paint, hidden-work proof and a tight regression threshold
  derived from the measured run. File bottlenecks by stage if a gate
  fails. Claim a measured improvement only for matched measurements.

### T24. Run the integrated acceptance

- **Contract.** All H sections. Depends on T16, T18–T23.
- **Work.** Follow the [integrated acceptance run](HERDR_ACCEPTANCE_RUN.md)
  on an arm64 phone and desktop against two isolated paired hosts.
  Use a shell/test agent and local provider
  stub where possible. Run existing emthin, Android, Herdr, Core and
  OpenShell gates for changed code. Document exact installation,
  optional pairing/provisioning, inventory enrollment, expected
  admin actions and rollback.
- **Done.** A reproducible transcript includes existing and newly
  launched fixtures, reuse of one VM and shared closure, reconnect,
  device revocation and user-data preservation. Rollback disables the
  new gateway/UI and leaves the existing Herdr/Core services and fleet
  state usable. Mark real-phone, host-resource and compatibility
  results separately from simulated checks.

## Test lanes to hand to the implementation model

| Area | Lane |
|---|---|
| Shared figures | mathed_core/mathed_mini tests and emthin tests, headless geometry and replay fixtures. |
| emthin desktop | `bash scripts/verify-invariants`, applicable Rust workspace checks and existing nested compositor E2E. Update/verify IPC docs if an emthin IPC method changes. |
| Android | Standalone Cargo tests, `test-java.sh`, gradle-free arm64 APK build, disposable `device-lane.sh` (including `terminal-reflow` and `terminal-pages`), and the real device checkpoints. |
| Core and Herdr | Follow their `just` lanes. Use fresh Core session and XDG state. Run schema/API compatibility checks for protocol edits and stable codec fixtures for Herdr. Run Herdr render scaling benchmark if its surface hot path changes. |
| Gateway | Independent Go and Rust fixture tests, concurrent lease/credit tests with scripted peers, disposable tailnet integration. |
| OpenShell and Nix | Fake CLI tests for launch/reconciliation first, then flake evaluation and source builds. Run the DAX probe only on a selected disposable VM. |

Use expert review for changes to the ABI, flow control, GPU submission,
remote access scopes and OpenShell deployment. Routine UI wiring, state
reducers, diagnostics, fixture scripts and integration test automation can
use the cheaper model. Keep run evidence in the task's review notes with
commands, versions and expected versus observed results.

## Execution blockers at this checkpoint

The foundation cards above are implemented and tested in-repo. The remaining
cards cannot be completed from this checkout without their owning inputs:

- **T08–T09** now have a native scene, terminal focus, explicit local
  Watch/Take gating, zoom-time scene transfer, grid reflow, GLES
  window-surface submission, multi-page survival and arm64 device lanes
  covering pan while streaming. Real shell/gateway control remains.
  Mouse-mode target dispatch, terminal selection, copy/paste and figure
  accessibility actions have code and device coverage. T05 has its
  Sessions dialog and
  figure-state overlays; T06's pinned desktop and arm64 Android libraries
  and `emthin-vt` styled-cell snapshot exist, T07 has bounded
  scrollback/zoom, and T10 has a desktop Core source and figure pass.
- **T12–T16** need the gateway sources, scripted auth identities, a disposable
  tailnet and a physical arm64 phone. T11 has a shared corpus and Rust
  validator, but no response from a real gateway, Core lease or phone exists
  here.
- **T17–T21**: the launcher plan/apply and runner status now have stub
  acceptance, and the OpenShell flake evaluates cleanly after removing conflict
  markers. Real VM/DAX measurements, source builds, live fleet status
  projection and service activation still need disposable VMs.
- **T22** needs an isolated Herdr 0.9.5 endpoint and its released protocol
  fixtures.
- **T23–T24** depend on the above and on matched benchmark hardware.

Those are external-input blockers, not simulated successes. T04 now has its
real Unix fixture transport and deterministic holder; T05’s native document
wiring and T06 remain the next implementation steps once their inputs exist.
