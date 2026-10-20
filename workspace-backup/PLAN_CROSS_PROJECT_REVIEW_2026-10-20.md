# Seven-Repo Cross-Project Review & Improvement Plan — 2026-10-20 (round 5)

**Date:** 2026-10-20 · **Scope:** `unfer/`, `australVM/`, `velysterm/`, `timepiece/`, `dynamic-arctic/`, `test/`, `emthin/`
**Predecessors:** `PLAN_CROSS_PROJECT_REVIEW_2026-10-18.md` (round 4, executed; its §8a session left one red gate recorded-not-fixed — W6) and `PLAN_EMTHIN_ANDROID_MATHED_2026-10-18.md` (the android-workstream round, whose P5 was blocked on a root-gated waydroid thaw). This round closes both deferrals and re-measures the federation.
**Reference discipline:** unchanged — the rea/`testing-depths.md` label vocabulary, claim→verifier everywhere, and the round-4 procedural rule "when a resolver reports one unresolved hit, diff the ledger's own table shape before touching the allowlist" applied first.
**Owner scope cuts (unchanged):** (1) the **mass gap** stays out of scope; (2) **no Lean 4 code is written** — `timepiece`'s Lean lane stays `unverified | not run` (§5).

---

## 0. Executive summary

Round 4's review was green-on-arrival-then-red-in-two-places; round 5 arrives to **two recorded deferrals and one flake**, all three now closed at their producing boundaries:

1. **W6 (recorded 10-19, red for a day): `check-attribution` 92 unresolved at both levels.** Round 4 deferred it as "an owner decision about someone else's attribution semantics". This round showed the deferral was narrower than stated: *verifying* the licence terms was ours to do and was one fetch away (prove2.me's own terms page, two GitHub `LICENSE` files), and the fix that follows from evidence is **ledger rows, not exemptions** — three real sources rowed with licences read firsthand, plus two narrowly-scoped allowlist entries for lines that genuinely name no source. **92 → 0 unresolved**, both gate levels green.
2. **The emthin xwayland flake, found by the full-suite run** (not by the gate): `stale_lock_with_corrupt_content_gets_cleaned_up` failed with display 269 where the fixture sat at 267. Root cause: the fixture hashed nanotime into only **1000 display slots**, so two tests in one parallel run could draw the same start and fight over one lock path — a *fixture identity* bug, not a lock bug. Fixed with disjoint per-process slot blocks; 20/20 stress runs + 5/5 full suites green.
3. **The android P5 blocker (root-gated container thaw) — rootless recovery found.** `waydroid container unfreeze` needs euid 0, but the documented `--repair-net` path restarts the **user** session service, which thaws the container in ~25 s without sudo. The device lane now runs end-to-end: e2e all steps green, and a **live-render proof** (inject 9 chars without touching focus → status `rev 1 → rev 10`, preview band 7,054 px re-rendered, ink 255 → 1,187 samples).

`check-all-gates`: **31 passed / 2 failed** on arrival → **33 passed / 0 failed / 0 gaps** at this plan's boundary.

| repo | claim (block) | measured today (cold) | verdict → action |
|---|---|---|---|
| `unfer` | 1800 cargo (10-18) | **1800 / 0 failed, 72 suites** | honest → date bumped |
| `australVM` | 134 dune / 16 suites + 108 pkg + 129 bridge (10-18) | **134 / 16 RC=0** (canonical `dune runtest --force` under `LD_LIBRARY_PATH`), pkg + bridge re-run | honest → date bumped |
| `emthin` | 347 cargo + 25 pad (10-18/19) | **353 / 0**; E2E **5/5**; pad **26 / 0** | **stale (+6/+1)** → recounted (§2.3) |
| `velysterm` | 996 cargo (10-18) | **997 / 0 failed, 25 suites** | **stale (+1)** → recounted (§2.3) |
| `dynamic-arctic` | 74 cargo (10-18) | **74 / 0 failed** | honest → date bumped |
| `test` | 75 pages / 460 links (10-17) | **75 / 460, pass** | honest → date bumped |
| `timepiece` | unverified, not run (10-04) | gates green; Lean lane not run | honest, unchanged (§5) |

## 1. What the arrival state showed (all fixed before review work)

### 1.1 W6 closed: `check-attribution` 92 → 0, and the deferral's framing was wrong

The 92 unresolved hits were 91 in `timepiece` (`Ported from prove2.me …` bundle annotations across four files, plus one `adapted from an external source` boilerplate line) and 1 in `test/unfer/intro.md` (a prose line containing "not a theorem derived from it"). Round 4 recorded the fix as blocked on an owner decision "*about someone else's attribution semantics*".

This round separated the two halves:

* **Verification (ours):** prove2.me's own `terms` page, read firsthand 2026-10-20, states that public contributions — explicitly including Lean code, theorem/definition statements and proofs — are licensed **Apache-2.0**. `PrimeNumberTheoremAnd` and `openai/math` both carry Apache-2.0 `LICENSE` files, read firsthand the same day. `timepiece/converted/README.md` already documents the per-file provenance headers and the Apache-2.0 notice the conversions keep. Nothing here was an opinion about someone else's semantics; it was a fact to be fetched.
* **The decision that remains the owner's:** whether vendored 57 MB of port sources belongs in git at all (`timepiece/Downloads/` + `converted/` are still **untracked** — recorded in §3 finding 6, not changed here).

The fix, at the producing boundary:

* **Three ledger rows** (root `ATTRIBUTION.md` + mirrored in `timepiece/ATTRIBUTION.md`): `prove2.me`, `PrimeNumberTheoremAnd` (tag/commit pinned), `openai/math` (commit pinned). Each carries the licence as *verified from source*, what was adapted, and where it landed. The prove2.me row says plainly that it was **added retroactively — the ports landed without one**, the same honesty the ProofFlow row already carries.
* **Two allowlist entries only**, each excusing a line that names no source: the skeleton-subtraction generator's boilerplate sentence (class 1, exact line content named; the real source is in the file's own line-1 header *and* now rowed), and `test/unfer/intro.md`'s prose disclaimer (class 2 — a reader's interpretation of a cited paper that explicitly denies deriving a theorem from it).
* The `unfer/scripts/check-attribution` copy and the `emthin/workspace-backup/` mirror were re-synced in the same change (`check-backup-sync` green), per round 4's P9.

