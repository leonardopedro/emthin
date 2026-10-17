# Seven-Repo Cross-Project Review & Improvement Plan — 2026-10-15 (round 3)

**Date:** 2026-10-15 · **Scope:** `unfer/`, `australVM/`, `velysterm/`, `timepiece/`, `dynamic-arctic/`, `test/`, `emthin/`
**Reference study (this cycle):** the AnaCP / AnaNC reference repositories (`AnaCP/`, `AnaNC/`, papers *AnaCP: Toward Upper-Bound Continual Learning via Analytic Contrastive Projection* (NeurIPS 2025) and *Continual Out-of-Distribution Detection with Analytic Neural Collapse* (AAAI 2026)) — studied as **reference-implementation discipline**: standalone modules with a two-method API (`update(X, Y)` / `predict(X)`), one closed-form solve per update (`torch.linalg.solve(A + reg·I, B)`), and reference-vs-reimplementation deviation tables. The federation already consumes both repos through `unfer/decide_core` (rows R1–R3 in `test/unfer/sequential-anacp-annc.md`); this round takes the *pattern* — a deviation table per reimplementation — and applies it to the review itself: every place today's measured state disagrees with a written claim is listed below with cause and fix, not silently corrected.
**Predecessor:** `PLAN_CROSS_PROJECT_REVIEW_2026-10-13.md` (round 2, executed: 28 gates green at its boundary). This document re-reviews the *measured* state two days later and executes the deferrals round 2 recorded.
**Owner scope cuts (2026-10-15):** (1) **the mass gap is out of scope** — no mass-gap item is executed, verified, or recounted this round; `unfer/fock_sirk/tests/mass_gap_campaign.rs`'s uncommitted control-bound fix (1e-6 → 1e-5) stays unverified by owner instruction and is excluded from commits. (2) After this plan's execution, the next workstream is the **emthin+mathed LLM-debuggability** task (see §9): make the app cheaply debuggable by an LLM, then use that surface to drive the app's documented goals to completion. Lean 4 code remains delegated; this plan writes none.

---

## 0. Executive summary

Round 2 built the claim→verifier culture; this round **ran every verifier again** and found the culture now catches drift quickly: the three stale counts (unfer, emthin, velysterm) were found by `check-status --recount` against fresh logs and are fixed here; `dynamic-arctic`'s status marker was honest while its README *prose* still said 66 (23 lib + 51 bin = 74 measured, three places). The real find is a **new functional regression**: emthin's E2E `run.sh` fails today (doc-copy, keyboard path, graceful shutdown all broken on a nested sway session) while its cargo suite is green at **347** — a doc/model seam failure invisible to cargo, now the top P-item. Two `timepiece` gates failed under `check-all-gates` (they `[gap]`ped in a bare shell): `check-gitbook-drift` had **9 unreviewed citation misses** from pages written 2026-10-14/15 — fixed by extending the gate with a sibling-source surface (unfer Rust/Cadabra/docs + reference-repo Python), scoped so Lean forward references cannot be masked. All 32 workspace gates pass at the end of this round's execution, 0 gaps.

| repo | claim this morning | measured today | verdict → action |
|---|---|---|---|
| `unfer` | 1792 cargo (10-13) | **1797 / 0 failed** | stale (+5: sequential + ESA-validation suites) → recounted, rev 28 |
| `emthin` | 345 cargo + 5/5 E2E (10-13) | **347 / 0 failed**; **E2E 3/5 FAIL** | count stale (+2) → recounted; **E2E regression → P1** |
| `velysterm` | 976 cargo (10-13) | **986 / 0 failed** | stale (+10: D1/D2 save-stamp work) → recounted |
| `dynamic-arctic` | 74 cargo (10-13) | **74 / 0 failed** | marker honest; prose "66" stale in 3 files → fixed |
| `australVM` | 110 dune / 14 suites | **110 / 14 / 0 failures** | honest (date bump) |
| `test` | 71 pages / 382 links | **71 / 382, pass** | honest |
| `timepiece` | unverified, not run | not run (Lean lane); **2 gates failed → 0** | drift gate fixed (P2 executed) |

