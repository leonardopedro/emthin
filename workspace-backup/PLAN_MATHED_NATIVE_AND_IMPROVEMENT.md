# Plan — mathed-on-Android adaptation + full-project improvement (2026-10-06)

> **Scope**: the whole project — `unfer/`, `australVM/`, `velysterm/`,
> `timepiece/`, `dynamic-arctic/`, `test/`, `emthin/` — plus the
> workspace-root plans. Deep review executed 2026-10-06 (direct tree
> reads + one explore agent for `emthin`; all claims below carry
> file:line citations).
> **Hard constraint**: no direct Lean 4 code. Lean-side items are
> written as *work orders* routed to `timepiece/CONSOLIDATED_PLAN.md`
> for the LLM-Lean4-specialist.
> **Improvement principle** (per the brief): *improving an existing
> feature beats creating a new one*. Every Priority-A item names the
> existing feature it improves; Priority-B items are the genuinely new
> mechanisms from the supplied mathed/Android/Nix plan, each marked
> **(N)** and justified against what already exists.
> **Relationship**: extends, does not replace,
> `PROJECT_REVIEW_AND_IMPROVEMENT_PLAN.md` (2026-10-04, agent
> organization), `PLAN_ANALYTIC_DECISION_ENGINE.md` (decide_core),
> `unfer/PLAN_MASS_GAP.md` (mass-gap pipeline, incl. today's sector
> correction), and `timepiece/CURRENT_WORK_ORDER.md` (Lean routing).

---

## 0. Executive summary

The supplied plan (mathed = Typst-on-Android UI via `typst_imaging` +
native Skia; emthin = Wayland compositor hosting apps inside document
figure boxes; embedded Nix userland; R2+SLSA+Ed25519 shipping) is
**~60% already built** in this workspace, which changes the work from
"create features" to "extend what exists":

- `velysterm` already has the mathed document model
  (`mathed_core`: Loro CRDT, `markers.rs` PropKind, `glyphs.rs`
  `byte_for_point`, `accessibility.rs`), a Bevy-free CPU frontend
  (`mathed_mini`: `MiniWorld` standalone `typst::World`, `DocLayout`
  raster cache), and a Bevy editor (`mathed`).
- `emthin` already **is** the plan's emthin layer: a nested Wayland
  compositor whose shell is a mathed document, `\app(#1,#2,640,400)`
  statements reserve figure slots, `docui/` is the shell, and a
  Smithay fork (`ningxilai/smithay` branch `emskin-patches`, locked
  0.7.0 @ `2eb24b3`, three local patches per its `AGENTS.md`) is
  already maintained.
- `velysterm/Cargo.toml:107-110` already declares
  `alacritty_terminal 0.25.1`, `portable-pty 0.9.0`, `arboard 3.6.1`
  — with **zero consumers** (grep across all crates): the plan's
  terminal host is half-scaffolded.
- `unfer_data` already owns the S24 golden release manifest and
  `unfer_taler`/`zenodo_store_module` the exchange/deposition
  surfaces the plan's signing/shipping pipeline would extend.

