# Seven-Repo Cross-Project Review & Improvement Plan — 2026-10-18 (round 4)

**Date:** 2026-10-18 · **Scope:** `unfer/`, `australVM/`, `velysterm/`, `timepiece/`, `dynamic-arctic/`, `test/`, `emthin/`
**Predecessor:** `PLAN_CROSS_PROJECT_REVIEW_2026-10-15.md` (round 3, executed: 32 gates green at its boundary). This round re-measures the *whole* federation from a cold shell, fixes what the gates caught on arrival, and executes the deferrals that are still open.
**Reference discipline (this cycle):** the workspace's own **claim→verifier** culture, exercised the hard way — this review ran every suite from scratch rather than trusting round 3's numbers, and the two gate failures it found on arrival were both *document-structure* defects the culture exists to catch. No new external reference is studied this round; the round-3 `rea/` five-depth lanes (`scripts/testing-depths.md`) remain the label vocabulary and are extended, not replaced.
**Owner scope cuts (unchanged from round 3):** (1) the **mass gap** stays out of scope — no mass-gap item is executed, verified or recounted here. (2) **No Lean 4 code is written**; `timepiece`'s Lean lane stays honestly "unverified | not run" and its 86 pending lemmas remain the specialist's obligation ledger (§5).

---

## 0. Executive summary

Round 3 built the claim→verifier culture; round 4 **re-ran every verifier from a cold shell** and found the culture's own weak spot: the gates that police *claims* were green while two *documents* had silently rotted — a markdown table that no longer tokenised (so the attribution ledger could not see three of its own rows) and four gate scripts whose shebang would fail for any human who ran them directly. Both were invisible to `check-all-gates` because it dispatches gates through `bash` rather than their shebang, and because `ledger_tokens` fails *closed* (a row it cannot parse simply contributes no token). The measured state of the seven repos is otherwise **honest**: five of seven status blocks matched their re-measured counts exactly, and the one that did not (`australVM`, 113→134) had drifted for a documented reason (two test suites landed 2h after the last recount).

Two whole suites were re-verified end-to-end that no gate covers: the **`avm-pkg`** crates (108 tests) and the **`umbrella`** demo (granted/denied paths), both shipped 10-17/18. One *undocumented* suite was discovered and is now documented: `emthin-pad`'s 20 host tests, which no status block, gate or CI job named.

| repo | claim this morning | measured today (cold) | verdict → action |
|---|---|---|---|
| `unfer` | 1797 cargo (10-15) | **1797 / 0 failed** | honest → date bumped |
| `fock_sirk` (release lane) | 401 across 56 binaries (10-06, dated) | **462 / 0 failed, 64 binaries** | dated claims fine; the *undated* R15 catalog row recounted |
| `emthin` | 347 cargo + 5/5 E2E (10-15) | **347 / 0**; E2E **5/5**; replay **13/13**; `emthin-pad` **20 / 0** | honest → date bumped; pad lane documented |
| `velysterm` | 996 cargo (10-15) | **996 / 0 failed** | honest → date bumped |
| `dynamic-arctic` | 74 cargo (10-13) | **74 / 0 failed** | honest → date bumped |
| `australVM` | 113 dune / 14 suites (10-17) | **134 dune / 16 suites**, +108 `safestos/pkg`, +129 bridge, +311 `runner.py` | **stale (+21/+2)** → recounted; cargo surfaces added to the block |
| `test` | 71 pages / 382 links (10-15) | **71 / 382, pass** | honest → date bumped |
| `timepiece` | unverified, not run (10-04) | gates green (6/0, `lake` gap); Lean lane not run | honest, unchanged (§5) |

`check-all-gates`: **32 passed / 0 failed / 0 gaps** at the review's close, **33 / 0 / 0** after P6 added a gate (§8). `check-status --recount` green for all four cargo-shaped suites against today's own logs.

## 1. What the gates caught on arrival (both fixed before any review work)

### 1.1 `check-attribution` red at *both* levels — a ledger table that stopped tokenising

