# PLAN_CROSS_PROJECT_REVIEW_2026-10-21.md — round 6

Scope as ordered: debug and test the workspace, including the emthin
Android app and the visuals/GUI/functionality of mathed inside it,
driven in a loop over this machine's Wayland with the observation
tooling emthin/velysterm already ship. **No Lean4 code** — timepiece
enters only through its gates.

Measurement rule for this round: every claim below has a command and a
log (in `/tmp/t_*.log` on the dev machine) behind it; counts were
re-read from logs, never from shell exit alone (see F2 — that
discipline caught two failures an earlier pass had masked).

---

## A. Arrival state: one red gate, agent loop down

`unfer/verify-invariants` arrived RED because **six executables were
dead**: `nix store gc` had collected the glibc their `PT_INTERP` named
(`…-glibc-2.42-67`), so every exec failed with a bare "required file
not found" — no library name, no path. The casualties included
`mathed_mini` (the engine behind emthin's whole semantic tier) and
`unfer_agent` (the smoke-gate target): the feedback loop itself was
down, not just a test.

**Fixed** by rebuilding in the current dev shells (unfer bare-shell
rebuilds now link the stable `nix-ld` `/lib64` interpreter; velysterm
rebuilt via `nix develop -c`). Gate re-run: **33 passed / 0 failed /
0 gaps**. `cell_tool` round-trip answered `"ok":true`.

## B. Full test sweep (serial, `-j 4` — see F3)

| repo | lane / command | result | log |
|---|---|---|---|
| unfer | `cargo test --workspace --all-targets -j4` (bare shell) | **2301 passed / 0 failed**, 135 blocks, RC=0 | `t_unfer_full.log` |
| unfer | README command: `nix-shell -p zlib-ng cmake --run "cargo test --workspace --exclude fock_sirk"` | **1846 / 0**, RC=0 (matches README digit-for-digit) | `t_unfer_readme.log` |
| unfer | `… --doc` doctests | 7 / 0, RC=0 | `t_unfer_doctests.log` |
| unfer | gates (`scripts/verify-invariants`) | 33 / 0 / 0 gaps | (round-6 rerun) |
| velysterm | `nix develop -c cargo test --workspace --all-targets -j4` | **RC=0** (0 failed across the run) | background run |
| velysterm | `cargo test --manifest-path crates/mathed_android/Cargo.toml` | **30 / 0** (21+9; AGENTS said 29 — fixed in P4) | live run |
| velysterm | `BOOT=1 scripts/android-e2e.sh` (device, Waydroid) | **30 passed / 0 failed**, RC=0 | `t_mathed_e2e.log` |
| emthin | `nix develop -c cargo test --workspace` | **353 / 0** (2 ignored) | `t_emthin.log` |
| emthin | `scripts/e2e/run.sh --config xdotool` | **5/5 twice today** (10:13 and re-run), RC=0 | `t_pad_e2e.log`, `t_emthin_e2e.log` |
| emthin | `scripts/agent/replay.sh figure-resize` | 13/13 asserts | live run |
| emthin-pad | `cargo test` in `emthin-android/` | **29 / 0** (AGENTS said 25 — fixed in P4) | `t_pad_module.log` |
| emthin-pad | device lane e2e (launch/draw/swipe/reinstall) | all steps pass | live run |
| australVM | `nix develop -c make test` | RC=0 | `t_avm_remake.log` |
| australVM | canonical `nix develop -c env LD_LIBRARY_PATH=… dune runtest --force` | **134 tests / 16 suites / 0 failed**, RC=0 (reconfirmed after the GC incident) | `t_avm_forced2.log` |
| dynamic-arctic | `cargo test --all-features` + `--no-default-features --lib` | **74 / 0**, RC=0 | `t_arctic.log` |
| timepiece, test/ | workspace sweep gates (`check-all-gates`) | green (Lean build out of scope by instruction) | sweep output |

**Count reconciliation (both numbers correct, different commands):**
`2301 (all-targets, all crates) = 1846 (README command) − 7
(doctests) + 462 (fock_sirk's own suites, the number
testing-depths already records)`. The delta between the two was
exactly fock_sirk + doctests, verified by unit-list diff
(`comm` over `Running` lines), not by assumption.

## C. Failures found and root-caused (the debug loop)

1. **velysterm `snapshot_replay` load-flaky golden.** `solve_ms` is
   wall-clock inside `result`; the normalizer's own doc says stripping
   wall-clock jitter is its job, but it only stripped top-level
   `timing_ms`. Passes idle (0 ms), failed under parallel load (3 ms).
   **Fixed at the normalizer** (not the golden — regenerating would
   just move the flake), golden regenerated and **diffed**: only the
   stripped `solve_ms` moved, no product value changed. Verified 3/3
   under artificial load + full workspace run RC=0.
2. **australVM loader mismatch (X4(b)) was my invocation, not the
   repo.** I ran `dune` in `nix-shell` (glibc 2.37) instead of CI's
   `nix develop` (2.42): the test exes relinked to 2.37 and could no
   longer satisfy the bridge's `GLIBC_2.39`. The Makefile already
   documents the loader mismatch; the *wrong-shell poisons `_build`*
   trap now has its own AGENTS note (P2). Canonical lane green: 134/16.
3. **GC sweep #2, mid-session (the headline).** The owner ran
   `sudo nix-collect-garbage -d` **twice at 13:19/13:20** while suites
   were between runs. It collected `n51dh…-glibc-2.42-84` — the pinned
   devshell glibc of australVM — leaving **521 australVM executables
   dead** (`_build` OCaml test exes, cranelift release `modhost` /
   `cps2cell`, pkg target bins, build scripts). The brand-new P1 gate
   caught all 521 within hours of being written. **Healed without a
   single relink**: `nix develop -c true` re-substituted the pinned
   glibc from cache.nixos.org (content-addressed — same hash), plus
   removal of one stale 2.37-linked ppx driver (regenerable dune
   cache). Re-verified: gate 3840 ELFs / 12 trees green, `make test`
   RC=0, forced runtest **134/16 RC=0**.
4. **Two `.c` "oddballs" in `_build/test/l9-scratch/` are genuinely
   ELF** (compiled scratch output misnamed `plain.c`/`annotated.c`,
   exec bit set) — checked before trusting the gate; not a parser bug.

## D. Visual/functional loop on this machine's Wayland

- **emthin agent loop, every tier exercised:** tier 1 `preview.sh`
  (0.113 s fast path through the rebuilt `mathed_mini`), tier 1b
  `--semantic` / `--diff`, replay (13/13), tier 2 live session on
  sway/Xvfb: 3 figures bound with the documented letterbox (367×275),
  `spawn kgx` bound a **real window** (window=1, title), page switch
  moved the live rect, and the core invariant held end-to-end: editing
  the document to `app(#3,#4,500,350)` re-letterboxed the live rect to
  459×321 with the client staying bound. `foot` reported ENOENT
  honestly; `down` shut down gracefully. The dmabuf-fix path (spawn →
  pixels → no SIGSEGV) behaved as AGENT_LOOP documents.
- **emthin-pad on Waydroid:** device lane e2e green after the
  documented thaw; `adb shell input swipe` scrolled the preview —
  screencap ink 1152 → 4567 near-white px, 207 621 px changed
  (visual proof the app's ScrollView works over adb). Idle-freeze
  behavior (adb hangs ~1 min after idle; chain with hard timeouts)
  observed exactly as documented.
- **mathed:** tier 1 `mathed_probe --local` — summary (parseable
  `name=value` line), `pages` (4 pages, figures `[0,2,0,0]`), `shot`
  → 720×1280 PNG, 110 719 bytes. Tier 2 after the documented recovery
  (`systemctl --user restart waydroid-session`; RUNNING+IP in ~10 s):
  **30/30 e2e checks**, including the boot-log, raster-density and
  tap hit-test steps.

## E. Improvements executed this round

| id | what | where |
|---|---|---|
| P1 | **`scripts/check-elf-interpreters`** — new workspace gate (auto-discovered by `check-all-gates`' `check-*` sweep): every executable ELF under any `target`/`_build` must have a live `PT_INTERP`. Design notes measured this round: ELF64 header offsets differ from ELF32 (54/56 vs 42/44 — the wrong choice reads phnum=0 and sees nothing); exec-bit filter keeps it at ~1.4 s (opening every rlib took 39 s); empty scan exits 2 (check-shebangs doctrine); remedy text covers both the re-substitute path (flake-pinned hash still in lock → `nix develop` alone heals) and the rebuild path (lock moved). Fail path proven (521 → 1 → 0), pass path 3840/12 green. | `scripts/check-elf-interpreters` |
| P2 | **Shell discipline note** — bare `nix-shell` dune runs relink exes/ppx drivers against a glibc outside the flake closure; symptom + remedy + pointer to P1 gate. | `australVM/AGENTS.md` |
| P3 | **Execution discipline section** — the three ways a run lied this round: parallel workspaces OOM'd the host twice (serial `-j4` passes); `| tail` without pipefail masked two real regressions; backgrounded `… &` died with the tool's process-group cleanup twice (zero-byte log = dead run, check the PID). | `scripts/testing-depths.md` |
| P4 | **Count drift fixed with evidence:** emthin pad 25→29 (AGENTS), workspace 347→353 + pad 25→29 (testing-depths), mathed_android 29→30 (velysterm AGENTS), unfer row 1797→1846 (testing-depths, matches the README it cites), emthin README status block → 29 pad / 353 / 5/5 re-run / `last_verified: 2026-10-21`. Historical dated references (the 2026-10-15 "347 module-green" regression story) left untouched. | `emthin/AGENTS.md`, `emthin/README.md`, `velysterm/AGENTS.md`, `scripts/testing-depths.md` |

## F. Harness findings (this session's own failures — F-labels)

- **F1 — tail-masked exits.** `cargo test | tail` without `set -o
  pipefail` reports `tail`'s status: two real failures (velysterm
  flake, australVM loader) passed as exit 0 until re-run with
  pipefail. Lesson now recorded in P3.
- **F2 — exit code is not a count.** Re-reading logs for numbers is
  what reconciled 1846/2301/462/7 and found the stale 347/1797/25/29
  claims. Prose counts were fixed only against a fresh log.
- **F3 — OOM ×2.** Two concurrent `cargo test --workspace` at default
  jobs crashed the 14 GiB host (the second crash ended the session).
  Serial + `-j 4` measured green; recorded in P3.
- **F4 — backgrounding.** `nohup … &` from the tool shell died twice
  (empty logs); supervised background runs + log inspection worked.

## G. Parked edits (session hygiene — not committed, per "commit only
when the owner asks")

- `velysterm/crates/kernel_client/tests/snapshot_replay.rs` + its
  regenerated golden (C1: normalizer fix; diff proves only `solve_ms`
  stripped).
- `scripts/check-elf-interpreters` (new), `scripts/testing-depths.md`,
  `australVM/AGENTS.md`, `emthin/AGENTS.md`, `emthin/README.md`
  (status block), `velysterm/AGENTS.md`, this file + its
  `workspace-backup-files` registration, and the mirror sync into
  `emthin/workspace-backup/`.

## H. Deferred / not claimed

- **GC recurrence prevention.** Detection (P1) is done; durable
  prevention is a system-config decision (nix gc roots,
  `keep-outputs`, or "run gates after any gc") — owner's call, not a
  reviewer's.
- **timepiece Lean build** — not run (instruction: no Lean4 code);
  its gates are green through the sweep.
- **android-e2e without `BOOT=1` (28-check path)** — not re-measured
  today; only the 30-check BOOT path.
- **GPU lanes** (delta GPU, `debug_cuda`) — skipped; no GPU on this
  host, existing skip pattern.
- **P1 boundary:** `PT_INTERP` existence only. DT_NEEDED version
  drift (the australVM X4(b) class) fails with a legible dynamic
  linker message and is deliberately out of the gate's scope.
