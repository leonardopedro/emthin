# Herdr, OpenShell and Android integration review

Reviewed 2026-11-18. Local source baselines are below. The emthin worktree
also contains in-progress Android and collaboration changes. Source paths
outside this checkout are given relative to its root.

| Checkout | Revision |
|---|---|
| emthin | `d86710cd9ed088c1c7747b68adc3d05454a0a797` |
| velysterm | `423a8c453128f085369d72d68b57bb92b13510c8` |
| Herdr 0.9.5 | `ee5bfb5041bb3e4a49d591c72308aae3c81ecdde` |
| Herdr Core 0.1.21 | `c7517808d64f06609e4ecf0db1757005373cc3a3` |
| OpenShell | `f3bc6ece8aedfc7d2bdfa0e04ab938458934a5dc` |
| Core's Ghostty submodule pin | `1e5cdb865807abc990181efb8ebfa9122955cce2`, uninitialized in this checkout |

## Integration decisions

| Evidence | Decision |
|---|---|
| Herdr 0.9.5 owns its PTYs, stored state, layout and Ratatui shell. It has no Core dependency. Its native endpoint already exports styled cells, snapshots, surface interest and optional delta and graphics capabilities. See `../herdr/src/protocol/{endpoint,wire}.rs`, `src/server/headless/{render,endpoint_requests,native_graphics}.rs`. | Integrate through existing endpoint protocols. Keep old state in Herdr and add a passive desktop view. Reuse its pane rectangles only to locate the source cells. Use the Core path for portable interactive figures. |
| Core's `schema/README.md`, `schema/hc.openrpc.json`, `src/{wire,viewers,server,terminals}.rs` and `src/api/types.rs` expose raw attach, flow control, event snapshots, authoritative paints and observer grids. Core stays headless. Terminal identity in its record is for terminal emulation, with the persistent service and terminal addressed by `core.id` and `tN`. | Build one reusable Core client for Android and desktop. Use document references to existing PTYs. Preserve Core's ABI and model of terminal ownership. |
| `../herdr-core/crates/ghostty-vt/{src/lib.rs,build.rs}` has a VT parser, formatter, input encoding and plain screen reads. There is no exported styled rendering API in this wrapper. The observed target mapping does not cover Android. | Prove pinned Ghostty arm64 Android compilation and a read-only styled-cell API first. Do not build the whole Core daemon into the APK. Reuse parser and renderer across platforms. |
| emthin's `docui/figures.rs` binds figures by marker pair. The Smithay frame already layers a page texture, figure elements and overlays. The pinned Smithay GLES frame has a custom-context drawing hook. Android `src/layout.rs` retains Typst frames on its layout worker and `PadSession` rejects results for a replaced document/Activity. | Add one existing figure content variant and reuse its document rect. Draw both terminal scene and static document into the platform's existing frame path. Keep terminal output off the document render and recovery lanes. |
| `../herdr-core/docs/security.md` treats the Core socket as owner capability. Its full API includes execution, files and peer administration. The `herdr-core.md` deployment note about remaining inside OpenShell policy applies to the sandboxed workload, with the Core host socket outside that policy. | Terminate remote control at a scoped host gateway. Pairing a phone alone grants neither arbitrary host RPC nor a launch profile. Explicitly enroll eligible terminals and separate watch, control, stop and launch rights. |
| Core uses `hc connect` as a byte transport, and the existing fleet plan uses Tailscale SSH for aggregation. Rex documents a tailnet HTTPS endpoint and QR address sharing. | Deploy a direct HTTPS/WSS endpoint per Core host. Aggregate host catalogues in the app and validate the full two-host route with SSH unavailable. Use the installed Tailscale VPN app on Android for the first release. [Rex remote access](https://www.superlogical.com/rex/docs/linux/remote-access), [Rex CLI pairing](https://www.superlogical.com/rex/docs/reference/cli). |

The rendering hypothesis is that removing the TUI-to-host-terminal display
stage reduces latency and work. Core and a native client can each hold VT
state. The desktop remains nested under its host compositor. The experiment
compares matching workloads and needs measured results before claiming a
latency advantage.

## OpenShell and Nix

`../OpenShell/openshell-nixos/fleet-plan.md` records an existing laptop
deployment with three Freebuff sessions and one Infinity session inside
one Ready VM. `fb-fleet` runs their PTYs under Core and connects those
PTYs to `openshell sandbox exec --tty`. `herdr-core-agent` is a separate
single-agent launcher. `herdr-core-config.toml` suppresses notes injection
and enables screen restoration. The Nix module and gateway configuration
already carry local OpenShell service and mount settings.

The fork CLI dispatches sandbox exec to `sandbox_exec_grpc` in
`../OpenShell/crates/openshell-cli/src/{main,run}.rs`. With a TTY its streaming
path forwards SIGWINCH as an exec resize message. The checked Nix
derivation can still select a release binary. Verify resize and termination
with the actual packaged CLI and guest session.

The existing fleet mounts the host store read-only at `/nix` and the
fleet directory read-write at `/workspace`. The fork documents
`OPENSHELL_VM_DAX_WINDOW_MIB`, the virtiofs DAX window and guest mount
flags. The driver source is in `../OpenShell/crates/openshell-driver-vm/src`
(`main.rs`, `driver.rs`, `runtime.rs`). Its libkrun route calls
`krun_add_virtiofs4`. The README and `dedup.md` record earlier comparisons
of guest cache growth with and without DAX on a roughly 126 MiB store
file. Those are historical measurements for those builds. Verify the
selected package, backend and live mount before displaying a sharing claim.
The alternate cloud-hypervisor profile needs its own evidence.

| Finding | Effect on work |
|---|---|
| The tracked `openshell-nixos/flake.nix` contains conflict markers at lines 50–58. Existing attributes mix a pinned upstream Core release and nixpkgs Herdr. | Resolve Nix evaluation first and package the two requested forks from explicit source pins. Keep upstream builds separately testable. |
| `fb-fleet` stages state, replaces a slot's config, adds a store path and reads a credential before the `DRY_RUN` branch. Its timeout and VM lookup can both fail before it runs Core or prints Ready. | Build a real plan path and guarded idempotent apply. Keep the current launchers out of the review execution lane. |
| Freebuff's executable is accessed via its per-session config tree. Infinity uses a staged store path and defaults to a lowercase `infinity` checkout path while this workspace has `Infinity`. | Put common executables into pinned read-only closures and resolve the checkout path explicitly. Keep writable per-session state independent, including credentials and settings. |
| `herdr-core-agent` emits idle before launch and done from a generic EXIT trap. | Use actual exit state and genuine agent events for turn status. Core's OSC 7501 contract stays unchanged. |
| `gateway.toml` uses an authenticated loopback gateway and the deployment opts in to bind mounts. `crates/openshell-sdk` and the existing CLI provide host-side access. | Reuse local authenticated management. Keep keys and OpenShell admin API on the host. Verify that an enrolled terminal actually belongs to its selected VM and slot. |

Within one host, Nix package reuse reduces duplicated disk files and
the DAX route can avoid duplicated guest caching of mapped file-backed
pages. The shared fleet VM and one inference service also avoid per-pane
VM and model-service instances. KSM counters cannot establish those
effects. Private process and GPU allocations need separate accounting.

## How the original sibling study carries over

The [Android sibling study](../emthin-android/SIBLING_STUDY.md) records
the inspected source and previous editor decisions for all nine siblings.

| Sibling | Reuse for the terminal integration |
|---|---|
| Kyneta | Extend owner and generation checks to reconnect, handoff, render completion and stable reading positions. |
| Pinetrie | Put terminal details and commands in the existing contextual actions while keeping the page visible. |
| Atuin Desktop (`desktop`) | Persist the document binding separately from live process/terminal state. |
| Fresh | Preserve the existing source editor and rendered page over one model. |
| Foot | Extend damage-only rendering, composing input and selection behavior to native terminal cells. |
| Helix | Reuse navigation history and anchored restoration when returning from a terminal figure. |
| Atuin CLI | Route actions by focus and command context. Existing shell-history tools continue working inside the attached terminal. |
| Starship | Read explicit context with bounded collection and refresh. Prompts remain part of the terminal's own output. |
| Infinity | Use application-owned turn/interrupt state and the existing provider, inside the OpenShell fleet. |

## Verification boundary

This review inspected source, revision pins and deployment documentation.
The emthin invariant gate passed during the review. Real Android/native
Ghostty linkage, gateway behavior, fleet launch, DAX and latency still need
the isolated implementation checkpoints. Zig and Go were absent from the
review shell's PATH, and the Core Ghostty submodule was uninitialized.

Continue with the [feature specification](HERDR_INTEGRATION_SPEC.md)
and [implementation cards](HERDR_INTEGRATION_PLAN.md).