`check-attribution` failed (workspace and the `unfer`-level copy) with exactly one unresolved hit: `emthin/docs/spectrum-decision-evidence.md:27`, a tier-2 `derived from` claim. The cause was **not** the new document. The root `ATTRIBUTION.md`'s *External sources* table had grown three new rows on 10-18 (**SpectrumOS**, **hermit-rs**, **hermit-c**) that were separated from the table by **blank lines**. `ledger_tokens`'s awk resets `intab` on any non-`|` line, so those three rows contributed **no token at all** — the ledger literally could not see its own newest sources, and a claim citing SpectrumOS could not resolve to it.

This is a *fail-closed* gate behaving correctly against a *document* defect, and it is the same class as round 3's timepiece drift: the machine was right, the prose was wrong. A markdown table with blank-line gaps also does not render as a table, so the defect was visible to a reader as three orphan paragraphs. Fixed by deleting the three blank lines (the producing boundary, not the gate): **98 ledger resolutions, 0 unresolved**. No allowlist entry was added — an exemption would have hidden three real ledger rows from every future scan.

### 1.2 Four workspace gates and three velysterm scripts would not run for a human

`scripts/{check-backup-sync,check-error-catalog,check-scenario-ledgers,sync-workspace-backup}` and `velysterm/scripts/{build-android,save-conflict-e2e,save-conflict-session}.sh` all used `#!/bin/bash`. This host is NixOS: **`/bin/bash` does not exist**, so a direct `./scripts/check-error-catalog` returned `rc=126` (bad interpreter) while `check-all-gates` reported `[pass]` — it dispatches every gate through `bash`, never the shebang.

That is the exact failure mode round 3's P7 fixed on the *python* gates ("a human's direct run cannot `[gap]` past a real miss"): a gate that is green through the wrapper and dead on direct invocation is green by accident. Fixed to `#!/usr/bin/env bash` (the shebang the working gates already used); direct invocation re-verified for all four (`rc=0`, not 126). The seven-script fix also propagated to the mirror, which is why `check-backup-sync` flagged drift twice during this round and was re-synced both times.

### 1.3 `scripts/README.md`'s gate table was broken mid-table

The "auto-discovery" paragraph sat **between** table rows, orphaning the `check-book-sync` and `check-status` rows out of the table (the same blank-line-interrupts-a-table defect as §1.1, in the file that documents the gates themselves). Repaired, and the two undocumented surfaces (`workspace-backup-files`, `waydroid-sudo-fixes.sh`) — the only two the round-3 federation sweep still leaves unnamed — are now described.

## 2. Deep review — per folder (measured 2026-10-18, cold shell)

### 2.1 `unfer` — 1797/0, honest
`cargo test --workspace --exclude fock_sirk` = **1797 passed / 0 failed / 72 suites**, exactly the block's number. `fock_sirk --release` (the Cadabra2 conformance lane, run separately by design) = **462 / 0 failed / 8 ignored / 64 suites** in 739 s. The README's three fock_sirk prose claims are all explicitly dated `2026-10-06` and sub-scoped, so they stay; the **undated** R15 catalog row ("401 across 56 binaries") was the one bare claim and is recounted to 462/64 with a date. Since round 3 the repo's activity has been CI-only (a dozen `ci:`/`fix(ci):` commits hardening the X7 cross-repo job) — no behavior change, which the unchanged count confirms.

### 2.2 `emthin` — 347/0 + 5/5 E2E + 13/13 replay, honest; **one undocumented suite found**
`nix develop -c cargo test --workspace` = **347 / 0** (matches). The acceptance lane re-ran clean from a cold start: `./scripts/e2e/run.sh --config xdotool` = **5 ran, 0 skipped, all 5 passed** (keyboard path, clipboard both ways, `list_state`, SIGTERM-graceful), and the tier-1 replay scenario = **13 ok / 0 failed** in ~0.8 s. Round 3's stale-state post-mortem is holding: no stale-IPC or leaked-Xvfb failure recurred.

