# Seven-Repo Review & Improvement Plan — with ClawSwarm / Agensh adaptations

> **Scope**: `unfer/`, `australVM/`, `velysterm/`, `timepiece/`, `dynamic-arctic/`,
> `test/`, `emthin/` — reviewed as **one project**.
> **Reference studies**: `ClawSwarm/` and `~/Downloads/agensh/` (ideas adapted,
> nothing copied — see §9).
> **Date**: 2026-10-04.
> **Decision log**: this plan predates the D-row convention; its decisions
> live inline in the item sections. New decisions use `## Decision log`
> (numbered `DN (date):` rows) per `PLAN_WORKSPACE_REVIEW_2026-10-14.md` §9.
> **Hard constraint**: no direct Lean 4 code in this plan. All Lean work is
> written as *work orders for the LLM-Lean4-specialist* (documentation,
> orchestration, and tooling around Lean are in scope; `.lean` sources are not).
> **Improvement principle** (continuing `unfer/PLAN_HARNESS.md`): improving an
> existing feature beats creating a new one. Every item below names the
> existing feature it improves; genuinely new mechanisms are marked **(N)**
> and justified.

Relationship to existing plans: this document extends, and does not replace,
`unfer/PROJECT_PLAN.md` (five-repo plan, all phases done), `unfer/PLAN_HARNESS.md`
(H1–H12, borrowed from deepseek-harness/qm), the per-repo `PLAN_parallel_*.md`
workstreams (A/B/C), and `timepiece/CONSOLIDATED_PLAN.md` (the Lean work order).
Where an item below overlaps an existing stage, the existing stage ID is cited
and the change is an *extension* of it.

> **2026-10-06 update**: `PLAN_MATHED_NATIVE_AND_IMPROVEMENT.md` (workspace
> root) is the successor review for the mathed-on-Android adaptation and the
> 2026-10-06 deep review — it records the mass-gap sector correction, the
> CI-honesty gaps, and the plan-element→existing-code mapping. Read that file
> for anything dated after 2026-10-04; this document remains the record for
> the agent-organization (ClawSwarm/Agensh) track.

---

## 0. Executive summary

The seven folders together are one system: a **modular probability kernel**
(`unfer`) with QFT/numerics engines and a mature capability/FFI surface; a
**verified compiler/VM** (`australVM`) that hosts kernel modules under policy;
a **human UI + AI-agent interface** (`velysterm`); a **Lean 4 verification
corpus and book** (`timepiece`); a **threshold-signing authority**
(`dynamic-arctic`); a **public documentation site** (`test/`); and a
**document-as-desktop compositor** (`emthin`). The engineering culture is
strong: pinned single toolchain (Rust 1.97.1), per-repo CI, golden gates,
frozen cross-repo contracts, and unusually honest status documentation.

The three findings that matter most:

1. **The project has an AI-agent interface but no agent *organization*.**
   `unfer_agent` (36 NDJSON ops), the S21–S28 trust machinery, and the
   LLM-Lean4-specialist work orders are all single-agent seams. Both reference
   projects are exactly about multi-worker cooperation (ClawSwarm: director →
   specialist workers → summarizer over a unified gateway; Agensh: 1,024
   self-organizing workers over shared workspace + message interface + shared
   context). The highest-leverage work in this plan is to extend the *existing*
   agent seams (§6–§7) so multiple specialist agents can work this corpus
   concurrently **under the project's own grant/audit model** — not to build a
   new harness from scratch.
2. **Knowledge and invariants are stored as prose.** `AGENTS.md` maintenance
   checklists, 61 documents with zero recorded backlinks in `timepiece`,
   byte-duplicated sources (`book.tex`, `ODE.tex` identical in `unfer/` and
   `timepiece/`), and stale READMEs (`australVM`, `dynamic-arctic` claim
   "PROBABLY NOTHING WORKS" against green test suites). PLAN_HARNESS H1/H2
   already target this; this plan adds the shared-context and doc-graph
   mechanisms from the reference projects.
3. **Concurrency hygiene is unproven everywhere agents will touch.** Multi-agent
   work only helps if merges are safe: the existing Loro CRDT sync (velysterm
   C13), the golden release gate (unfer S23/S24), and `verify-invariants`
   (H1) are the right substrates; Agensh's claim/verify/merge loop and the
   STORM-style write-time conflict check are the right discipline to layer on.

---

## 1. What the whole project is (system map)

```
                    test/  (public book+docs site, GitBook + check_site.py CI)
                      ▲ documents
 timepiece/  ─────────┴────────────  unfer/  (probability kernel + QFT + consensus)
 (Lean 4 proofs,        ▲ proves / contracts        │ uk_* C ABI (65+ symbols)
  book.tex source)      │ (unfer_contracts,          ▼
                        │  nanoda re-verification)  australVM/  (Austral compiler +
                        │                            Cranelift JIT + Cedar policy +
                        │                            Why3 gate, module host)
                        │                                ▲ hosts modules; path-dep
 velysterm/  (mathed editor family + kernel_client +     │ on unfer_ffi
              unfer_agent NDJSON AI interface)  ─────────┘
      ▲ document model (Loro CRDT, \app statements)
 emthin/  (nested Wayland compositor: the document is the desktop; GPL-3.0)
 dynamic-arctic/  (Arctic/Shine threshold Schnorr; reused by unfer_consensus)
```

- **Frozen contracts** (additive-only): `uk_*`/`uz_*` symbols + C signatures,
  `prob_kernel::Session` API, `unfer_protocol` serde types + UK-#### codes,
  NDJSON agent ops, `module.toml` grant vocabulary, S29 symbol registry.
- **Shared policy**: single Rust toolchain 1.97.1 across unfer/australVM/
  velysterm/dynamic-arctic; nix flakes per repo; OCaml 4.14 in australVM.
- **Out of scope but adjacent** (at the workspace root, touching this project):
  `driftwm/` (GPL-3.0 code already adapted into `emthin/`), `ProofFlow/`
  (named in `emthin/docs/REWRITE_PLAN.md` Part 2), `Engram/`,
  `engram-reference/`, `prove2me_workspace/` (feeds
  `timepiece/PROVE2ME_REUSABLE_THEOREMS.md`), `open-jev/`. This plan only
  touches those where a listed item requires it.

---

## 2. Deep review — per folder

### 2.1 `unfer/` — the kernel (hub)

**State**: ~15 Rust crates in one workspace (`prob_kernel`, `unfer_ffi`,
`unfer_protocol`, `fock_sirk`, `nested_fock_algebra`, `logos`, `ode_sirk`,
`unfer_consensus`, `unfer_identity`, `unfer_data`, `unfer_taler`, `unfer_edge`,
`qfm`, `qfm_text`, `sirk_core_model`) + 8 Austral modules. S21–S35 security and
validation stages are built (effect kinds, admin refuse list, secret
sanitizer, budgets/rate limits, sensitive latch, credential vault, capability
RPC, Lean4 export verification via nanoda, Cadabra2 subprocess coupling,
Logos CNL→UNF, Austral→DeltaNet→UNF→TED). CI, golden release gate, fuzz and
property tests, heavy-physics test scripts all exist.

**Strengths**: the trust model is the project's crown jewel — metered symbols
deny at a single chokepoint, secrets never serialize into snapshots/blueprints,
audit is sanitized and capped. Cross-repo registration discipline (S29) exists
even though it is manual. AGENTS.md is a genuine maintenance backlog.

**Gaps found**:
- **G-U1 Doc duplication**: `book.tex` and `ODE.tex` are byte-identical copies
  in `unfer/` and `timepiece/` (verified by checksum, 2026-10-04). One must be
  the source of record; the other a generated/synced copy with a drift gate.
- **G-U2 AGENTS.md overload**: single checklist items are multi-hundred-word
  essays (S30…S30m). Knowledge that belongs in `docs/` (e.g. the Cadabra2
  `@()`/`unwrap` gotchas) is embedded in checklist prose; new agents can't
  discover it, and PLAN_HARNESS H1 can only gate what is machine-checkable.
- **G-U3 Symbol registration drift risk**: five hand-maintained mirrors
  (PLAN_HARNESS H2) remain hand-maintained.
- **G-U4 Session growth**: `SessionBlob` still unforkable/uncompacted until H3
  lands; editor sessions and agent transcripts have no retention policy.
- **G-U5 New crates under-documented**: `logos`, `ode_sirk`, `unfer_consensus`,
  `unfer_data`, `unfer_identity` have tests but no dedicated docs (noted in
  PLAN A "Current state"; still true where `docs/` has no per-crate page).
- **G-U6 `qfm_text_runs/`** shrank to 184K but its retention policy is
  undocumented; `logs/` likewise.

### 2.2 `australVM/` — the verified compiler/VM

**State**: OCaml Austral frontend (CPS IR) + Rust Cranelift/Cedar bridge +
C runtime lineage, plugin seam (`Vm_plugin`/`Compiler_plugin`), Why3 gate and
DeltaNet UNF passes, Tidepool/Egison/cap-std module archetypes, Mirage
unikernel scaffold. PLAN B complete through B11.

**Strengths**: "the application is defined by its plugins" is executed
consistently; the Why3→OCaml extraction cycle is byte-pinned; Cedar gives
zero-runtime-overhead policy checks at JIT time.

**Gaps found**:
- **G-A1 Stale self-description**: README still says "PROBABLY NOTHING WORKS"
  and describes Phase 11 only; the plugin/unikernel/unified-VM work isn't in
  the README's architecture section. Same class of problem in
  `dynamic-arctic/README_arctic.md`.
- **G-A2 Known test failures are parked**: JitTest `run`-name reuse collisions
  (`DuplicateDefinition`/`IncompatibleSignature`) in the process-global JIT are
  documented (AGENTS.md S36b) but not fixed — this is a real isolation defect
  of the JIT host, not a test artifact, and will bite multi-module agent
  workloads first.
- **G-A3 Build-artifact weight**: `australVM/_build` is ~22 GB on disk (plus
  `calltree.html`, `error.html` checked in at top level). Build outputs should
  be redirected/ignored and tree-checked artifacts removed.
- **G-A4 Test ergonomics**: running OCaml tests requires loader gymnastics
  (`LD_LIBRARY_PATH=$HOME/.local/lib`, or `/lib64/ld-linux-x86-64.so.2 …`)
  because the bridge `.so` is built against host glibc. This should be one
  command (a wrapper script that detects and re-execs under the right loader).

### 2.3 `velysterm/` — UI / AI-agent interface

**State**: `mathed_core` (Loro CRDT doc model, semantic index, glyphs,
accessibility), `mathed_mini` (winit+softbuffer editor + kernel bridge),
`mathed` (Bevy), `mathed_biblio` (hayagriva), `kernel_client` (`unfer_agent`
NDJSON, 36 ops, timing, repair hints). velyst consumed upstream at `v0.0.1`
(not forked). Plans C1–C16 complete; CHANGELOG maintained.

**Strengths**: clean crate layering; the headless `KernelBridge` shared by both
frontends; collaborative editing (export_delta/import_delta) already exists —
the seed for multi-agent document work.

**Gaps found**:
- **G-V1 Stale progress doc**: `PROGRESS.md` narrates mid-2026 "Stage A/R"
  work and predates the C1–C16 completion recorded in `PLAN_parallel_velysterm.md`;
  two documents disagree about the state of the world.
- **G-V2 Archived orphans**: `delta_algebra`/`delta_sirk` are kept outside the
  workspace glob; fine, but their findings (GPU/wgpu experiments) are not
  written up anywhere, so the knowledge is effectively lost.
- **G-V3 Agent interface is request/response only**: `unfer_agent` has no
  cursor-based batched delivery or event stream, so any multi-agent layer
  would have to poll ad hoc (see C1).

### 2.4 `timepiece/` — Lean 4 corpus + book

**State**: `Book/`, `BookProof/`, `Audits/`, `GapCertificate`, `Layout`,
extraction scripts, and a very large documentation corpus (61 documents in
`DOC_INDEX.md`, `CONSOLIDATED_PLAN.md` >15k lines with dated
"State of the project" appendices, `STATUS.md` ~12k lines). `book.tex` is the
prose source of the physics program; `unfer_contracts/` carries the
proof-facing seams (MASS_GAP_SPEC, regeneration bundle).

**Strengths**: the "honest boundaries" discipline (`HONEST_BOUNDARIES_SM.md`,
refuted comparison operators recorded as refuted) is exemplary; nanoda
re-verification of Lean exports is an independent-checker pattern reused from
`logos`; the dated state sections give any new agent a precise handover.

**Gaps found** (documentation/tooling only — no Lean code here):
- **G-T1 Doc graph is unlinked**: `DOC_INDEX.md` reports **0 backlinks**
  across 61 documents. The index machinery exists (adapted from `typos`,
  Apache-2.0 — keep the attribution) but cross-references aren't mined from
  the corpus, so "which plan supersedes which" lives in readers' heads.
- **G-T2 Plan sprawl**: many parallel PLAN_* and REVIEW_* documents plus
  15k-line CONSOLIDATED_PLAN. Supersession is stated in prose, not machine
  readable. A single "current work order" pointer per track (NS / QYM / QG /
  SM / RandomMap / book) is missing.
- **G-T3 Corpus duplication with unfer** (G-U1).
- **G-T4 Reproducibility bundles are the project's benchmark gold** —
  `unfer_contracts/MASS_GAP_REGENERATION.md` is a self-contained
  reproduce-the-artifact task, currently used once (see N3).

### 2.5 `dynamic-arctic/` — threshold signing

**State**: Arctic two-round threshold Schnorr + Shine robust VPSS, native
robustness + identifiable aborts + PSS; library reused by `unfer_consensus`
(`MintAuthority::Threshold`, 64-byte aggregate fits `op.signature`); axum
AT-Protocol prototype behind a `server` feature; 32 unit tests.

**Strengths**: correct minimal-dependency reuse story (path dep, default
features off); deterministic session-bound nonces keep consensus replay-safe.

**Gaps found**:
- **G-D1 Stale README(s)** ("PROBABLY NOTHING WORKS"; README vs README_arctic
  split is confusing).
- **G-D2 Compiler warnings** were outstanding at last review (unused import,
  2 dead structs) — re-verify and clear; add `clippy -D warnings` to CI.
- **G-D3 The server surface is unloved**: prototype API has no health
  endpoint, no normalized message schema, no ops runbook — while the project
  already standardized those patterns elsewhere (see C4/X6).