What is genuinely new: the Skia renderer (a *justified* patch/fork of
`typst_imaging` — the house rule bars convenience forks, not justified
ones, and emthin's Smithay fork is the in-project precedent), the
Android NDK host, Nix embedding (`unfer_nixvm` is 0 LOC of glue today),
and the R2/SLSA pipeline.

The three most consequential findings of today's review:

1. **The mass-gap campaign measures the wrong object** (sector
   correction, 2026-10-06): `certified_mass_gap`
   (`unfer/fock_sirk/src/certificate.rs:176`) returns
   `co.value - ce.value` — a difference of two *one-particle*
   reflection-sector grounds — while the physical ground is the outer
   vacuum at `E₀ = 0` (`AGENTS.md` S45; `mass_gap_spec.rs:135`).
   Compounding it, the documented positivity shift is absent, so
   measured θ₀ is negative at every coupling. Written up as
   `unfer/PLAN_MASS_GAP.md` §7 (work order §M4.9, which **gates M4.3**
   — do not populate the gap table from the current 20 fixtures).
2. **The timepiece plan header rotted again (F1 redux)**:
   `CONSOLIDATED_PLAN.md`'s header names 2026-10-05b as the newest
   entry while a 2026-10-06 entry exists below it (line 16643), and
   `CURRENT_WORK_ORDER.md`'s anchor shorthand stops at §10-04b. The
   2026-10-06 entry also predates the sector correction.
3. **CI honesty gaps**: `unfer`'s fuzz workflow is untracked; three
   Austral modules have no CI job; `smoke_gate`'s two most valuable
   smokes degrade to `[gap]` in CI (no velysterm/australVM siblings in
   the `verify-invariants` job); `emthin` has **no CI of its own**
   (only `cargo check -p emthin` from unfer's cross-repo job) despite
   ~18.4k LOC and a maintained Smithay fork; `timepiece`'s
   `axiom_gate.lean`/`Audits.lean` soundness gates run on demand only.

---

## 1. What exists (verified 2026-10-06) — plan element → verdict

| Supplied-plan element | Existing code | Verdict |
|---|---|---|
| mathed interactive Typst document UI | `velysterm/crates/{mathed,mathed_core,mathed_mini,mathed_biblio}` | **exists** — improve |
| `typst_imaging` → Skia translator (`imaging::Command` sink) | `typst_imaging` is an upstream git dep (`velysterm/Cargo.toml:22`, voxell-tech/velyst v0.0.1); the house rule bars *convenience* forks, not justified ones (`velysterm/AGENTS.md:76-81`); emthin's Smithay fork is the in-project precedent | **patch/fork `typst_imaging` with a Skia backend** (B1) — justified: the command sink is its internal abstraction; decision criterion: whether that type is public API |
| 60/120 FPS Android rendering | nothing GPU; `mathed_mini` rasterizes on CPU | **(N)** new Android host (B2) |
| Android NDK host (NativeActivity, EGL/`ANativeWindow`, `skia-safe`, zero JNI) | nothing; Bevy's x11/wayland features pull `libudev-sys` (`velysterm/Cargo.toml:30-41` documents the hazard) | **(N)** justified: no mobile target exists; supplied plan requires it |
| emthin Wayland layer hosting apps in figure boxes | `emthin/` — nested Wayland compositor, document-as-desktop, `\app` slots, `docui/` shell | **exists** — improve (B7: CI, socket path) |
| "Optional emthin Wayland loader" (Smithay headless, `$TMPDIR/wayland-0`) | emthin *is* the compositor; `--spawn` exists (`emthin/src/cli.rs`) | exists in substance — extend with socket-path config |
| touch → layout coordinate mapping | `mathed_core::glyphs` `byte_for_point`/`band_for_byte` (UTF-8 byte-offset discipline, `velysterm/AGENTS.md:63-67`) | exists substrate — extend for touch (B3) |
| metadata tags in Typst | `markers.rs` PropKind; emthin `\app` statements | exists — extend |
| Nix embedded in Android sandbox (PRoot/nix-portable) | `unfer/unfer_nixvm/` — 0 Rust LOC, self-described "glue, not a fork" | **(N)** upgrade the existing glue crate (B4) |
| built-in terminal (`alacritty_terminal` on PTY) | deps declared, zero consumers (`velysterm/Cargo.toml:107-110`) | half-scaffolded — wire into a real crate (B5) or remove |
| dynamic document layout & multi-app spawning | emthin `\app` + `util.rs spawn_child`; `DocLayout` cache | **exists** |
| compositing & input routing | emthin compositor + `emthin-dbus` (fcitx5) + `emthin-clipboard` | **exists** |
| R2 + GH Actions + SLSA + Ed25519 shipping of a Nix closure | `unfer_data` S24 golden release manifest; `unfer_taler`; `zenodo_store_module` | extend existing release machinery (B6) |
| targetSdkVersion 28 (W^X avoidance) | — | constraint for B2 |

---

## 2. Priority A — correctness and honesty fixes (improve existing features)

Each item is non-Lean unless marked *[Lean order]*.

- **A1. Record the sector correction in timepiece's own voice.**
  *[Lean order — prose entry, not `.lean`]* New dated entry
  (2026-10-06b) in `timepiece/CONSOLIDATED_PLAN.md` recording: the
  M1 campaign measures an intra-sector diagnostic, not the physical
  gap; the positivity precondition (`mass_gap_spec.rs:135`) is
  violated (θ₀ = −1.601/−1.696/−3.009 at g = 0.5/1/2); the
  one-sidedness lemma in `docs/VERIFY_MASS_GAP_NORM_RESIDUAL.md` is
  conditional on H PSD. In the same commit: fix the header
  (F1 redux — it names 2026-10-05b as newest while 2026-10-06 exists
  at line 16643) and extend `CURRENT_WORK_ORDER.md`'s anchor shorthand
  with §10-06/§10-06b. The house convention is content-based citations,
  never line numbers.
  **Recorded 2026-10-08 — done**: the §"State of the project — 2026-10-06b:
  the sector correction" entry exists in `timepiece/CONSOLIDATED_PLAN.md`, the
  header names it the newest entry, and `CURRENT_WORK_ORDER.md` carries
  §10-06/§10-06b in its anchor shorthand (§10-06b wired into the mass-gap
  routing row).
- **A2. Emit §M4.9 rows** (work order exists:
  `unfer/PLAN_MASS_GAP.md` §7). Apply the documented positivity shift
  to the one-particle `h` before enclosure, then measure the physical
  gap via the `‖Hφ₁‖` route — whose error is **quadratic** in the
  directly measurable residual q (`√(E²+q²) − E = q²/(√(E²+q²)+E) ≤
  q²/(2E)`, verified in Cadabra: `unfer/docs/mass_gap_norm_residual.cdb`,
  tests `unfer/prob_kernel/tests/mass_gap_norm_residual.rs`, 5 tests
  green). Reachability: 3 digits need q ≲ 4.1e-3 (~730× below the
  current q ≈ 2.98); via `‖d‖` with `‖H‖ ≥ 6.0e3`, `‖d‖ ≲ 6.8e-7`.
  Keep the 20 pinned fixtures as diagnostics; **M4.9 gates M4.3**
  (`timepiece/CONSOLIDATED_PLAN.md` §M4.9, line 16873).
  **Recorded 2026-10-08 — done**: `fock_sirk/src/certificate.rs` implements
  §M4.9 — `physical_gap` rows with `physical: true` and `q` beside `gap`, the
  one-particle rows tagged `physical: false`, and no row emitted when the
  enclosure is missing (the §M4.9 acceptance, in code).
- **A3. Measure the residual turnover past m = 128** (open measurement,
  non-Lean) — **measured 2026-10-07: there is no turnover.** The probe was
  rebuilt as `fock_sirk/tests/zz_turnover.rs` (`#[ignore]`; release run,
  7 291 s for m = 4…256) and is committed with its table. `q₀` rises
  monotonically 2.73 → 12.9 (rel_err 3.87 → 30.7 against a `≈ 4e-3`
  target), θ₀ stays ≈ −1.65, and rank collapses from m = 128 (rank 50 <
  m/2) onward, so the past-128 rows carry the probe's `DEGENERATE` flag and
  bound the trend without being the window's own residual. Negative result:
  for this configuration (harmonic shifts, window width 2) more `m` buys
  residual, not convergence. Full table + conclusion:
  `unfer/docs/VERIFY_MASS_GAP_NORM_RESIDUAL.md` §"The sweep past m = 128".
- **A4. Commit or delete `decide_core`** (5,240 LOC, 91 tests, 0
  integration tests — entirely untracked; its CI golden step and
  `docs/DECIDE_SPEC.md` are untracked too). Its calibration contract
  fails all six candidates by design (`docs/DECIDE_SPEC.md:36-47`), so
  the honesty boundary is already written; what is missing is the
  commit.
  **Recorded 2026-10-08 — done**: `decide_core/` (sources + proptest
  regressions), `docs/DECIDE_SPEC.md`, and the CI golden step ("decide
  calibration report is fresh") are all tracked; `git status` is clean for
  both paths.
- **A5. Track `.github/workflows/fuzz.yml`** (untracked; `fuzz/Cargo.toml`
  honestly says "property test in CI; fuzz target unrun" — the file's
  absence makes reality weaker than the claim).
  **Recorded 2026-10-08 — done**: the workflow is tracked.
- **A6. Close the CI coverage gaps** (copy velysterm's `rust.yml`,
  which already has `cargo doc` and a `--no-default-features` check):
  add jobs for `client_module`, `gatekeeper_module`,
  `durable_status_module`; clone velysterm + australVM as siblings in
  `verify-invariants` so `smoke_gate`'s two smokes stop printing
  `[gap]`; add `--locked` (zero occurrences in unfer's `ci.yml`
  today); lint non-default features (only default features are clippy'd
  today).
  **Recorded 2026-10-08 — done**: all four legs are in unfer's `ci.yml` —
  `client_module + gatekeeper_module` and `durable_status_module` e2e jobs,
  the velysterm + australVM sibling checkouts in `verify-invariants` (the
  comment carries the A6 tag), `--locked` throughout, and the clippy job
  over the non-default feature set plus `cargo check
  --no-default-features` and `cargo doc` with `-D warnings`.
- **A7. Fix stale hand-written symbol counts** — `docs/LODY_PATTERNS_PLAN.md:274`
  ("84 uk_"), `docs/IMPROVEMENT_PLAN.md:172` ("65"),
  `PLAN_parallel_unfer.md:55` ("21"); actual registry is 91 `uk_*` +
  5 `uz_*` (`unfer_protocol/src/symbols.rs:43`). Extend the count-ban
  gate (`scripts/verify-invariants:149`) beyond its current 4 files.
  **Recorded 2026-10-08 — done**: the three counts are gone (grep clean),
  and H1.12's count-ban now covers eight live docs (README, AGENTS,
  `docs/ARCHITECTURE.md`, `unfer_ffi/README.md`, `docs/LODY_PATTERNS_PLAN.md`,
  `docs/IMPROVEMENT_PLAN.md`, `PLAN_parallel_unfer.md`, `PLAN_HARNESS.md`).
- **A8. Fix `README.md` test-count drift** — 1,640 (line 102) vs 676
  (line 194) in one file; `scripts/check-status:17-23` does not
  recompute counts, so CI cannot catch it. Make it recompute, or cite
  the exact command.
  **Recorded 2026-10-08 — done**: `scripts/check-status --recount REPO=LOG`
  recomputes counts from suite logs and refuses a log containing a failed
  suite; the README verification block states one fresh pair (1774 / 401)
  and dates 676 as a historical rev-23 claim.
- **A9. `unfer_edge` authentication** — no auth anywhere; `/api/cap/*`
  attributes every request to the admin principal
  (`unfer_edge/src/main.rs:223`, with the in-code admission "the caller
  comes from the authenticated session" in a full deployment); default
  bind `0.0.0.0:3000` (`config.rs:37,98`). Minimum: document +
  default loopback. Proper: session-authenticated caller identity.
  **Recorded 2026-10-08 — minimum done**: `listen` and `backend` default to
  loopback (`127.0.0.1:3000`), tests assert both the default and the
  env-override path, and the config comment states that widening the bind is
  "session auth behind a proxy, not a bigger bind". Full session-authenticated
  caller identity remains open, as written.
- **A10. Live-JIT module-grant enforcement** — the six module e2e jobs
  prove `modhost authorize` denies, not that a live JIT call is denied
  (`unfer/docs/MODULES.md:95-104`; `australVM/AGENTS.md` gotcha: JIT
  principal is the *calling function*, manifests grant by *module*).
  Thread the module name into `australVM/safestos/cranelift/src/cps.rs
  check_call_permission()`.
  **Recorded 2026-10-08 — done**: `check_call_permission(caller, module_name,
  callee)` threads the module at both live call sites, preferring the
  manifest's module principal over the CPS header (falling back to the caller
  name for legacy binaries), with unit tests at the bottom of `cps.rs`.
- **A11. `kernel_client` durability** — add the `unfer_ffi` dependency
  so `BoardLog` is durable on the `unfer_agent` NDJSON host (the C2
  gap; the durable `BoardLog`/`BOARD_STREAM` machinery exists in
  `unfer_protocol/src/board.rs` but the agent host never writes it).
  **Recorded 2026-10-08 — done**: `velysterm/crates/kernel_client` depends on
  `unfer_ffi` (manifest comment carries the G1/A11 tag).
- **A12. Fix the two flaky `unfer_ffi` tests**
  (`action_apply_resolves_and_merges_applied_result`,
  `action_submit_queues_pending_record_with_provisional_result`) —
  fail ~1/10 under parallel execution, pass in isolation
  (`unfer/README.md:124-132`). The documented prerequisite is an
  inventory of which test touches which store (shared action store /
  owner log / audit ring behind four mutexes); lock reordering was
  tried and made it worse.
  **Recorded 2026-10-08 — closed** (three tests, not two —
  `queue_drops_oldest_when_full` joined them): the lock inventory is finished
  in `unfer/docs/TEST_LOCKS.md` (`reset_durable_for_tests` wiped all four
  shared stores; every multi-lock test now takes one order P → ACTION →
  AUDIT → DURABLE), and the fixed binary ran 100/100 clean at 16 test
  threads.
- **A13. Run `axiom_gate.lean` + `Audits.lean` in CI** — the soundness
  gate (transitive axiom closure outside {propext, Classical.choice,
  Quot.sound}) is on-demand only; the lakefile comment says "Never a
  default target". Use timepiece's own path-filter + weekly + dispatch
  pattern from `verify-invariants` (`.github/workflows/ci.yml:33-68`).
  **Recorded 2026-10-08 — done**: `timepiece/.github/workflows/axiom-gate.yml`
  runs both, with the same `**/*.lean` path filter + weekly cron +
  `workflow_dispatch` shape — the triggers live in their own file because
  `on:` with `paths` is workflow-level, and adding it to the shared CI file
  would gate every job in it. The gate exits 0 even when it prints `BAD`, so
  the workflow requires the `GATE: checked` tally and fails on any `BAD`
  line; each of the five axiom audits is re-run with output captured and every
  `depends on axioms:` list stripped of the allowed three (`sorryAx`
  included), plus a format-drift guard so a wording change in `#print axioms`
  fails loudly instead of letting the whitelist match nothing.
  `Audits/Trivial.lean` is deliberately excluded (a Verso section-count
  reproducer, not an axiom audit). **Verified locally on the pinned 4.28.0
  toolchain**: `lake exe cache get` → the 7-chapter cone built (8 032 jobs) →
  `lake env lean axiom_gate.lean` checked **196 theorems in 53 s, 0 BAD,
  exit 0**; then the Audits cone built (8 071 jobs) and all five scripts ran
  clean (`Verification`: 11 axiom lines, all within the allowed three; the
  other four audit by `#check`/`#eval`, elaboration = the gate).
- **A14. Give `emthin` a CI of its own** — no `.github/workflows`
  exists; only `cargo check -p emthin` runs, from unfer's cross-repo
  job. `emthin/AGENTS.md` documents the tests (`xwayland_satellite`,
  `docui`, `figure_render`, `keymap`, `session`, figure binding) —
  add a workflow running `cargo test -p emthin` plus the clipboard/dbus
  crate tests.
  **Recorded 2026-10-08 — done**: `emthin/.github/workflows/ci.yml` runs
  rustfmt + `cargo test --locked --workspace` — "all three workspace crates,
  not just the compositor", per the job's own comment quoting this
  acceptance — with the sibling checkouts its path deps need.
- **A15. australVM bridge freshness** — the OCaml side links
  `safestos/cranelift/target/release/libaustral_cranelift_bridge.{a,so}`;
  a stale `.so` silently misses `uk_*` symbols (`australVM/AGENTS.md`).
  Add a `make` prerequisite that checks the `.so` is newer than the
  bridge sources.
  **Recorded 2026-10-08 — done**: `australVM/Makefile` fails with "error:
  stale bridge" when any bridge source is `find -newer` the bridge, with the
  staleness rationale (a stale `.so` silently misses `uk_*` symbols) in the
  comment above it.
- **A16. H1.7 vacuum-initialization invariant** — either fix
  `nested_fock_algebra/src/lib.rs:95` (`bosonic: BTreeMap::new()`, 0
  inner universes, against AGENTS.md S45's ≥1 empty inner universe) or
  stop claiming the invariant; `scripts/verify-invariants:103-108`
  honestly reports `[gap]` forever, which is the worst of the three
  options.
  **Recorded 2026-10-08 — done**: no permanent `[gap]` — H1.7 is three
  positive checks now (outer `vacuum()` is `bosonic: BTreeMap::new()`,
  inner-operator sources create one empty inner universe first via
  `OuterBosonCreate(InnerBosonicState::vacuum())`, and the false ">= 1 empty
  inner universe" claim stays retired from AGENTS/PLAN_HARNESS).

---

## 3. Priority B — adapting the supplied plan (new features, each justified)

Ordered so each step has a consumer for the previous one. All are
**(N)** new mechanisms; the justification column is the "good
justification" the brief requires.

- **B1. Skia backend for `typst_imaging`** — render the
  `imaging::Command` stream to `skia-safe` GPU surfaces.
  The house rule bars *convenience* forks, not justified ones
  (`velysterm/AGENTS.md:76-81`), and the project already
  maintains a justified fork: emthin's Smithay
  (`ningxilai/smithay`, branch `emskin-patches`, three
  documented patches per `emthin/AGENTS.md`). *Justification
  for patching typst_imaging here*: the command sink is
  typst_imaging's own internal abstraction — a Skia backend is
  a peer of its existing CPU raster backend, and keeping the
  command enum and its backends in one crate avoids mirroring
  an internal, unstable type across a crate boundary.
  *Decision criterion*: if the command type is already public
  API, an in-workspace `mathed_skia` crate is the lighter
  option (no fork to maintain); if it is internal, the fork
  is the only route short of an upstream API change. Verify
  which it is first (source check of the v0.0.1 tag), then
  pick. Either way the renderer is Bevy-free (like
  `mathed_core`/`mathed_mini`) so `emthin` and the Android
  host can depend on it without the `libudev-sys` chain
  (`velysterm/Cargo.toml:30-41`), and it fixes the real
  problem: `mathed_mini`'s softbuffer CPU raster cannot reach
  60/120 FPS on a GPU surface.
  **Recorded 2026-10-07 (source check of the v0.0.1 tag) — criterion
  resolved: the sink abstraction IS public API, so the lighter route
  wins.** `typst_imaging` has no `Command` enum of its own:
  `pub fn render_frame(frame: &Frame, sink: &mut impl PaintSink)`
  takes the trait from the external crates.io crate `imaging` 0.0.1
  (forest-rs/imaging, `Apache-2.0 OR MIT`), where `pub trait
  PaintSink` (paint) and `pub enum Command` (record: PushContext /
  PopContext / PushClip / PopClip / PushGroup / PopGroup / Draw)
  are both public, and imaging's own docs show external
  implementors. Upstream velyst already consumes it exactly that
  way: its renderer calls `typst_imaging::render_frame` with
  `imaging_vello::VelloSceneSink` (`imaging_vello` 0.0.2, `impl
  PaintSink for VelloSceneSink`), and velysterm's renderer does the
  same. **Decision: NO typst_imaging fork; write an in-workspace
  `mathed_skia` crate implementing `imaging::PaintSink`, a peer of
  `VelloSceneSink`.** `mathed_mini` today uses `imaging` +
  `imaging_vello_cpu` (CPU) plus optional `softbuffer` under `gui`;
  no skia appears in `velysterm/Cargo.lock` (only tiny-skia), so
  `skia-safe` would be the one genuinely new heavy dependency —
  that is the cost of the GPU surface B1 exists to deliver.
  **DONE 2026-10-08 — `mathed_skia` implemented, tested, gated.**
  `velysterm/crates/mathed_skia/` (workspace glob member) implements
  `imaging::PaintSink` on `skia-safe` 0.90 over any caller-supplied
  `skia_safe::Surface` — raster or GPU; the raster path is exercised
  by tests, the GPU path is compile-verified only (no GPU context
  exists in this environment, and nothing claims GPU pixels).
  18 integration tests drive the public `imaging::Painter` API and
  assert pixel readback plus every externally observable
  `Diagnostics` counter (fault latch, clip underflow, filters /
  split-cap / non-sRGB-gradient / luminance counters);
  `cargo test -p mathed_skia` is green under the crate's documented
  `nix-shell -p freetype fontconfig pkg-config` link environment,
  and the CI gates (`cargo clippy --workspace --all-features
  --all-targets -- -D warnings`, `cargo fmt --check`) are green.
  Fidelity scope and counted approximations live in the crate's
  module docs.
- **B2. Android host crate** — `android-activity` `NativeActivity`,
  EGL/Skia GPU surface on `ANativeWindow`, zero JNI; cross-compile
  with the NDK; **targetSdkVersion 28** to avoid the W^X restrictions
  that block PRoot-style embedding (per the supplied plan).
  *Justification*: the project has no mobile target at all, and the
  supplied plan's raison d'être is running mathed + emthin + the Nix
  userland on  unrooted Android. CI: an NDK cross-compile job (build
  only; no emulator in CI).
  **Recorded 2026-10-07 — the CI leg is UNAVAILABLE in this
  environment**: no `ANDROID_NDK_HOME`/`ANDROID_HOME`, no
  `cargo-ndk`, `~/Android` absent, so no cross-compile job can be
  built or run here. Must be reported as UNAVAILABLE, never passed.
  The crate itself remains an open work item.
- **B3. Touch interaction layer** — extend `mathed_core::glyphs`
  (`byte_for_point`, `caret_for_byte`) with touch→layout coordinate
  mapping and Typst metadata tags. *Justification*: the substrate
  (UTF-8 byte-offset geometry, `DocLayout` cache) already exists; the
  plan's "mathed interaction layer" is an extension of it, not a new
  one. Reuse emthin's `\app` statement grammar for app slots.
  **Recorded 2026-10-07 — verified a genuine extension**:
  `mathed_core`'s glyph module (704 lines) exposes exactly
  `build_glyph_index`, `caret_for_byte`, `band_for_byte`,
  `byte_for_point`, `rects_for_range` — no touch API anywhere, and
  the crate has no `tests/` directory. Touch→layout mapping and
  Typst metadata tags are new surface on an existing substrate, as
  the justification claims.
- **B4. `unfer_nixvm` → real embedding crate** — custom-prefix Nix
  closure, PRoot/nix-portable, runnable unrooted. *Justification*: the
  crate exists as 0 LOC of glue with build notes transcribed from a doc
  rather than a run (`unfer/ci.yml:249-253` records "this step has
  never been observed green") — upgrading it is improving an existing
  feature, and B6 ships its output.
  **Recorded 2026-10-07 — verified**: `unfer/unfer_nixvm/` holds
  only `flake.nix`, `flake.lock`, `README.md`, and
  `run_sandboxed_sidecar.sh`, and the CI honesty comment ("this
  step has never been observed green") is present in
  `unfer/.github/workflows/ci.yml`. Justification holds unchanged.
- **B5. Terminal host crate in velysterm** — in-process PTY terminal on
  the already-declared `alacritty_terminal` + `portable-pty` deps.
  *Justification*: the dependencies are already in the workspace with
  zero consumers (`velysterm/Cargo.toml:107-110`); either wire them
  into the plan's Phase-7 terminal or delete them — dependency hygiene
  demands one of the two. The terminal is a prerequisite for the
  Android "Termux/Linux desktop apps inside figure boxes" story.
  **Recorded 2026-10-07 (nuance)**: "zero consumers" is true only
  within `crates/`. The three workspace deps (`alacritty_terminal`
  0.25.1, `portable-pty` 0.9.0, `arboard` 3.6.1) ARE consumed by
  `examples/velyst_demo/examples/terminal.rs` (253 lines: PTY →
  alacritty grid → Typst), whose manifest pulls all three
  `workspace = true`; `examples/*` are workspace members. So the
  hygiene fork in the road is real, but there is a working
  reference implementation to lift — B5 is promote example → crate,
  not first wiring.
- **B6. Release/shipping pipeline** — extend `unfer_data`'s S24 golden
  release manifest with Ed25519 signing, SLSA provenance, and a
  Cloudflare R2 binary cache for shipping the B4 closure.
  *Justification*: the golden-manifest gate and the
  `zenodo_store_module`/`unfer_taler` surfaces already exist; the
  plan's pipeline is an extension of the existing release machinery,
  not a new one. GitHub Actions job emits provenance; the R2 cache is
  the distribution leg for unrooted devices.
  **Recorded 2026-10-07 — justified as written**: the S24 golden
  release manifest and the `zenodo_store_module`/`unfer_taler`
  surfaces are existing release machinery, so signing / SLSA
  provenance / R2 extend it rather than introduce a new mechanism.
  (Plan record only; no pipeline code written.)
- **B7. emthin improvements** — (a) configurable Wayland socket path
  (`$TMPDIR/wayland-0` per the plan's headless-loader mode; today the
  socket path is fixed) so the Android host can run the compositor
  headless over a UNIX socket; (b) keep the Smithay fork discipline
  (three local patches documented in `emthin/AGENTS.md`) intact — any
  Skia/Android work must not destabilize `emskin-patches`.
  **Recorded 2026-10-07**: (a) the claim "today the socket path is
  fixed" is **stale — already done**: emthin has `--wayland-socket
  <NAME>` (a `pub wayland_socket: Option<String>` CLI flag), `main`
  copies it into env `EMTHIN_WAYLAND_SOCKET_NAME`, and
  `init_wayland_listener` honors it via
  `ListeningSocketSource::with_name`, falling back to `new_auto()`;
  the e2e harness already passes the flag to pin the display.
  **No work item remains for (a).** (b) Smithay fork discipline =
  **verify-only**: AGENTS.md documents the `emskin-patches` branch
  and the three patched files; B1's Skia work lands in velysterm
  (and later the Android host), not in the Smithay fork, so the
  constraint is satisfied by construction — re-verify at B1
  implementation time.

---

## 4. Priority C — owner decisions (recorded, not made)

- **C1. `test/` LICENSE** — `test/AGENTS.md` rule 2: no LICENSE yet,
  owner decision, do not add speculatively. **Recorded 2026-10-07**:
  no speculative LICENSE was added; the open question is tracked as
  owner decision G-S1 in `PROJECT_REVIEW_AND_IMPROVEMENT_PLAN.md`
  (X9 row), which records that no default was chosen.
- **C2. RandomMap/RH track authority (timepiece flag F2)** — three
  documents claim authority (`IMPLEMENTATION_PLAN.md`, `RandomMap.md`,
  `RandomMap2LegacyDraft.md`) and none is reconciled; `CURRENT_WORK_ORDER.md`
  row 5 records "no current work order exists". A decision, not code.
- **C3. `qfm` pipeline refactor** — `docs/IMPROVEMENT_PLAN.md:193-196`,
  the only open item of 30, deferred per maintainer directive
  (17-arg `dense_forward_sirk`, 5-tuple returns, retained stubs at
  `qfm/src/pipeline.rs:547`). **Recorded 2026-10-07**: the deferral
  is already captured in the doc itself — Tier 5 item 26 reads
  "Deferred: deliberately left untouched per maintainer directive (the
  qfm surface is a stability/sensitivity area, not a correctness
  defect)" — so no separate decision record is needed; the plan2
  record is this line.
- **C4. RH route sequencing** — the unanswered question in
  `timepiece/AGENTS.md`'s (stale) Priority Attack Order: are the RH
  analytic route's remaining `sorry`s (`uniform_variance_bound`,
  `jensen_bohr`, `convergent_series_has_no_poles` — note the
  Formalization State table itself is stale: three RandomMap2 rows
  marked `*Sorry*` are proved, flag F2) next in queue, or parked
  behind the QYM/QG/NS/SM/book rows of `CURRENT_WORK_ORDER.md`?

---

## 5. Verification state (2026-10-06)

**Green today** (RUSTUP_TOOLCHAIN=1.97.1, `--offline`):
`fock_sirk` mass_gap_campaign 14 passed / 4 ignored; mass_gap_properties
12; qcd_mass_gap_certified 3; qym_mass_gap 10; `prob_kernel`
mass_gap_norm_residual 5 (with `cadabra2-cli` on PATH; skips cleanly
without it); `cargo fmt --all -- --check` and
`cargo clippy --workspace --all-targets -- -D warnings` clean; M3
(`check-mass-gap-spec-parity`) and M9 (`check-non-claims`) gates pass.

**UNAVAILABLE in this environment** (must be reported as UNAVAILABLE,
never passed): Aeneas/Charon (`/home/leo/Projects/.toolchain/aeneas-bin/aeneas`
absent), `lean4export` 3.1.0, `kani`, `elan`/`lake` (Lean build
untested here), `cadabra2-cli` absent from PATH (available only via
`nix-shell -p cadabra2`; the repo seam `prob_kernel::symbolic` omits
the `-l` flag and applies substitutions without `repeat=True`, so the
pure-Cadabra `.cdb` corpus likely cannot run through that seam — a
separate defect worth its own look).

**Cadabra engine limitations recorded** (`unfer/docs/VERIFY_MASS_GAP_NORM_RESIDUAL.md`):
`expand()` does not open a polynomial power of a sum; `canonicalise()`
does not distribute products of sums; `distribute()` actively corrupts
the binomial (`2(2+a+b)² → 4(1)²`).

**Measured (A3, 2026-10-07)**: residual turnover past m = 128 — **there is
none.** `q₀` climbs monotonically 2.73 → 12.9 over m = 4…256 (rel_err
3.87 → 30.7), first rank collapse at m = 128 (rank 50 < m/2; rows flagged
`DEGENERATE` from there on), θ₀ flat at ≈ −1.65. Release run 7 291 s;
table and conclusion in `unfer/docs/VERIFY_MASS_GAP_NORM_RESIDUAL.md`, probe
committed as `fock_sirk/tests/zz_turnover.rs`.

**Cross-repo seam hazard** (standing): a green run in one repo says
nothing about its path-dependents — `australVM/arctic_authority` →
`dynamic-arctic`; `velysterm/kernel_client` → `unfer`; `emthin` →
`velysterm/{mathed_core,mathed_mini}`; `unfer_consensus` →
`dynamic-arctic`. CI clones siblings for exactly this reason; A6/A14
extend that discipline to the jobs that lack it.

**Final verification (2026-10-07, after the A/B/C records above)**:
every gate green in both shapes — `scripts/check-attribution`
(workspace: 236 hits, 0 unresolved; `--repo-only` via the unfer mirror:
67 hits, 0 unresolved), `check-status` with `--recount` against both
suites' fresh logs, `check-book-sync`, `check-non-claims` (M9),
`check-mass-gap-spec-parity` (22 claims, M3), `verify-invariants` (41
passed, 0 gaps with python3 on PATH; 38 + 3 explained gaps without),
`check_protocol_docsync` (61 ops + 114 UK codes), `check_site` (62
pages, 286 links), `tree-size`; `cargo fmt --check` and `cargo clippy
--all-targets -- -D warnings` clean in unfer and velysterm
(RUSTUP_TOOLCHAIN 1.97.1). Suites, measured today from the suites' own
logs: unfer **1774** (debug == release suite by suite, 68 suites, both
EXIT=0) — the gate caught the stale 1769 block first, and the +5 is X6's
new `ops_e2e` binary plus G3's four escalation tests; velysterm **850**
(19 suites, EXIT=0) — its stale 797 header never matched its own 711
breakdown, so the block and the per-crate line were restated from one
log. One environment event, recorded because it invalidated a first
attempt: `/` hit 100% mid-run (a test failed writing a temp export with
ENOSPC, the linker bus-errored); ~118 G of recompilable build caches
(emthin/australVM `target`, australVM `_build`, cargo `incremental`
dirs, /tmp scratch) were reclaimed at the owner's request and every run
above was re-done clean.B2's NDK CI leg stays UNAVAILABLE here (no `ANDROID_NDK_HOME`, no
`cargo-ndk`, `~/Android` absent).

**A13 verification (2026-10-08, after the A-item records above)**: the
soundness gate ran end-to-end on the pinned local 4.28.0 toolchain —
`lake exe cache get`, the 7-chapter import cone (8 032 jobs), then
`lake env lean axiom_gate.lean` → `GATE: checked 196 theorems in 53 s`,
**0 BAD, exit 0**; the Audits leg built its cone (8 071 jobs) and all five
audit scripts ran clean (Verification's 11 `#print axioms` lines all within
{propext, Classical.choice, Quot.sound}; the other four audit by
`#check`/`#eval`, so elaboration is their gate). The new workflow file is
YAML-validated (8 steps, workflow-level paths + `43 5 * * 1` cron +
`workflow_dispatch`); it cannot be *executed* on GitHub from here — first
real run happens on push/schedule/dispatch.


---

## 6. Routing

- **Lean-side prose** (A1, the §M4.9 consumption side, C2/C4): new
  dated entry in `timepiece/CONSOLIDATED_PLAN.md`, append-only, with
  content-based citations — work orders for the LLM-Lean4-specialist.
  No `.lean` files are written by this plan.
- **Non-Lean Rust** (A2–A16, B1–B7): normal PRs in the owning repo,
  each citing its existing-feature improvement; gates per repo
  (`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
  plus the repo's own invariant scripts).
- **Docs** (A7, A8, C1–C4): same-repo doc edits; the count-ban and
  parity gates (`scripts/verify-invariants`, `check-mass-gap-spec-parity`,
  `check-non-claims`) must stay green.
