# Seven-Repo Cross-Project Review & Improvement Plan — 2026-10-13 (REA round 2)
**Date:** 2026-10-13 · **Scope:** `unfer/`, `australVM/`, `velysterm/`, `timepiece/`, `dynamic-arctic/`, `test/`, `emthin/`
**Reference study:** `rea/` — REA (`rea-agents` v6.4.0, MIT, local checkout) — **round 2**: obligation ledgers, harm-ranked error-copy audits, testing depths, admission criteria.
**Predecessor:** `PLAN_CROSS_PROJECT_REVIEW_2026-10-12.md` (round 1, executed S1+S2: 28 gates green). This document does not restate delivered items; it re-reviews the *measured* state of the federation and layers the second REA study on top.
**Hard constraint (unchanged):** no direct Lean 4 code. Lean work is specified as contracts/work orders for the LLM-Lean4-specialist. Improving an existing feature beats creating a new one; new mechanisms are marked **(N)** with justification.

---

## 0. Executive summary

Round 1 built the gate culture (28 gates, `check-all-gates`, catalogs, scenario contracts, error codes, changelogs, TODO ledgers). Round 2's review **ran every repo's own suite for the first time since those claims were written** and found the culture's soft spot: **status claims drift silently because nothing runs their verifier**. Measured today:

| repo | claim (`README status:`) | measured today | verdict |
|---|---|---|---|
| `unfer` | 1792 cargo (2026-10-12) | **1792 / 0 failed** | ✅ honest |
| `emthin` | 310 cargo + 5 E2E (2026-10-04) | **345 / 0 failed** | ❌ stale (+35) |
| `velysterm` | 935 cargo (2026-10-08) | **976 / 0 failed** | ❌ stale (+41) |
| `dynamic-arctic` | 66 cargo (2026-10-05) | **74 / 0 failed** | ❌ stale (+8) |
| `australVM` | 110 dune / 14 suites (2026-10-04) | blocked: bridge `.so` deleted by the 2026-10-09 disk cleanup → `dune runtest` cannot link; `make bridge` rebuild in progress | ⛔ environmental |
| `test` | 66 pages / 311 links (2026-10-12) | **66 / 311, pass** | ✅ honest |
| `timepiece` | "unverified \| not run" | not run (Lean lane) | ✅ honest marker |

`check-all-gates`: **28 passed / 0 failed / 0 gaps**. All suites are green — no regressions anywhere. The three stale claims are exactly what the 2026-10-09 plan's P1 predicted: `check-status --recount/--run` exists but only unfer's CI feeds it a log, and the *root* `scripts/check-status` is a stale fork of unfer's copy (no `--run`). REA's round-2 docs give the missing pieces: **obligation ledgers** (a claim closes only via a named verifier; unknown stays unknown), **harm-ranked error-copy audits** (the E5 contract pinned error *codes* but never audited user-visible *copy*), and **testing-depth lanes**.

Five measured doc/code drifts were also found and are fixed in this round's execution: velysterm AGENTS.md "36 ops" vs the generated 61-op catalog; emthin AGENTS.md a swallowed heading + an undocumented `verify-invariants`; timepiece AGENTS.md documenting the `check-mass-gap-spec-parity` script that S2 deleted; `unfer/result-1` polluting git status; and the workspace-wide **uncommitted S2 deliverables** (36 untracked + ~30 modified files across 6 repos — all gates green, none committed).

---

## 1. System map

Unchanged from round 1 §1 (see `PLAN_CROSS_PROJECT_REVIEW_2026-10-12.md`). Sizes this round: unfer 142.7k rs lines / 301 files · velysterm 63.1k / 95 · emthin 23.7k / 69 · australVM 14.0k / 27 (+120 OCaml) · dynamic-arctic 3.3k / 11 · timepiece 3.0k rs + 1202 Lean · test 66 prose pages. Zero `TODO/FIXME/HACK` markers in code anywhere (TODO ledgers hold).