### 2.6 `test/` — the public documentation site

**State**: GitBook tree (`book.json` empty), `SUMMARY.md` TOC covering basics /
software / unfer / the book / proofs, `literature.bib` + `references.bib`,
`scripts/check_site.py` (SUMMARY drift, internal links, backlinks) run in CI
(SHA-pinned checkout; pattern attributions to `ax` and `typos` are stated in
the workflow header — good practice, keep).

**Strengths**: the site is the public face of the whole program and its CI
already checks link/TOC health; the software/ section already argues the
project's design ideas (altgit, digital sovereignty, Logic-Bayesian AI).

**Gaps found**:
- **G-S1 No license file** for site content (author + ORCID are credited in
  `README.md`, but reuse terms are unstated).
- **G-S2 No cross-repo docs pipeline**: `test/` narrates `unfer`/`timepiece`
  results by hand; nothing fails when upstream docs move (no inclusion/sync
  gate like PLAN_HARNESS H2's doc-sync).
- **G-S3 The prose is where the multi-agent story should land** (see G8): the
  site already hosts `software/bai.md` ("Scalable Logic-Bayesian AI
  Architecture") and `software/searchllm.md`; the agent-organization design
  belongs beside them.

### 2.7 `emthin/` — the compositor

**State**: smithay-based nested Wayland compositor where shell UI = a `mathed`
document and each app is a figure (`\app` statement) in the page flow; JSON-RPC
control protocol (`docs/ipc.md`); clipboard/dbus/fcitx5/xwayland-satellite
substrate kept; rewrite executed (Emacs shell gone). GPL-3.0 with correct
dependency-license notes.

**Strengths**: "the document is the layout authority" is a genuinely novel,
coherent design; the IPC plane is documented and is an external-frontend seam.

**Gaps found**:
- **G-E1 README's own "not yet implemented" list** (interactive launcher,
  dormant-figure click-to-start, free placement decision) is the natural
  backlog; the keymap relaunches "the first dormant figure", which is a
  placeholder behavior.
- **G-E2 REWRITE_PLAN Part 2** (ProofFlow adaptation, NL → logos CNL) is
  unstarted.
- **G-E3 No agent-facing control surface**: the JSON-RPC socket is for bars and
  scripts; nothing lets an AI worker (from velysterm's `unfer_agent`) place or
  reconfigure figures — though the pieces exist (C7).

---

## 3. Cross-cutting findings

1. **X-F1 Self-description rot** (G-A1, G-D1, G-V1): status docs lag reality in
   three repos. The project already knows how to fix this (doc-sync gates) — it
   needs one applied to README/AGENTS status headers.
2. **X-F2 Byte-duplicated sources** (G-U1/G-T3) with no drift gate.
3. **X-F3 No cross-repo CI**: dependents path-depend on `unfer`'s working tree
   ("green-workspace rule"), but no CI job builds the three-repo combination,
   so the rule is enforced by discipline only.
4. **X-F4 Single-agent everything**: agent interface, work orders, verification
   runs. Both reference projects show the same tasks benefit from concurrent
   workers, and this project's own trust model (grants, meters, approvals) is
   exactly what a multi-agent layer must be wrapped in.
5. **X-F5 No benchmark/eval harness for agent work on this corpus** — despite
   having ideal benchmark material (regeneration bundles, golden gates).
6. **X-F6 Attributions are local, not central**: license/pattern credits are
   scattered in workflow headers and plan prose (ax, typos, driftwm,
   deepseek-harness, qm, Cadabra2/Why3 seams). A central ledger is missing.
7. **X-F7 ABBA lock-order inversion in `unfer_ffi`'s test locks — FIXED.** About a
   quarter of full-suite runs hung indefinitely: five tests blocked forever on
   `DURABLE_TESTS_LOCK` and the holder never returned. Two distinct defects, both
   now fixed and verified over 46 consecutive clean runs (was 3-in-12 hanging):

   - *The deadlock.* `t6_emitted_certificates_are_durably_recorded_end_to_end`
     took `DURABLE` first, then `ACTION`, then `AUDIT` — while four sibling tests
     took `ACTION` first and wanted `DURABLE` later. Classic ABBA: each held what
     the other needed. A probe printing acquire/release against the libtest thread
     name identified `t6_...` as the holder. Fixed by taking durable last, matching
     the majority order.
   - *A missing lock.* `audit_trail_never_persists_fixture_secret` called
     `uk_owner_clear()` while holding only `AUDIT_AGENT_TESTS_LOCK`, never
     `ACTION_TESTS_LOCK` which every other owner-log test holds, so it could wipe
     lines another test had just written. That surfaced as an intermittent
     *failure* on `assert!(uk_owner_clear() >= 2)`, not a hang.

   Nine tests still use four different lock orders, and several re-acquire a lock
   they already hold in a nested scope. That is safe today only because the
   ordering now happens to be consistent; the durable fix is to consolidate to a
   single group lock or one canonical order, so the next test added cannot
   reintroduce this. Not yet done.

   **Consolidation was attempted and is not safe as a mechanical change.** Merging
   the four mutexes into one `TEST_GROUP_LOCK` hangs the suite immediately: at
   least one test acquires two *differently named* locks in nested scopes — fine as
   separate mutexes, an instant self-deadlock once they are the same one. The
   re-acquisitions are therefore not all benign, and a name-substitution pass
   cannot distinguish a nested re-acquisition from a sequential one. Reverted.
   Doing this properly means auditing the ~90 call sites for nesting first, or
   restructuring so nested acquisitions cannot occur at all.

   **A second attempt: reordering rather than merging, also reverted.** By
   inspection every multi-lock test now takes ACTION → AUDIT except
   `audit_trail_never_persists_fixture_secret`, which takes AUDIT → ACTION —
   because the missing lock added to fix its failure was appended *after* the
   existing one, i.e. the fix for the deadlock class introduced an instance of it.
   Reordering it to ACTION → AUDIT is right in principle and made things
   measurably worse: **2 failures in 20 runs** (`owner_logger_...` and
   `action_submit_queues_pending_record_...`), against 0 in 20 reverted. No hangs
   either way, so this is not a deadlock — it is shared state that neither mutex
   covers, which means my model of what these tests touch is incomplete.

   So the order was left as-is on the evidence, not on the principle, and that is
   unsatisfying enough to flag: the real fix is an inventory of which tests touch
   the action store, the owner log, and the audit ring, and which mutex each of
   them actually holds. Two of those tests fail intermittently and the ordering
   fix made one worse, so there is more than one missing lock. That inventory is
   the prerequisite, and it has not been done.

   **CLOSED (A12) — the inventory was done, and it changed the answer.**
   `unfer/docs/TEST_LOCKS.md` is the inventory: who touches which store, holding
   which mutex, with evidence. Headline findings: `reset_durable_for_tests`
   wipes **all four** shared stores while eleven callers held one lock or none;
   the spawn-pacing knob was guarded by **two different mutexes** that did not
   exclude each other at all; and the "shared state neither mutex covers" guess
   above was wrong — every flake resolved to an under-held store. One order
   (P → ACTION → AUDIT → DURABLE) now holds mechanically across `lib.rs` +
   `handles.rs` (acquisition-order scan came back empty after
   `audit_trail_never_persists_fixture_secret` — the last inversion, caught
   deadlocking the suite live — was converted), and the fixed binary ran
   **100/100 clean at 16 threads** with `timeout 60` so a deadlock counts as a
   failure. The merging attempt stays reverted (nested re-acquisitions make it
   an instant self-deadlock), and the honest residue is written down in the
   inventory: an acyclicity scan proves lock order, not the existence of locks,
   and no full-workspace campaign was rerun as part of A12.

   **2026-10-08 re-verification found two more members of the -1010 class and
   fixed them.** A fresh 20-run campaign (release, 16 threads, `cargo test -p
   unfer_ffi` — lib + `tests/ffi.rs` binaries, under machine load) failed
   **8/20 runs**, all in two tests that hold *no* locks:
   `session_evolve_append_failure_surfaces_uk1010_at_ffi` (6×) and
   `poll_with_too_small_buffer_keeps_event_for_retry` (2×). The captured value
   (`left: -1010, right: 0` on the sibling-model evolve) named the mechanism the
   inventory already documents: `store_session` inherits `handles::durable()`
   — the kernel-GLOBAL slot — so a concurrently-live failing store
   (`resolved_checkpoint…`'s FlushFailAfterN) surfaces as a spurious -1010 in
   lock-free models. Both tests carried comments *asserting the opposite*
   ("cannot race any other test's global durable store"), which is why A12's
   scan missed them — the inventory had trusted two false comments. Fix: both
   take `DURABLE_TESTS_LOCK` (single lock, trivially ordered; the swap-window
   installer already holds A→U→D). Post-fix campaign, same protocol: **0/20**.
   The lesson is recorded in `unfer/docs/TEST_LOCKS.md`: a comment claiming
   "no race" is a hypothesis, and the campaign is the evidence.

---

## 4. Reference studies (what is worth adapting)

### 4.1 `ClawSwarm/` (Apache-2.0, The-Swarm-Corporation)

This is the first of three reference studies (§4.2 and §4.3 the others), and
the one this plan adapts most heavily: the C-items of §6 rebuild ClawSwarm's
operational posture on this project's own stack, alongside the X-items of
§5 that harden what the stack already has. What is worth studying here is
not its domain or its marketing but its *mechanics* — how a long-running
multi-agent service stays operable: one ingest surface, narrow-role workers,
bounded memory, documented config precedence, and deployment a stranger can
repeat. The licensing rules that govern every adaptation are in §9, and the
last paragraph below marks the hard exclusions.

*[Reconstructed 2026-10-05 — the original opening paragraph of this
subsection was destroyed by a bad scripted edit (an end-index matched a `**`
in this section). This replacement is written to fit the section as it now
stands rather than restored verbatim; the body below is original.]*

ClawSwarm runs a multi-channel agent service on a **unified messaging gateway**
(gRPC, normalized `UnifiedMessage`, poll-with-cursor + stream + health, optional
TLS) feeding a **hierarchical swarm** — a director agent that only plans and
delegates via structured orders, specialist **workers** with narrow roles/tools
(response / search / code via Claude-as-a-tool / token-launch), and a
**summarizer** that condenses swarm output per channel. Supporting mechanics:
persistent **markdown memory** with char cap and RAG retrieval on overflow;
per-interaction DB logging; **config precedence** + interactive **onboarding
wizard** generating `claw_config.yaml`; env-var tables with "omit to disable"
channels; local model backends (`vllm/`, `hf/`, `vllm-server/` spec prefixes,
separate director/worker models); long-running agent loop with signal handling;
Docker/systemd/Railway deployment runbooks; reply extraction from raw agent
output (marker-based); `tools_registry` for pluggable tools.

**Adapt with care**: Apache-2.0 permits reuse with attribution, but see §9 —
we adapt mechanisms, not code. **Do not adapt**: the token-launch/Solana fee
tooling (speculative finance; unrelated to this project's mission even though
`unfer_consensus` has market machinery), the "10,000 agents" framing, and the
OpenClaw-comparison marketing.

### 4.2 `~/Downloads/agensh/` — "Agensh: Scaling Organizational Intelligence to 1,024 Agents" (Microsoft Research technical report; LaTeX reconstruction)

Orchestrator-free multi-agent organization: every worker runs the same
**five-step cooperation loop** (gather context → claim sub-task → take action →
verify results → merge progress) asynchronously, over three infrastructure
components: a **shared workspace** (Git platform; version history, PRs,
exposed merge conflicts), a **message interface** (team channel + priority
direct messages delivered mid-turn), and a **shared context board**
(append-only, typed entries: `OBSERVED` / `FACT` / `FAIL` / `CLAIM` /
`PATCH_SUMMARY`; capped entry sizes with `detail` unfold; `board_grep` over the
full history with AND/OR syntax; new entries pushed to peers mid-turn).
Runtime: event-driven workers with durable per-worker queues, event IDs for
dedup, retry with backoff, session restore after router restart, idle
detectors, "time-budget reminders" (T−45min stop features / T−5min merge and
verify), staggered agent activation. Reported result: agent count is a
*scaling dimension* (1→128 agents ≈ +49% relative test-pass rate on hard
reproduction tasks; 1,024 agents on one task), with emergent specialization,
integration roles, and standardized cooperation protocols at scale.

**Adapt with care**: the paper text is copyrighted; the worker prompt is
expressive content — **paraphrase mechanisms, never copy prompt text**. The
companion code (`microsoft/Agensh`) has **unknown-to-us licensing; do not copy
code from it** until its license is verified. Cite the paper wherever its
mechanisms are used (§9).

### 4.3 `loop/` (Apache-2.0, Soket AI) — one adaptation landed, the rest assessed

**Landed 2026-10-08**: two exec-output-capture ideas in the existing kernel
worker (`sanitize` control chars before bytes become a `Diagnostic`,
`truncate_tail` keeping a failure's tail under a byte cap) — see the `loop`
row in `ATTRIBUTION.md` and `velysterm/crates/kernel_client/src/worker.rs`.

**Assessed 2026-10-08 (deprioritised backlog)** — each remaining mechanism
against what this project already has. "Backlog" means: not a gap today; the
stated trigger is what would make it one.

| loop mechanism | what this project already has | verdict / revisit trigger |
|---|---|---|
| session fork / search / stats | `kernel_client::session` (G2: one versioned `sessions.json`, spawn/restore, claim replay) and `board_grep` over G1's board | **backlog** — revisit when the org harness accumulates session history worth searching (N1 multi-run campaigns); fork = branch-a-worker for G2 retry, useful then |
| sandbox registry (`KrunSandbox`/`LocalSandboxRuntime`) | grants/meters/approvals at the loopback (C5 `GrantSet`, UK codes), G4 verify-before-merge | **backlog, honest gap**: no OS-level isolation for worker exec; revisit if workers ever run untrusted code beyond the gate-run machinery — the trust model today is grants, not namespaces |
| skills (markdown → system prompt) | **the kernel side already exists**: H13 `SkillRegistry` (`uk_skill_*`, org-scoped, grant-free listing, `unfer_ffi/src/handles.rs:1388`) | **partially present** — the missing piece is loading visible skills into `org_runner` worker prompts (needs an agent-channel skill op + prompt wiring + tests); first candidate when this backlog is picked up |
| compaction (long-context summarization) | C6 `uk_summarize` + C2 bounded memory (cap 8000, BM25 retrieval) — the project's chosen bounded-memory design | **backlog** — revisit only if workers accumulate long transcripts (coop steps are short by design) |
| faux provider (scripted fake LLM) | deterministic kernel + `three_worker_tests` already assert loop mechanics without live models | **mostly covered** — revisit if live-provider smoke tests need offline determinism |
| `detect_compat` (provider capability probing) | C7 `model_spec` prefixes; loopback smoke `#[ignore]`d behind `UNFER_SMOKE_BASE_URL` | **only relevant beside live providers** — revisit with the C7 smoke expansion |
| opt-in telemetry | C1 cursored event logs, audit ring, N2 benchmark report (the refuse-to-fake guards) | **mostly covered** — revisit as per-worker step metrics when N2 grows task families |

---

## 5. Baseline improvement plan (existing features, no borrowed designs)

Ordered small → large. Owner: `eng` = general engineering (no Lean),
`spec` = LLM-Lean4-specialist work order only, `docs` = writing.

### X1 — Source-of-record for duplicated LaTeX (U) — *unfer + timepiece*
**Improves**: the existing "single source of truth" discipline (PLAN_HARNESS
H2). `book.tex` and `ODE.tex` are byte-identical in both repos today.
**Work**: designate `timepiece/` as the source of record (it hosts the book
build); replace `unfer/` copies with a sync script + drift gate in both CIs
(checksum equality, failing with a "run the sync" message). Any tooling that
reads `unfer/book.tex` keeps working via the synced copy.
**Acceptance**: editing the source copy without syncing fails CI in the other
repo; a `make sync-book` (or script) restores green. Owner: eng.

### X2 — README/AGENTS status headers + doc-sync gate (S) — *all repos*
**Improves**: X-F1; PLAN_HARNESS H2's doc-sync idea, applied to status prose.
**Work**: each repo gets a one-line machine-readable status block in README
(`status:`, `tests:`, `last_verified:`); a script compares it against reality
where cheap (test counts from `cargo test -- --list` / `dune runtest` output /
`pytest --collect-only`) and fails on drift. Rewrite `australVM/README.md` and
`dynamic-arctic/README(_arctic).md` to the true state (green suites, plugin
architecture, `server` feature). Fold `velysterm/PROGRESS.md` into
`CHANGELOG.md` + `PLAN_parallel_velysterm.md` (one history, one status).
**Acceptance**: `scripts/check-status` green; stale claims gone. Owner: docs+eng.

### X3 — AGENTS.md knowledge extraction (M) — *unfer*
**Improves**: G-U2. **Work**: move the S30* essays (Cadabra2 gotchas, module
audits, per-theory derivations) from `AGENTS.md` into `docs/` pages
(`docs/CADABRA2_GOTCHAS.md`, per-module verify docs already exist — link
them); leave the checklist items as one-line invariants + pointer, as
PLAN_HARNESS H1 expects. No knowledge is dropped — every extraction keeps a
link. **Acceptance**: `AGENTS.md` ≤ ~120 lines; every removed paragraph
reachable from `docs/` in ≤2 clicks; H1 gate unchanged. Owner: docs.

### X4 — JIT isolation & test ergonomics (M) — *australVM*
**Improves**: G-A2/G-A4 (existing JIT host + test scripts).
**Work**: (a) namespace JIT-registered `run` symbols per test (or reset the
symbol table between tests) so `JitTest` collisions can't happen; retire the
"pre-existing failure" note in AGENTS.md S36b. (b) One entry point
`make test` that detects the loader situation and re-execs test binaries under
the correct dynamic loader; document the glibc caveat once.
**Acceptance**: `dune runtest` green with no manual env/`ld-linux` invocation;
S36b note replaced by the wrapper's comment. Owner: eng.

### X5 — Build-artifact hygiene (S) — *australVM, velysterm, emthin*
**Improves**: G-A3. **Work**: confirm `_build/`, `target/`, `calltree.html`,
`error.html`, `qfm_text_runs/`, `logs/` are gitignored and untracked; move any
tracked artifact into `docs/` as a small excerpt or delete; document retention
for `qfm_text_runs/` and `logs/` (G-U6); add a `scripts/tree-size` check that
fails CI if a build directory is tracked. **Acceptance**: `git ls-files` shows
no build outputs; retention policy documented. Owner: eng.

### X6 — Ops surface for the two servers (M) — *unfer_edge + dynamic-arctic*
**Improves**: G-D3; the existing S22 admin seam and axum prototype.
**Work**: standardize both HTTP surfaces on: `/healthz` (the `ax` pattern,
Apache-2.0 — keep the attribution in `test/.github/workflows/site-check.yml`
style), a version/build endpoint mirroring `unfer_agent`'s `version` op,
env-based config with the precedence order of §C3, and a short `docs/RUNBOOK.md`
per server (start, rotate, backup, revoke). **Acceptance**: `curl /healthz`
documented and covered by an integration test. Owner: eng.

### X7 — Cross-repo integration CI (M) — *all repos*
**Improves**: X-F3; the existing green-workspace rule.
**Work**: one workflow (in `unfer/` or a small `ci/` job in the root container)
that checks out the sibling repos at pinned refs and runs the documented
"Verify" commands of velysterm + unfer + dynamic-arctic tests and the
`australVM` plugin tests via the X4 wrapper. It must run on changes to the
`unfer` working tree (the path-dep hazard) and be allowed to fail-fast with a
clear "which repo broke which" message.
**Acceptance**: a deliberate break in `prob_kernel` shows up as a named
failure in the combined job. Owner: eng.

### X8 — Lean work-order hygiene (S, spec) — *timepiece*
**Improves**: G-T1/G-T2 (existing DOC_INDEX + CONSOLIDATED_PLAN).
**Work** (tooling and docs only — **no Lean code**):
(a) extend `scripts/doc_index.py` output with backlink mining from `[[...]]`/
markdown links (the "Backlinks (0 links)" section should be non-empty);
(b) add a `CURRENT_WORK_ORDER.md` with one row per track (NS / QYM / QG / SM /
RandomMap / book / mass-gap) pointing at the *current* authoritative plan
section and its superseded predecessors;
(c) everything Lean-shaped discovered along the way is appended to
`CONSOLIDATED_PLAN.md` as a work order for the LLM-Lean4-specialist, never
implemented here.
**Acceptance**: DOC_INDEX shows real backlinks; each track has exactly one
"current" pointer. Owner: docs (+spec for the work orders).

### X9 — Attribution & license ledger (S) — *all repos*
**Improves**: X-F6; the existing scattered credits.
**Work**: `ATTRIBUTION.md` per repo (unfer already has `docs/ATTRIBUTION.md` —
extend its scope to "borrowed patterns", not just carbon credits) listing every
borrowed pattern/component: source, license, what was adapted, where it landed
(ax /healthz, typos index-graph, driftwm canvas mechanics, deepseek-harness,
qm, Cadabra2 subprocess seam, Why3 LGPL seam, velyst upstream, ClawSwarm,
Agensh). Plus `test/` content license (G-S1).
**Acceptance**: every "adapted from" phrase in the tree points at a ledger row.
Owner: docs.

---

## 6. Adapted from ClawSwarm (improvements of existing features first)

### C1 — Cursor-based delivery + event stream for `unfer_agent` (M) ★
**Improves**: the existing NDJSON agent interface (`kernel_client`,
36 ops) — G-V3. **Source**: ClawSwarm gateway `PollMessages`/`StreamMessages`
+ agent-runner cursor batching (adapted; Apache-2.0 credited in X9).
**Work**: additive NDJSON ops `events_poll {since_cursor, max}` and
`events_stream` (server-push over the existing std.io/loopback channel; NDJSON
stays the degenerate single-capability mode per S28) exposing `KernelEvent`s
with monotonic cursor ids; agent runner loop in `kernel_client` batches by
cursor and keeps its own checkpoint, so a crash replays exactly-once
(compose with PLAN_HARNESS H7 idempotency). New symbols follow the S29
checklist (registry row → generated artifacts).
**Acceptance**: two concurrent consumers each see every event exactly once;
cursor survives restart; UK-#### codes assigned; `EXPECTED_SYMBOLS.txt` etc.
regenerated. Owner: eng.

### C2 — Persistent agent memory with bounded context + retrieval (M) ★
**Improves**: PLAN_HARNESS H3 (session log/compaction) and S23/S27
(sanitizer/vault) — not a new store. **Source**: ClawSwarm markdown memory +
`AGENT_MEMORY_MAX_CHARS` + RAG-on-overflow.
**Work**: on top of H3's event log, add a memory view: after each session
segment, append a sanitized (`sanitize_sensitive`) summary record; a
`memory_read {query}` op returns the full recent memory when under the char cap
and the most relevant chunks (simple BM25/embedding retrieval over the log)
when over. Never stores secrets (S27 rule holds: vault material is opaque and
non-serializable). Editor-side, `mathed`/`mathed_mini` show the same memory
via `KernelBridge`.
**Acceptance**: memory overflow path returns relevant chunks deterministically
in tests; secret-scan gate still green. Owner: eng.

### C3 — Config precedence + onboarding generator (S) ★
**Improves**: S22 admin config + `module.toml` + `QfmConfig` call sites — one
vocabulary, one precedence. **Source**: ClawSwarm `config.py` precedence and
`clawswarm onboarding` wizard.
**Work**: documented precedence `defaults < config file < env < flags` for
`unfer_edge`/`dynamic-arctic` servers and `module.toml`; an `init` subcommand
that interactively writes a commented starter config (wizard output is
deterministic given answers, so it is testable); the S22 hard keys stay
unpatchable and refusal stays byte-identical (unchanged invariant).
**Acceptance**: wizard-generated config passes the existing validation; H1
invariant for S22 still green. Owner: eng.

### C4 — Normalized message schema on the two HTTP surfaces (S)
**Improves**: X6 (the runbook/health work). **Source**: ClawSwarm
`UnifiedMessage` normalization ("one ingest API; add platforms without touching
agent logic").
**Work**: a small shared serde type (in `unfer_protocol`, additive) for
ingested messages `{id, source, channel, thread, sender, text, attachments,
ts}` used by `unfer_edge` webhooks and the dynamic-arctic `/api/v1/*` routes,
so downstream handlers are channel-agnostic. This is schema reuse, not a new
broker. **Acceptance**: both servers deserialize the same type; fuzz target
exists (the project already fuzzes `unfer_protocol`). Owner: eng.

### C5 — Worker role presets over GrantSet (M) ★
**Improves**: PLAN_HARNESS H10 (agent-presets) and S21 effect kinds.
**Source**: ClawSwarm narrow-role workers with focused tools ("director does
not execute tools; workers have narrow toolsets").
**Work**: named grant presets (`reader`, `prover-runner`, `reviewer`,
`integrator`, `maintainer`) as data over the existing `GrantSet` vocabulary —
each preset is a least-privilege grant set, and `agent_spawn`/capability minting
(S28) accepts a preset name. The director-style "orchestrates but has no tools"
role is expressible as a preset with zero mutation grants.
**Acceptance**: `GrantSet::is_subset_of` invariants hold (no
Mutate→Observe relabeling); preset misuse (worker with `maintainer` grants)
fails at the loopback with an audit entry. Owner: eng.

### C6 — Summarizer pass for agent output (S)
**Improves**: the existing result channel (S46 discipline) and repair hints.
**Source**: ClawSwarm's summarizer agent condensing swarm output per channel.
**Work**: when an agent session's output is destined for a human surface
(editor overlay, `test/` docs draft, console), route it through a
summarization step (local model via C8 or a worker preset from C5) that emits
channel-appropriate plain text; the raw output stays in the session log. Keep
S46 semantics: summarization is an `observe`-kind effect.
**Acceptance**: overlay shows only summarized text; full trace retrievable.
Owner: eng.

### C7 — Local-model backends for the agent interface (M)
**Improves**: `qfm`/`qfm_text` + `kernel_client` model configuration.
**Source**: ClawSwarm `vllm/`/`hf/`/`vllm-server/` spec prefixes + separate
director/worker models.
**Work**: model specs in config accept prefixes (`vllm/…`, `hf/…`,
`openai-compatible:<url>`); `WORKER_MODEL_NAME`-style override for workers vs
primary model; document in the runbook. No model code is copied — only the
spec-prefix convention.
**Acceptance**: a documented smoke test runs one agent op against a local
OpenAI-compatible endpoint. Owner: eng.

### C8 — Deployment runbooks + container story (S)
**Improves**: `unfer/deploy/` and X6. **Source**: ClawSwarm deployment section
(env-var table with "omit to disable", Docker volume notes, systemd 24/7).
**Work**: per-server env-var table in docs; Dockerfile or nix-based container
where the repo already has flakes; systemd unit examples; secret handling via
env/secrets manager (never committed — matches S23).
**Acceptance**: a fresh machine can boot `unfer_edge` + dynamic-arctic from
the runbook alone. Owner: docs+eng.

**Explicitly not adapted from ClawSwarm**: token launch/fee claiming (see §11).

---

## 7. Adapted from Agensh (improvements of existing features first)

★ = the priority items (they improve the agent seams C1–C5 extend).

### G1 — Shared context board on the kernel surface (M) ★
**Improves**: the existing `KernelEvent`/audit pipeline (S23 ring) and the
`unfer_agent` op set — the board is a **typed, sanitized, append-only view**
of what the audit ring already does, not a new database. **Source**: Agensh
shared context (OBSERVED/FACT/FAIL/CLAIM/PATCH_SUMMARY; capped entries with
`detail`; grep over full history; mid-turn push). Paraphrased mechanics only.
**Work**: additive ops `board_write {kind, text≤N, detail?}`, `board_read
{limit}`, `board_grep {expr}` where `expr` supports `,` (OR) and `&` (AND),
case-insensitive; entries carry `worker_id`, `ts`, cursor id (reuse C1);
every write passes `sanitize_sensitive`; `CLAIM` and `PATCH_SUMMARY` are
`Mutate`-kind effects (S21 approval lane), the rest `Observe`-kind. Storage:
the S23 ring plus a durable append-only log (compose with PLAN_HARNESS H4
durable audit instead of adding a store). New symbols: S29 checklist.
**Acceptance**: board survives restart; grep semantics unit-tested; a
`FAIL` entry prevents a peer from re-deriving (checked in the org-harness test,
G2). Owner: eng.

### G2 — Cooperation loop as the agent-runner protocol (M) ★
**Improves**: `kernel_client`'s worker thread + C1's event loop — the loop is
**prompt-and-protocol policy around the existing ops**, exactly as Agensh keeps
the loop in workflow instructions rather than hard-coded runtime.
**Work**: define the five-step loop (gather → claim → act → verify → merge) as
the standard worker prompt template **written for this project** (no copied
text), with board verbs from G1 and acceptance criteria supplied per task;
the runner implements the mechanical parts: deliver queued events at loop
start, deliver priority board entries/DMs on every tool return (mid-turn),
retry failed deliveries with backoff, restore session after restart (H3 fork
log), idle detector prompt after N minutes, and the two time-budget reminders
when a task carries a deadline (compose with S25 meter windows for budget
enforcement).
**Acceptance**: an integration test drives 3 workers over one task; claim
collision detected; merged output passes the verify gate. Owner: eng.

### G3 — Claims with overlap resolution via direct messages (M) ★
**Improves**: S21/S28 approval flow + `action_submit`/`gate_approve`.
**Source**: Agensh `CLAIM` + direct-message collision settling.
**Work**: `CLAIM {scope}` entries (G1) with scope strings matched for overlap
(prefix/regex over file paths, module names, Lean chapter names); overlap
raises a `KernelEvent` to both workers; `agent_dm {to, text}` additive op
(delivered mid-turn per G2) for negotiation; unresolvable overlap escalates to
the human approval lane (existing behavior). Claims expire on idle timeout.
**Acceptance**: two workers claiming overlapping scopes reach a resolution or
escalation without duplicate merged work in the integration test. Owner: eng.

### G4 — Verify-before-merge discipline on existing gates (M) ★
**Improves**: H1 `verify-invariants`, the golden release gate (S23/S24), and
the S36/S37 cycles — not new checks. **Source**: Agensh "verify results" step +
`PATCH_SUMMARY(files= | idea= | evidence=)`.
**Work**: the merge step of G2 requires a `PATCH_SUMMARY` whose `evidence`
field is machine-generated gate output (invariants runner, golden manifest
diff, test log excerpt); `gate_approve` refuses a merge whose evidence is
missing or stale (older than the patch). This makes the human approval lane
faster and stricter at once.
**Acceptance**: a merge with hand-written (non-machine) evidence is refused;
golden gate still regenerates only via `UPDATE_GOLDEN=1`. Owner: eng.

### G5 — Write-time conflict detection for document edits (M)
**Improves**: velysterm C13 collaborative editing (Loro CRDT
export_delta/import_delta) — **the project already has the merge engine**;
what's missing is the STORM-style stale-edit warning (Agensh related-work
pattern) and the "exposed conflict" UX.
**Work**: when an agent (or second human) imports a delta that touches blocks
changed since its base revision, surface a structured conflict record
(revision ids + block ranges) instead of silent CRDT resolution; the worker
merges per G2 step 5 (re-check against acceptance criteria, re-merge). For the
code repos, the same policy is "sync `main` before merging" (already Agensh's
worker rule) enforced by the G4 evidence freshness check.
**Acceptance**: stale-import test yields a conflict record; CRDT convergence
tests stay green. Owner: eng.

### G6 — Typed shared context for the docs/plans corpus (M) ★
**Improves**: X8 (work-order hygiene) and `timepiece`'s "State of the project"
appendices — the board is where *plan* knowledge goes, so CONSOLIDATED_PLAN
stops absorbing every observation.
**Work**: the G1 board (or its durable log) becomes the receptacle for
work-order observations (`FACT` about a theorem's dependencies, `FAIL` about a
refuted approach — cf. the recorded Faris–Lavine refutations, `CLAIM` on
modules under edit); `CURRENT_WORK_ORDER.md` (X8) links each track's recent
board entries; monthly rollup appends to CONSOLIDATED_PLAN as before, so the
specialist's handover improves without any Lean work here. **Acceptance**:
rollup script exists; board entries cited from the work order. Owner: docs.

### G7 — Emergent-role support: integrator/reviewer presets at scale (S)
**Improves**: C5 role presets + `unfer_consensus` participation.
**Source**: Agensh's emergent specialization (workers choosing reviewers,
integrator hand-off, first-valid-responder selection).
**Work**: protocol-level (docs + prompt template + small ops) support for
*request/accept role hand-off*: `agent_dm` messages of the form
`role_request{reviewer}` / `role_accept{claim_id}` recorded on the board; the
runner tracks which worker holds which role per claim. Keep it deliberately
thin — the emergence is the workers' business, our job is to make hand-off
observable and auditable.
**Acceptance**: role hand-off visible in board history; no new privilege is
granted by role messages (roles are labels over existing presets). Owner: eng.

### G8 — Write the story into `test/` (S)
**Improves**: G-S3. **Source**: both reference projects' positioning
(ClawSwarm: multi-agent messaging layer; Agensh: org harness + scaling claim).
**Work**: a `software/org-harness.md` page (plus SUMMARY.md entry) describing
this project's agent-organization layer — claims, board, verify-before-merge,
capability-gated workers — and honestly contrasting it with orchestrator-worker
systems, citing Agensh for the scaling claim and ClawSwarm for the
gateway/hierarchical pattern. `check_site.py` already guards links/TOC.
**Acceptance**: site check green; citations present. Owner: docs.

### G9 — Budget nudges & staggered activation (S)
**Improves**: S25 metering and `unfer_consensus` boot behavior.
**Source**: Agensh staggered activation (one agent per 30s→3s) and T−45/T−5
reminders.
**Work**: (a) agent spawn staggering built on the existing meter window (S25)
— spawn rate is config, denial at the same single chokepoint; (b) task-scoped
deadlines emit board-wide nudges at configurable checkpoints ("stop claiming
new scope", "merge or report blocked") — pure event policy on G1/G2.
**Acceptance**: meter tests extended (spawn burst → UK-4601 at the
chokepoint); nudge events observed in board history. Owner: eng.

---

## 8. New features — justified enlargements (N)

Only three, each with a reason it can't be an improvement of an existing
feature:

### N1 — Org-harness runner binary (M)
A long-running orchestrator-free worker host (`org_runner`) composing C1–C5
and G1–G4: N workers × cooperation loop over the board + workspace, wrapped in
presets, metering, approvals. **Why new**: `kernel_client` is a single-session
client and `unfer_agent` is a pipe protocol; nothing today hosts *many* worker
sessions with durable queues. Built strictly from the C/G pieces above (its
API is the NDJSON ops; no parallel kernel surface). Benchmarks below gate it.

### N2 — Reproduction-benchmark harness (M)
Score agent organizations on project-native reproduction tasks, ProgramBench-
style (Agensh's evaluation framing, our own tasks): e.g. *mass-gap
regeneration* (`timepiece/unfer_contracts/MASS_GAP_REGENERATION.md` — rebuild
the certified bands from the bundle), *module reconstruction* (from a `.cell`
blueprint + golden manifest, rebuild behavior passing `tools/module_builder`
tests), *docs-site reconstruction* (regenerate `test/` pages passing
`check_site.py`). Scoring = test-pass rate of the rebuilt artifacts against the
project's own golden gates; time-budget + offline mode as harness options.
**Why new**: X-F5 — no eval harness exists; the tasks already exist as
one-off regeneration bundles. This is what makes the Agensh scaling claim
*checkable for this project* before anyone runs 128 workers.

### N3 — (deferred) multi-node federation demo — only if N2 shows scaling value.

---

## 9. Copyright & licensing compliance (mandatory reading for every item above)

1. **ClawSwarm** (Apache-2.0, © The-Swarm-Corporation / Kye Gomez):
   - Reuse of *code* is legally allowed with the Apache-2.0 attribution
     requirements (license copy, NOTICE, state-changes). **Policy for this
     project: adapt ideas, do not copy source.** Where any snippet is ever
     copied, it must (a) be recorded in the per-repo `ATTRIBUTION.md` (X9),
     (b) keep the Apache-2.0 notice, and (c) land only in repos whose license
     is compatible: MIT/Apache dual (`unfer`, `velysterm`, `dynamic-arctic`,
     `timepiece` as declared) and GPL-3.0 (`emthin`) are both fine receiving
     Apache-2.0 code; never re-license Apache code as MIT.
   - Do not use the name "ClawSwarm", its badges, or its branding for our
     features (trademark/endorsement risk). Feature names in this plan
     (board, org_runner, cooperation loop) are deliberately generic.
   - Its own dependencies (Swarms framework, Claude Agent SDK, etc.) carry
     their own licenses — we do not import them at all (all items above are
     reimplementations over our own stack).
2. **Agensh** (Zhan, Song, Dong, Huang, Lian, Xia, Wei — Microsoft Research
   technical report, "Agensh: Scaling Organizational Intelligence to 1,024
   Agents"):
   - The paper text/figures are copyrighted; **quote nothing, copy no prompt
     text**. Describe mechanisms in our own words (the §7 items already do).
   - The companion repository's license is **not verified** — treat as
     all-rights-reserved: no code, no prompt strings, no config copying.
   - Cite the paper (author-year + title) wherever G-items are documented
     (this plan, `test/software/org-harness.md`, AGENTS.md entries).
   - Third-party infrastructure named by the paper has its own licenses and
     is not adopted wholesale: Gitea (MIT, fine if ever needed),
     Mattermost (Team Edition MIT / Enterprise proprietary — if a message
     channel is ever needed, prefer an all-open alternative or the project's
     own board/DM ops, which is what §7 does anyway).
3. **Existing obligations to keep visible** (X9 ledger): `typos` (Apache-2.0)
   in `timepiece/scripts/doc_index.py` and `test/scripts/check_site.py`
   patterns; the `ax` /healthz pattern (Apache-2.0); `driftwm` (GPL-3.0)
   code adapted into `emthin/` (GPL-3.0 — compatible; keep notices); Cadabra2
   (GPL-3.0) and Why3 (LGPL) as **subprocess-only** seams in `unfer` (the
   established pattern — never link them into the MIT/Apache binaries);
   `velyst` upstream (MIT/Apache, consumed as dependency).
4. **`test/` content license** (G-S1): add explicit terms (e.g. CC BY 4.0 for
   prose) keeping the ORCID author credit.
5. **Generated-artifact provenance**: anything in this tree "adapted from"
   must name source + license at the point of use (the project already does
   this well — the site-check workflow header is the template).

---

## 10. Sequencing & acceptance gates

Phase 0 — hygiene & truth (small, first):
X1, X2, X5, X9, G-D-README fixes. Gate: `check-status`, `tree-size`, sync
drift gate all green.

Phase 1 — the agent seam becomes multi-worker-ready:
C1, G1, C5, G3 (ops on the frozen surface, S29-compliant), C3, X6.
Gate: two concurrent consumers + board round-trip tests; S21–S28 invariants
unchanged (H1 gate green).

Phase 2 — the cooperation loop:
G2, G4, G5, C2, X4. Gate: 3-worker integration test with claim collision →
verify → merge, evidence freshness enforced.

Phase 3 — the organization layer:
N1, N2, C6, C7, G7, G9, X7. Gate: N2 benchmark runs 1 vs 4 workers on the
mass-gap regeneration task and reports test-pass rate + time-to-threshold.

Phase 4 — story & scale-out:
G6, G8, X8, C8, N3-justification. Gate: `test/` site check green with the new
page; DOC_INDEX backlinks non-empty; a decision record for N3.

Throughout: any Lean-shaped work discovered is written as a work order in
`timepiece/CONSOLIDATED_PLAN.md` / `CURRENT_WORK_ORDER.md` for the
LLM-Lean4-specialist. **No `.lean` file is edited by any item of this plan.**

---

## 11. Explicit exclusions

- **Token launching / fee claiming / Solana tooling** (ClawSwarm
  `TokenLaunch`): speculative finance unrelated to the project's mission; the
  existing market machinery in `unfer_consensus` (vAMM/NegRisk, math-bond) is
  research infrastructure and is out of this plan's scope too.
- **Copying prompt text** from either reference project (see §9).
- **Direct Lean 4 code** (hard constraint; specialist's lane).
- **Mattermost/Gitea deployment** as-is: the board/DM ops (G1/G3) are the
  in-project substitute; external infra may be revisited only if N2 shows the
  need, with license review.
- **Voice/canvas/companion-app surfaces** (the OpenClaw-style product layer):
  out of scope; `emthin`'s document desktop is the UI thesis of this project.
- **Anything in the adjacent root repos** (driftwm, ProofFlow, Engram,
  open-jev, engram-reference, prove2me_workspace) except where an item above
  explicitly names a seam (emthin's REWRITE_PLAN Part 2 ProofFlow adaptation
  stays its own work order).

---

## Appendix — item index, with delivery status

**Read this column before trusting any row.** "present in the tree" is not
"verified to work", and this project's own rule (§D) is that a limitation is
recorded when it is discovered rather than left to read as green.

| verdict | meaning |
|---|---|
| **DONE** | implemented, and the item's own acceptance criterion is met **and machine-checked** |
| **PARTIAL** | a real part is done and tested; the acceptance criterion is not fully met, or the enforcement the plan asked for is absent |
| **NOT DONE** | not implemented |
| **DEFERRED** | deliberately postponed, with the condition for revisiting stated |

Status as of **2026-10-05**, established by reading the tree and by running the
gates (`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo fmt --all -- --check`, `cargo test -p unfer_protocol` = 330, `-p unfer_ffi`
= 229, `-p unfer_edge --features audit` = 83, `dynamic-arctic` = 19 + 47) on
toolchain 1.97.1.

| ID | Improves (existing feature) | Size | Owner | Status | Evidence / what is missing |
|----|----------------------------|------|-------|--------|--------------------------|
| X1 | book.tex/ODE.tex single source | S | eng | **DONE** | `unfer/scripts/{sync-book,check-book-sync}` + mirrored in `timepiece/`; gates on both CIs; both pairs still byte-identical |
| X2 | README/AGENTS status headers | S | docs | **DONE** | Status blocks in all 7 READMEs; `unfer/scripts/check-status` gates *presence*, and **2026-10-06: the drift half landed** — `--recount REPO=LOG` recomputes a count from that repo's own suite log (refusing a log with a failed suite) and the CI test job tees its run and passes it in (`ci.yml:377`). Verified 2026-10-07: recount PASS on the live log (1769), and a planted mismatch/failed log are both refused (exit 1). Later the same day it caught real drift twice — rev 25's 1769 went stale at **1774** when X6's `ops_e2e` (a new test binary) and G3's four escalation tests landed, and `velysterm`'s 797 (a header that never matched its own 711 breakdown) was restated to the measured **850**, both from the suites' own fresh logs. `dynamic-arctic` was found stale at 62 and corrected to the measured 66; `README_arctic.md`'s 19 is library-only and now says so. A stale count is now a red build, not a silent claim. |
| X3 | AGENTS.md knowledge extraction | M | docs | **DONE** | Extracted (118K → 16K; `docs/CADABRA2_GOTCHAS.md`, `CONSENSUS_LEDGERS.md`, `QFM_STAR_TOPOLOGY.md` all linked). **2026-10-08: the row's last gap is closed** — the budget is machine-enforced (`unfer/scripts/verify-invariants`: "X3: AGENTS.md stays within the 120-line extraction budget", `wc -l ≤ 120`), and `AGENTS.md` sits at **111 lines**. The extraction's link requirement was already the gate's companion (every removed paragraph reachable from `docs/`). |
| X4 | australVM JIT isolation + test wrapper | M | eng | **DONE** | (b) `australVM/Makefile:47-72` `make test` re-execs under the right loader; used by CI and by X7. (a) JIT symbols are namespaced (`test/JitTest.ml:11`). The **falsified** `README.md` claim about a nonexistent `AGENTS.md §S36b` was removed 2026-10-05 and replaced with the live rule. |
| X5 | build-artifact hygiene | S | eng | **DONE** | `unfer/scripts/tree-size` gate is real and green; `_build`/`target`/`calltree.html`/`error.html` untracked. `qfm_text_runs/` (31 files) is tracked *by design* as pinned evidence. **2026-10-07: the retention half landed, so the row's gap is closed** — the policy is written *in the gate's header* (tracked = cited evidence; only `metrics.ndjson`/`train.log`/`train.toml` under `qfm_text_runs/` plus `logs/` are whitelisted by name) and enforced: a tracked file outside the allowlist fails `tree-size` (planted violation verified fail → untrack → clean). Amending retention is now a deliberate allowlist edit, not a default. |
| X6 | server ops surface (/healthz, runbook) | M | eng | **DONE** | `/healthz` + `/version` + runbooks on both servers. `dynamic-arctic` has real router-level integration tests (`mod ops_surface`). **2026-10-07: `unfer_edge`'s e2e probe landed** — `unfer_edge/tests/ops_e2e.rs` boots the real binary under Pingora on a free port and asserts over a raw socket: `GET /healthz` → 200 `{"status":"ok"}`, `GET /version` → 200 with `service`/`version` of this build *and* the effective listen address in the config provenance, `GET /metrics` → 200 JSON, `POST /healthz` → 405 (refused locally, not forwarded upstream). No new dependency (raw HTTP/1.1), readiness by polling (no fixed sleep), server stderr drained and reported on failure. 5/5 runs green at 0.10 s each. |
| X7 | cross-repo CI | M | eng | **DONE** | `unfer/.github/workflows/ci.yml` `cross-repo` job, named per-repo verdicts. 2026-10-05: siblings now come from an **explicit pin block** with the resolved SHA echoed per failure, and **emthin is covered** (`cargo check -p emthin`) instead of silently unbuilt. emthin's remote URL was *inferred* and its dep list is unrun, so it is best-effort and reports **SKIP ≠ PASS**. |
| X8 | DOC_INDEX backlinks + work-order pointers | S | docs/spec | **DONE** | `timepiece/CURRENT_WORK_ORDER.md` (one row per track); `DOC_INDEX.md` reports 201 backlinks; `doc_index.py --check` gates freshness. RandomMap honestly flags its missing order rather than inventing one. |
| X9 | attribution & license ledger | S | docs | **DONE** | All 7 repos carry a ledger pointer; root `ATTRIBUTION.md` has 41 rows (14 added in this pass: niri, foot, Bevy, ax, cloudflare-os, SafestOS, Azuro, qm, pattern+pattern-main, zentinel, claurst, cloud-hypervisor-build, the DeepSeek Engram paper, EWM). 2026-10-05: the **falsified** "a grep for ClawSwarm/Agensh returns zero files" claim was replaced with the real rule (§9.2 *requires* citation) and both projects moved into the borrowed table. 2026-10-07: **machine-checked** — `scripts/check-attribution` (mirrored at `unfer/scripts/`, wired into unfer CI's hygiene job) enforces policy #4 fail-closed with a two-tier allowlist, so file-wide exemptions for `derived from` can never swallow an attribution claim; fail→fix verified. Licences verified firsthand from downloaded siblings: zentinel Apache-2.0, claurst GPL-3.0, cloudflare-os Apache-2.0, deepseek-harness MIT, why3 LGPL-2.1, pattern **AGPL-3.0** (read from the clone then on `main`; the plan's MPL-2.0 note was resolved by the 2026-10-08 two-folder layout — see the dispositions below), emskin/EWM GPL-3.0; **ax, qm, pattern-main, cloud-hypervisor-build were recorded unfound at this point** — their rows carried re-verify notes; dispositions below. `test/` content licence stays an **open question, not a default** (owner decision, G-S1). **2026-10-07 disposition (re-verification sweep):** `ax`, `qm`, `cloud-hypervisor-build` were re-verified absent — API 404 *and* SSH not-found for each under the account, no local copy anywhere — and their rows now read **retired** instead of *re-verify*: the adaptation claims stay on the record, the licences stay doc-sourced/unverified, and `cloud-hypervisor-build`'s regeneration recipe survives at `australVM/cloud_hypervisor_vm/setup.sh`. **2026-10-08 disposition (owner layout — supersedes the branch-name reading):** `pattern-main` is a **folder**, and the workspace now carries both: `pattern/` was switched to branch `rewrite-v3` (tip 208bf142, the P11.19 bridge, above the `MPL-2.0 relicense` commit — **MPL-2.0** read from the working tree, with `pattern_memory` and `crates/pattern_unfer` present; history unshallowed; upstream tracking set) and a fresh single-branch clone `pattern-main/` was created on `main` (tip 50241994, **AGPL-3.0** read from its LICENSE, `pattern_memory` absent, the agent-platform crates present). Both licences were read from the working trees 2026-10-08; the plan's §P11.19 split (`../pattern` MPL / `../pattern-main` AGPL) now matches the layout, and both `pattern` ledger rows were rewritten accordingly. (2026-10-07: the clone had been re-verified in sync with its origin after a fetch, and rewrite-v3's licence, the pattern_memory crate, and the bridge commit were located and read on that branch as a durable remote ref.) **2026-10-08 disposition (owner restored two siblings):** `ax` and `qm` are now **cloned locally** — `ax/` = `github.com/google/ax` (**Apache-2.0**, LICENSE read firsthand), `qm/` = `github.com/yc-software/qm` (**MIT**, LICENSE read firsthand, replacing the doc-sourced guess) — so their four rows flip from *retired* back to **verified from the clone**. **2026-10-08 disposition (owner: recreate, NixOS-native):** `cloud-hypervisor-build`'s retirement is likewise reversed — the owner clarified the original was a Debian-package build tree on *another* PC (SpectrumOS + crossvm patches, **vhost included**), so the committed `australVM/cloud_hypervisor_vm/setup.sh` recipe regenerated it on this NixOS machine: pinned clones + checksum-pinned spectrum0 tarball, `vhost.patch` + `cloud-hypervisor.patch` applied (verified in the clones' `git log`), release binaries built and smoke-checked (`cloud-hypervisor v50.0-2-g2ffa2c05a`, `crosvm 0.1.0`), recipe shebangs switched to `#!/usr/bin/env bash` and minijail's hardcoded `/bin/echo` dropped for NixOS, **no .deb** produced (owner decision — the binaries + flake images are the NixOS artifacts); licences upgraded from doc-sourced to firsthand-read in the regenerated tree; `unfer/unfer_nixvm/flake.nix`'s dead `/media/...` path input was repointed **to the tracked recipe itself, `path:/home/leo/Projects/australVM/cloud_hypervisor_vm`** (owner note: the recipe is all NixOS needs — the two `configuration.nix` copies are byte-identical, so the composition evaluates from committed files only and the build tree is launch-only; flake.lock updated, both `*-with-unfer` outputs `nix eval`-verified to real `nixos-disk-image.drv`, and the recipe's own `vm-perf` evals standalone). The launch scripts were adapted past their shebangs on the same occasion: patched binaries resolved from the regenerated tree (env-overridable, with a fallback to `setup.sh`'s default output when run from the recipe dir), kernel/initrd from `/run/booted-system/{kernel,initrd}`, virtiofsd from PATH via a new recipe devShell (`nix develop` → virtiofsd 1.13.3 verified), `DISK=` for the with-unfer image — each missing dependency a named error before any `sudo`). |
| C1 | `unfer_agent` cursor/stream events | M | eng | **DONE** | `events_poll` done and tested (FFI, NDJSON, runner checkpoint, two-consumer exactly-once, gap detection). **2026-10-07: the push half exists** — the row's only gap. `events_stream {"on": bool}` writes each event over the same std.io channel as it happens: **off by default**, so S28's degenerate single-capability mode stays byte-identical for a client that never asks; a push is `{"cursor": N, "event": {...}}` **without `id`** (a response always has one), and the reply states that rule; the outbox is bounded at 64 with a cumulative `stream_dropped` counter — an eviction is counted, never hidden; and every event stays in the per-model log, so the **cursor** — not the push — carries exactly-once when a consumer switches modes. `main()` drains the outbox after each response, in cursor order. Registered in SESSION_OPS/AGENT_OPS, documented beside `events_poll` (docsync PASS, 59 ops). 6 tests (opt-in default, push shape, cursor continuity both directions, off-leaves-the-log, bounded-and-counted, refusal changes nothing) **plus an end-to-end smoke through the real binary**: no unsolicited line while off, a push after the response while on, `events_poll` since the pushed cursor returns `[]`, log intact after off. The spec asked for NDJSON ops; the C ABI keeps `uk_events_poll` alone — a push needs a channel that outlives the call, which the result channel is not. |
| C2 | bounded agent memory + retrieval | M | eng | **PARTIAL** | `unfer_protocol::memory` complete (cap 8000, BM25, redaction, deterministic retrieval) with `uk_memory_read/append`. **2026-10-07: the NDJSON half exists** — the row's "no NDJSON op" gap. `memory_append {model_id, worker?, text}` and `memory_read {model_id, query?, budget?}` reach the *same store* (the session's own `Memory`): same caps, budget defaulted to the cap and clamped at 4× exactly as the FFI clamps it, `text` required non-empty (a blank record is noise a named-field channel can refuse), `worker` defaults to `"kernel"`, the append reply adds `effect_kind: "mutate"`, and both ops push the FFI's own `KernelEvent`s into the model's cursored log — so with C1's `events_stream` on, an append arrives as a pushed line, one dialect across both transports. Registered in SESSION_OPS/AGENT_OPS, documented in PROTOCOL.md's Bounded-memory section (docsync PASS, 61 ops); 5 tests (round-trip with counts, empty refused, dead handle → UK-1004 both ways, `budget: 1` truncates *and says so*, push shape) plus a smoke through the real binary. **Still missing: the editor view** — and the model is ready for it (`MemoryRead::explain()` exists specifically "for an editor overlay"); no frontend renders it. Same shape as G5's UI half: recorded under ordered item 7 rather than faked here. |
| C3 | config precedence + init wizard | S | eng | **DONE** | Four-layer ladder implemented + tested on both servers (`unfer_edge/src/config.rs`, `dynamic-arctic/src/config.rs`, 19 tests); S22 hard-key invariant unchanged. |
| C4 | normalized message schema | S | eng | **DONE** | `unfer_protocol::ingest` used by both servers; cross-server parity test passes. **2026-10-05: two defects closed.** (a) D.1(a) — the four properties were written twice (CI test + fuzz target); they are now single-sourced in `unfer_protocol::ingest_props` and both callers invoke those predicates. (b) **P2 was vacuous and the caps were not enforced by parsing** — only in `IngestMessage::new` — so a POST to `/api/v1/ingest` with a 100 MB `text` allocated 100 MB, contradicting the module's own documentation. Fixed with `deserialize_with` caps on every field; 5 new tests give P2 teeth per field. |
| C5 | GrantSet role presets | M | eng | **DONE** | All five plan presets plus `orchestrator`; `GrantSet::is_subset_of` invariants hold; misuse refused at the loopback with an audit entry. |
| C6 | summarized agent output | S | eng | **DONE** | `uk_summarize` + `unfer_protocol::summarize` were already done and tested; **2026-10-06: the UI half landed, so the acceptance is met end to end.** `mathed_mini::kernel_bridge::overlay_text` routes the inline annotation and the results-panel footer through `uk_summarize` (channel `editor`) before display — through the *symbol*, so the handle is validated (S28), an unknown channel is refused, and the observe-kind `KernelEvent::Summarized` record is kept (S46). Text within the 320-char editor budget passes through verbatim, because summarizing is lossy by construction; text that does not fit is summarized, flagged `… lossy: the full output is in the cell region and the run log`, and a zero-selection says so with the character count instead of showing an empty overlay. The source is never modified: the full trace stays in `results()`, `run_log()` (verbatim outputs), the cell region and every export. Verified by three tests including an end-to-end `\kernel` run whose stream is long enough to exceed the budget. |
| C7 | local-model spec prefixes | S | eng | **DONE** | `model_spec.rs` prefixes + `WORKER_MODEL_NAME` worker override; loopback smoke test exists (a real-endpoint test is `#[ignore]`d behind `UNFER_SMOKE_BASE_URL`). |
| C8 | deployment runbooks | S | docs | **DONE** | `unfer/deploy/` (env tables, systemd units, Dockerfile) + both RUNBOOKs. Dockerfile remains **never built** (D.2). |
| G1 | shared context board (kernel ops) | M | eng | **DONE** | Board ops, kinds, caps, grep semantics, redaction, cursors — all done and tested. Durability closed at the type level 2026-10-05 (`board::BoardLog` wraps any H4 `DurableStore`, 8 tests including a literal restart and a secret-never-reaches-the-log check). **2026-10-07: the host wiring landed** — `kernel_client` takes the deliberate `unfer_ffi` dependency (Cargo.toml comment records the G1/A11 trade), and `unfer_agent`'s `AgentState` parses `UNFER_DURABLE_DIR` + `unfer_ffi::durable::Backend`, calls `open_store`, and builds the board with `BoardLog::open(Arc<store>)`; every mutation is flushed before the reply. Verified: `cargo test -p kernel_client --bin unfer_agent -- durab restart board` = **19 passed**, incl. `board_entries_survive_a_restart_when_durability_is_configured`, `a_restart_after_a_gate_run_reservation_still_replays`, and `without_durability_the_reply_makes_no_durability_claim` (the no-false-claim guard). Commit `9fbae47` (velysterm). Also: writes use `redact_secrets` rather than `sanitize_sensitive` (a reasoned deviation, documented). |
| G2 | cooperation loop protocol | M | eng | **DONE** | Five-step loop, own-word prompt, mid-turn delivery, retry/backoff, idle detector, deadline reminders — done and tested (26 + 31 tests); the plan's acceptance already exists as `coop_loop::three_worker_tests::three_workers_collide_resolve_and_the_merge_passes_the_gate` (3 workers, collision → resolve → verify → merge). **2026-10-07: session restore landed**, which was the row's only remaining gap. New `kernel_client::session`: one versioned, atomically-written `sessions.json` per host (sharing `checkpoint::atomic_write`), where an unreadable or wrong-version record is *refused* rather than guessed at — `spawn` returns `SpawnRefusal::CorruptSession` (UK-1006, the kernel's own "malformed and cannot be replayed" session-log code) instead of silently starting the worker over. `OrgHost::with_session_dir` is the caller `mark_gathered` always said was missing: it persists after every `step`/`tick`/`expire_claims` and restores on `spawn` — step, gathered cursor, seen delivery ids, delivered nudges, elapsed minutes — and replays the worker's scopes through `Coop::claim`, so the ledger entry and its `CLAIM` board entry are rebuilt together (a ledger-only restore would be a claim no board reader can see). A restored step past Claim whose scopes cannot be re-seated rewinds to Gather rather than act on a claim nobody records; a failed write surfaces through `session_error()` and a non-zero exit in the `org_runner` binary instead of passing as durable. Deliberately out of the record: the board (G1's `BoardLog`), queued deliveries, per-worker memory — a second copy of durable state is a second thing to keep honest. Verified end-to-end through the binary: a second life resumes past Claim (report shows `evidence_refusals: 2` where a fresh start shows 0) and a corrupted record refuses both spawns with UK-1006. 17 new tests (9 in `session`, 8 restore paths in `org_runner`); kernel_client lib 155 passed (was 138), fmt/clippy clean. |
| G3 | claims + DM collision resolution | M | eng | **DONE** | Scope overlap, `agent_dm`, expiry, 3-way collision reporting — done and tested. **2026-10-07: the explicit escalation path landed.** New `BoardKind::Escalate` (`ESCALATE`, `Mutate` — it asks a human to act) plus the `agent_escalate` op: the reason is the entry's text, the current holders its `detail` (evidence travels with the request), a loop-exhaustion escalation says "no live claim holders" rather than implying a collision, and repeats are deduped per worker+scope exactly as nudges are so retries cannot flood the lane. The `agent_claim` overlap reply now carries `escalate_with: "agent_escalate"` — the path is in the message, not only in a prompt — and the backtrack-exhausted error names the op. Registered in SESSION_OPS/AGENT_OPS, documented in `PROTOCOL.md` (docsync PASS, 56 ops). Tests: 4 in `coop.rs`, 3 in `unfer_agent` (full lane path incl. dedup, reason-required refusal, no-holders case); effect-kind property now pins exactly three mutating kinds. |
| G4 | verify-before-merge evidence | M | eng | **DONE** | Evidence validation incl. freshness-older-than-patch and fail-closed on a missing base; merge withheld on non-pass/stale. Note: the refusal lives in `patch_submit`/`OrgHost`, **not** in `uk_gate_approve` — a defensible layering, but not the plan's literal wording. |
| G5 | write-time conflict detection | M | eng | **PARTIAL** | `ConflictTracker`/`DeltaPacket`/`ConflictRecord` implemented and tested against the real Loro CRDT, applied-then-reported. **2026-10-07: the conflict now reaches the board.** Two agent ops: `sync_edit` records a document's own footprint; `sync_import` classifies a peer's packet against it (new `ConflictTracker::classify_from`, because one agent serves many peers and the footprint is the document's, not one pair's) and on overlap writes a `FAIL` entry — text `"<doc>: <explanation>"`, detail the structured record with both revisions and the specific overlaps — so **G2 step 5 finally has an input**. Registered in SESSION_OPS/AGENT_OPS, documented in `PROTOCOL.md` (docsync PASS, 58 ops). Honest boundaries kept: a clean import writes nothing (an entry per import would bury the conflicts), a retry returns the original cursor instead of a second entry, empty or reversed block declarations are refused (UK-1001) rather than read as "clean", and the op never applies the delta. Tests: 9 in `unfer_agent` + 1 in `mathed_core`; fmt/clippy clean. **Still missing: the UI consumer** — and the finding that scopes it: *no production code imports document deltas anywhere* (`export_delta`/`import_delta`/`classify` are test-only; the presence transport reserves a `b'D'` delta frame that nothing sends; C13's `CollabSession` was never built). An overlay with no arriving deltas would be dead UI, so the remainder is C13's stages 2–3 plus a conflict view — recorded under ordered item 7, not closable inside G5. |
| G6 | typed board for plans corpus | S | docs | **DONE** | `CURRENT_WORK_ORDER.md` per-track board citations + `scripts/board_rollup.py` (idempotent, FAIL-first ordering), already run once into `CONSOLIDATED_PLAN.md`. **2026-10-07: the two gaps are closed.** (a) Tests: `timepiece/scripts/test_board_rollup.py` — 15 stdlib-only tests covering the script's three claims (append-only prefix survival, the double-rollup refusal, `--force` keeping the old block, `--dry-run` writing nothing) plus parsing, month buckets, marker-vs-heading detection, FAIL-first rendering and the OBSERVED exclusion. (b) Gate: a `board-rollup` CI job runs the tests, smokes `--list-months` against the real `BOARD.ndjson`, and asserts the byte-identical invariant after `--dry-run` against a probe plan (the real plan already carries the month, and the script's refusal precedes `--dry-run` by design). Verified locally: 15/15, smoke exit 0, workflow YAML parses. `BOARD.ndjson` stays labelled a seed corpus (D.3). |
| G7 | role hand-off observability | S | eng | **DONE** | `agent_handoff` request/accept; only an accept confers a role; a structural test asserts no grant/token/capability appears in a hand-off message. |
| G8 | `test/` org-harness page | S | docs | **DONE** | `test/software/org-harness.md` + SUMMARY entry, both citations present, scaling numbers explicitly stated as unmeasured. |
| G9 | spawn staggering + budget nudges | S | eng | **DONE** | Staggering at the S25 chokepoint (UK-4601, audited, and deliberately a *second* limiter beside the day meter); `nudge.rs` with urgency ordering and dedup. 2026-10-05: the two pacing setters were **dead code with a false doc claim** ("for the console") — there is no `uk_spawn_pace_set` entry, unlike every sibling reset function. Now `#[cfg(test)]` with the missing console seam stated rather than implied. |
| N1 | org_runner multi-worker host | L | eng | **DONE** | `kernel_client/src/{org_runner,coop_loop}.rs` + binary, 31 tests, convergence without an orchestrator. Not in velysterm's README/AGENTS. |
| N2 | reproduction-benchmark harness | L | eng | **DONE** | Harness + report + guards (refuse to fake a scaling number). **2026-10-07 (D.4): coverage 100% (12/12)** — the two `Unavailable` families are provisioned: `mass-gap` runs `unfer/scripts/regen_mass_gap.sh` (the project's own gate, which reports its own UNAVAILABLE steps rather than laundering them into a pass) and `module-reconstruction` runs `unfer/tools/module_reconstruct.sh` (pinned `.cell` blueprint → golden manifest → `tools/module_builder run`), path-anchored so `cargo test`'s package-root cwd cannot turn a gate into a "no such file" failure. Full sweep, three worker counts: **measured 100%, 12 passed, 0 failed** (`docs/BENCH.md`). The two guards are untouched. The honest limit is stated where the number is: the gates receive `UNFER_BENCH_WORKERS` but no task bundle has agent-level parallelism to spend it on, so equal pass rates across N are the expected result — the report has the right to state a 1-vs-N number now, and the number is "no difference measured". |
| N3 | multi-node federation demo | — | — | **DEFERRED** | Conditional on N2 showing scaling value. |

### Not in this plan, but found and fixed while executing it

Several defects were outside the item list and are recorded here because all
had been sitting in a green-looking tree:

1. **unfer's own CI gate was red.** `cargo clippy --workspace --all-targets -- -D
   warnings` (ci.yml:254) and `cargo fmt --all -- --check` (ci.yml:253) both
   failed on `unfer_protocol`, `fock_sirk` and `unfer_edge` — 3 missing
   `#[cfg(test)]` on test modules (leaking test code into release builds), plus
   ~15 lints. Also the root cause of the "3 warnings" G-D2 recorded against
   `dynamic-arctic`: they were never in that repo at all, but in
   `unfer_protocol`. Both gates are green as of 2026-10-05.
2. **The ingest cap hole** (C4 above), which no item of §5–§8 was looking for.
3. **velysterm's own clippy gate was red.** `rust.yml` runs
   `cargo clippy --workspace --all-features --all-targets -- -D warnings`, and
   the committed tree did not pass it — 11 lints across `kernel_client`
   (unused imports, `unnecessary_sort_by`, three `collapsible_if`,
   `field_reassign_with_default`, `clone_on_copy`, `needless_borrow`). The one
   worth keeping is the last: `if !((a - b).abs() <= 1e-9)` refused a calibrator
   install through a negated f64 comparison, and since `<=` on NaN is *false*,
   that refusal silently depended on the negation nobody reads. It is now
   `!delta.is_finite() || delta > 1e-9`. Green as of 2026-10-06 for
   `kernel_client` and `mathed_mini`; the workspace-wide job still cannot   be run to completion on this machine, because `mathed` → `bevy_vello` →
   `libudev-sys` needs `pkg-config`, which is absent. So "the workspace gate is
   green" is **unrun**, not passing — same discipline as D.1/D.5 above.
4. **The "single pinned toolchain" claim was false in two ways; the whole
   project now runs 1.99.0 (2026-10-08).** (a) Both `velysterm/flake.nix` and
   `unfer/flake.nix` set `RUSTUP_TOOLCHAIN = "stable"`, which *overrides*
   `rust-toolchain.toml` (the env var beats the toolchain file): `nix develop`
   silently ran stable (1.98.1) while CI and bare builds ran the pinned
   1.97.1 — two compilers, two clippy lint sets, and a shell-only clippy
   failure (`chunks_exact_to_as_chunks` in `mathed_core/transform.rs`) that CI
   never saw. Both attrs are deleted; the toolchain file is the single source
   of truth, and that lint is fixed so a future stable bump stays green.
   (b) The pins themselves were not single: unfer/velysterm/dynamic-arctic/
   australVM said 1.97.1, emthin said 1.92.0, and `timepiece/unfer_contracts`
   (the Rust subtree of a Lean repo) had *no* pin at all — it inherited
   rustup's default, a nightly. All seven pin sites (including
   `australVM/safestos/cranelift`), the CI `toolchain:` inputs, the
   `RUSTUP_TOOLCHAIN` fallbacks in `tools/module_reconstruct.sh` and
   `scripts/regen_mass_gap.sh`, and every current-state doc claim now say
   **1.99.0** with `components = ["clippy", "rustfmt"]` declared; the global
   rustup default is 1.99.0 as well. Verified on 1.99.0: rustfmt clean in all
   repos (zero style drift), `clippy -D warnings` green in every workspace
   (default *and* feature-gated for unfer), plus the three compatibility
   points that could not be settled by argument: `nanoda_lib` (git dep of
   `prob_kernel`) compiles (`cargo check -p prob_kernel --locked` → 0);
   Charon/Aeneas still applies to the Rust source under the 1.99.0 pin — four
   extraction runs (ambient 1.99.0, forced 1.97.1, forced 1.99.0, repeat)
   yield a **byte-identical `SirkCoreModel.lean`**, with only the `.llbc`
   intermediate's identifier ordering varying run-to-run *regardless* of
   toolchain (proved by A≠C with identical env); and Kani keeps
   `nightly-2026-08-21` installed because kani-0.68.0 pins exactly that
   toolchain — the one nightly that must not be cleaned. `nightly` (alias)
   stays for the cargo-fuzz workflow and the pattern/loop-desktop repos.
5. **`mathed_mini`'s reproducibility test asserted wall-clock equality.**
   `run_all_record_executes_every_block_and_is_stable` compared two
   `run_all_record` outputs byte-for-byte *including* `timing_ms` — a real
   measurement (`kernel_bridge.rs` `pub timing_ms: u64`) — so it only held
   while the box was idle: the 1.99.0 verification runs under parallel build
   load recorded 60/70 ms in run 1 against 20/30 ms in run 2 for the identical
   trace (2026-10-08). The assertion now parses both records and compares them
   with every `timing_ms` leaf removed; content-derived fields (hashes,
   offsets, bodies) must still match exactly. Passes under load after the fix.

S ≈ ≤1 day, M ≈ 2–5 days, L ≈ 1–2 weeks. ★ marks the priority chain
C1 → G1 → G3 → G4 → G2 (the smallest path to concurrent agent work on this
corpus under the project's own trust model).

### The remaining work, ordered

1. ~~**G1 host wiring** — expose the durable board to `unfer_agent`.~~
   **Done 2026-10-07** — `kernel_client` takes `unfer_ffi`, `unfer_agent`
   opens the store from `UNFER_DURABLE_DIR` and wraps it in `BoardLog`
   (velysterm `9fbae47`); 19 board/durability/restart tests green,
   including the literal restart and the no-durability-claim guard.
   G2's restore step is now unblocked.
2. ~~**C4's UI half / D.6** — route the `mathed` result channel through
   `uk_summarize` via `KernelBridge`, then close C6.~~ **Done 2026-10-06** —
   see C6 and D.6 below; both overlays summarize, the raw trace stays
   retrievable, and the observe-kind record is asserted from the event log.
3. ~~**D.4** — provision the two `Unavailable` N2 task families so coverage is
   100%; until then Target 4b stays unmet and no scaling number may be stated.~~
   **Done 2026-10-07** — both families provisioned and a full sweep at three
   worker counts measures 12/12 (see the N2 row and `velysterm/docs/BENCH.md`).
   The guards stayed; Target 4b's report now exists, and its number is
   "no difference measured", stated with the reason.
4. ~~**M7's public tables** (from `PLAN_MASS_GAP.md`) — `test/numerics/qym-mass-gap.md`
   and `test/proofs/proof-plans.md` do not yet carry the M1 finding~~ — **already
   done, verified 2026-10-07**: `qym-mass-gap.md` carries the full campaign
   section ("The 2026-10-06 campaign: no coupling certifies a positive gap",
   the 20 fixtures, the 87× window, the sector correction) and its T11/T12
   status row states "none certifies a positive gap"; `proof-plans.md` carries
   it as open item 6 of the honest ledger. Both pass the site's own gates:
   `python3 scripts/check_site.py` (exit 0) and `unfer/scripts/check-non-claims`
   (PASS). The stale "not yet updated" notes in `PLAN_MASS_GAP.md` §M7 were
   corrected in the same pass.
5. ~~**X2's drift half** — recompute test counts in `check-status` instead of
   documenting that it will not.~~ **Done 2026-10-06** (`--recount`, CI-wired);
   verified 2026-10-07 against a live log — it caught the stale 1764 block and
   passed after the recount to 1769. The same day it caught that 1769 again
   (1774 after X6 + G3) and `velysterm`'s stale 797 (measured 850) — two live
   catches, not just the planted ones.
6. ~~**X3's line budget, X5's retention policy, M4's bijection check, M5's Kani
   harness** — each a small gate that is greppable or documented today but not
   enforced.~~ **All four closed 2026-10-07**: X3's budget *is* gated
   (`scripts/verify-invariants:227` requires ≤120 lines; `AGENTS.md` is at
   111). X5 now has a written policy **enforced by the gate itself** —
   `scripts/tree-size` documents retention (tracked = cited evidence; only
   `metrics.ndjson`/`train.log`/`train.toml` under `qfm_text_runs/`, `logs/`
   whitelisted by name) and fails on a tracked file outside that allowlist
   (verified fail → untrack → clean on a planted violation). M4's bijection
   check passes (`check-mass-gap-claims`: 10 rows ↔ 10 tests on both sides).
   M5's harness exists as 15 `kani::proof` functions in
   `fock_sirk/src/mass_gap_kani.rs` (12 proven, 3 recorded CBMC timeouts —
   the known permanent 3/15).
7. **The rest of the PARTIAL rows above**, each with its own line item.
   1. ~~**G2's session restore**~~ — **Done 2026-10-07**: `kernel_client::session`
      + `OrgHost::with_session_dir` persist/restore the loop and replay claims;
      a corrupt record refuses the spawn (UK-1006) rather than starting fresh.
      See the G2 row.
   2. ~~**G5's board half**~~ — **Done 2026-10-07**: `sync_edit`/`sync_import`
      route the `ConflictRecord` to a `FAIL` entry so G2 step 5 can read it.
      **G5's UI half remains**, and its blocker is now written down: no
      production code imports document deltas at all, so the work is C13's
      stages 2–3 (`CollabSession`, the reserved `b'D'` delta frame) plus a
      conflict view in one frontend — a C13 line item, not a G5 patch.
   3. ~~**C1's push half**~~ — **Done 2026-10-07**: `events_stream` over the
      same channel, cursor-backed exactly-once; see the C1 row.
   4. ~~**C2's NDJSON half**~~ — **Done 2026-10-07**: `memory_append` /
      `memory_read` on the agent channel; see the C2 row.
   5. ~~**X9's machine check**~~ — **Done 2026-10-07**: `scripts/check-attribution`
      (mirrored into `unfer/scripts/`, run by unfer's CI hygiene job) enforces
      policy #4 fail-closed; 14 ledger rows added, 10 sibling repos downloaded
      and their licences verified firsthand. The `test/` licence inside X9
      stays an open owner question (G-S1) — recorded in `ATTRIBUTION.md`, not
      defaulted; see the X9 row.
      Remaining below: the two **frontend views** — G5's conflict
      view (blocked on C13's delta transport) and C2's memory view
      (`MemoryRead::explain()` is written for it; no frontend renders it).
      Both are frontend work behind features that do not exist yet, so they
      are recorded rather than half-built.

### Related plans, executed 2026-10-06

Two sibling plans were executed in the same pass; their status lives in their own
documents, and both feed rows above:

- **`PLAN_ANALYTIC_DECISION_ENGINE.md`** — D1–D15 landed as `unfer/decide_core`
  (91 tests) + `unfer_protocol::decide` + a pinned calibration report. The
  headline is also negative and is enforced in code: **no candidate met the R1–R5
  contract**, so the engine emits *scores*, and `calibrated: true` is not licensed.
  D6 and D13 are PARTIAL by their own acceptance. See that file and
  `unfer/docs/DECIDE_SPEC.md` §0.
- **`unfer/PLAN_MASS_GAP.md`** — M1–M9 executed. The headline is that the certified
  window is **intrinsically loose in the current construction**, and this is a design
  question rather than a tuning one: the one-step recurrence's step norms
  `τ_k = ‖(H − z_k I)u_k‖` grow without bound (`6.4e+1 → 6.0e+3` for `m = 4…48`) and
  `certified_width` inherits them, so `‖r‖` rises (→ 5.70 at `m = 48`) while `θ₀`
  converges (→ ≈ −1.66). The two obvious escapes were measured and **both fail**:
  fixing the window width leaves `resid₀` climbing `2.73 → 4.82`, and moving the
  window off the spectrum lowers it only where rank collapses to 3–9 and `θ₀`
  freezes. **But the method is not missing an exponential rate for that reason**:
  with harmonic shifts the inverse-free chain is a cleared-denominator form of the
  resolvent chain (scalars removed by the unit-norm frame), and the start vector is
  free, so `papers/Hashimoto.md`'s exponential rate is reachable. **The blocker is a
  quantity mismatch** — that rate is for the *φ-function approximation error*, not
  for `|E₁ − E₀|`, and the gap is `0.091` inside a `−1.7 … +337` spectrum (~3.7e3
  ratio). So the programme needs its own eigenvalue-error bound. Consequence of the
  measurement: no coupling certifies a positive gap; window ~87× the gap at `g = 1`.
  **Three explanations of mine were wrong and are retracted there:** (i)
  backward error (measures `6.0e-11`…`7.0e-08` against a width of ~3–7);
  (ii) "the extraction is unvalidated", from `θ₀`'s shift-invariance — which is the
  *expected* signature of a converged extremal value on a shift-independent
  subspace — plus a `θ₀ > E₃` comparison that spanned two different Hilbert spaces;
  (iii) "the exponential rate cannot transfer to an inverse-free construction", which
  ignored that the polynomial chain with harmonic shifts *is* the resolvent chain in
  cleared-denominator form. M4.6 option (b) is rewritten accordingly: the open
  question is a gap-specific eigenvalue bound, not resolvent application. That changes
  §M4.3's premise, and three new Lean work orders (§M4.6–§M4.8) are filed in
  `timepiece/CONSOLIDATED_PLAN.md`.
  M2/M4/M5/M7 are PARTIAL; the reasons are under each item.

---

# Part IV — 2026-10-04 update: the documentation pass, and the path to the project's goals

This part records the documentation/pedagogy pass over `test/` and
`timepiece/Book/`, the Cadabra-module documentation in `unfer/`, and then —
the reason those documents matter — a goal-oriented statement of **what is
still needed** for the project to achieve what it is for.

## A. What the documentation pass changed

**`test/` (the public face).** Reviewed against the current state of
`../timepiece` and `../unfer`; the pages were accurate but pre-dated the
mass-gap formalization wave. Improved:

- `numerics/qym-mass-gap.md` — two new pedagogical sections. *How the band
  turns a number into a theorem* decomposes the certified width
  `δ = ‖r‖ + c(n)u‖Ĝ‖ + h_O` term by term (where the level sits / what the
  arithmetic did / what reading the number did), states T6 in the right
  direction (computed minus widths is a *lower* bound; bands can fail to
  close but never lie), and explains why approximation is therefore
  admissible in a proof at all. *The proof status (2026-10-04)* is a status
  table of the T1–T12 chain — the proof has **started**: finite-precision
  layers, certified-gap theorem, stopping rule, reader and gap table are
  closed; nanoda re-verification is green; Aeneas identity proofs,
  per-coupling rows, the uniform form gap and the continuum leg remain.
- `numerics/certification.md` — a new *trust boundary* section (what is
  proved / the one trusted component / what is never trusted / what is
  independently re-checked), and the Lean-side section now carries the
  full certified-gap chain and the two remaining analytic inputs.
- `proofs/proof-plans.md` — the honest ledger refreshed: the one-particle
  form gap is now stated in its precise form (uniform in the derivative
  parameter `η`, with the truth-value-probe discipline first), and the
  continuum leg is listed explicitly as open item 5.

**`timepiece/Book/` (the pedagogical layer).** Prose-only edits in
`SirkReliability.lean` — no code, no `#check` lines, no cross-references —
three insertions: the *division of responsibility* behind the three-term
width (why it is a bound, not a tolerance); a clarification the numerics
forced on the program (the `1.932` run is the historical lattice-era
instantiation — the object of record is the gauge-fixed nested-Fock
operator, whose gap is its own ≈ 0.09 at g = 1, and the two are
instantiations of one theorem, not two measurements of one number); and the
external-check paragraph (nanoda + Aeneas) with the 2026-10-04 status and
pointers to the work order and planner plan.

**`unfer/` Cadabra modules.** All 15 `docs/*.cdb` modules received a
comment-only **MODULE DOCUMENTATION** header: purpose, role in the program,
certified content (which CHECKs, consumed by which tests), and — the point
of this pass — **improvement notes tied to the project's goals** (e.g. the
`qg_densitized_hamiltonian.cdb` header records the known audit finding that
its `diff` handle is a hand-typed tautology and what fixing it requires;
`yang_mills_hamiltonian.cdb` records its role as the mass-gap Hamiltonian of
record and the reflection-invariance CHECK that would strengthen the sector
split). The extraction pipeline reads only the run's *stdout markers*, so
comment headers are inert to it. Two stale line-number references
(`…cdb` lines 72/77 in `AGENTS.md` and `VERIFY_CDB_TRUNCATION_AUDIT.md`)
were converted to content-based references so future header edits cannot
rot them.

## B. The goals of the project, restated as checkable targets

The seven folders exist for one program with four goals. Each goal is stated
here as *checkable targets*, because a goal nobody can fail is not a goal.

**Goal 1 — the physics: foundations of the nested-Fock quantization
program.** Prove spectral properties of the gauge-fixed Hamiltonians of
record (QYM, QED, QG, NS, SM) on the nested Fock space, with the numerics
serving the proof rather than substituting for it.

- *Target 1a*: certified gap certificates for every coupling row of the
  gap table, emitted from the recorded pipeline and re-verified by nanoda
  (partially met — `g = 2, m = 4` row; the per-`g` campaign is planner item
  M1).
- *Target 1b*: the one-particle form gap — uniform in `η` for the
  non-abelian QYM fiber — **proved or refuted**. This is the single
  outstanding analytic input; the programme's honest record (the abelian
  case was settled negatively) demands a truth-value probe before any proof
  attempt.
- *Target 1c*: the continuum leg — gap-preserving norm-resolvent convergence
  of the truncation family, or an a-priori continuum gap bound. Until then
  every certified number is a *truncated* statement, and every document says
  so.
- *Target 1d*: the NS mainstream leg — ESA of `N_NS` and `N_L` discharged
  (or the orbit route taken), closing the sector where the mainstream
  generator is not bounded below.

**Goal 2 — trust: every number that enters a claim is proof-carrying.**
Bands on the numerics side, Lean theorems in the middle, an independent
checker at the end, and one command that regenerates the whole chain.

- *Target 2a*: `scripts/regen_mass_gap.sh` (planner item M2) reproduces the
  pinned bundle byte-identically and fails at the right step on a planted
  source change.
- *Target 2b*: the f64 paths carry the Kani/proptest complement (M5), and
  the spec-parity gate (M3) keeps the proof-facing contract single-sourced.
- *Target 2c*: the same discipline extends beyond the mass gap: the H1
  checklist-as-gate (PLAN_HARNESS) and the golden release gate cover the
  kernel surface.

**Goal 3 — the systems: a working digital-mathematics environment.** The
kernel, VM, editor, and compositor as one usable stack.

- *Target 3a*: cross-repo CI (X7) enforces the green-workspace rule that the
  path-dep topology makes critical.
- *Target 3b*: the agent interface becomes multi-worker-ready (the C1 → G1
  → G3 → G4 chain): cursor-based events, the shared context board, claims
  with collision resolution, verify-before-merge — all under the existing
  grant/audit model.
- *Target 3c*: the servers get the X6 ops surface and runbooks; `emthin`'s
  own backlog (launcher, dormant-figure UX) proceeds on its REWRITE_PLAN.

**Goal 4 — the organization: scale the work itself.** The corpus is too
large for one agent at a time; the Agensh/ClawSwarm adaptations (Part 5–7)
are how the project scales its *own* production, and the reproduction
benchmark (N2) is how that scaling claim gets measured instead of asserted.

- *Target 4a*: the 3-worker integration test (claim collision → verify →
  merge) green.
- *Target 4b*: N2 benchmark reports 1 vs 4 workers on the mass-gap
  regeneration bundle as its first task family.
- *Target 4c*: `test/` carries the org-harness page (G8) with citations and
  honest comparisons.

## C. What is needed now — the ordered path

Ordered by leverage over the goals above; each item names its owner lane.

1. **Truth-value probe of the uniform-in-`η` one-particle form gap**
   (Goal 1b; LLM-Lean4-specialist, work order in `CONSOLIDATED_PLAN.md`
   2026-10-04 §M4.4). Everything about how the QYM story is *told* depends
   on whether this is true.
2. **Per-coupling certificate emission + gauge-fixed T6/T10 instantiation**
   (Targets 1a, 2a; planner M1/M2 + specialist §M4.1/§M4.3). The gap table
   is starved of rows without it.
3. **The continuum leg** (Target 1c; specialist §M4.5). The single missing
   leg to any non-truncated claim; start from `ChapterSpectralGapStability`.
4. **Regeneration + spec-parity + f64-complement gates** (Targets 2a–2b;
   planner M2/M3/M5). Trust infrastructure is cheaper before the proof
   traffic grows.
5. **NS ESA hypotheses** (Target 1d; specialist — the standing `C` items).
6. **Cross-repo CI and status-doc sync** (Target 3a; engineering X7/X2).
7. **The agent-organization seam** (Targets 3b/4a; engineering C1→G1→G3→G4
   →G2, then N1).
8. **The reproduction benchmark** (Target 4b; engineering N2).
9. **Continuous documentation upkeep** (Goal communication; docs) — every
   new pinned fixture lands with its doc row; every boundary sentence stays
   verbatim (M7/M9).

Two disciplines carry through the whole list, and the documentation pass
above exists to make them teachable: **the honest boundary is part of the
result** (a truncated claim stated as truncated is progress; an overstated
claim is a defect), and **the improvement principle** (before anything new:
name the existing feature it improves).

> Lean-shaped work discovered anywhere on this path is filed as a work order
> in `timepiece/CONSOLIDATED_PLAN.md` and executed by the LLM-Lean4-
> specialist; this plan writes no Lean code.

## D. Verification limitations of the delivered work — and their planned solutions (2026-10-05)

The honesty boundary applies to the plan itself. Six things delivered so far
are **not** verified in this environment. Each is recorded here with the
evidence for its status and the planned solution, so that no downstream
reader mistakes "present in the tree" for "proven to work". Item letters
refer to the sections above.

**D.1 — The fuzz target has never run here (C4's `unfer_protocol` fuzz
target).** *Limitation*: this environment has no `libfuzzer-sys` available
offline, no `cargo-fuzz`, and no nightly toolchain, so a libFuzzer target
cannot even be compiled. *What holds instead*: the target is excluded from
the workspace, so it can never break `cargo test`, and the same four
properties it asserts are asserted by a 20k-message property test that does
run in CI. *Planned solution*: (a) single-source the four properties — the
fuzz target and the property test assert predicates from one shared module,
so they cannot drift; (b) a nightly CI job (network-enabled, free to fetch
`cargo-fuzz`) runs the target for a bounded iteration count against the
seed corpus and archives any crash artifacts; (c) until that job exists,
every status line says "property test in CI; fuzz target unrun" — no
document cites fuzzing as evidence.

**D.2 — The Dockerfile has never been built (C8).** *Limitation*: no
container runtime is available here, so the Dockerfile is *reviewed
source*, not a built artifact. Its `HEALTHCHECK` is documented as weaker
than it looks: the probe can pass while the service is not actually doing
useful work, so a green healthcheck is not evidence of service health.
*Planned solution*: a CI job builds the image and boots `unfer_edge` +
`dynamic-arctic` from the runbook alone (which is C8's own acceptance
criterion), then exercises one real request through the X6 ops surface and
fails the job if the server cannot answer it; the in-image `HEALTHCHECK` is
then replaced with that same request-level probe, so the in-image check and
the CI check mean the same thing.

**D.3 — `BOARD.ndjson` is a seed corpus, not captured evidence (G1/G6).**
*Limitation*: its entries were transcribed from existing plan entries to
give the G1 board format something to be tested and rendered against. A
seed that reads like captured evidence is worse than none, so both the
board file and the file that cites it label it as a seed corpus.
*Planned solution*: the label stays until live runs exist; the first real
runs of the org harness (N1/G2) capture genuine `FACT`/`FAIL`/`CLAIM`
entries into a separate log with run ids, and the transcription is
*replaced*, not merged, once real entries exist. Nothing may cite seed
entries as observed behavior.**D.4 — N2 cannot yet support a scaling claim.** *Resolved 2026-10-07: both
`Unavailable` task families are provisioned and a full sweep measures 12/12.*
The original limitation was this: the harness reported *measured 50% (6/12
trials)*: one task family ran and passed at every worker count, while the other
two task families were declared `Unavailable` with their reasons recorded in
the report. There was therefore no worker-count comparison to make, and the
code refused to fake one — `pass_rate()` returns `None` rather than `0.0` at
zero coverage, and `Report::comparable` refuses to compare runs with differing
coverage. Both guards are deliberate and stay as they are. Target 4b now has
its report (three worker counts, equal coverage, all passing — see the N2 row
and `velysterm/docs/BENCH.md`), and no scaling claim is repeated anywhere in
the corpus: the gates do not spend `UNFER_BENCH_WORKERS` on agent-level
parallelism, so the earned number is "no difference measured", stated with
its reason where the number appears.

**D.5 — timepiece's `verify-invariants` was not run for the G6 commit.**
*Limitation*: the gate triggers a full Mathlib build, which this environment
does not run. What is verified instead: **zero** `.lean` files were touched
in any repo by that work (confirmed: 0 changed `.lean` paths), so the gate
is genuinely *unrun*, not *passing* — and the status record must say so
rather than letting silence read as green. *Planned solution*: run
`verify-invariants` once in CI (scheduled job, or on the next
Lean-touching change) and record the result in the status block; wire the
gate behind a `.lean`-path filter so that "unrun" can only ever mean
"nothing Lean-shaped changed", and documentation-only commits like G6 stop
being a special case.

**D.6 — C6 has no UI wired.** *Limitation*: `uk_summarize` exists and is
tested on the kernel side, but no editor overlay consumes it yet, so C6's
acceptance criterion ("overlay shows only summarized text; full trace
retrievable") is unmet at the UI half. *Planned solution*: wire the
`mathed`/`mathed_mini` result channel through `KernelBridge` to
`uk_summarize` before display (the seam C6 names), keeping S46 semantics —
summarization is an `observe`-kind effect and the raw output stays in the
session log — and close C6 only when the acceptance criterion is met end
to end.

Until each solution lands, the corresponding status line reads **unrun** /
**reviewed source only** / **seed corpus** / **measured 50% (6/12)** as
above. A limitation is recorded at the same moment it is discovered; the
planned solution is recorded at the same moment the limitation is.

### D.7 — Status of D.1–D.6 after the 2026-10-05 execution pass

| § | was | now |
|---|------|-----|
| **D.1a** — single-source the four properties | unmet | **met.** `unfer_protocol::ingest_props` holds them; `ingest::property::arbitrary_json_never_panics` and `fuzz/fuzz_targets/ingest_message.rs` both call those predicates and assert nothing themselves. |
| **D.1a, unexpectedly** — P2 was vacuous | *not known to be broken* | **found and fixed.** The caps lived only in `IngestMessage::new`; `IngestMessage` derived `Deserialize`, so an over-long `text` on `/api/v1/ingest` was allocated in full. Both the 20k property test and the fuzzer had missed it because neither ever generated an over-long field. Caps are now applied on the deserialize path and 5 tests give P2 teeth per field. |
| **D.1b** — a nightly job that runs the fuzz target | absent | **met as a workflow** (`unfer/.github/workflows/fuzz.yml`, bounded run, crash artifacts archived, 30-day retention). **Unrun until its first scheduled run completes** — the status line stays "property test in CI; fuzz target unrun" until then. |
| **D.2** — build the image and boot the servers in CI | absent | **still absent.** The Dockerfile remains reviewed source only. |
| **D.3** — replace the seed `BOARD.ndjson` | labelled seed | **still a seed**, and still labelled as one in both places that cite it. Replacing it needs a real N1/G2 run. |
| **D.4** — make the two `Unavailable` task families runnable | 6/12 trials | **resolved 2026-10-07: 12/12.** `mass-gap` runs `regen_mass_gap.sh`, `module-reconstruction` runs the new `tools/module_reconstruct.sh` (pinned `.cell` → golden manifest → `module_builder run`); full sweep at 1/2/4 workers measures 100% coverage, 12/12 passed. Both guards untouched; the scaling number is "no difference measured" with the worker-insensitivity stated next to it. |
| **D.5** — `verify-invariants` in CI | unrun | **met.** `timepiece/.github/workflows/ci.yml` job `verify-invariants`, filtered to `.lean` changes, plus weekly (`cron '17 4 * * 1'`) and `workflow_dispatch`. Known limit kept in the plan: it still does not run on *every* commit, so "unrun" can still mean "a non-Lean commit". |
| **D.6** — wire `uk_summarize` into the editor | unmet | **met 2026-10-06.** Both overlay surfaces (inline annotation, results panel) render `uk_summarize`'s editor-channel output for text that does not fit the budget; text that fits passes through verbatim (summarizing is lossy by construction, so running it over short text would only drop information the overlay has room for). The lossy flag is *surfaced in the line itself*, not just returned, and a zero-selection says why instead of showing nothing. Full trace retrievable: the raw string is never modified — `results()`, `run_log()` (with the verbatim Jupyter outputs), the cell region and the exports all still carry it. Proof that it goes through the symbol rather than the pure Rust function beside it: the summarizer session's cursored event log records `Summarized{channel: "editor", lossy: true}` after a long text and nothing after a short one. C6 is closed. |

Two further honesty notes about this pass, so nothing here reads as more than it
is:

- The **`cross-repo` job's emthin step has never been observed green.** Its
  remote URL was inferred from the sibling naming convention and its system
  dependency list was transcribed from `emthin/docs/build-notes.md`, not from a
  run — the authoring machine has no `pkg-config` and cannot build emthin at all.
  It is deliberately best-effort so it cannot take down four steps that are known
  good, and it reports **SKIP**, never PASS, when the repo is absent. Treat its
  first run as a hypothesis about the dependency list.
- The sibling refs in that job are **branch names, not SHAs.** Pinning to a
  commit that was never pushed would break every run, which is worse than
  tracking a branch; the resolved SHA is echoed per failure so a red run is still
  attributable to an exact tree.

### D.8 — two velysterm tests are flaky, and the flake cannot say why (found 2026-10-06)

*Limitation*: `mathed_mini`'s `export::tests::rules_binary_json_contract_roundtrips`
and `export::tests::rules_engine_new_ops_roundtrip` intermittently fail with
`left: None`, i.e. `mathed_rules_engine_with_bin` gave up rather than wrong
output. It gives up for five different reasons — spawn failed, `try_wait`
errored, the 5 s deadline passed, the child exited non-zero, or the answer was
not parseable JSON — and the test asserts only the final `Option`, so a failure
cannot say which one happened. **It is not new work**: with every 2026-10-06
change stashed (clean tree) the same 4-test filter failed 1 in 6 runs, and with
the changes present 2 in 6; run alone, each test passed 6 of 6 in both states.
The machine was at load ~6.5 throughout (a heavy suite running in parallel), so
the working hypothesis is spawn pressure or the deadline, but that is a
hypothesis, not a diagnosis.

*What holds instead*: the failure is an `Option` the caller degrades to the
identity path by design, so nothing wrong is ever rendered — and the pair has
never been observed failing for a correctness reason.

*Planned solution*: (a) have the seam carry the failure reason (a `Result` or a
test-only hook returning which of the five it was), so the flake names itself;
(b) re-run the pair under that instrumentation and fix the actual cause; (c)
until then, no status line says "velysterm suite green" without the qualifier
"except a known flaky pair (rules export stubs, load-sensitive)". Silently
retrying until green would be the dishonest option and is not what happens.

**Resolved 2026-10-08 — with one honest residue.**

- **(a) Done.** The seam grew a reason type: `mathed_mini::export::RulesFailure`
  (`Spawn` / `Wait` / `Deadline` / `Exit { code, stdin }` / `Output`) and
  `mathed_rules_engine_detailed` returning `Result<String, RulesFailure>`; the
  public `Option` API is untouched (production still degrades to identity), and
  `rules_engine_within` (private) makes the 5 s budget shortenable so the
  `Deadline` arm is tested in 200 ms instead of 5 s. Every rules test now
  asserts through `detailed`, so a failure prints e.g.
  `Err(Spawn("Text file busy (os error 26)"))` instead of `left: None`.
- **(b) Diagnosed to ETXTBSY, root cause *not* observable — recorded, not
  papered over.** Under the documented load (6 CPU hovers, 4 concurrent test
  processes) the instrumentation reproduced **5 failing runs in 40** and named
  the reason every time: `Spawn("Text file busy (os error 26)")`. Three
  observation passes then failed to catch the writer: (1) unfiltered
  `strace -f -tt -yy` on a live failure shows the creator's
  `open(O_WRONLY) → write → chmod → close(= 0)` completing **800 µs before** the
  failing `execve`, and the failing path referenced exactly *once more in the
  whole trace* — no second opener anywhere; (2) a `/proc/*/fd` scan wired into
  the error path at the moment of failure reports **no holder** ("holder already
  closed, or not fd-based"); (3) cross-process: no other instance references the
  path. So the write-open is real (`execve` fails iff `i_writecount > 0`) but
  shorter-lived than every observation window — the cause stays **unknown**.
- **The fix for a transient-by-definition failure.** ETXTBSY *cannot* persist:
  every write-open closes. The seam now retries the spawn **5 times** with
  5/10/15/20/25 ms backoff, re-scanning `/proc` on each attempt so an exhausted
  retry still reports the holder if one is visible; non-ETXTBSY spawn errors are
  never retried. This is not "retry until green": the retry is bounded, the
  reason is attached on exhaustion, and the identity contract never changed.
- **Verified**: same stress as the baseline — **0 failures in 60 runs** (300
  test executions under load; baseline was 5/40 runs). Full `mathed_mini`
  suite green (251 incl. the new `rules_failure_reasons_name_themselves`),
  clippy `-D warnings` clean.
- **Hygiene fixed in passing**: `rules_engine_new_ops_roundtrip` and
  `rules_engine_select_ops_use_the_slice` never removed their stubs (1119
  leftovers accumulated in `/tmp`); both now clean up, and the stale files were
  deleted. My own first draft of the reason test asserted `stdin: None` on the
  `exit 3` stub and flaked on a legitimate EPIPE — the assertion now pins the
  exit code and leaves the stdin note free, because a stub that never reads
  *should* be able to EPIPE the writer.
- **(c) The qualifier can go**: no status line needs the "except a known flaky
  pair" escape any more — the pair is fixed and the count in the status block
  is recounted separately anyway.
