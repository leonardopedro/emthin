# Testing depths — the R15 lane labels, per repo

Round-2 R15 deferred "formal depth labels"; this catalog is the execution.
The taxonomy is `rea/docs/testing.md`'s five behavioral depths, adopted
whole (ideas only — the reference repo ships no LICENSE, nothing is
vendored; see workspace `ATTRIBUTION.md`):

| lane | meaning |
|---|---|
| **module** | in-process, no production boundary: units, pure functions, model invariants |
| **composition** | wiring across modules in one process; fixtures are materialized, not real user state |
| **boundary** | exactly one production boundary crossed: a socket, a filesystem, a compositor, a GPU |
| **acceptance** | a whole product journey: the thing a user runs, driven end to end |
| **conformance** | a contract or golden held against an independent producer (registry, CAS, paper datapoints) |

Why a catalog and not inline tags: the seven AGENTS.md files name their
verification surfaces in seven different structures (Maintenance Checklist,
Testing, Commands, Scripts & gates, Verify, …), and a taxonomy paragraph in
each would drift seven times. The labels live here, next to the gates that
police claims; the AGENTS files keep owning the *commands*. A suite's lane
is a property of what it crosses, not of where it lives — so a `#[test]`
can be a boundary test, and one command can honestly carry two lanes.

What a label licenses: a green **module** run never backs a boundary claim
(the emthin E2E regression of 2026-10-15 is the worked example — 347
module-green through a red acceptance lane; see
`emthin/docs/measurement-surfaces.md` for the classification rule). When
reviewing a change, cite the lane that actually crosses the seam you
touched. Unavailable authority is a **named skip** (`[gap]`, exit 2, SKIP)
— never a silent one, never a pass; that half was already implemented by
the gap culture and is inherited here, not re-litigated.

## unfer