Resolution counts: **372 hits — 206 ledger, 25 internal cross-ref, 141 allowlisted, 0 unresolved**. The round-4 table-shape rule was checked *first*: both tables were intact; the rows simply never existed. Ledger rows were added before — not after — any allowlist entry.

### 1.2 The flake was a fixture-identity collision, not a lock bug

The full-suite background run failed `stale_lock_with_corrupt_content_gets_cleaned_up` (`left: 269, right: 267`): the corrupt-lock fixture had written `start`, but by the time `setup_connection` scanned, *another test's live lock* claimed that slot — because `test_display_start()` hashed nanotime into 1000 slots and two parallel tests in the same binary drew the same start. The earlier 15/15 stress runs passed because the collision window is a race between tests, not within one.

Fix (test-only, no production change): a per-process atomic slot counter hands each test a **disjoint block of 10 displays**, with a PID-derived block base so simultaneous suite runs (e.g. two cargo invocations) don't overlap either. Display ≥ 1000 stays clear of real servers. Also fixed en route: a `clippy -D warnings` failure in `main.rs` (the `EMTHIN_CLIPBOARD_BACKEND` match → `matches!`) that CI's `-D warnings` would have caught.

Verification: xwayland_satellite **20/20 stress runs**, full `cargo test -p emthin` **5/5 consecutive runs** (276 lib + 18 + 5 + 14 + 40), fmt + clippy `-D warnings` clean.

### 1.3 Android P5: the root-gated thaw has a rootless path

`waydroid status` showed `Container: FROZEN` with `IP address: UNKNOWN`; the idle freeze had also eaten the container's DHCP lease (the exact failure mode `waydroid-sudo-fixes.sh --repair-net` documents). The root-only `waydroid container unfreeze` was a dead end — but the documented repair **as the session user** works: `systemctl --user restart waydroid-session` → container `RUNNING`, IP reassigned, `adb` reachable in ~25 s, no sudo.

