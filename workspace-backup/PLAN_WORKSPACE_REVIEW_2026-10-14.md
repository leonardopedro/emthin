# Workspace Review & Improvement Plan — REA + pattern/Engram/harness adaptations
**Date:** 2026-10-14 · **Scope:** `unfer/`, `australVM/`, `velysterm/`, `timepiece/`, `dynamic-arctic/`, `test/`, `emthin/` (+ `verifiedUniqueAliases/` as the eighth member)
**Reference studies this cycle:** `rea/` (deep — obligations/testing/tool-design/user-visible-errors), `pattern/` (Loro sync + agent runtime), `Engram`/`engram-reference` (agent memory), `deepseek-harness/` (agent harness), `qm/` (ADR/ops discipline), `ProofFlow/` (proof autoformalization), `hayagriva`/`why3`/`cloud-hypervisor-build` (already-integrated upstreams, surveyed only).
**Predecessors (not restated):** `PROJECT_REVIEW_AND_IMPROVEMENT_PLAN.md` (2026-10-04, ClawSwarm/Agensh/loop) and `PLAN_CROSS_PROJECT_REVIEW_2026-10-12.md` (2026-10-12, REA R1–R12 + B1–B7 — **S1+S2 fully executed**, 28/0/0 at its stage boundary). This plan supersedes neither; open items are carried forward by reference.

---

## 0. Executive summary

The federation is the same seven checkouts the predecessors mapped, plus `verifiedUniqueAliases` (the PayPal confidential-VM attestation repo) which completed its own live audit this week (verify.html 5/5 against a KDS-verifiable OCI E5 host; silicon-root fail-closed behavior on VLEK-provisioned E5/E6 hosts documented).

**Baseline re-measured today (2026-10-14):** `scripts/check-all-gates` = **26 passed, 2 failed, 0 gaps**. Both failures share one root cause (see F1 below), introduced *after* the 10-12 green boundary — proof that the gate layer works and proof that a sync gate for *the ledger-pointer convention itself* is missing. Trees are clean except two untracked velysterm scratch artifacts. australVM's 22 code TODOs are all in vendored OCaml compiler files (`lib/*.ml`) and are already outside the TODO-ledger gate's scan scope by policy.

This cycle's plan concentrates on three adaptation axes the predecessors did not cover:

