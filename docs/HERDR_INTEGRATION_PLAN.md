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

## T06–T10. VT, native scenes and input

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
| Android | Standalone Cargo tests, `test-java.sh`, gradle-free arm64 APK build, disposable `device-lane.sh`, and the real device checkpoints. |
| Core and Herdr | Follow their `just` lanes. Use fresh Core session and XDG state. Run schema/API compatibility checks for protocol edits and stable codec fixtures for Herdr. Run Herdr render scaling benchmark if its surface hot path changes. |
| Gateway | Independent Go and Rust fixture tests, concurrent lease/credit tests with scripted peers, disposable tailnet integration. |
| OpenShell and Nix | Fake CLI tests for launch/reconciliation first, then flake evaluation and source builds. Run the DAX probe only on a selected disposable VM. |

Use expert review for changes to the ABI, flow control, GPU submission,
remote access scopes and OpenShell deployment. Routine UI wiring, state
reducers, diagnostics, fixture scripts and integration test automation can
use the cheaper model. Keep run evidence in the task's review notes with
commands, versions and expected versus observed results.