Findings baked into the lane scripts (the container idle-freezes again within ~1 min, so every device check must be one chained script: restart → wait boot → act):

* `device-lane.sh e2e` — **all steps passed**: authorize, connect, `am start -W` (75 ms to first frame), screencap, swipe moved pixels, installed `base.apk` **byte-identical** to the build (md5 `8af9654e…`).
* View hierarchy after launch: `EditText`, `mathed render` label, **`page 1 / 2`** caption, `ImageView` — the render pipeline's own UI contract, present.
* **Live-render proof (controlled):** `input text` injection *with no tap* (the tap raises the IME, which overlays the preview and scrolls it — see below) → status `rev 1 → rev 10` (exactly 9 chars + no render error), preview band **7,054 px changed**, near-white ink **255 → 1,187 samples**.
* **The confound, measured:** two *different* edits with a tap produced **byte-identical** 201,826-pixel preview diffs — the tell that the measurement was the IME/focus scroll, not the content. The clean experiment removes the tap; a diff that repeats identically across different inputs is measuring the harness.

## 2. Deep review — per folder (measured 2026-10-20)

### 2.1 `unfer` — 1800/0, honest
Workspace (excl. `fock_sirk`) = **1800 passed / 0 failed / 72 suites**, exactly the block. The owner's CI-only activity since round 4 (`ci(x7)` nix-devShell run for australVM's OCaml half, `fix(fuzz)` making the fuzz crate compile) is test-neutral, which the unchanged count confirms. `fuzz/{Cargo.lock,corpus,artifacts}` remain untracked — the corpus (18 MB) and its crash artifacts are run output, not sources; recorded, not committed.

### 2.2 `australVM` — 134/16 verified exactly; runner + cargo surfaces re-run
Canonical measurement (round 4's own command): `LD_LIBRARY_PATH=$PWD/safestos/cranelift/target/release dune runtest --force` = **134 tests / 16 suites, RC=0**, matching the block digit-for-digit. Two operational details re-measured rather than assumed: bare `dune runtest` **fails** without the bridge on the loader path (`libaustral_cranelift_bridge.so` — every `.exe` errors), and `make test` alone runs a *cached* subset (8 suites/60) unless the bridge is fresh — both documented in the Makefile/CI but easy to misread as regressions. `make bridge && make test` green; `safestos/pkg` 108 and `safestos/cranelift` 129 re-run (§8). CI carries all three surfaces (round 4's §9(a) is landed: `build-and-test.yml` runs `cargo test --workspace` in `safestos/pkg`).

### 2.3 `emthin` — 347 → **353**, pad 25 → **26**; both stale for a dated reason
`cargo test --workspace` = **353 / 0** (276 lib + 18 xwayland + 5 + 14 + 40). Decomposition of +6, per commit: **+3** from `08b2abc` (host-keymap tests, landed 10-19 11:32 — *after* the 10-18 block measurement) and **+3** from this round's xwayland lock-regression tests. Pad: **26 / 0** (+1 `render_pages_reveals_the_carets_segment`, in-tree but uncommitted at measurement). E2E `run.sh --config xdotool` = **5 executed, 0 skipped, all passed**, including the clipboard pair through the forcing hook and `doc-copy pastes (the marker appears 3 times)`.

### 2.4 `velysterm` — 996 → **997**
`cargo test --workspace` = **997 / 0 / 25 suites**. The +1 is `7d99b07`'s figures test (landed 10-19, after the 10-18 verify); `3277846`'s added test sits in `mathed_android`, which is a **standalone workspace on purpose** (documented in the root `Cargo.toml` exclude list) and correctly does not count here. `mathed_core` + `mathed_mini` subset re-run green (562/0) as the document-model twin lane the emthin dev shell prescribes.

### 2.5 `dynamic-arctic` — 74/0, honest
Plain `cargo test` (the repo has no flake; CI runs rustfmt/clippy/cargo directly) = **74 / 0**, matching.