| lane | surface | evidence |
|---|---|---|
| module + composition | the workspace suite (1797) | `cargo test --workspace --exclude fock_sirk` (README Verification) |
| boundary | `ops_e2e` (real socket), the fuzz lane | `cargo test -p unfer_ffi --test ops_e2e`; `.github/workflows/fuzz.yml` |
| conformance | physics/SIRK goldens against Cadabra2 CHECKs (462 across 64 binaries, 2026-10-18; the README's per-suite prose keeps its own dated history) | `cargo test -p fock_sirk --release`; heavy lane `scripts/run_heavy_tests.sh` + `.github/workflows/heavy-tests.yml` (weekly) |
| conformance | decide calibration report pinned to its run | `decide_report` golden gate (D6 in ci.yml) |
| named skip | three intermittents in `unfer_ffi`'s action/owner-log group | documented in README; counted, not hidden |

## emthin

| lane | surface | evidence |
|---|---|---|
| module | the workspace suite (347): docui, keymap, session serde, figure binding, IPC protocol units | `nix develop -c cargo test --workspace` |
| boundary | clipboard/dbus crate against a real session bus; xwayland supervisor timing | the `emthin-dbus` suites inside the same run |
| boundary | the tier-1 replay scenario: sandboxed `doc` → `apply` edit script → closed-vocab `expect` (13 asserts, ~0.8 s; the scenario cannot skip a check it was written to make) | `./scripts/agent/replay.sh scripts/agent/scenarios/figure-resize.txt` (CI `e2e` job, first step) |
| acceptance | the §6 step-7 E2E: XTEST keystrokes → document, clipboard both ways, `list_state`, SIGTERM | `./scripts/e2e/run.sh --config xdotool` (5 checks; CI `e2e` job) |
| named skip | IME commit (needs the host's fcitx *engaged*, not merely installed) | `scripts/e2e/README.md`; clipboard-free rerun is `E2E_NO_X=1` |
| module | `emthin-pad`'s 24 host tests against the real `DocModel` — sync/undo, UTF-16 conversion, and the mathed page seam (`mathed_mini`'s paged layout), incl. the per-page RGBA base64 contract the Java side decodes, chunk continuity across pagination, the scale/chunk fallbacks, and the canvas pin (Java `pageView` bg == `mathed_android::engine::BG`) (standalone workspace — **not** in the 347 above) | `cargo test` inside `emthin-android/` |
| module | `mathed_android`'s 29 host tests: the engine, the agent-loop debug protocol and `mathed_probe` (also its own workspace, outside the 996) | `cargo test --manifest-path crates/mathed_android/Cargo.toml` |
| boundary | `mathed_probe --local shot` — the frame the window *would* blit, rasterised on the host with no device (semantic tier: `summary`/`figures`/`layout`) | `crates/mathed_android/target/debug/mathed_probe --local …` |
| named skip | the APK build + waydroid run | `emthin-android/build-apk.sh` hard-pins four Nix store paths (SDK/build-tools/NDK/JDK); root-side checks in `scripts/waydroid-sudo-fixes.sh` — owner-run, no CI |

## velysterm

| lane | surface | evidence |
|---|---|---|
| module + composition | mathed_core / mathed_mini / mathed / mathed_biblio CPU suites (996 incl. doc-tests; round 3 added tier-1b `semantic_facts` + `docs_diff` inspect tests, then the stub-race regression test) | `nix develop -c cargo test --workspace` |
| boundary | `mathed`'s `kernel_smoke` and `mathed_mini`'s `overlay_renders_green` — document → KernelBridge → prob_kernel → annotation/pixels | CI `smoke` job (headless) |
| boundary | slint_blocks driving the real helper process (process boundary) | `cargo test -p mathed` with the helper built; self-skip when absent |
| conformance | `AGENT_OPS` registry ↔ dispatch ↔ `docs/AGENT_OP_CATALOG.md` | `python3 scripts/check-op-catalog` (+ the F4 prose guard) |
| named skip | GPU delta suites without an adapter; slint tests without the helper | the Cadabra2 skip pattern — SKIP, never FAIL |

## dynamic-arctic

| lane | surface | evidence |
|---|---|---|
| module + composition | the workspace suite (74: 23 lib + 51 bin/server) | `cargo test` (README Verification) |
| boundary | the unfer threshold-mint integration (crosses into `unfer_consensus`) | `cargo test` incl. the bin target |
| conformance | the paper's Figure 7a datapoints | `repro/repro_fig7a`; weekly `.github/workflows/heavy-repro.yml` (a record, not a gate) |

## australVM

| lane | surface | evidence |
|---|---|---|
| module + composition | the OCaml suite (110 dune tests / 14 suites) | `make test` / dune |
| conformance | symbol census vs `SYMBOL_REGISTRY`; extracted Tutorial.lean | `scripts/verify-invariants`; `verify/ArcticAuthority/Extracted/Tutorial.lean` |
| named skip | the Lean lane where `lake` is absent | `lake` = recorded `[gap]`, never a silent pass |

## timepiece

| lane | surface | evidence |
|---|---|---|
| conformance | maintenance invariants (6 checks incl. the gitbook drift gate) | `bash scripts/verify-invariants`; `scripts/check-gitbook-drift` |
| conformance | vendored mass-gap certificate byte-identical to live unfer | H1.4 `cmp` check inside verify-invariants |
| named skip | the `lake` build gate; the whole Lean lane | `TIMEPIECE_SKIP_LAKE=1` / lean-toolchain absent → `[gap]`; the status block says "not run", honestly |
| obligation ledger | planned-but-unproved lemmas cited by the GitBook | `scripts/pending-lemmas.txt` (documented forward references — reported `[pending]`, not failed) |

## test (the GitBook)

| lane | surface | evidence |
|---|---|---|
| module | SUMMARY consistency, link graph, backlinks/orphans | `python3 scripts/check_site.py [--backlinks]` |
| conformance | attribution and error-copy ledgers vs the workspace | `scripts/check-attribution`, `scripts/check-error-catalog` |

## Scope notes

- The labels are descriptive, not enforced: nothing fails a build for a
  mis-labeled suite. Enforcement of the *claims* around suites is the job
  of `check-status` (counts), the F4/P4 prose guards (prose), and the
  per-repo catalog gates.
- "Prune tests that mirror implementation details" (rea's other half of
  R15) is a per-suite review decision and stays with each repo's owner;
  nothing this round pruned a passing test.
- Adding a verification surface? Give it a lane row here in the same
  change — the row is small, and its absence is the drift this catalog
  exists to prevent.
