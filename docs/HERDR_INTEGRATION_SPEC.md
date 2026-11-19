# Native terminal figures for emthin

Specification for the Android pad and desktop compositor. The [implementation
plan](HERDR_INTEGRATION_PLAN.md) assigns each contract to a small change. The
[source review](HERDR_INTEGRATION_REVIEW.md) records the checked baselines.

## H1. Product and ownership

A user can pair the Android pad with a host, find existing Herdr Core sessions,
and pin any enrolled session to a document. They can watch multiple live
figures, take control of one, type with the phone IME or a hardware keyboard,
detach, and resume the same program from desktop or mobile. A pinned Infinity
or Freebuff terminal uses the same OpenShell VM, workspace and Nix closure it
used before pinning. Existing Herdr 0.9.5 users get a local desktop view of
their panes and can continue their workflow in Herdr. Core is the backend for
new portable, interactive figures.

~~~mermaid
flowchart LR
    Doc["mathed document and figure rectangles"]
    A["Android pad · terminal scene"]
    D["emthin desktop · terminal elements"]
    G["Host gateway · Tailscale and session access"]
    C["Herdr Core · PTYs and holders"]
    H["Herdr 0.9.5 · existing panes"]
    O["OpenShell fleet VM"]
    N["Host Nix store · read-only virtiofs and DAX"]
    W["Host fleet directory · read-write workspace"]
    Doc --> A
    Doc --> D
    A <-->|"WSS over Tailscale"| G
    D <-->|"Local Unix socket or remote WSS"| G
    D <-->|"Local Unix socket for standalone sessions"| C
    D <-->|"Stable cell surface"| H
    G <-->|"Local Core attach protocol"| C
    C -->|"PTY runs OpenShell sandbox exec"| O
    N --> O
    W <--> O
~~~

| Owner | Data and behavior |
|---|---|
| mathed and emthin | Figure membership, size, page placement, commands for inserting and editing figures, reading position and local presentation. |
| Herdr Core | Terminal allocation, PTY and holder lifecycle, authoritative screen and modes, scrollback, program exit, and application-reported status. |
| Herdr 0.9.5 | Its existing pane IDs, layout, screen projection, input, agent detection and restore behavior. |
| Host gateway | Host and device trust, enrolled terminal catalogue, remote streams and control leases, and approved fleet launch profiles. |
| OpenShell and Nix | VM lifecycle, workload policy, bind mounts, shared store closure, guest runtime and host inference routing. |

The new capabilities are live terminal figures and paired remote access.
Existing figure, preview, input, navigation, recovery and status paths are
extended to expose them. The user-facing boundary is a document containing
terminals. Core stays headless and Typst pagination stays the page model.

## H2. Document contract and commands

