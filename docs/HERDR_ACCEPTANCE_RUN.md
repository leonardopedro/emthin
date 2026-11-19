# Integrated acceptance run

Use this run for the [implementation plan](HERDR_INTEGRATION_PLAN.md),
starting with the disposable Android fixture. The inputs here are test
contracts. Gateway commands and test instrumentation are deliverables of
the implementation cards. Record an explicit gap for hardware or a
deployment you cannot exercise. Each section produces a result that can
be reviewed independently.

## Fixture setup

| Input | Required shape |
|---|---|
| Client builds | One identified desktop build, one arm64 disposable test APK (`com.emthin.pad.test`), and the corresponding normal APK. Record Git revisions, build flags, archive hash and the installed test APK hash. |
| Host configurations | Two paired test nodes with different Core IDs. Host A runs the OpenShell fleet. Host B can run the deterministic fixture directly in its isolated Core. Use explicit fixture Core socket and state paths with a scratch `HC_SESSION` and scratch XDG paths. |
| Host A inventory | Two long-lived named fixture slots in one Ready VM using a shared Nix closure. Enroll slot A in the gateway. Slot B is a sentinel for isolation within the fleet lifecycle. Keep a visible ordinary Core terminal outside remote inventory to test filtering. Provide one empty, approved launch slot. |
| Pairing grants | Same-owner test phone and desktop with watch/control on enrolled slots. Include a watch-only grant and an unapproved device identity in fake-peer tests. Only the launch-test scope grants launch and stop. |
| Test document | Two or more pages with an ordinary document heading and prose, one Wayland figure for the desktop lane, two Core terminal figures and one malformed terminal reference. Add enough intervening text to move a figure across a page boundary on an explicit edit. The document itself contains no trust data. |
| VT fixture | Print a deterministic marker, styles, a double-width glyph and a combining cluster. Echo a nonce, report actual PTY size on resize, emit explicit OSC 7501 state, and expose a deterministic exit path. Include a bounded full-screen redraw mode and one bracketed-paste test. Give each process instance a distinct nonce. |
| Agent fixture | Run the same echo/resize fixture through the real OpenShell exec path. Use local provider fixtures for any Infinity startup exercise. Track the guest process/group ID by a host-controlled verification probe. |
| Resources | Record backend, binary hashes, Nix closure paths and mount configuration. Use a disposable VM for the DAX on/off comparison. Keep the production fleet and user document out of the measurement lane. |

Use the fixture's generation and positive echo ack to distinguish “sent”
from “rendered”. Record a separate Core ID, terminal address, guest process
nonce and document marker pair for every observed slot. Do not collect
real terminal scrollback, device keys, provider secrets or full environment
dumps in the run report.

## Run A. Android scene and document ownership

This lane exercises T05–T09 with a fake host first and then a fixture Core.

1. Open the two-page fixture in the disposable APK. Assert that only
   explicit Pair or preconfigured fixture transport opens a connection.
   Open Sessions, pin a fixture at the caret, then verify one Undo and
   one Redo remove and restore its source and figure. The server observes
   no process launch/kill for either action.
2. Scroll to the other page and back. Restore the same source anchor.
   Leave one terminal producing updates while zooming/panning. The active
   figure position matches the current Typst rectangle, while the Core grid
   stays unchanged. An explicit W/H edit changes the controller grid
   exactly once at its final size. Its next fixture resize report matches.
3. A watching figure has readable output and rejects input. Explicit
   Take control, type a hardware key, commit composed CJK then emoji,
   and send a bracketed paste. Assert raw input and process echo match
   the fixture exactly. Back dismisses the IME first and leaves terminal
   focus next. No terminal input changes document text or undo state.
4. Start IME composition, then release the writer before commit. Deliver
   the late commit through the disposable IME path. The fixture records
   zero bytes from that old generation and the document caret returns to
   its anchored location. Repeated focus/blur does not transfer control.
5. Background, rotate, foreground and destroy/recreate while terminal
   output is active. Reconnect as watch, confirm the fixture instance
   nonce stays the same and verify no stale page, input, lease or upload
   completion can revive a released session. Assert the live parser,
   view and recovery lanes respect their respective owners.
6. Save, force-stop and reopen the test APK. Assert the reference and
   document reading position persist. Check accessibility exposure of
   caption, state and the explicit bounded screen reading action.

Capture accepted layout count, page raster and upload count, terminal
damage/submit count, stream open/close and instance nonce before and after
each action. Output-only updates require zero new Typst layouts and zero
page-raster encodes. Page rotation, layout changes and context loss can
legitimately upload a page texture. The counter is scoped to its cause.

## Run B. Pairing, control and zero SSH

This lane requires an actual arm64 phone, the Tailscale client and two
test Core hosts. Use the [gateway v1 contract](HERDR_GATEWAY_V1.md) for
transport assertions.

1. Pair host A by scanning its QR code and host B by pasting its link.
   Confirm the host CLI shows the verified requesting node and pairing
   identity. Before approval, stream access must fail. After approval,
   the app shows two distinct hosts and just their enrolled sessions.
   Record both Core IDs and the per-host pairing outcomes.
2. Repeat one rejected pair case, such as wrong Core ID or denied node,
   and verify any existing hosts remain connected. Reject a
   non-canonical URL and certificate mismatch in automated transport
   fixtures. Use the managed fixture host for physical scan tests.