**New find:** `emthin-android/` ("emthin-pad") is a **standalone Android app** (JNI `cdylib` + `MainActivity.java`, gradle-free `build-apk.sh`, own `rust-toolchain.toml`) that embeds `DocModel` verbatim. It carries its own `cargo test` = **20 passed / 0 failed in 0.45 s** — a real, green suite that **no status block, no gate and no CI job named**, because its empty `[workspace]` table keeps it out of the workspace run by design. Now documented: an `# emthin-android` section in `AGENTS.md` (layout, the seam, the four hard-pinned Nix store paths that make the APK build an owner-run lane, and both verification lanes) and two rows in the R15 catalog. The block now names the lane so the next reader cannot mistake 347 for everything in the tree.

### 2.3 `velysterm` — 996/0, honest
`nix develop -c cargo test --workspace` = **996 passed / 0 failed / 25 suites**, exactly the block. Round 3's stub-race regression test (`a0b1776`) is stable across this cold run. Only working-tree noise is the owner's own: a `.gitignore` line for `/.github_bk/` (their workflow backup) and the excluded `mathed-screenshot.png`.

### 2.4 `australVM` — the one stale block, plus two unverified surfaces
`make bridge && make test` = green; `dune runtest --force` = **134 tests / 16 suites** against a block claiming **113 / 14**. Cause found and dated: `test/TotalityTest.ml` + `test/DerivAstTest.ml` landed in commit `652376e6` at **10-17 22:47**, i.e. *after* `7a8a8329` recounted the block at **10-17 10:46**. A same-day recount that missed a late-evening commit — precisely the drift class the P4 guard exists for, and the reason a count claim needs the date it was measured on.

Two Rust surfaces in this repo were green but **claimed by no one**: `safestos/pkg` (`avm-cas`, `avm-eval`, `avm-virtio`, `avm-vmm`, `avm-cli` — the avm-pkg engine shipped 10-17) = **108 / 0 / 5 ignored / 11 suites**, and `safestos/cranelift` (the bridge) = **129 / 0 / 15 suites**. `runner.py` = **311 programs, 0 FAIL**. All three are now in the block and the README's Verification snippet with reproduction commands. The README's narrative "Current Status: Phase 11" was two milestones behind the tree (it predated group borrows, `extern rust` tier 1, the umbrella subfolder and avm-pkg); a "Since Phase 11" section now points at each shipped plan instead of rewriting the Phase-11 history.

`umbrella/` (a plain subfolder since today) has **0 `#[test]`s**; its verification *is* the two demo paths, both re-run cold and matching the README byte-for-byte: granted → `RESULT -1781872834555808882`, denied → `UK-4001 … 'anon' is not granted 'rs_demo_checksum'`, exit 1.

### 2.5 `dynamic-arctic` — 74/0, honest
`cargo test` = **74 / 0**, matching. Untouched this round apart from the date; the weekly `heavy-repro.yml` record lane is untouched by design.

### 2.6 `test` — 71/382, honest
`check_site.py` = **71 pages, 382 internal links, pass**; link graph clean.