~~~text
#17 Fleet · Infinity #18
\app(#17, #18, 640, 400, terminal: "hc://6ab09a20fd1288983dfbbac2bb349231/t3")
~~~

- Add the optional named `terminal:` argument to the existing `app` figure
  contract in `mathed_core::figures`. Resolve it with the shared marker parser
  and figure index. A Core reference is exactly `hc://<32 lowercase hex>/t<N>`,
  where N is a positive canonical decimal u32. The second component is a
  per-core address. Existing Core allocates it monotonically. Its
  `Terminal.identity` field is the TERM_PROGRAM/version string. Keep the
  record's `created_ms` in the local binding registry and reject an
  unexpected replacement at the same address. When recovering a Core with
  a reset registry, provision it with a new Core ID.
- Terminal figures use the same caption, stable marker pair, Typst placeholder
  and recovered rectangle as other figures. A terminal reference excludes
  Wayland `id` and `launch:`. Invalid syntax, duplicate arguments and mixed
  bindings produce owner diagnostics and an inert rectangle when dimensions
  are valid. Preserve source through Android and desktop save, undo, clone and
  clipboard paths. Apply the existing duplicate marker policy.
- Use the existing document edit transaction to insert a figure at the caret
  with fresh markers, an editable caption and the normal figure dimensions.
  “Sessions” opens a host/session picker with state, caption and “Pin to
  document”. “Go to figure” goes through existing navigation history.
  “Rebind” replaces only the reference in one undoable edit. An unavailable
  binding keeps its caption and rectangle, with a relevant action such as
  “Pair host”, “Reconnect” or “Rebind”.
- A document can carry references to existing sessions through Loro or file
  export. Socket paths, network endpoints, device approvals, credentials,
  VM identifiers, buffers and output remain in their owning services or local
  registries. Importing a document creates no processes or host connections
  to unknown endpoints. In offline preview, use the same placeholder and
  caption as disconnected mode. Only a visible reference on a locally trusted
  host can auto-attach as an observer.
- Document resize edits W/H through the existing figure edit path. The
  committed logical rectangle and unzoomed terminal cell metrics produce the
  controller grid by flooring each dimension. Deduplicate identical grids and
  coalesce rapid rectangle changes, with a final committed update. Figures
  smaller than 20 columns or 5 rows show “Enlarge figure to control”.
  Fit observers' authoritative grids into their rectangles with
  letterboxing. Pinch zoom, viewport movement,
  device rotation, IME insets and moving a figure to another page affect the
  view. Explicit source edits to W/H and terminal font choices can change
  the grid. Document reflow alone preserves the grid if its rectangle size
  is unchanged.
- Hiding or deleting a figure, undoing insertion, leaving the page and closing
  the app detach or release the relevant viewer. Multiple figures pointing to
  one session share a client replica when their viewports permit it. A focused
  figure owns display facts for the controller. When a terminal ends, show its
  outcome and available last screen for the current viewing session. Opening
  that document later shows the finished record while it remains in Core.
  “Launch” and “Stop session” live in the Sessions actions and require explicit
  action. A stop confirms the exact session and leaves the VM running.

## H3. Core client and session lifecycle

Implement a reusable, transport-independent Rust client crate in emthin
(e.g. `crates/emthin-core-client`). Keep its wire tests free of Ghostty,
Smithay, Android and a running service. Use the checked OpenRPC and the
existing client as references.

- Local v1 connection uses Unix sockets, newline JSON-RPC, and `hello` with
  `generation: 1, framed: true` and supported features. Switch after the
  complete reply while preserving buffered bytes. Verify returned `core.id`
  before attaching any binding. Ordinary emthin clients omit the peer-core
  identity field. `HC_SOCKET` discovery belongs to an explicit local profile.
- Use the 9-byte little-endian frame header, 16 MiB wire payload ceiling,
  and independent control requests on channel zero. Maintain per-connection
  request IDs, per-channel ownership and bounded queues. Attach replies precede
  initial stream data. Event channels carry JSON lines, potentially split
  across DATA frames. The 256 KiB initial stream credit is replenished after
  bounded consumption by the parser or event decoder. Fragmented transport
  data, stalled streams and repaint coalescing must leave other streams and
  control responsive. The Core sends a repaint if pending output exceeds
  4 MiB. A locally exhausted parser budget closes and reattaches the stream
  from a fresh paint. Arbitrary byte dropping inside a VT stream corrupts
  state.
- Watch with `terminal.attach` `observe: true, resize_notifications: true`.
  Apply ordered RESIZE before DATA and preserve the CAN that starts a
  replacement paint. Require confirmed resize notifications for live native
  observation in the first release. An older path can still list sessions
  and shows “Viewer update needed” for observation. Credit and CLOSE are
  valid in either mode. Interactive DATA, RESIZE, FACTS, FOCUS and automatic
  replies belong to the active stream. Use AUTOMATIC_INPUT only when hello
  and attach both confirm it. The local legacy fallback uses DATA only for
  replies still required by the program. Observers send no query replies.
- Local catalogue starts with the `events.subscribe` snapshot. Treat
  `missed` as a replacement and discard covered updates. A new connection
  establishes a new sequence epoch. Source matching is by Core ID and
  canonical local address. A qualified machine address is a route. Avoid
  pairing a relay's ID with a remote machine's address.
- “Take control” is explicit. A successful interactive attach negotiates
  the current grid and display facts. Release or connection loss closes that
  channel and, if still visible, reattaches as observer. A reconnect opens
  in watch mode. Held modifiers, pending IME commits and unsent input become
  invalid at the end of the control generation. Each operation carries the
  document, binding and connection generation that produced it.
- Core keeps its last-active-viewer policy for `hc attach` and other direct
  clients. A standalone local connection coordinates figure focus within
  that client. A managed fleet uses the same gateway for phone and desktop
  control, with at most one gateway controller per terminal. An unmanaged
  `hc attach` can still resize a managed session. Treat a detected external
  size change as loss of local grid authority and release control instead
  of fighting it.
- Present statuses reported by OSC 7501 with provenance “Reported”, and
  use “Unknown” when absent. Distinguish running, lost, restarted, finished,
  offline and endpoint mismatch. A core restart may reconnect to live
  holders. A kept record can restart into a fresh shell. Reattaching to that
  address requires recognizing the restarted program generation and a new
  control action. Track program generations from attach epochs and
  lost/restarted events. Core's public records omit `program_started_ms`.
  A host or VM reboot can end the original program. Titles and labels help
  users find sessions and carry no authorization.

## H4. Native rendering and input

Use the Core fork's pinned libghostty-vt revision for native VT processing.
A small client VT crate may depend on its thin wrapper, with additive
read-only render state and input APIs if needed. Verify the pinned headers
and Android build before committing to a wrapper design. Keep PTY, holder,
server storage and transport I/O out of the VT/renderer boundary.

- A terminal processing worker owns the parser, scrollback and terminal state.
  Publish owned bounded cell updates with generation, grid, pen/style,
  grapheme width/continuation, cursor, palette and damage. The renderer
  consumes newest committed screen state. Frame skipping can coalesce
  presentation updates after parsing. Use modes from parsed state when
  encoding keys, mouse, paste and focus. Plain text is for copy, accessibility
  and diagnostics, with no second parser.
- Render grid backgrounds, cached glyphs, decorations, cursor and selection
  into the existing frame. Use a shared scene representation and atlas and
  platform-specific GL submission. Key atlas entries by font, glyph and
  raster scale. Preserve wide and combining clusters, default/indexed/true
  colors, inverse, underline, alternate screen, bracketed paste, cursor mode
  and application mouse mode. Keep ligatures, arbitrary shaders and new
  terminal protocols out of release scope. Show a capability limit for
  terminal images until an end-to-end bounded image lane exists. Do not
  advertise image support before that lane passes.
- Desktop adds a terminal render element to `element.rs` and the existing
  figure rendering stack. Draw directly to the Smithay GLES frame with
  damage clipping and restored GL state. Keep document texture revisions
  independent of terminal damage. Maintain existing Wayland figure clipping,
  ordering, hit routing and fallback behavior. A headless proof fixture can
  use a diagnostic cell dump before GPU work. The release path uses one
  compositor submission and draws terminal glyphs without a TUI shell,
  nested terminal window or per-update page raster.
- Android starts from the retained Typst layout in `src/layout.rs`. Export
  the same figure rectangles and stable keys used by desktop, in page
  coordinates for the accepted layout revision. Reuse the existing editor,
  reading anchors, page windows, status and action catalogue. Integrate a
  native GLES view for visible page content with the static rendered page
  texture, live rectangles and overlays in the same coordinate space.
  Page rasters upload on accepted page revision or context loss. Terminal
  output updates its own scene. JNI passes layout/cell batches or native
  scene ownership, with no base64 or PNG terminal frame transport. The render
  worker sees captured document data only. Stale scene completions cannot
  update a recycled page or late Activity. A software/unavailable-GPU mode
  keeps the normal document reader and inert figure actions.
- A terminal-focused Android `InputConnection` holds composing text locally
  and sends committed text once to the active control generation. A mode-aware
  encoder handles hardware keys and Ctrl/Alt/Esc/Tab/arrows from a compact
  key row. Explicit paste uses the current terminal paste mode. The keyboard
  first dismisses on Back, then focus leaves the terminal. Release on pause,
  connection loss, figure invalidation or controller change. Restore document
  caret and reading position with existing anchors. No document undo entries
  come from terminal input.
- A swipe scrolls the document until a terminal is explicitly focused.
  Inside a focused terminal, use terminal scrollback by default and report
  mouse/wheel events only in active application mouse mode. Provide a
  local selection gesture and contextual copy. Output cannot send input,
  open URLs or write the system clipboard by itself. Expose figure captions,
  connection state, control/close actions and a bounded screen-text reading
  action through the current Android accessibility path. Avoid announcing
  every output update.
- Visible clients coalesce drawing to display cadence. Keep bounded work per
  stream and a byte budget for scrollback, glyph textures and pending uploads.
  Start with 8 attached visible sessions per frontend, 1,000 history lines
  requested per attach, grid ceilings of 300 x 120 and 32 MiB per-frontend
  scrollback. A placeholder explains when a limit is reached. Use a 15
  visible-pane benchmark override to characterize scaling. Hidden figures
  have no live glyph raster/upload work. On Android stop live streams on
  background and recreate from Core on return without starting programs.

## H5. Pairing, transport and access

Deploy a per-host `emthin-connect` gateway. Go tsnet provides an embedded
Tailscale node and tailnet peer identity on its listener. A private Unix
listener accepts the same framed gateway protocol from the owning desktop
user after verifying peer UID. Its local policy has equivalent terminal
scopes. Phone and remote desktop initially use the installed Tailscale
client. The release target is zero SSH after tailnet enrollment. [Tailscale
tsnet server API](https://tailscale.com/docs/reference/tsnet-server-api).

1. The host owner enables the service and displays a QR URL with the canonical
   HTTPS tailnet hostname, expected 32-hex Core ID and pairing version.
   Candidate encoding is
   `https://<host>.<tailnet>.ts.net/pair?v=1&core=<core-id>`. The phone opens
   “Pair host”, scans or pastes it, then shows host and request status.
   Treat the link as address information. Rex documents the same distinction
   for its QR. [Rex CLI pairing reference](https://www.superlogical.com/rex/docs/reference/cli).
2. Validate scheme, canonical host, port 443, exact path, version, expected ID,
   length and absence of credentials/fragments. Follow zero redirects.
   Use verified HTTPS with tsnet TLS and normal CA/hostname checks. Ask tsnet
   WhoIs for the actual socket peer. Allow pairing requests from the owner's
   tailnet user identity by default and reject tagged nodes. Store grants by
   stable WhoIs node ID, owner identity and approved Core ID. Tailnet network
   policy and registry grants both apply.
3. `POST /v1/pair` requests a grant. The host CLI displays the verified
   device and owner and allows approve or deny. The phone polls
   `GET /v1/pair/<request-id>` for up to five minutes. Requests belong to the
   verified node and expire server-side. Keep one pending request per node.
   “Forget” removes local trust and disconnects. Host revoke removes the
   grant and closes active streams and leases. Owner mismatch, unavailable
   Tailscale, unreachable host, expired request, changed Core ID, invalid
   certificate and denied device have distinct recovery messages.
4. Each enabled fleet host supplies its own endpoint. Android aggregates
   catalogues by verified Core ID and shows the host for each row. This
   reaches multiple hosts without delegating through `hc machine.add` or
   an SSH hop. Existing CLI aggregation can keep its configured transport.
   Zero SSH applies to the remote login path from the client to the Core
   host. The VM continues to use OpenShell's local authenticated exec path.

Use an authenticated WSS endpoint `/v1/stream` with WebSocket subprotocol
`emthin-terminal.v1`. Its first message is a binary channel-zero JSON-RPC
`hello` frame. Reuse Core's framed envelope. Each binary WebSocket message
contains one complete frame after WebSocket fragmentation is reassembled.
The [gateway v1 contract](HERDR_GATEWAY_V1.md) specifies field shapes,
frame bounds, state transitions and examples for the shared fixtures.

| Channel-zero method | Contract |
|---|---|
| `hello` | Return gateway version, Core ID and capability names. Require this before further calls. |
| `sessions.list` | Return a bounded revisioned projection of enrolled local sessions. Support unchanged revisions and pages. The client refreshes on foreground, while the picker is open and after lifecycle changes. |
| `terminal.attach` | Accept a local `tN`, observe/control mode, bounded history, size and display facts. Resolve under the enrolled Core ID and active device grant. Return gateway channel and negotiated attach capabilities before any stream frames. |
| `control.request` / `control.respond` | Request handoff for a busy session. Send a revisioned prompt carrying the current lease ID to the controller. Accept releases the old channel and reserves the next attach for the requester for 15 seconds. Deny, expiry and disconnect clear the request. |
| `session.stop` | Accept an address, current program generation and request ID. Apply an explicitly confirmed stop to a target with a stop grant and no automatic replacement. Return stale-target on mismatch. |
| `fleet.status` / `fleet.launch` | Return approved profile readiness and run one approved profile/slot after explicit action. The launcher contract is in H6. Advertise only implemented capabilities. |

A session row contains Core ID, local address, user-facing label, grid,
program generation, reported status, lifecycle, and allowed actions. Keep
full argv, cwd, environment, topology and arbitrary labels on the host.
The gateway can consume local Core events once to maintain the projection
and send a channel-zero `sessions.changed` revision hint to attached clients.
Clients coalesce hints, replace the latest snapshot only and reload on
reconnect. Revision and channels belong to one connection epoch.

The gateway terminates control RPC and opens local Core streams itself.
A writer lease binds a device and the specific channel to one session.
If a controller exists, return “In use” with an option to request handoff.
The current controller explicitly releases or accepts before the new one
acquires. Closing a connection, invalidating the terminal/program generation,
or revoking the grant closes its streams. Release by CLOSE. Serialize lease
checks and stream forwarding so an old generation cannot send after
handoff. A viewer registered only to watch never acquires a controller.
Enrolled inventory is configured on the host. A fleet label alone grants
no access. Resolve scope before attaching and recheck it on each inbound
input or control action.

Forward safe, expected per-channel frame kinds and credit accounting.
Budget both legs independently or preserve the exact end-client credit
through a bounded forwarding buffer. Use one bounded read/write pipeline
per connection with fair scheduling and no cross-channel head-of-line
wait on credit. Bound requests and reads before allocating (initial
defaults: 64 KiB remote control messages, up to 8 live attaches per frontend,
16 MiB frame hard ceiling). Close a slow/revoked stream independently.
Reject unknown channel IDs, invalid lengths, unexpected frame directions,
unsupported features and unapproved RPC methods. Keep the listener on
the tailnet. Pairing and session controls never accept a caller-provided
socket, executable, filesystem path, shell arguments or raw host RPC.
Logs record device, scope, session, outcome and timings with output and
input excluded.

## H6. OpenShell fleet and sharing

Reuse `../OpenShell/openshell-nixos` as the deployment root. A host may
enroll existing terminals immediately after matching its live Core and
sandbox inventory. A host owner explicitly grants watch, control, launch
and stop rights separately. The default fleet profile exposes the existing
approved fleet slots to paired devices with the selected rights.

- Fleet registration records host Core ID, canonical terminal address,
  sandbox ID, deployment generation, profile and slot. For existing
  terminals, the host operator verifies the matching `openshell sandbox exec`
  process and runtime before enrolling. Launch-managed registration is
  created from the successful local launcher result. Listing a known VM
  with a matching name alone is insufficient to reuse it.
- Keep one libkrun OpenShell fleet VM per host, with distinct agent sessions
  in it. Share `/nix` read-only and the selected fleet directory read-write
  as `/workspace`. Use the existing Freebuff and Infinity command profiles
  with absolute store paths. Their per-slot writable state and credentials
  use separate directories. The fleet VM is the isolation boundary for those
  workloads. Reuse the existing host inference service and policy for its
  CPU/Vulkan endpoints. The phone gets the approved terminal view and status.
- Split `fb-fleet` into read-only plan and explicit apply paths. Plan returns
  structured requested slots, backend, policy, mounts and expected closure
  from configuration and read-only lookups. Apply creates missing mutable
  directories without replacing existing ones, stages missing store artifacts
  once, and uses the existing authenticated OpenShell CLI/SDK and Core to
  launch. Launch requests contain profile, slot and client request ID
  only. Serialize by slot and persist completion plus in-flight state.
  Retry reconciles via resolved sandbox ID and Core result. An uncertain
  launch reports “Needs reconciliation” and avoids duplicate processes.
- Wait for Ready, verify the resolved VM policy/profile and effective mounts,
  then start `openshell sandbox exec --tty` under Core. Pin the CLI build
  that provides streaming gRPC exec with resize forwarding. Use its
  long-running timeout setting and validate that Core's grid reaches the
  guest PTY. A failed Ready check leaves a failure state and launches
  zero terminals. Existing compatible
  Ready sandboxes and live slots are reused. Reuse after VM replacement
  requires reconciling the deployment generation. VM stop is an operator
  action distinct from detach and session stop. Existing non-libkrun
  profiles remain independently usable and show their actual capabilities.
- Verify that stopping the host PTY closes the specific guest exec and its
  managed foreground workload. Show “Stop unverified” while the guest
  outcome is uncertain. Enable remote Stop for a profile after its
  targeted teardown passes. If host PTY closure leaves a workload alive,
  use the existing scoped exec lifecycle or track its guest process group
  for a targeted teardown.
- Pin compatible emthin, velysterm, Herdr, Core, OpenShell, Ghostty/libkrun
  source revisions and Nix inputs. Package the requested fork builds
  expressly. Keep the upstream Core release package as a separate
  compatibility input. Root the shared store closure during fleet use with
  the Nix package/profile so garbage collection cannot remove running agent
  binaries. Build per target architecture and reuse the same closure within
  that host. Keep deployment, host-only secrets and service startup behind
  separate explicit host provisioning.
- Reuse the fork's libkrun virtiofs DAX path and its
  `OPENSHELL_VM_DAX_WINDOW_MIB=128` baseline. Report disk sharing from verified
  identical store paths/content in the host and guest. Report memory sharing
  from an effective DAX mount and repeatable read/mapping evidence on the
  intended backend. “Configured”, “Verified”, “Unavailable” and “Unknown”
  stay distinct. Report host and guest memory metrics with separate private
  RSS/PSS and page-cache evidence. Scope DAX savings to file-backed data on
  that host. Agent heaps, GPU allocations, private model state and other
  hosts have their own allocation. KSM counters alone establish neither
  DAX use nor guest page-cache sharing.
- Infinity status starts from the existing OSC 7501 wrapper after separating
  process alive from agent idle and reporting actual failure/exit. Its
  existing owned turn/cancellation state is the source for any later
  per-turn reports. Until a producer emits them, show unknown turn status.
  Core interruption follows its existing signal semantics. Agent-specific
  cancellation belongs to the application. Provider smoke tests can use a
  local stub and fixture program for the same routing.

## H7. Legacy Herdr adapter

Keep the older runtime and its state independent. Anywhere “Sessions” offers
legacy Herdr, show a separate local endpoint label and its reported or
inferred status provenance. Implement the read-only desktop adapter first.

- Negotiate generation 1 `endpoint.hello.v1`/`endpoint.welcome.v1` and the
  baseline `shell.snapshot.v1` and `shell.surface.v1` codecs. Read the
  published pane rectangles/IDs from the active-tab pane surface and crop
  the selected pane into a figure drawn by the same glyph renderer, with
  Herdr's grid letterboxed into the document rectangle. Server layout
  handles addresses inside that surface only. Use the existing surface
  interest/activation path where supported. Start with full replacements.
  Leave optional retained/delta/scroll and graphics codecs unadvertised
  until implemented and checked against released fixtures.
- A local binding is `terminal: "herdr-local://<local-profile-id>/<pane-id>"`.
  Use canonical percent encoding for pane IDs. The local profile ID is a
  random 32-hex registry key. Registry records server boot/revision and
  pinned selection. A change of boot requires confirmation of the pane's
  current identity. If the pane leaves the delivered active-tab surface,
  show “Pane not on active Herdr tab” and offer “Open in Herdr”. A missing
  pane keeps its document rectangle. A remote phone can display the
  caption and backend-required state for such figures.
- Stay passive. Reuse the server-reported surface size and prove a second
  client leaves the existing client's layout, selection and focus intact.
  If the shipped endpoint cannot provide a passive surface, add an
  opt-in capability for a bounded single-pane read-only surface. Freeze
  generation-1 codecs. The release adapter needs only display, bind and
  handoff to the existing Herdr client. Any future interactive legacy mode
  needs pane-targeted input and explicit focus ownership instead of sending
  keystrokes to whichever pane happens to be current.

## H8. Delivery criteria

Use the [integrated acceptance run](HERDR_ACCEPTANCE_RUN.md) for fixture
setup, evidence and rollback.

| Capability | Evidence required |
|---|---|
| Android daily use | Pair by QR or paste on a real arm64 phone, attach to an existing OpenShell session, enter CJK/emoji and control keys, rotate, background, reconnect, hand off to desktop, and restore document/reading position. |
| Native display | Core attach output changes terminal damage only. Record one compositor submission per frame and no terminal raster encode/decode or Typst layout on the output path. Old native Herdr surface and existing Wayland figures still work. |
| Zero SSH | Direct phone and desktop WSS sessions reach two enrolled hosts with SSH stopped or blocked in the test environment. Disconnect/revoke works without ending programs. |
| OpenShell integration | Attach an existing terminal without starting a VM or program, launch a fixture slot in the same Ready fleet through an approved profile, stop just that slot, and verify the existing adjacent slot remains active. |
| Disk and memory | Record backend/package versions and effective mounts. Compare controlled guest cache and host PSS with/without DAX for the same store file. Confirm the same closure is used by multiple slots and mutable homes stay independent. |
| Latency | Report p50/p95/p99 for input to present and first usable paint, plus CPU, allocations, uploads, host RSS/PSS and frame misses at 1 and 15 panes. Compare the current Herdr-in-terminal baseline and native Core on the same host and workload. Record refresh, temperatures, renderer, RTT and run count. |

Set a candidate performance gate of p95 at or below two refresh intervals
for warmed local echo on the declared reference host at one visible pane,
and lower p95 and client CPU than the matched TUI baseline during redraw.
Keep a 15-pane regression budget after the initial benchmark. Measure
server processing, client parse/draw and presentation separately. Use the
same trace on Android with local-network and shaped link measurements.
These are release targets. A direct Rex performance claim requires a matched
Rex run. The architectural target is removing the extra display stage.