3. With SSH disabled or blocked on both fixture hosts, attach to one
   terminal on each host. Send one control nonce to each and read back
   the echo. Record the transport classification and target Core ID.
   The phone and remote desktop routes both use WSS. The Core translation
   stays local to each gateway, including when a fixture host is a relay
   peer in the existing CLI configuration.
4. Start with the desktop controlling host A and the phone watching. A
   phone control request returns In use, then the desktop receives a
   handoff request. Accept it and take control from the phone. Check
   the desktop's old channel closes before the phone's input appears.
   Observe a second handoff denied and prove the controller is unchanged.
5. Put the phone in airplane mode with an uncommitted input, reconnect
   over Tailscale, then prove no queued text was replayed. Confirm
   the program nonce and current page survive. Revoke the phone's
   host A grant. Its active connection must close and reopening must be
   denied. Host B remains reachable, and host A's original program
   remains alive for the desktop.

Run the fake peer lane for policy failure and frame/credit races.
Time spent unable to get a real tailnet or physical Android device is
a gap for Run B even if all transport fixtures pass.
Restore the phone's fixture grant before running the authorized launch
and stop cases in Run C.

## Run C. OpenShell existing and new slots

1. Record the Ready fleet sandbox ID, device profile, mount facts,
   agent closure paths and current Core records. Use real read-only
   status queries and the configured guest check. Attach to the existing
   enrolled slot. Its Core address, guest process nonce and VM/closure
   identifiers must agree with the baseline. Launch counters stay zero.
2. Request one approved empty slot from the phone. Before starting it,
   the host checks readiness and effective mounts. Reissue the same
   request after success. The repeated attempt yields the same slot,
   Core address and guest process nonce. Concurrent duplicate launch
   and uncertain-reply recovery also pass in fake launcher tests.
3. Resize its document figure with desktop control. Verify the Core
   grid and guest PTY grid agree. Then switch the viewer to observer
   or close emthin. The guest stays alive and the current fleet
   profile and mount set stay intact.
4. As an authorized mobile user, stop the launched slot. Observe end of
   the host PTY and verify the instance disappears from the guest
   process group. Confirm slot B still echoes its nonce and the VM
   remains Ready. Mounted data and per-slot homes persist. If guest
   termination is uncertain, the API and UI must show that outcome.
5. Repeat the plan path and Ready failure with fake CLI programs.
   Check no agent/provider secrets are opened, no Nix store/write
   action runs in plan, and no Core terminal is started on failure.
   Check a misleading sandbox name cannot match a different VM.

Keep legacy Herdr compatibility verification with the native desktop
renderer lane. Its passive view must retain the original client's focus
and grid. After an endpoint restart, require confirmation for the
local pane binding. Its files and runtime are a separate rollback unit.

## Run D. Nix sharing and display measurements

For sharing, use the same regular Nix store file and fixture operation
in both runs. Pin the OpenShell driver and libkrun build. Record the
effective guest `/nix` and `/workspace` mount flags, host and guest
store paths and content hashes, closure sizes, and DAX window settings.
Use a file that fits the tested window for the first comparison. Hold
VM size, workload, host cache strategy and guest baseline constant.

- Disk evidence comes from one host store closure visible at the
  same guest path and serving more than one slot. Compare hashes and
  deployment mount provenance. Guest inode numbers may differ
  across layers. Verify independent mutable writes in two scratch
  slot homes and failed guest writes to the read-only store.
- Memory evidence comes from a DAX-enabled mount plus comparable
  guest cache results while reading or mapping the same file. Capture
  guest cache pre/post and host PSS/RSS or matching mapping detail
  pre/post for the same backend. Record byte units and which process
  each metric describes. Use repeatable commanded runs, with a separate
  DAX-off comparison. Record private process allocations separately.
- Report measured host-local evidence and the package/VM fingerprint
  with the result. Changing driver, parent VM or mount configuration
  clears Verified. One deployment's evidence must never mark another
  host or backend as verified.

For performance, compare native Core rendering and Herdr inside a
terminal on the same desktop with identical fixture workload and
logical grid. Start with one pane, then 15 visible and predominantly
hidden panes. On Android use the same workload with local network and a
recorded shaped link. Record p50/p95/p99 for first paint and input to
actual presentation, CPU, allocations, uploads, missed frames, and
host/client memory. Measure server parse, transport/RTT, client parse,
draw/submit and presentation separately when timing supports it.
Report the target refresh rate and hardware. A test log of draw calls
counts as submission evidence only. Compare to the release gates in
[H8](HERDR_INTEGRATION_SPEC.md#h8-delivery-criteria).

## Result and rollback

Record one row for every run.

| Field | Content |
|---|---|
| Test record | Run ID, card(s), UTC time, package hashes and exact relevant config revisions. |
| Environment | Desktop and phone model/OS, VM backend, Nix and OpenShell revisions, Tailscale topology, refresh and test routes. |
| Result | Pass, fail or gap, with observed evidence and expected outcome. |
| Workload identity | Fixture Core ID/address, VM ID, process nonce, acceptance counters. Avoid secret and production identifiers. |
| Cleanup | Close fixture clients, revoke fixture grants, remove only disposable app data and stop only fixture services and VMs created for this run. Record any persistent evidence. |

Disable the new service in the disposable deployment and reopen the
document. Its captions and positions remain available with unavailable
terminal state. Existing Core, old Herdr and the OpenShell fleet remain
usable through their original entry points. Verify that any rollback
stays within the selected test deployment.