## 2. Deep review — per folder (2026-10-13, measured)

### 2.1 `unfer` — the kernel
- Suite: 1792 passed / 0 failed (`--workspace --exclude fock_sirk --locked`), matching the block exactly; the X2 recount path is the only one in the federation that closes this loop, and it shows.
- `heavy-tests.yml` (S1) is well-formed; voice.sh tagged FAIL contract (S2 R1b) in place; TODO ledger + changelog present.
- Leftovers: `result-1` → nix store piper-voice symlink, untracked and **not gitignored** (`.gitignore` has `result` only) — pollutes `git status`. `scripts/check-attribution`, `unfer_protocol/src/ops.rs` (the S2 `agent_dm_read` registration fix!), `voice/*` edits and the S1/S2 artifacts (heavy-tests.yml, CHANGELOG.md, cliff.toml, TODO_LEDGER, check-todo-ledger) are all **uncommitted**.

### 2.2 `australVM` — verified compiler/VM
- The 2026-10-09 137 GB disk cleanup deleted `safestos/cranelift/target/`, which contained the **bridge `.so` the OCaml link step needs** (`-laustral_cranelift_bridge`). `dune runtest` now fails at link (`ld.bfd: cannot find -laustral_cranelift_bridge`) — the suites themselves are fine (each ran OK before the link error surfaced at the end). Recovery: `make bridge` (AGENTS.md documents it; the README's Verification block documents the `LD_LIBRARY_PATH` half). Rebuild executed this round; dune recount follows.
- modhost has strict celld CLI contracts (`--help`, `--json` NDJSON, `kind`-tagged error events, exit 0/1/2) — the model surface for R-adaptation of error copy.
- Leftovers: S2 artifacts (CHANGELOG.md, cliff.toml, TODO_LEDGER, check-todo-ledger, AGENTS.md edit) uncommitted.

### 2.3 `velysterm` — UI / AI interface
- Suite: **976 passed / 0 failed** under `nix develop -c cargo test --workspace` (25 suites incl. Bevy `mathed`, doc-tests, `mathed_skia`, delta GPU crates with their graceful SKIP, and the undocumented `mathed_snapshot_scratch`). Block claims 935 (2026-10-08) — stale.
- **Drift found:** `AGENTS.md` says "`AGENT_OPS` (36 ops …)"; the generated catalog says **61 ops**; `check-op-catalog` verifies registry↔dispatch↔catalog but not the prose. Historical plan docs (24/47 ops) are period records and stay.
- `mathed_snapshot_scratch` is a `crates/*` workspace member with passing tests but **no documentation anywhere** (AGENTS/README/docs silent) — owner decision needed: document it or exclude it.
- 29 dirty/uncleaned files: the S2 work (op catalog gate, SCENARIOS, MEGAFILE_DECOMPOSITION, AGENT_LOOP, android scripts + crates) is complete and gated but **uncommitted**.

### 2.4 `timepiece` — Lean corpus + book
- Status marker honestly says "unverified | not run" — the model for unverifiable claims.
- **Drift found:** AGENTS.md's scripts table still documents `check-mass-gap-spec-parity` (deleted in S2 as a byte-identical, cwd-broken fork of unfer's gate) and omits the S2-added `check-todo-ledger`. `prompts` carries an owner edit (deliberate, left alone). `pending-lemmas.txt` (103 lines) + the three-rule contract verified working in S2.
- Lean tree not reviewed here (specialist's lane); non-Lean gates green.

### 2.5 `dynamic-arctic` — threshold signing
- Suite: **74 passed / 0 failed** (`--all-features --locked`); block claims 66 (2026-10-05) — stale.
- Cleanest repo: verify-invariants (S2), bench usage-validation, upstream attribution. `scripts/verify-invariants` still untracked (uncommitted).

### 2.6 `test` — prose site
- `check_site.py`: 66 pages / 311 links, SUMMARY consistent, no broken links — matches the block. 17 no-outgoing-link pages reported (warning, by design). Nothing to do this round.

### 2.7 `emthin` — document-as-desktop compositor
- Suite: **345 passed / 0 failed** (9 suites: emthin lib 271 + xwayland 15+5 + clipboard 14 + dbus 40); block claims 310 (2026-10-04, pre-voice/error-contract tests) — stale.
- **Doc defect found:** `AGENTS.md` line 97 — `…`docs/ipc.md`.## Agent loop` — a missing newline swallows the `## Agent loop` heading into the previous paragraph (renders as inline text). Also the Protocol gates list omits the repo's own `scripts/verify-invariants` (it exists, runs in `check-all-gates`, and is documented only in the root README).
- Error contract (E5) is code-complete; the wire still carries only `code`+`message` — the doc explicitly reserved `retryable`/`remediation` as an additive extension (this round implements it, N2 below).
- All S1/S2 work uncommitted (10 modified + 6 untracked).

## 3. Cross-cutting findings (round 2)

1. **Claim→verifier loop is open in 5 of 7 repos.** `check-status --run` can close it locally but nothing invokes it; no repo but unfer feeds `--recount` in CI; the root `scripts/check-status` is a stale fork (no `--run`), a duplicate of the class S2 already deleted once (`check-mass-gap-spec-parity`).
2. **Prose counts are unguarded.** unfer's H1 gate bans hand-maintained counts in `EXPECTED_SYMBOLS.txt`; the same doctrine does not cover AGENTS/README prose (velysterm "36 ops" shipped stale).
3. **Error copy is unaudited.** Codes are a closed set; the human-facing strings behind them (emthin IPC messages, voice FAIL reasons, modhost errors, harness errors) were never reviewed for harm/recovery guidance — REA keeps a harm-ranked ledger for exactly this.
4. **The disk cleanup had a second-order effect** on australVM (bridge `.so`) that no gate flagged until a human ran the suite.
5. **S2 deliverables are uncommitted workspace-wide** — cross-repo CI (which clones siblings from GitHub `main`) cannot see any of it; the session-hygiene rule added in S2 applies but was not applied to S2 itself.
6. **Two OOM events this session** from parallel `cargo test` on the 14 GB box — suites must run serially (`-j 2`); nothing documents this.

## 4. REA reference study — round 2 (new docs, not adapted in round 1)

| # | REA pattern (doc) | Federation target | Kind |
|---|---|---|---|
| R13 | **Obligation ledgers** (`reconstruction-obligation-ledgers.md`): a claim closes only when a named owner supplies fixtures *and* a verifier of comparable authority; contradictions/unknowns keep it open; absence of evidence never claims closure | Status blocks + scenario rows become obligations: `check-status --recount` = the comparable-authority verifier; unverifiable suites (dune, Lean) stay explicitly "unverifiable", never silently asserted. Implemented via F1+F2 below | improve existing |
| R14 | **Harm-ranked error-copy audit** (`user-visible-errors.csv`): every user-visible error string reviewed for leakage/risk with a proposed replacement and verification status | Workspace error-copy audit over emthin IPC messages, voice.sh FAIL reasons, modhost errors; worst offenders fixed at the producing boundary (reusing E5's category/remediation discipline) | improve existing |
| R15 | **Testing depths** (`testing.md`): module/composition/boundary/acceptance/conformance lanes; "report unavailable authority as a named skip"; prune tests that mirror implementation details | Each repo's AGENTS.md Verify section already names its lanes informally; formal depth labels deferred (see §10 deferred) — the gap culture (`[gap]`, exit 2, SKIP) already implements the "named skip" half | partially present |
| R16 | **Error-contract schema** (`error-contract.schema.json`): `retryable` + `remediation.action` ride the wire, validated against a closed code set | emthin error notification gains `retryable`/`remediation` fields (the doc reserved exactly this as additive); a unit test pins code↔(retryable,remediation) in both directions | improve existing (N2) |
| R17 | **Admission criteria** (`roadmap.md`): new capabilities must return evidence through shared workflows, preserve observations/unknowns, declare authority limits | Root `scripts/README.md` "Adding a gate" gains an admission checklist (evidence command, honest-skip declaration, catalog/error-copy rows) | improve existing |

### Explicitly NOT taken (round 2)
- REA's CLI/MCP dual-adapter parity machinery — the federation's three machine surfaces (NDJSON agent, JSON-RPC IPC, modhost) are different protocols, not two adapters of one surface; catalog-sync gates (round 1) already cover the drift risk.
- REA's per-provider verification lanes — no federation repo substitutes engines behind one interface.
- No Node/TS tooling; patterns are realised in the existing bash/python/Rust gate style. No source vendored (MIT would allow attribution-licensed copying; workspace policy #1 forbids copying anyway).

## 5. Findings-fix list (this round's execution items)

- **F1 — recount the stale claims.** Update README status blocks from today's measured logs: emthin 310→345, velysterm 935→976, dynamic-arctic 66→74, unfer date-bump (re-verified today), test date-bump; australVM after the dune recount. Verify each with `check-status --recount REPO=LOG` against the captured suite log before writing.
- **F2 — close the loop in CI.** Wire `check-status --recount` into the CI test jobs of emthin and velysterm (both already clone unfer as siblings → the script is present; tee the suite log, then recount). dynamic-arctic's CI is standalone: inline the 4-line cargo-log sum with a cite-comment pointing at the canonical parser (small, documented duplication) or skip with a recorded reason — decided during execution.
- **F3 — kill the root check-status fork.** Replace `scripts/check-status` with a thin delegating wrapper to `unfer/scripts/check-status` (the CI-wired, `--run`-capable copy). Doc references stay valid; the fork logic dies.
- **F4 — velysterm op-count prose.** AGENTS.md: drop the literal "36 ops", point at the generated catalog. Extend `check-op-catalog` with a prose guard: any `N ops` in velysterm `AGENTS.md`/`README.md` must equal the registry count (unfer H1 doctrine applied workspace-wide); prove non-vacuous by mutation.
- **F5 — emthin AGENTS.md repairs.** Restore the `## Agent loop` heading (missing newline); add `verify-invariants` to the Protocol gates list.
- **F6 — timepiece AGENTS.md sync.** Remove the deleted `check-mass-gap-spec-parity` rows (script table + copy-pasteable block); add the `check-todo-ledger` row.
- **F7 — unfer gitignore.** Add `result-1` (nix build symlink class: `result*`).
- **F8 — australVM recount.** After `make bridge`: `dune runtest --force` → recount 110/14 or correct the block; note the cleanup recovery in the README Verification block if not already covered.
- **F9 — OOM/serial note.** Root `scripts/README.md`: suites run serially on this box (`-j 2`), parallel cargo test OOMs (two events recorded 2026-10-13).
- **F10 — commit hygiene.** Per-repo conventional commits of the verified S1/S2+state (no push, per every AGENTS.md), so cross-repo CI finally sees the gates. Owner-decision files (`velysterm/mathed_snapshot_scratch`, `mathed-screenshot.png`, `timepiece/prompts`) are excluded and noted.

## 6. REA-adapted execution items

- **N2 (R16) — emthin wire-level error semantics.** `OutgoingMessage::Error` serializes `retryable` + `remediation` derived from `IpcErrorCode`'s documented table (additive; existing clients parse `code`/`message` unchanged). Unit test pins code↔(category,retryable,remediation) against `docs/error-contract.md` in both directions (same bidirectional pattern as `the_documented_error_codes_are_the_only_codes`). Docs: `error-contract.md` wire section + `ipc.md` error row updated; `check-ipc-docsync` must stay green.
- **N1 (R14) — error-copy audit.** `ERROR_COPY_AUDIT.md` at workspace root: harm-ranked table (location, trigger, current copy, risk, replacement, status) over emthin IPC messages, voice.sh FAIL reasons, modhost errors. Fix at the producing boundary any copy that leaks internals, omits recovery, or could ride a credential (reusing round 1's redaction policy). Re-audit rule added to `scripts/README.md` admission checklist.
- **N3 (R17) — admission criteria.** `scripts/README.md` "Adding a gate" → "Adding a gate or feature": evidence command required, honest-skip declared, catalog/error-contract/error-copy rows updated in the same change, no hand-written counts in prose (F4 guard).

## 7. Lean4 delegation boundary (unchanged)

Everything that writes or refactors Lean 4 stays with the LLM-Lean4-specialist: `timepiece/Book`, `BookProof`, `PnpProof`, `RandomMap`, `logos` outputs, `test/` theorem pages. Contract: pinned `lean-toolchain` + mathlib v4.28.0; acceptance = `axiom-gate` + `verify-invariants` + `check-book-sync` + the `pending-lemmas.txt` three-rule contract; no `sorry`/`unsafe` in merged proofs. The 88 `[MISS lean]` citations remain listed as pending obligations (the R13 discipline already applied there: pending ⇒ listed, unlisted miss ⇒ fatal).

## 8. Copyright & compliance

REA is MIT (verified `rea/LICENSE`, © 2026 morluto). Round-2 adaptations (obligation-ledger closure semantics for status claims; harm-ranked error-copy audit format; testing-depth labels; admission criteria; wire-level `retryable`/`remediation` shape) are **ideas, not code** — no source vendored. The `ATTRIBUTION.md` ledger row for REA (added in S1) already covers "the tagged error contract … generated catalogs … scenario contracts … CI layering … credential redaction"; the round-2 claims in N1–N3 resolve to the same row (policy #4, enforced by `check-attribution`). No other ledger rows change.

## 9. Sequencing & acceptance

| stage | items | acceptance |
|---|---|---|
| S3a (this session) | F1–F10, N1–N3 | each item's own check green; `check-all-gates` exit 0 at the stage boundary; emthin suite green after N2 |
| S4 (specialist) | Lean items per §7 | axiom-gate + verify-invariants + drift contract green |
| deferred (recorded, not executed) | R15 formal depth labels per repo; heavy-lane scheduling for dynamic-arctic `repro_fig7a`; `mathed_snapshot_scratch` disposition | owner decision + a future round |

Global invariant: `bash scripts/check-all-gates` must exit 0 at every stage boundary. Suite runs on this 14 GB box are **serial** (`-j 2`); never parallel `cargo test` (F9).

## 10. Execution log (S3a, executed and verified 2026-10-13)

All F-items and N-items executed; every check run green. Commits are local
(no push, per each repo's AGENTS.md).

- **F1 — status recounts.** Measured every cargo-shaped suite serially
  (`-j 2`): unfer 1792/0, emthin 345/0, velysterm 976/0, dynamic-arctic
  74/0, australVM dune 110 tests/14 suites/0 failures (after the bridge
  rebuild below). Blocks updated; each verified through the canonical
  `check-status --recount REPO=LOG` before writing (emthin 310→345,
  velysterm 935→976, dynamic-arctic 66→74, australVM date-only bump —
  its count claim was honest, only the date was stale).
- **F2 — CI recount wiring: deferred with reason.** emthin/velysterm CI
  clone siblings from GitHub `main`, and the S2 gate work was itself
  uncommitted until F10 — wiring `--recount` into those jobs before the
  commits landed would have run the new parser against old checkouts.
  Revisit next round now that all six repos are committed.
- **F3 — root check-status fork removed.** Replaced with a 3-line
  delegating wrapper to `unfer/scripts/check-status` (the CI-wired copy
  with `--recount`/`--run`). Verified: presence sweep rc=0 and
  `--recount dynamic-arctic=…` reachable through the wrapper.
- **F4 — op-count prose guard.** velysterm AGENTS.md's "36 ops" dropped
  (registry has 61); `check-op-catalog` gains `prose_op_counts()` —
  any `N ops` in AGENTS.md/README.md must equal the registry count.
  Proven non-vacuous by mutation both ways: plant "60 ops" → RC=1 with
  a named line; restore → RC=0. (Historical plan docs and CHANGELOG are
  out of scope by design — period records.)
- **F5 — emthin AGENTS.md.** Swallowed `## Agent loop` heading restored
  (missing newline); `verify-invariants` added to the Protocol gates
  list. check-links green.
- **F6 — timepiece AGENTS.md.** Deleted-gate rows removed from the
  scripts table and the copy-pasteable block (with a pointer to the
  canonical unfer copy); `check-todo-ledger` row added.
- **F7 — unfer .gitignore.** `result-*` class ignored (the `result-1`
  piper-model symlink polluted git status); `git check-ignore` proves it.
- **F8 — australVM bridge.** `make bridge` rebuilt the `.so` the 2026-10-09
  disk cleanup deleted; `dune runtest --force` → 110/14/0; date bumped.
- **F9 — OOM note.** Root scripts/README.md documents the 14 GB serial-
  suite rule (two parallel workspace builds OOM-killed this box twice
  today; `-j 2` for the big workspaces, gates are cheap).
- **N1 — error-copy audit.** Root `ERROR_COPY_AUDIT.md`: harm-ranked
  ledger over emthin IPC messages, voice.sh FAIL reasons, modhost
  errors. **Zero rank-1 findings** (S2's producing-boundary redaction
  holds); the one structural rank-3 (recovery text doc-only) closed
  itself via N2; the `voice.sh` llm excerpt stays a documented,
  deliberate rank-2 (bounded 300 B, redacted URL, splice target is the
  user's own document). Attribution claim resolves (check-attribution
  255 hits, 0 unresolved).
- **N2 — wire-level error semantics.** emthin `error` notification now
  carries `category`/`retryable`/`remediation` (additive; old clients
  unaffected). `IpcErrorCode` gains the three accessors; two new pin
  tests: `the_documented_error_semantics_match_the_wire_fields`
  (cell-for-cell, byte-equal remediation, both directions) and
  `error_notification_carriers_the_semantic_fields` (wire JSON shape).
  Suite: 273 passed / 0 failed (271 + 2 new). Docs updated;
  `check-ipc-docsync` green.
- **N3 — admission criteria.** Root scripts/README.md "Adding a gate"
  extended: evidence command + recount, honest-skip declaration,
  catalog/contract rows in the same change, error-copy row, scenario
  row.
- **F10 — commits (local, no push).** Six repos committed in
  conventional style: unfer `e00b929`, australVM `3830e640`, velysterm
  `ffc16d1` (42 files), timepiece `1b5d79c`, dynamic-arctic `d88eb54`,
  emthin `715c6a4`. Owner-decision files deliberately left untracked:
  `timepiece/prompts` (owner edit), `velysterm/crates/mathed_snapshot_scratch/`
  (passing tests, undocumented) and `velysterm/mathed-screenshot.png`
  (unreferenced binary); `.freebuff`-class tool artifacts untouched.

### Stage-boundary verdict (S3a)

`check-all-gates`: **28 passed / 0 failed / 0 gaps**, rc=0, after all
commits. Every new gate was proven non-vacuous by mutation before
trust. No check weakened, skipped or suppressed. Deferred (recorded):
F2 CI wiring (now unblocked), R15 depth labels, repro_fig7a scheduling,
`mathed_snapshot_scratch` disposition.