### 2.7 `timepiece` — honest, unchanged
`verify-invariants` and `check-gitbook-drift` pass under `check-all-gates` (round 3's python-fallback and sibling-surface fixes still holding). The Lean lane stays `unverified | not run` (§5). The owner's uncommitted `prompts` edit is theirs and untouched.

## 3. Cross-cutting findings (round 4)

1. **A gate that fails closed hides document structure.** Both §1.1 defects were *tables that stopped being tables*, and both surfaced only as an unrelated downstream symptom (an unresolvable claim; an orphaned gate row). `ledger_tokens` cannot be made to fail loudly without breaking the CI shape, so the rule this round adds is procedural: **when a resolver reports one unresolved hit, diff the ledger's own table shape before touching the allowlist.** An exemption would have made the symptom disappear and the defect permanent.
2. **`check-all-gates` dispatches by `bash`, humans dispatch by shebang.** The runner's `interp=(bash)` is correct (it must run a script whose shebang the host lacks), but it means the aggregate gate **cannot see** a broken shebang. Seven scripts were red-by-shebang and green-by-wrapper. The cheap durable fix is the one applied here: uniform `#!/usr/bin/env bash`. A `check-shebangs` gate is proposed in P6.
3. **Cargo-shaped recounting is the only automated count check, by documented choice.** Dune, Lean and the link graph are honest *manual* claims — which is why `australVM`'s drift survived a green gate for a day. The remedy is not a new gate but the block carrying the date it was measured on (now done) and the README snippet naming the command.
4. **A suite can be green, documented nowhere, and outside every count.** `emthin-pad` had 20 passing tests reachable from no status block, no gate, no CI job and no AGENTS section — the round-2 "finding class 5" in its purest form, and found only by listing directories rather than by any verifier. The sweep that finds these is a *manual* habit; nothing automates it.
5. **Commit-subject quality is still degraded** (`mm`, `a`, one-letter) in `australVM`, `timepiece` and `emthin` since round 2. Noted again in P7 as owner hygiene; still not gated.

## 4. P-items (this round's execution)

- **P1 — `check-attribution` red: repair the ledger table, do not exempt the claim.** §1.1. Acceptance: 0 unresolved, no new allowlist entry, both gate levels green.
- **P2 — shebang parity for every directly-invoked script.** §1.2. Acceptance: direct invocation of the four workspace gates returns their real code, not 126; `check-all-gates` still 32/0/0.
- **P3 — recount + honest dates.** All seven status blocks re-measured from today's own logs; `australVM` block and README prose recounted (113→134, 14→16) and its two unclaimed Rust surfaces added; `fock_sirk`'s undated R15 row recounted (401→462); five dates bumped only where a suite was actually re-run. `check-status --recount` must accept all four cargo suites against today's logs.
- **P4 — document `emthin-pad`.** §2.2. AGENTS section + two R15 lane rows + the block naming the lane. Acceptance: the block's pointer resolves; the catalog carries a row in the same change.
- **P5 — repair `scripts/README.md`'s broken gate table and close the last two undocumented-surface rows.** §1.3. Acceptance: the table renders as one table; a sweep finds no unnamed surface.
- **P6 — `check-shebangs` gate (new, cheap).** Make P2 durable: flag any executable script in the seven checkouts or `scripts/` whose shebang names an interpreter that does not exist on the running host *and* is not `env`-mediated, or that disagrees with the interpreter the file actually needs. Mutation-proven (a `#!/bin/bash` must FAIL). Must not fire on the Nix-store-pinned `build-apk.sh` (its shebang is already `env bash`) or on CI-only `/bin/bash` runners — so the rule is "portable shebang" not "no `/bin/bash`".
- **P7 — commit hygiene reminder (observation, not a gate).** Record in the root `scripts/README.md` that `mm`/`a` subjects defeat `git-cliff` changelog generation (australVM's own AGENTS says its changelog is generated from conventional commits); no enforcement added.
- **P8 — commit every measured fix, per repo, no push.** The umbrella→subfolder integration (staged in australVM since this morning) lands here with the rest.

## 5. Lean4 delegation boundary (unchanged)

No Lean 4 code is written this round. `timepiece`'s Lean lane stays `unverified | not run`; the `lake` gap stays a recorded `[gap]`, never a silent pass; `scripts/pending-lemmas.txt` (86 entries) remains the specialist's obligation ledger and is not edited.

## 6. Copyright & compliance

No new external source is taken this round. `check-attribution` green (261 hits: 98 ledger, 25 internal cross-ref, 138 allowlisted, 0 unresolved). The SpectrumOS/hermit rows it could not previously see are now live tokens; no ledger row was added or removed.

## 7. Sequencing & acceptance

| stage | items | acceptance |
|---|---|---|
| S1 (review, done in this document) | P1, P2, P5 (the three on-arrival repairs) | `check-all-gates` **32/0/0** after each |
| S2 (measurement) | cold re-run of every suite in §2 | this document's §0 table; `check-status --recount` green |
| S3 (this plan's execution) | P3, P4, P6, P7, P8 | P3: recounts land, gates green; P4: pointer resolves + catalog row; P6: mutation-proven; P8: commits land per repo, no push |

## 8. Execution log

S1–S3 executed 2026-10-18; stage boundary green — `check-all-gates` **33/0/0** (was 32/0/0 at the review's close; P6 added `check-shebangs`), `check-status` rc=0 (7 blocks, four cargo recounts accepted against today's logs, P4 prose guard mutation-proven twice).

| item | landed | evidence |
|---|---|---|
| **P1** attribution | Deleted the three blank lines inside `ATTRIBUTION.md`'s *External sources* table (SpectrumOS, hermit-rs, hermit-c rows) so `ledger_tokens` can tokenise them again. No allowlist entry added. | `check-attribution` 98 ledger / **0 unresolved**; `check-all-gates` both levels green |
| **P2** shebang parity | `#!/bin/bash` → `#!/usr/bin/env bash` in `scripts/{check-backup-sync,check-error-catalog,check-scenario-ledgers,sync-workspace-backup}` and `velysterm/scripts/{build-android,save-conflict-e2e,save-conflict-session}.sh`. | direct invocation `rc=0` (was 126); mirror re-synced (`check-backup-sync` green) |
| **P3** recounts | `australVM` 113/14 → **134/16** (block + README prose + the two new cargo surfaces + `runner.py` in the Verification snippet); `fock_sirk` R15 row 401/56 → **462/64**; dates bumped to 10-18 for `unfer`, `emthin`, `velysterm`, `dynamic-arctic`, `test` (each actually re-run). | `check-status --recount` accepts 74/347/996/1797 against today's logs; P4 guard mutation-proven on two real shaped lines |
| **P4** emthin-pad | New `# emthin-android` AGENTS section (layout, the JNI seam, the four pinned Nix store paths, both lanes) + two R15 rows + the emthin block naming the lane. | 20/0 host tests recorded; block pointer resolves |
| **P5** scripts map | Repaired the mid-table paragraph in `scripts/README.md`; documented `workspace-backup-files` and `waydroid-sudo-fixes.sh`. | table renders as one; no unnamed surface remains |
| **P6** check-shebangs | New workspace gate `scripts/check-shebangs` (X10): every executable script under `scripts/` (root + all seven checkouts) must carry a shebang runnable on the host. `env`-mediated → pass; absolute interpreter **absent** → FAIL rc=1; absolute but present → reported, accepted (CI runners legitimately have `/bin/bash`). Discovered automatically by `check-all-gates` (**32 → 33 gates**). Built portable: this host's bare-shell `find` rejects GNU `--exclude-dir`, so the scan prunes with `-name … -prune` instead (measured failure first, then the fix). | mutation-proven both ways: `#!/bin/bash` → FAIL rc=1; `#!/bin/sh` (present here) → note, rc=0; restored → rc=0 |
| **P7** commit-hygiene note | `scripts/README.md` records *why* `mm`/`a` subjects cost something: australVM's own AGENTS documents its `CHANGELOG.md` as git-cliff-generated, so every `mm` ships without a changelog entry. Observation, explicitly not a gate. | `scripts/README.md` §"Commit subjects" |
| **P8** commits | Per-repo conventional commits, **no push** (owner remote state untouched); the umbrella subfolder integration lands in australVM with the rest. Owner-excluded files (mass-gap diff, `mathed-screenshot.png`, `timepiece/prompts`, `.github_bk`) stay out. | each repo's log |

## 9. Next workstream (recommendation, owner-decided)

Round 3's S3 (emthin+mathed LLM-debuggability) shipped and held under this round's cold re-run, so the cheapest high-value follow-on is **closing the coverage holes this round found rather than starting a new surface**: (a) wire `emthin-pad`'s 20 tests and australVM's `safestos/pkg` 108 into CI so neither can rot silently again (both are one `cargo test` line; australVM's CI deliberately stays otherwise narrow); (b) make `check-shebangs` (P6) real so P2 cannot regress; (c) the owner-side pushes that close round 3's remaining "CI job green" deferrals. The genuinely new direction — australVM's Sprint-8 cloud profile (`docs/spectrum_plan.md`) and the avm-pkg backlog in `PLAN_avm_pkg.md` §5 — is real work but is *product* work, not review work, and belongs to the next owner-assigned cycle.