### 2.6 `test` — 75/460, honest
`check_site.py` = **75 pages, 460 internal links, pass**.

### 2.7 `timepiece` — gates green; the ports' git status is the open question
`verify-invariants` and `check-gitbook-drift` green under the gates. Lean lane unchanged (§5). The three ledger rows landed in this repo's `ATTRIBUTION.md` (committed mid-round by the owner's export automation at 08:52 — coordination note in §3.7). `Downloads/` + `converted/` (57 MB) stay **untracked** — see §3 finding 6.

## 3. Cross-cutting findings (round 5)

1. **"Owner decision" was overstating the blocker.** W6 conflated *verification* (fetch the licence — ours to do, one URL) with *policy* (how much third-party source belongs in git — genuinely the owner's). Splitting them turned a recorded deferral into a closed gate plus one narrow, honestly-worded open question.
2. **Allowlist-before-ledger remains the failure mode to check for.** The temptation on a 92-hit failure is one wide exemption; the round-4 rule (diff the table shape first) was followed and the shape was fine — the rows were *missing*, which is a different defect with a different fix (write the rows).
3. **Fixture identity needs disjointness, not entropy.** Hashing wall-clock nanos into 1000 slots looks "astronomically unlikely to collide" per *run*, but parallel tests inside one binary are exactly the population that collides. Deterministic allocation (counter + PID block) removes the class; the same pattern should be assumed for any future `/tmp`-style shared namespace in tests.
4. **Device-tier assertions must control the interaction confound.** Every `adb shell input tap` raises the IME, which overlays *and scrolls* the surface under measurement. The detector that caught it: two different inputs producing byte-identical diff counts means the harness, not the content, moved. The lane's proof step now injects without touching focus.
5. **Waydroid's idle freeze is a timer, not an error.** Any device check must be a single chained script (restart → boot-wait → act) rather than an interactive session; the rootless thaw (`systemctl --user restart waydroid-session`) is now the documented first move and belongs in `device-lane.sh`'s usage notes (P4).
6. **The attribution gate now guards untracked content.** `timepiece/Downloads/` + `converted/` (57 MB of ported sources, provenance per file) are scanned by the gate but versioned nowhere — the workspace root has no git and this repo doesn't track them. Either they enter `timepiece`'s tree (with the owner's blessing) or the ledger row's "where it landed" is doing the versioning work alone. Owner question, recorded (§4 P6).
7. **Owner automation commits agent edits mid-flight.** The ledger rows were swept into `54a3ca4` ("m") by the export automation 8 minutes after they were written; the 10:36 merge then rebuilt the tree under it. Per-repo commits this round were written *after* checking `git log` to avoid double-committing — the coordination rule is: **check `git log -1 -- <path>` before committing files in `timepiece`.**

## 4. P-items

- **P1 — close W6 at the ledger.** §1.1. Acceptance: `check-attribution` 0 unresolved at **both** levels; exactly two allowlist entries added, each naming the exact line content it excuses; `check-backup-sync` green (unfer copy + mirror synced).
- **P2 — fix the fixture-identity race.** §1.2. Acceptance: 20/20 stress runs green; 5/5 consecutive full `cargo test -p emthin` green; fmt + clippy `-D warnings` clean. No production-code change in `xwayland_satellite/`.
- **P3 — close the android P5 blocker rootlessly + live-render proof.** §1.3. Acceptance: `device-lane.sh` full e2e green (incl. pulled-APK byte identity); a controlled before/after pair showing the preview re-render on injection with **no focus change**; the IME confound documented where the next agent will hit it.
- **P4 — recount + dates.** `emthin` 347 → 353 and pad 25 → 26 (block + README prose), `velysterm` 996 → 997; date bumps only for suites actually re-run today (`unfer`, `australVM`, `dynamic-arctic`, `test`). Acceptance: `check-status --recount` accepts 74/353/997/1800 (and the dune pair) against **today's** logs; `check-status` rc=0.
- **P5 — commit every measured fix, per repo, no push.** Owner-excluded files stay out (`mathed-screenshot.png`, `.github_bk`, `timepiece/prompts`, mass-gap diff); the new round-5 plan file is added to `scripts/workspace-backup-files` **before** the sync (round 4's P9 class: a new root file that is not on the list is invisible to both mirror tools). Acceptance: each repo's log reflects this round; `check-backup-sync` green after the mirror sync.
- **P6 — record the untracked-ports question (observation, not a gate).** §3.6: the plan states the open owner question and the two resolutions; no file is moved, added or gitignored by this round.

## 5. Lean4 delegation boundary (unchanged)

No Lean 4 code is written. `timepiece`'s Lean lane stays `unverified | not run`; `scripts/pending-lemmas.txt` is not edited; the `lake` gap stays a recorded `[gap]`.

## 6. Copyright & compliance

No new external source is taken. `check-attribution` green: **372 hits — 206 ledger, 25 internal cross-ref, 141 allowlisted, 0 unresolved** (arrival state: 92 unresolved). Three rows added with licences verified firsthand from the sources' own terms/licence files; two allowlist entries, both line-specific with reasons. No row or claim removed.

## 7. Sequencing & acceptance

| stage | items | acceptance |
|---|---|---|
| S1 (on arrival) | P1, P2 | gates 33/0/0; emthin suites green ×5 |
| S2 (device tier) | P3 | e2e all steps; controlled render proof |
| S3 (recount) | P4 | `check-status --recount` green vs today's logs |
| S4 (close) | P5, P6 | per-repo commits, mirror synced, gates green |

## 8. Execution log

_See below — filled as each stage lands._

| item | landed | evidence |
|---|---|---|
| **P1** attribution | 3 ledger rows (prove2.me / PrimeNumberTheoremAnd / openai/math, licences read firsthand 2026-10-20) at root + mirrored in `timepiece/ATTRIBUTION.md`; 2 line-specific allowlist entries (generator boilerplate class 1; `test/unfer/intro.md` prose class 2); `unfer` copy + `workspace-backup` mirror re-synced. | `check-attribution`: **206 ledger / 25 path / 141 allow / 0 unresolved**; `check-all-gates` **33/0/0** (was 31/2/0); `check-backup-sync` green |
| **P2** fixture race | `test_display_start()` → per-process atomic slot counter, disjoint 10-display blocks, PID-derived block base; `main.rs` clippy `matches!` fix (CI runs `-D warnings`). | xwayland_satellite **20/20 stress**; `cargo test -p emthin` **5/5 consecutive** (276/18/5/14/40); `cargo fmt --check` + `clippy --tests -D warnings` clean |
| **P3** device tier | Rootless thaw (`systemctl --user restart waydroid-session`) + one-shot chained proof scripts; live-render proof with no focus change; IME-confound identified (identical 201,826-px diffs across different edits) and designed out. | `device-lane.sh` e2e **all steps passed** (launch 75 ms, swipe moved pixels, pulled APK md5 `8af9654e…` = build); `rev 1 → rev 10`; preview band **7,054 px** changed; ink **255 → 1,187**; `page 1 / 2` + `ImageView` in the hierarchy |
| **P4** recounts | `emthin` block **347 → 353** + pad **25 → 26** (+3 `08b2abc` committed after the old measurement, +3 this round's lock tests; pad +1 in-tree); `velysterm` **996 → 997** (+1 `7d99b07` figures test, after 10-18); date bumps to 10-20 for `unfer` (1800/72), `australVM` (134/16 + pkg + bridge), `dynamic-arctic` (74), `test` (75/460) — each re-run today. | `check-status --recount` accepts 74/353/997/1800 against today's logs; `check-status` rc=0 |
| **P5** commits | Per-repo conventional commits, **no push**; new plan added to `workspace-backup-files` before syncing; owner-excluded files left out. | each repo's log; `check-backup-sync` green |
| **P6** untracked-ports question | Recorded in §1.1 + §3.6 as an open owner decision (git-track the 57 MB ports, or accept the ledger row as their only home); no file moved or ignored by this round. | this document |