`check-all-gates`: **32 passed / 0 failed / 0 gaps** (was 30/2 at 14:05).

---

## 1. Deep review — per folder (measured 2026-10-15)

### 1.1 `unfer` — 1797/0 (rev 28)
The +5 since 10-13 is `decide_core/tests/sequential.rs` (4) plus the new ESA/validation suites. Per-crate from one log (target-name attribution): `unfer_protocol` 402, logos 352, `unfer_ffi` 229, `prob_kernel` 143, `decide_core` 116, `unfer_consensus` 111, `nested_fock_algebra` 98, `qfm` 91, `unfer_edge` 63, `qfm_text` 62, `unfer_data` 52, `ode_sirk` 36, `unfer_taler` 30, `unfer_identity` 7, `sirk_core_model` 5. Only dirty file: `fock_sirk/tests/mass_gap_campaign.rs` (owner-excluded, §0). Commit subjects have regressed to `mm`/`a` one-letter messages (53d90ae, 02e9f87, 5786461, …) — noted in P6, not fixed here (owner habit, not a gate).

### 1.2 `emthin` — 347/0 cargo; **E2E failing**
Cargo green (emthin 293 + dbus 40 + clipboard 14). E2E `./scripts/e2e/run.sh --config xdotool` measured **4 failures across 4 runs**: `doc-copy` (marker absent, document reads empty), `typed text reaches the document` (empty doc), and `emthin did not exit within 30s of SIGTERM → SIGKILL` (the script itself says "that is a real failure of the signal path"). The app's own log shows saves and clipboard working — the document the *control client* reads back is empty, i.e. the doc-model/save seam, not the compositor. Root-cause is P1 and is deliberately **not guessed at** in this plan. Also found: `run.sh`'s `cleanup()` kills sway but leaks `Xvfb :99` — the stale server made the next run "start against a workspace it did not create" (the script's own comment names this hazard); fixed in P1 as the cheap half.

### 1.3 `velysterm` — 986/0
+51 over the 935 of 10-08: kernel_client +18, mathed_core +15, mathed +11, mathed_mini +7 — the D1/D2 save-stamp (`ac43b70`) and scenario-verifier work. `mathed_snapshot_scratch` still untracked-and-undocumented (round-2 deferral) → P5. Owner-decision files (`mathed-screenshot.png`) untouched.

### 1.4 `timepiece` — gate failure, fixed
`verify-invariants` and `check-gitbook-drift` pass in a bare shell but **fail when python3 is present** — i.e. on every dev box and in CI. 9 unreviewed `[MISS lean]` citations, all from pages written 10-14/15 (`unfer/sequential-anacp-annc.md`: `DECIDE_SPEC`, `_generate_replay_data`, `ananc.py`, `answer_choice`, `answer_score`, `id_score`, `projected_dim`; `mathematical-foundations/ode-singularity.md`: `qg_unitarity_check`, `transformed_esa_charts`). Every token has a real home — they cite the Rust engine, the reference repos, and Cadabra modules, none of which the gate could see. Fixed by extending the resolver (P2, executed): foreign extensions `.py`/`.cdb`; a **sibling-source token surface** (unfer `*.rs`/`docs/*.cdb`/`docs/*.md`, AnaCP/AnaNC `*.py`/`*.md`) *scoped by citing page* — `test/unfer/**` pages may resolve against unfer+reference tokens; math/the-program pages may resolve only `unfer/docs` artifacts, so a planned Lean lemma can never be masked by a `.rs` comment mention. Proven non-vacuous by three mutations (bogus token → FAIL; Rust token from math page → FAIL; same token from unfer page → PASS). 0 unreviewed misses, 86 pending entries unchanged, 0.5 s.

### 1.5 `dynamic-arctic` — 74/0; prose drift fixed
Status marker honest (74). README body claimed "66 (19 lib + 47 bin)" in three files (`README.md` ×2, `README_UPSTREAM.md`, `docs/RUNBOOK.md`); measured plain `cargo test` = `--all-features` = 74 = 23 lib + 51 bin. All four spots recounted with the cause recorded (the bench-validation commit recounted the block but not the prose).

### 1.6 `australVM` — 110/14/0
Re-run properly this time (`nix develop -c` + `LD_LIBRARY_PATH` for the bridge `.so`, the README's documented invocation — round 2's bare `dune runtest` rc=127 was my invocation error, not a repo defect). Honest; date bumped.

### 1.7 `test` — 71 pages / 382 links, pass
Site checker green; the two pages that caused the timepiece drift are now citation-clean. `the-program/statistical-model-theory.md` remains untracked (written earlier this session) → committed in P3.

## 2. Cross-cutting findings (round 3)

1. **Bare-shell gates lie by `[gap]`.** timepiece's two python gates reported "skip, no interpreter" on this host while python3 sat in `/usr/local/bin` outside the default non-login PATH; `check-all-gates` has the fallback, direct invocations don't. Any human running a gate directly saw green-by-skip. The gates themselves are fine; the *invocation* needs the same fallback (P7).
2. **E2E is the only thing standing between "cargo green" and "app works".** emthin proved the exact failure mode the status-block culture cannot see: suites green, product broken. The E2E run is not wired into any CI job (emthin CI runs cargo + linkcheck only) → P1 wires it after the fix.
3. **Count prose is still unguarded** beyond the velysterm op-count guard (round 2 F4): four files across two repos shipped stale test-count prose the same day their blocks were recounted → P4 generalizes the prose guard to test counts in Verification sections.
4. **`git log` subjects have degraded** to `mm`/`a` in unfer/timepiece/test since round 2's conventional commits. Not gated; noted (P6) as a hygiene observation for the owner.

## 3. P-items (this round's execution)

- **P1 — emthin E2E regression: root-cause and fix, then wire E2E into CI.** Reproduce under `run.sh`; instrument the control-client document read; the failure is in the doc-model/save seam (app log saves fine, reader sees empty). Fix at the producing boundary; make `cleanup()` also kill Xvfb; then add an E2E job (or step) to emthin CI with `E2E_NO_X=1` for the headless lane. Acceptance: 5/5 twice in a row from a clean shell, CI job green.
- **P2 — timepiece drift gate sibling surface** (executed in this document's preparation): resolver + scoping + three mutation proofs; verify-invariants 6/0/1gap(lake).
- **P3 — recount + commit hygiene (this round's measured fixes).** All README recounts, the dynamic-arctic prose fixes, the timepiece gate change, and `test/` untracked pages committed per repo in conventional style, **no push**; owner-excluded files (§0 mass-gap diff, velysterm scratch/png, timepiece/prompts) stay out.
- **P4 — test-count prose guard.** Extend the round-2 prose guard: `check-status` (or the per-repo catalogs) flags any `N tests pass`/`# N passed` claim in README/AGENTS Verification prose that disagrees with the repo's own `--list`/recount number; proven non-vacuous by mutation.
- **P5 — `mathed_snapshot_scratch` disposition** (round-2 deferral, now decided): document it — a `crates/` member with 5 passing tests reachable from no doc is exactly finding class 5 of the 10-09 plan; AGENTS/README gain a one-paragraph owner note, or the crate is removed by owner decision. Default: document.
- **P6 — repro_fig7a heavy-lane scheduling** (round-2 deferral): dynamic-arctic's heavy bench gets a recorded scheduling note (heavy-tests.yml class, `#[ignore]`d like unfer's) or an explicit "owner runs manually" row; no gate change.
- **P7 — direct-invocation parity for python gates.** timepiece's `verify-invariants`/`check-gitbook-drift` gain the same python3 fallback `check-all-gates` has (or document the wrapper as the only supported entry), so a human's direct run cannot `[gap]` past a real miss on a box that has the interpreter.

## 4. Reference-study application (AnaCP/AnaNC discipline)

The reference repos ship one module, two methods, and deviations from the papers *written down next to the code*. The federation's analogous habit — the deviation table — is applied in two places this round: (a) §1's per-repo claim-vs-measurement tables (the review *is* the deviation table); (b) emthin's P1 post-mortem must land as a deviation-style note in `emthin/docs/` (what the cargo suite measures vs what the E2E measures vs what a user sees), so the next seam regression is classifiable without re-deriving it. Nothing is vendored from either repo (no LICENSE; reimplemented-from-equations only — the `ATTRIBUTION.md` rows 65–66 already govern).

## 5. Lean4 delegation boundary (unchanged)

No Lean 4 code is written this round. `timepiece`'s Lean lane stays "unverified | not run" honestly; the 86 pending forward references in `pending-lemmas.txt` remain the specialist's obligation ledger; `lake` stays a recorded `[gap]` (toolchain absent).

## 6. Copyright & compliance

AnaCP/AnaNC: ideas only, no code/text (rows 65–66 in workspace `ATTRIBUTION.md`; `check-attribution` green). No other ledger rows change.

## 7. Sequencing & acceptance

| stage | items | acceptance |
|---|---|---|
| S1 (preparation, done) | P2, all recounts (§0 table), dynamic-arctic prose | `check-all-gates` **32/0/0**; `check-status` rc=0 with 4 recount lines |
| S2 (this plan's execution) | P1, P3–P7 | P1: E2E 5/5 ×2 + CI job; P4/P7: mutation-proven; P3: commits land, gates stay green |
| S3 (next workstream) | emthin+mathed LLM-debuggability (§9) | see §9 |

## 8. Execution log

S2 executed 2026-10-15; stage boundary green — `check-all-gates` **32/0/0**,
`check-status` rc=0 (7 blocks, P4 prose guard active).

| item | landed | evidence |
|---|---|---|
| **P1** emthin E2E | Root cause is **not** the doc-model/save seam this plan hypothesized: state carried across runs. A SIGKILLed emthin leaves a dead `emthin.ipc` that the `[ -S ]` readiness gate cannot tell from a live one (new emthin: `AlreadyInUse`); cleanup leaked `Xvfb :99` and the next run *adopted* whatever answered there. Symptom (empty `list_state`, keystrokes not landing, SIGTERM hang) vs cause fingerprint: clipboard/transport kept passing because they never touch carried state. Fixes: reap Xvfb + stale IPC (`lib.sh`), cleanup kills Xvfb (`run.sh`), `E2E_DUMP_STATE` raw-frame dump (`step7.sh`). **5/5 ×3 consecutive runs**; CI `e2e` job added; deviation note `emthin/docs/measurement-surfaces.md`; post-mortem in `scripts/e2e/README.md`. | emthin `9a85802`, `db1ad23`. *Acceptance half-deferred:* "CI job green" needs a push/dispatch — no push authorized; job is YAML-validated and runs the exact locally-verified command |
| **P2** | (preparation; resolver + scoping + 3 mutation proofs) | timepiece verify-invariants 6/0/1gap(lake) |
| **P3** commits | Per-repo conventional commits, no push. One near-miss caught and corrected: `git add -A` swept owner-excluded `mathed-screenshot.png` (velysterm) and the timepiece CONSOLIDATED_PLAN entry into two commits — both reset and recommitted cleanly; the plan entry got its own honest `docs(plan)` commit. | see each repo's log |
| **P4** prose guard | `check-status` P4 half: headline shapes `# N passed` / `N tests pass` held to the block, every repo. Mutation-proven both shapes (348/75 → FAIL with file:line; reverted; rc=0). The fock_sirk annotation gained the scope label the rule implies ("# fock_sirk: 401 passed …"); sub-scope prose stays out of scope by construction. | unfer `5b4fa1f`; also fixed dynamic-arctic's 66→74 prose (`95dd60b`) |
| **P5** scratch crate | Measured: 0 tests of its own — the round-2 "5 passing tests" was a mis-scoped count. Documented-not-deleted in velysterm AGENTS (delete the day it stops building); crate source committed because `members = crates/*` would break fresh clones without it. | velysterm `29f5a3c` |
| **P6** repro_fig7a | Weekly `heavy-repro.yml` (unfer heavy-tests.yml discipline): release build, datapoints uploaded, record-not-gate; AGENTS pointer. | dynamic-arctic `81daee2` |
| **P7** python fallback | Both timepiece gates search the uv locations like `check-all-gates`; A/B proven under a stripped PATH: original `[gap] python3 not on PATH`, patched runs all 88 citation lines. | timepiece `b827ead` |
| **F2** CI recount | emthin + velysterm test jobs tee their run into `check-status --recount` (canonical gate, unfer sibling already cloned); scope = the block's plain workspace run. Environment-count-delta remedy recorded in the workflow: scope the block, do not delete the gate. | emthin `0d2dcf7`, velysterm `c1e5150` |
| **R15** depth labels | `scripts/testing-depths.md`: per-repo lanes over rea's five depths + named-skip rows; descriptive (the gates enforce claims; the catalog says which lane a green licenses). Mirrored. | emthin `681f74d` |
| **federation P5** | Mechanical sweep of australVM + timepiece scripts against AGENTS/README: exactly two undocumented surfaces (`check-todo-ledger`, `pending-lemmas.txt`), both now documented; re-sweep clean. | australVM `f829dea1`, timepiece `4168c9f` |

**CI-resolution note (2026-10-17, PLAN_valen P2/P3 batch).** The aggregate gate was run red mid-batch (**30/2/0**) and is green again at the final tree (**32/0/0**); both causes drifted *after* the S2 boundary above, and both were repaired rather than waived:

1. `workspace/check-backup-sync` — PLAN_valen's P0 added the Valen row to the root `ATTRIBUTION.md` (10-16 15:51) without `scripts/sync-workspace-backup`, leaving the byte-identical mirror in `emthin/workspace-backup/` stale. Synced (emthin `64193dd`). CI nuance recorded so the two dates are not confused later: australVM's linkcheck *curls that mirror from `emthin/main`* and only needs the file to exist to resolve `../ATTRIBUTION.md`, so that job's health never depended on the row's content; GitHub byte-identity lands with this batch's emthin push.
2. `timepiece/check-links` — 5 broken links, all pointing at `MISSING_FILES_CONTENTS.md`, a run artifact that was never tracked. Root cause was committed, not in any working tree: `DOC_INDEX.md` had been committed with unresolved merge-conflict markers whose HEAD side still advertised that row. Repaired locally with the repo's own `scripts/doc_index.py`, then — after `fetch` showed the owner had pushed their own repair of the same defect concurrently (`fd749d3` series: 0 conflict markers, 0 stale references) — the local commit was **dropped as superseded** before any push, so no shared history was rewritten; `check-links` green on `origin/main` (65 documents, 307 links).

**Resolution map for the standing CI deferrals:** P1's acceptance half ("CI job green needs a push/dispatch") stays push-deferred except where this batch closed it by pushing: australVM `a08b5302` (PLAN_valen P2+P3) + `7a8a8329` (status-block recount), emthin `64193dd` + `7e40006` + the mirror re-sync this note itself requires. `umbrella` has no GitHub remote yet: `origin` is configured (`git@github.com:leonardopedro/umbrella.git`, branch `master`) and the push is one command, but repository creation needs API credentials this host does not carry (no `gh`, no token — SSH authenticates only against repositories that already exist), so the owner creates the empty repo and then `git push -u origin master` completes it. Deviation 2's velysterm format/doc jobs and the D.8 probe record were already resolved at their own trees and are untouched here. australVM's CI surface stays deliberately unchanged — linkcheck only: PLAN_valen added no workflow because P2/P3 acceptance was defined as local gates plus execution tests, and that is what shipped: `verify-invariants` **22/0/0 with zero gaps under the flake devshell** (why3 1.8.2 + alt-ergo are devshell packages; the earlier 18/0/3 came from a bare shell — the Why3 golden parse and the extraction checks now *pass* rather than skip), `check-links` 226 documents OK, `check-todo-ledger` green, `check-status` rc=0 with the block recounted from today's own `dune runtest --force` log (**113 dune / 14 suites, 2026-10-17**), bridge lib + integration tests, OUnit `JitTest`, `test-programs/runner.py` full suite, umbrella granted/denied runs.

## 9. Next workstream (owner-assigned, 2026-10-15)

**emthin + mathed must become cheaply and reliably debuggable by an LLM** — low RAM, low token cost — so an LLM can edit a document (including app windows through Wayland) and make it look nice graphically and functionally. Reference discipline: `rea/`. Then use that debuggability to drive the app's documented goals (emthin `docs/AGENT_LOOP.md`, velysterm `PLAN_parallel_velysterm.md` goal statement, mathed crate docs) to completion. Planned shape (refined after the rea/ study): a headless deterministic document-replay harness (feed an LLM a compact doc-model dump + scenario script, get back an edit script), a snapshot/diff surface that renders "what changed" in a few hundred tokens, and reuse of the existing E2E nested-session machinery as the acceptance lane. Executed after S2.

### §9 execution log (2026-10-15, continuation — S3)

Study first, then built. `rea/` contributed two disciplines carried into the surfaces: the **five behavioral depths** (already adopted by R15's `scripts/testing-depths.md`, now extended with the new lanes) and the **closed-vocabulary scenario contract** (a declarative scenario admits a fixed verb set; an unknown verb fails rather than silently passes — replay.sh's rule, and its negative test proves it: `frobnicate` → rc=1).

| piece | landed | evidence |
|---|---|---|
| **doc-model dump (tier 1b)** | velysterm `inspect::semantic_facts` → `mathed_mini --semantic-json`: defs + use counts (the `plan_rename` count) + source lines, free identifiers (deduplicated; argument positions excluded — `$norm(x)$` reports `norm`, not dummy `x`), models, statement counts, translators, templates. **No Typst compile** — it reports the `$foo$` that tier 1 honestly refuses to render (measured: tier-1 exits 1 on that doc, tier 1b exits 0). Implemented from the uncommitted WIP tests found in the tree (they did not compile — the asserted function did not exist). | velysterm `ca2ce45` (9 tests: 4 semantic + 5 diff) |
| **snapshot/diff** | `inspect::docs_diff` → `--diff <before> <after>`: figures added/removed/changed (caption-first matching survives the `figure_key` renumbering an insertion causes; key-matching catches a caption edit as one change), page + diagnostic + semantic deltas; a compile break errors *naming the side*, never a silent removal. Surfaced as `preview.sh --diff`. | velysterm `ca2ce45`, emthin `a246b10`; measured 0.03 s / ~526 B |
| **replay harness** | `emthin/scripts/agent/replay.sh` — line-based scenarios with a closed vocabulary (`doc`/`apply`/`expect`; patch or full-document edit scripts), sandboxed, transcript on stdout (`^{` = data), sandbox kept on failure, scenario-relative paths. Demo scenario: 13 asserts green in ~0.8 s; three negative paths verified rc=1 (wrong assert with got/want, unknown verb, compile-breaking edit surfaced as "after-doc failed to compile"). | emthin `a246b10` (`scripts/agent/scenarios/figure-resize.{txt,patch}`) |
| **acceptance lane reuse** | CI `e2e` job runs the replay step *first*, then the session E2E; lanes labeled in `scripts/testing-depths.md` (replay = boundary, E2E = acceptance). | emthin `0e9efa2`, workspace testing-depths |
| **docs** | AGENT_LOOP.md Tier 1b/delta/Replay sections + measured cost table (0.02/0.03/0.8 s, sizes); SKILL.md loop steps renumbered; velysterm + emthin AGENTS pointers. | emthin `a246b10`, velysterm `ca2ce45` |
| **goal closure** | emthin REWRITE_PLAN "Known gaps": **§6 step 7 unverified → RESOLVED** (5/5 ×5, CI), deviation kept in place as historical text; residual = IME-commit leg (host fcitx) as a *named skip*. Part-2 "Still open": the nested session now exists, so the only true residual is asserting the `\formal` badge (a task, not a limitation). W11 row updated. | emthin `442d1ad` |
| **recount** | velysterm 986 → **995** (the 9 new inspect tests) — block, README headline and per-crate prose updated from this run's own log; `--recount` gate: `995 tests, matches the block`, rc=0. | velysterm `ca2ce45`, `/tmp/velysterm_tests.log` |

**Deviations found this continuation (claim → measured → fix):**

1. §0 says `fock_sirk/tests/mass_gap_campaign.rs`'s control-bound fix is *uncommitted* and excluded from commits. Measured: the **owner committed it themselves** at 14:23 (`305f330`, part of the documented `mm` one-letter series, before this continuation ran). Not my commit, not my verification: the exclusion still held on my side (never staged, mass-gap suite never run); no history was rewritten.
2. velysterm's **CI `format` and `doc` jobs were red at HEAD** — found by running each job's *exact* command locally (the pinned 1.99.0 rustfmt both sides): `format.rs` failed rustfmt (owner's 10-14 save-stamp work), and 17 intra-doc links across mathed_core/kernel_client/mathed_mini had rotted. Both fixed mechanically and verified green (`rustfmt --check $(git ls-files '*.rs')` rc=0; `cargo doc --workspace --all-features --no-deps --document-private-items` with `-D warnings` rc=0). | velysterm `d0c61ec`, `0df7dd7`
3. The tree carried **uncommitted WIP tests that did not compile** (`semantic_facts` asserted but absent) — the workspace count could not have been produced from that state; resolved by implementing the function (row 1 above).
4. R15's catalog said velysterm 986 — stale after this work; updated to 995 with a lane note.
5. **Suite observation (counted, not hidden):** `mathed_mini::kernel_bridge::tests::rich_media_annotations_are_inline_thumbnails` (stub-kernel subprocess, 15 s settle) failed in one full-workspace `-j 2` run of the final battery and passed on immediate rerun, in isolation, and in the earlier full run — a load-sensitive intermittent of the unfer_ffi class, recorded here rather than papered over. The verified 995 comes from the passing run the recount gate accepted; the failing run was refused by that same gate ("a broken run cannot evidence a count").

   **Root-caused and fixed 2026-10-16 (continuation).** Not load
   sensitivity but two probe-proven races on the stub seam:
   (1) **name collision + truncate-on-open** — the stub name is pid +
   content-hash + nanosecond nonce; measured on this host the clock's
   minimum gap is 30 ns and 1M concurrent builds yield **3,752 identical
   paths**, so a sibling test's non-exclusive `File::create` could rewrite
   the stub mid-`exec`;
   (2) **inherited write-fd at exec** — a `fork()` inside the µs write
   window inherits the write-fd, so `execve` fails with ETXTBSY although
   the creator already closed and *no second opener exists* (the holder
   inherits, never opens — which is exactly why D.8's strace and `/proc`
   scans found nothing, and why the holder exits inside every observation
   window): A/B probe **0/8000 failures without forking neighbors,
   110–132/8000 with**.
   Fixed at the producing boundaries with repo precedents only: exclusive
   stub creation (`create_new(true)` + suffix advance, the
   `export::stub_rules_bin` pattern) and a bounded ETXTBSY spawn retry
   with backoff on the worker's kernel launch (the D.8 retry on the other
   seam; non-ETXTBSY errors never retried) — velysterm `a0b1776`.
   The regression test forces the collision (nine callers, one base name)
   and is mutation-proven: restoring truncate-on-open fails it
   deterministically (all nine paths identical), the current code passes.
   **Verified**: retry probe 0/8000 with forkers; **1,160/1,160** test
   executions across repeated full-suite runs at the parameters that
   failed 3× in ~1,280 before; workspace **996 / 0 failed**, `--recount`
   gate green (**995 → 996**, +1 = the regression test); fmt / clippy
   `-D warnings` / `doc -D warnings` / `--no-default-features` all green
   at the final tree. The D.8 residue in
   `PROJECT_REVIEW_AND_IMPROVEMENT_PLAN.md` is updated to match.

**Cross-batch note (2026-10-17):** the PLAN_valen P2/P3 batch ships inside australVM plus the new `umbrella` sibling repository and logs its CI state in the §8 note above; S3's ordering, scope and acceptance are unchanged by it.