1. **Evidence-closure discipline** (REA `docs/reconstruction-obligation-ledgers.md`): upgrade the two SCENARIOS.md contracts from honest statuses to *closure-checked obligations* — authority-matched verifiers, per-case-kind coverage, and the rule a green lower-tier run cannot close a higher-tier claim (which the federation already believes in prose: velysterm's "do not record a GUI behaviour as verified on the basis of a headless run").
2. **User-visible error audit** (REA `docs/user-visible-errors.csv`): a workspace-level harm-ranked catalog of every user-visible error surface with copy-risk review and verification results, machine-checked against the code enums that already exist.
3. **CRDT persistence hardening** (pattern `pattern_memory::loro_sync`): adopt the `SyncedDoc` doctrine — echo suppression + `RejectAndNotify` external-modification policy — in mathed_core's save path, where today a stale editor can silently clobber a document edited outside the app.

Everything is specified as improvements of existing features. The one new-feature candidate (agent memory, Engram) is *deferred with justification* (§5). Lean4 work remains delegated (§6). Copyright discipline in §7.

---

## 1. System map (unchanged shape, updated status)

| repo | role | gate state 2026-10-14 | notes since 10-12 |
|---|---|---|---|
| `unfer` | research core (prob_kernel, unfer_protocol, consensus, logos, qfm, decide_core) | **red** — `check-attribution` (F1) | heavy-tests CI landed (E4); voice.sh tagged FAIL (R1b) |
| `australVM` | module runtime (Austral→CPS→cranelift, modhost, Cedar auth) | green | TODO-ledger gate green (22 markers are vendored `lib/*.ml`, out of scan scope by policy) |
| `velysterm` | document engine + UIs + kernel_client | green | 2 untracked scratch artifacts (F2); megafile roadmap (MEGAFILE_DECOMPOSITION.md) open |
| `timepiece` | Lean4 corpus + book + drift gates | green | pending-lemmas mechanism carries the 86 forward references; Lean items delegated |
| `dynamic-arctic` | threshold-signature artifact | green | verify-invariants landed (B4) |
| `test` | GitBook prose site | green | fixed this cycle (F1) |
| `emthin` | Wayland compositor (document-is-the-layout) | green | error contract (E5) + ipc docsync (E3) landed |
| `verifiedUniqueAliases` | confidential-VM attestation + auditor | green (own repo gates + live verify.html 5/5) | OCI E5/E6 fleet findings documented in its AGENTS.md |

---

## 2. Findings (this cycle)

### F1 — Attribution gate red: a gate-convention drift with no self-check *(fixed this cycle)*
Commit `3e8d9e3` ("a") in `test/` replaced the X9 ledger pointer `[../ATTRIBUTION.md](../ATTRIBUTION.md)` with a backticked path, breaking **both** attribution gates (`scripts/check-attribution` and unfer's copy). The change was well-intentioned (fear of an out-of-tree link on the published GitBook) but unnecessary: `check_site.py` and `check-links` both pass with the link form. **Fixed at cause** (`test` commit `ec27a2f`): link restored, README census bumped 314→315 in the same change. Residual gap: nothing machine-checks *which* form the pointer takes, so a future copy-edit can re-break it — covered by item **P0** below.

### F2 — velysterm dirty tree: parked scratch without a note
`crates/mathed_snapshot_scratch/` (Cargo.toml + src/, dated 10-11) and `mathed-screenshot.png` are untracked scratch from a snapshot-render experiment. The repo's session-hygiene rule allows "a commit or a note in the session record" — this document's execution log is that note (item **P5**). They are not deleted: the scratch crate may be the seed of the `mathed_snapshot` idea in `MEGAFILE_DECOMPOSITION.md`'s orbit.

### F3 — Scenario contracts lack closure authority *(the REA gap)*
`emthin/scripts/e2e/SCENARIOS.md` (E1–E6) and `velysterm/scripts/SCENARIOS.md` (A1–A8) already carry honest statuses — but nothing binds a status to an *authority tier* and a *verifier command*. REA's obligation ledger closes a claim only when a verifier of **comparable authority** enumerates the obligation and its result is recorded; "a green unit test cannot close a packaged-process obligation". The federation already believes this (velysterm's headless-run rule, emthin's exit-2 "declined ≠ passed") but no artifact enforces the pairing. Items **P1a/P1b**.

### F4 — User-visible error copy is unreviewed across surfaces *(the REA gap)*
Four user-visible error surfaces exist (emthin `IpcErrorCode`, voice.sh `FAIL <code>`, modhost `--json` errors, unfer_agent `UK-####` diagnostics). emthin's is documented and enum-pinned; the others are documented in prose. None has REA's harm-ranked review (does the copy leak internals? does it tell the user what to do? is repetition expected to help?). Item **P2**.

### F5 — Document persistence can silently clobber external edits *(the pattern gap)*
mathed_core's save path writes the Loro-backed document without checking whether the file changed on disk since load. pattern's `loro_sync::SyncedDoc` solved exactly this class: mtime/hash echo suppression + a default-deny `RejectAndNotify` conflict policy so a stale writer surfaces a conflict instead of silently merging. Item **P3**.

### F6 — The gate suite has no scheduled runner
`check-all-gates` exists and is the stage-boundary invariant, but nothing runs it on a schedule; a break like F1 sits unnoticed until the next session happens to run it. A tiny scheduled workflow per workspace-root (or a `cron` note in `scripts/README.md`) closes this. Item **P4**.

---

## 3. Reference study (this cycle)

### 3.1 REA (deep) — four disciplines, adapted not copied (MIT, morluto; ledger row exists from E1)

| # | REA discipline | What it says | Federation adaptation |
|---|---|---|---|
| D1 | **Reconstruction obligation ledgers** | Claims close only when: one manifest binding names the owner, every required case kind (positive/negative/malformed/cancellation) has authenticated evidence, and a verifier of **comparable authority** enumerates the obligation ID and passes. Dispositions (`blocked`, `out-of-scope`) never count as verified. | P1: SCENARIOS.md rows gain `authority` + `verifier` + `case kinds`; a machine gate rejects a status whose authority tier exceeds the verifier's tier |
| D2 | **User-visible error audit** | `user-visible-errors.csv`: harm_rank, location, trigger, current copy, user risk, proposed replacement, implementation status, verification result. Copy must not leak internals; must state the recovery step; must say whether repeating helps | P2: workspace `docs/user-visible-errors.csv` over the four surfaces, with the same columns minus provider-specific ones; drift gate vs the code enums |
| D3 | **Tool-design doctrine** | Design from the caller's question; extend an existing tool when intent+result match; complete evidence inline; observed/derived/inferred stay distinct; limits only for real constraints; "a plan without execution does not establish runtime behavior" | Already the law of `unfer_agent`'s op surface (op-catalog gate) and emthin's IPC; the *discoverability evaluation* half ("evaluate representative broad/direct/negative tasks on the intended host") is adopted into P1's authority tiers for agent-surface changes |
| D4 | **Testing hierarchy** | real public workflows > production-boundary integration > producer goldens > module tests; preserve malformed/cancellation/cleanup/permission/capacity/identity coverage when pruning; "simulated providers do not establish real-provider behavior"; report unverified coverage explicitly | Codified as the authority-tier vocabulary in P1; also the honesty rule for future heavy-tests CI annotations |

### 3.2 pattern/ (MPL-2.0) — `pattern_memory::loro_sync`
`SyncedDoc<B>`: two-doc CRDT sync (caller memory doc ↔ disk doc) with an ingest thread, mtime/hash **echo suppression**, and `ConflictPolicy::RejectAndNotify` as the default — external changes that are based on a stale view are surfaced, not silently merged (`AutoMerge` is explicit opt-in). Also `DirWatcher` + `PathFanoutRouter` for multi-file sets.
**Adaptation (P3):** the *doctrine* (echo suppression, reject-and-notify default, explicit force override) into mathed_core's save path. Not the crate itself — mathed_core's persistence is a single-document load/save, and vendoring a two-doc sync engine would be scope inflation. Licence note: MPL-2.0 file headers; we adapt the design idea only (attribution row required).

### 3.3 Engram / engram-reference (DeepSeek memory paper)
Memory architecture for persistent agents. This is the *reference implementation* class the old plan's C2 ("persistent agent memory with bounded context + retrieval") wanted. **Deferred with justification:** kernel_client's `HashMap<u64, Session>` is deliberately in-memory per document; a memory subsystem is a new feature whose requirement has not been observed in the current workflows (tool-design doctrine: "Custom orchestration languages… need an observed requirement those primitives cannot satisfy"). Carried as a backlog row.
**Resolved 2026-10-14 (S4-2): still not observed** — assessment with evidence (C2 memory exists in-loop; sessions persist loop state by design and exclude C2 memory with rationale; the one observed retrieval need, lemma reuse, already has engram-backed storage) in `PLAN_S4_BACKLOG_2026-10-14.md`'s execution log.

### 3.4 deepseek-harness (dsh) — agent harness
Open-source agent harness with a BENCHMARK.md discipline. Relevant to `unfer_agent`'s NDJSON loop and emthin's `scripts/agent/` loop. **Adopted:** the practice of a benchmark/discipline doc *paired with* the harness — emthin's `docs/AGENT_LOOP.md` is that already; unfer's `voice/` and `logos` harnesses gain a "harness contract" section only where cheap (deferred, backlog).

### 3.5 qm — ADR discipline
Architecture Decision Records in `adrs/`. The federation records decisions in plan documents and AGENTS.md gotchas — richer in context, weaker in retrieval. **Adopted lightly (P6):** the workspace plan docs gain a standing "Decision log" section convention (numbered D-rows) instead of a new ADR tree — improving the existing plan-doc feature rather than creating a parallel record system.

### 3.6 ProofFlow — dependency-graph proof autoformalization
"Dependency graph approach to faithful proof autoformalization in Lean 4". Relevant to `unfer/logos` (LLM formalize→Lean4). **Delegated with the Lean4 boundary:** the pipeline change is Rust-side but its acceptance is Lean output; it goes to the LLM-Lean4-specialist as a *study note* in §6, not as work for this agent.

---

## 4. Plan items (ordered by value × feasibility ÷ risk)

| # | item | kind | repo | acceptance |
|---|---|---|---|---|
| **P0** | Ledger-pointer self-check: `scripts/check-attribution` accepts the link form **and** a backticked/path form of `../ATTRIBUTION.md`, but never neither; negative tests prove it bites | improve (F1 residual) | root scripts | gate green; removing the pointer entirely fails; a `](../ATTRIBUTION.md)` link and a `` `../ATTRIBUTION.md` `` path both pass |
| **P1a** | emthin SCENARIOS.md → obligation rows: each E1–E6 gains `authority` tier (`real-session`, `harness`, `headless`, `unit`) + `verifier` command + required case kinds; closure rule stated (lower-tier run never closes higher-tier row) | improve (D1) | emthin | doc updated; `check-ipc-docsync` untouched and green |
| **P1b** | velysterm SCENARIOS.md → same obligation shape for A1–A8 | improve (D1) | velysterm | doc updated |
| **P1c** | `scripts/check-scenario-ledgers`: closed vocab for status/authority; every row's verifier names an existing script; wired into `check-all-gates` auto-discovery | improve (D1) | root scripts | gate green; a bogus status value or a missing verifier path fails |
| **P2** | `docs/user-visible-errors.csv` (harm-ranked) + `scripts/check-error-catalog`: CSV shape valid, codes cross-check emthin's `IpcErrorCode` + voice.sh's 5 FAIL codes + modhost's `--json` codes; every row has verification status | improve (D2) | root docs + scripts | gate green; mutating a code in the CSV without the enum fails |
| **P3** | mathed_core save path: external-modification detection (mtime+hash at load vs save) with `RejectAndNotify` default and explicit `force`; typed error surfaced through the existing Diagnostic channel | improve (pattern) | velysterm | new unit tests green in `cargo test -p mathed_core`; no behavior change when file untouched |
| **P4** | Scheduled gate runner: a weekly `workflow_dispatch`+cron workflow running `check-all-gates`, or documented cron line in `scripts/README.md` if no CI target exists for the workspace root | improve (F6) | root | workflow file or doc note; `check-all-gates` unchanged |
| **P5** | velysterm scratch: parked-note (this log) + decision row (keep as seed vs remove) | hygiene (F2) | velysterm | note recorded; tree policy satisfied |
| **P6** | Decision-log convention (D-rows) adopted in this plan + the two predecessors' headers | improve (3.5) | root docs | convention written; D1–D3 seeded from this cycle |
| **P7** | Final verification: `check-all-gates` 28+/0/0-ish, per-repo suites green | verify | all | every gate green; negative tests cited |

**New-feature candidate (deferred, justified):** agent memory (Engram/C2) — no observed requirement yet (§3.3). **Delegated:** ProofFlow→logos note and all Lean4 (§6).

---

## 5. What this plan deliberately does NOT do

- No Lean4 code, no lemma work (owner's boundary).
- No vendoring of pattern/REA code — design adaptation with attribution rows only.
- No changes to `dynamic-arctic` (clean and upstream-bounded), `timepiece` (Lean-heavy), or the OCaml compiler sources in australVM.
- No git push to the seven repos (per-repo rules: push only on owner request). `verifiedUniqueAliases` pushes are part of its own attested pipeline and already done.

## 6. Lean4 delegation boundary (LLM-Lean4-specialist)

Unchanged from the 10-12 plan §7 (pinned toolchain, axiom-gate + verify-invariants acceptance, no `sorry`/`unsafe`). **New note for the specialist:** evaluate ProofFlow's dependency-graph autoformalization (arXiv 2510.15981, local checkout `../ProofFlow/`) as a candidate discipline for `logos`' formalize pipeline; the Rust-side orchestration can be implemented by a non-Lean agent once the specialist signs off on the output contract.

## 7. Copyright & compliance

- REA is MIT (ledger row from E1); `pattern` is MPL-2.0 (design idea adapted, no code copied — attribution row required in `velysterm/ATTRIBUTION.md` for P3); Engram/ProofFlow/deepseek-harness consulted as references only (no copied text or code); `qm`'s ADR shape is a convention, not copyrightable content. All adaptation claims resolve to ledger rows via `check-attribution` (policy #4).
- Copyleft boundaries unchanged (Cadabra2/Why3 subprocess-only; MPL containment respected).

## 8. Sequencing

| stage | items | boundary |
|---|---|---|
| S3 (this session) | P0, P1a–c, P2, P3, P5, P6, P7 | `check-all-gates` green incl. the new gates; velysterm mathed_core suite green |
| S4 (owner/specialist) | P4 if a CI target must be chosen by owner; Lean4 notes; backlog (agent memory, dsh harness docs, ProofFlow) | — |

## 9. Decision log

**Convention (P6, adapted lightly from `qm`'s ADR discipline):** every
plan document in the workspace root carries a `## Decision log` section;
decisions are numbered `DN (date):` rows stating the decision and its
rationale in two sentences. This improves the existing plan-doc feature
rather than creating a parallel `adrs/` tree — retrieval stays where the
context is. The two predecessor plans carry the pointer in their headers.

- **D1 (2026-10-14):** ledger-pointer gate accepts link *or* backticked path form — the pointer's existence is the policy; the markup is not. Rationale: test/ is a published GitBook where an out-of-tree link is dead weight; the 3e8d9e3 edit showed the need.
- **D2 (2026-10-14):** scenario closure authority tiers are `real-session > harness > headless > unit`; a row's status may never exceed its verifier's tier. Rationale: the federation already believes this in prose; making it machine-checked is the adaptation.
- **D3 (2026-10-14):** mathed_core adopts pattern's RejectAndNotify persistence doctrine *by implementation, not vendoring*; force-overwrite stays possible behind an explicit parameter.
- **D4 (2026-10-14):** the velysterm scratch (`crates/mathed_snapshot_scratch/` + `mathed-screenshot.png`, 10-11) is **kept as a seed**, not deleted: it may be the seed of the `mathed_snapshot` idea in `MEGAFILE_DECOMPOSITION.md`'s orbit. Parked per the repo's session-hygiene rule by this note (F2); revisit when the megafile decomposition lands.
- **D5 (2026-10-14):** `mathed`'s dependency on `mathed_mini` must request the `kernel` feature explicitly (`default-features = false, features = ["kernel"]`): isolated `cargo check -p mathed` builds compile `mathed_mini` without `KernelBridge`/`KernelResult` and fail; whole-workspace builds masked it via feature unification. Discovered during P3 verification, fixed at cause in `crates/mathed/Cargo.toml`.
- **D6 (2026-10-14):** autosave's dirty timer is cleared on every save *attempt* (success or refusal): before P3 the timer was never cleared, so an idle document was rewritten every frame — and under reject-and-notify that would additionally spam the conflict warning. One save per edit burst is both the fix and the only behavior compatible with the notification.

---

## 10. Execution log

**S3, 2026-10-14 — all items executed in this session.**

| item | status | evidence |
|---|---|---|
| P0 | done | `check-attribution` patched to accept link **or** backticked `../ATTRIBUTION.md`, never neither; proven non-vacuous in both directions (pointer removed → fail; each accepted form → pass) |
| P1a | done | `emthin/scripts/e2e/SCENARIOS.md` E1–E6 carry obligation-closure rows (authority tier + verifier + case kinds + closure semantics) |
| P1b | done | `velysterm/scripts/SCENARIOS.md` A1–A8 same shape |
| P1c | done | `scripts/check-scenario-ledgers` (pure bash/awk): closed vocab + verifier-existence + ID bijection; negative-proven (bogus status fails, missing verifier path fails, deleted row fails); auto-discovered by `check-all-gates` |
| P2 | done | `docs/user-visible-errors.csv` (16 rows, harm-ranked) + `scripts/check-error-catalog` (shape + bidirectional code-set parity vs `IpcErrorCode`/`voice.sh`/`modhost`); negative-proven in 4 directions (deleted row → parity fail; mutated code → bidirectional fail; bad retryable → shape fail; restore → green) |
| P3 | done | `mathed_core::format`: `FileStamp` (mtime+len+FNV-1a), `SaveError::ExternalModification`, `save_snapshot_guarded` (reject-and-notify default, `force` override), `load_with_stamp`; wired in `mathed/src/main.rs` (`EditorDoc::{stamp, save_conflict}`, `save(explicit)`); 12/12 format tests green incl. 5 conflict-path tests (external edit, appearing file, vanishing file, forced overwrite, echo stamp); `cargo test -p mathed_core` 293/0; `cargo check -p mathed` green in the flake devShell (udev shim) — see D5/D6 for two defects found and fixed at cause on the way |
| P4 | done | workspace root has no CI target (not a git repo) → the plan's allowed alternative: documented weekly cron line in `scripts/README.md` ("Scheduled runs") |
| P5 | done | scratch parked by D4 in this log — the session record the repo's hygiene rule asks for |
| P6 | done | Decision-log convention written (§9); D1–D6 seeded; predecessor headers carry the pointer |
| P7 | done | see "Final verification" below |

**Final verification (P7):** `check-all-gates` over all seven repos +
workspace section = **31 passed, 0 failed, 0 gaps** (baseline at plan
time was 26/2/0; both F1 failures fixed at cause, +2 workspace gates
landed and auto-discovered). `cargo test -p mathed_core` 293/0 (incl.
12 format tests, 5 conflict-path); `cargo test -p mathed` 57/0 (bevy
frontend incl. the rewired save path); `cargo clippy` clean on both
changed crates. Negative tests for every new gate cited in the rows
above. Attribution: `check-attribution` 259 hits, 0 unresolved (new
adaptation claims in `format.rs`, `main.rs`, `velysterm/ATTRIBUTION.md`
resolve to the root ledger's `pattern` row, extended this session).

