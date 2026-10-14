# S4 Backlog — carried from `PLAN_WORKSPACE_REVIEW_2026-10-14.md` §8
**Date:** 2026-10-14 · **Owner:** owner / LLM-Lean4-specialist (S3 items are done and closed; this document is what S3 deliberately left open)
**Predecessor:** `PLAN_WORKSPACE_REVIEW_2026-10-14.md` (S3 executed: 31/0/0 at its stage boundary; this doc completes its S4 column). Not a new review cycle — the S4 tail, given rows and acceptance so it can be picked up without re-reading the S3 plan.

Scope discipline unchanged from S3 §5: no Lean4 code here (owner's boundary — §3), no vendoring (design adaptation + attribution rows only), no pushes without the owner asking.

---

## 1. Backlog rows

| # | item | kind | repo | owner | acceptance |
|---|---|---|---|---|---|
| **S4-1** | **ProofFlow → logos study note.** Evaluate `../ProofFlow/` (dependency-graph proof autoformalization, arXiv 2510.15981) as a discipline for `logos`' formalize pipeline: does a dependency-graph-driven scheduling of which obligations to formalize next improve the LLM formalize→Lean4 loop's acceptance rate? Output is a *study note* (fit, gaps, verdict), not code. | study (§6 of S3) | unfer/logos | LLM-Lean4-specialist, then owner | note lands in `unfer/docs/`; states (a) which ProofFlow mechanisms map onto logos' pipeline, (b) which do not and why, (c) a go/no-go for the Rust-side orchestration the specialist's contract would gate; no Lean4 code until the specialist signs the output contract |
| **S4-2** | **Engram agent-memory revisit.** The deferred new-feature candidate (S3 §3.3): decide whether `kernel_client`'s in-memory `HashMap<u64, Session>` still suffices, or an observed requirement justifies a bounded persistent memory with retrieval. **Resolved 2026-10-14 — still not observed; evidence in the execution log below.** | new feature (deferred, justification on record) | velysterm/kernel_client | owner | either a written "still not observed" row updating S3 §3.3, or an observed workflow that the tool-design doctrine's bar (`"an observed requirement those primitives cannot satisfy"`) admits — only then design, with Engram (`engram-reference/`) as reference-only (paper reproduction row already in the ledger) |
| **S4-3** | **Harness-contract docs for voice/logos.** S3 §3.4 adopted deepseek-harness's BENCHMARK.md-paired-with-harness practice "only where cheap (deferred, backlog)": write the harness contract for `unfer/voice/` and `logos`' LLM loop where it is still missing. | improve (dsh) | unfer | owner/agent | each harness's doc states entry command, environment preconditions, honest skip semantics, redaction rules (workspace `scripts/README.md` redaction policy); `check-links` + attribution green |
| **S4-4** | **D1/D2 scenario rows → real-session upgrade.** The desktop save-conflict rows close at `unit` authority (binary's own `save()`, in-process). The upgrade path: run the conflict live with a display and an external editor (open doc in `mathed`, rewrite the file from a shell, Ctrl+S twice, observe refusal + notified overwrite). | improve (S3 P1b follow-through) | velysterm | owner (needs a display) | `scripts/SCENARIOS.md` D1/D2 authority raised to `real-session` (or `harness` with a scripted driver) **only after** a recorded run; verifier row updated in the same change; `check-scenario-ledgers` green |
| **S4-5** | **Scheduled gate run wired for real.** S3 P4 documented the cron line (workspace root has no CI). If an owner-chosen CI target ever exists for the workspace root, the cron note becomes a `workflow_dispatch`+cron workflow. | improve (F6 completion) | workspace root | owner | either the workflow file running `check-all-gates` weekly, or the cron line confirmed installed and its log rotated; `check-all-gates` unchanged |

## 2. Decision log

Convention per `PLAN_WORKSPACE_REVIEW_2026-10-14.md` §9 (`DN (date):` rows).

- **D1 (2026-10-14):** S4 stays a backlog, not a plan cycle. Rationale: every item here is either gated on an owner resource (display, CI choice) or on the Lean4 specialist's boundary; scheduling them as work would re-litigate S3's deferral decisions.
- **D2 (2026-10-14):** S4-1's ProofFlow evaluation is a *study note* first. Rationale: the tool-design doctrine ("a plan without execution does not establish runtime behavior") cuts both ways — a pipeline change whose acceptance is Lean output needs the specialist's output contract before any orchestration code, so the note's go/no-go is the honest deliverable.

## 3. Execution log

- 2026-10-14: S4 backlog written from S3 §8's S4 column.
- 2026-10-14 (follow-up session — S3's three suggested follow-ups, all done):
  1. **Save-conflict E2E recorded** (partial S4-4: the *unit-tier* half now
     exists; the real-session upgrade stays open): `save_conflict_end_to_end`
     in `crates/mathed/src/main.rs` drives the binary's real `save()` through
     create → external rewrite → refused Ctrl+S (external bytes intact,
     conflict armed) → autosave still refuses → second Ctrl+S overwrites.
     Verifier `scripts/save-conflict-e2e.sh` (flake-aware); scenario rows
     **D1/D2** added to `velysterm/scripts/SCENARIOS.md` closing at `unit`.
     Gate extended: `check-scenario-ledgers` now parses any `[A-Z][0-9]+`
     row id (D-rows would otherwise have been silently unchecked) —
     negative-proven: closure value `VERIFIED` fails, restore passes.
  2. **Commits (no pushes):** velysterm `c680cb5` (build fix), `ac43b70`
     (save hardening), `77aefbe` (scenario rows + verifier); emthin
     `e578a7a` (E1–E6 ledger). Trees clean except velysterm's parked
     scratch (D4).
  3. **Attribution gate stale-list defect found and fixed at cause:**
     `check-attribution`'s `ROOT_DOCS` was a hand-maintained 3-doc list —
     the 10-12, 10-09 and both 10-14 plan docs were never scanned. Now
     globs `PLAN_*.md` at scan time (8 root docs scanned). Negative-proven:
     an isolated attribution-claim phrase planted in this doc (with no
     ledger token in its window) → gate exit 1; restore → exit 0. Full
     sweep after all changes: **31/0/0**.
- 2026-10-14: S4-1, S4-2, S4-3, S4-5 not started — each row's acceptance
  stands as written; S4-4's real-session half remains open by design.
- 2026-10-14 (second session — owner asked "why not runned"; three items
  executed, S4-5 declined by owner selection):
  1. **S4-1 done** → `unfer/docs/PROOFFLOW_STUDY.md`. Headline: the
     question's premise was stale — `logos::formalize` **is** the
     ProofFlow adaptation (parity-locked port, rewrite plan §11–§17), so
     the Rust orchestration the note was to gate already exists. Delta:
     two adopt candidates (per-stage model config, LLM call log — Rust-only,
     output contract unchanged), defer ProofFlowBench until a first live
     run, permanent substitution for the Lean-server oracle. **Compliance
     finding fixed at cause:** the port landed with *no ledger row*; ProofFlow
     rows added to root + unfer ATTRIBUTION (MIT, LICENSE read firsthand).
  2. **S4-2 done — "still not observed" (the assessment the acceptance
     asked for).** Evidence: (a) the bounded memory the old C2 wanted
     already exists — `unfer/docs/PROTOCOL.md` §210 and
     `kernel_client/src/org_runner.rs`'s `WorkerSlot.memory`; (b) what must
     survive a restart already persists with a written scope —
     `kernel_client/src/session.rs` (G2, atomic + versioned) explicitly
     *excludes* C2 memory: "a session is the loop's own state — nothing
     that can be re-derived from something more durable"; (c) the one
     observed retrieval-memory need — lemma reuse across formalize runs —
     is already served by `logos/src/formalize/memory.rs`'s engram-backed,
     UNF-hash-addressed store. Revisit trigger: a worker whose C2
     conclusions must survive restart **and** cannot be re-derived from
     the board. No such workflow appears in BENCH.md or the coop docs, so
     an Engram-style cross-session agent memory still fails the
     tool-design doctrine's bar.
  3. **S4-3 done** — harness-contract sections written: `unfer/voice/README.md`
     (entry / preconditions / closed 5-FAIL-code set with catalog pointer /
     `LLM_URL_DISP` redaction / per-user state) and `unfer/docs/FORMALIZE.md`
     §7 (entry / optional key never defaulted empty / exit codes / honest
     skip when no live model / header-only key, body asserted keyless /
     artifacts).
  4. **Root files now have git** (owner decision: *copy, not move*):
     `emthin/workspace-backup/` mirrors the coordination files; refreshed by
     `scripts/sync-workspace-backup`, gated by `scripts/check-backup-sync`
     (byte-identity; negative-proven: drift → exit 1, resync → 0).
     `check-links` (emthin) and `check-attribution` exclude the mirror with
     the reasoning in-script: identical text must not be resolved against a
     different window than its gated source.

## 4. Copyright & compliance

Unchanged from S3 §7: ProofFlow/Engram/deepseek-harness consulted as references only; any claim added while executing these rows resolves through `check-attribution` to a ledger row, internal path, or allowlist entry; no Lean4 code (§above).
