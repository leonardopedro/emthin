# Workspace scripts — the federation's gates, one map

Seven sibling checkouts, one gate culture: every repo carries its own
`verify-invariants` (repo-specific promises, machine-checked) and
`check-links` (relative markdown links resolve), and this directory
carries the cross-repo ones. Run everything with:

```bash
bash scripts/check-all-gates          # all seven repos + the ledger gate
bash scripts/check-all-gates unfer emthin   # a subset
```

**python3 note:** this machine's python3 is uv-managed and lives at
`/usr/local/bin/python3` — outside the default *non-login* PATH. Both
`check-all-gates` and the individual gates search that path themselves,
but a bare shell may still report `[gap] … python3 not on PATH`. That is
a gap (check not enforced), never a pass.

**Memory note (F9, 2026-10-13):** this box has 14 GB RAM. **Never run two
`cargo test`/`cargo build` suites in parallel** — two concurrent workspace
builds (unfer + emthin) OOM-killed the machine twice on 2026-10-13. Run
repo suites one at a time, and pass `-j 2` to cargo on the big workspaces
(unfer, velysterm); the suites are green either way, only slower. The
gates in this directory are cheap (grep/python) and safe to parallelise;
`check-status --run` already runs its registered suites serially.

## Workspace gates (this directory)

| script | what it enforces |
|---|---|
| `check-all-gates` | runs every repo gate below; `[gap]` for a missing gate or interpreter, `[FAIL]` only for real violations; exit ≠ 0 iff something FAILED |
| `check-attribution` | policy #4 of `../ATTRIBUTION.md`: every adaptation claim (`adapted/inspired/ported/borrowed from`, `based on/derived from`) in the seven checkouts + root plan docs resolves to a ledger row, an internal path, or a reviewed allowlist entry |
| `check-scenario-ledgers` | the obligation-closure tables in `emthin`/`velysterm` SCENARIOS.md (REA discipline): closed vocab for authority/closure, every row's verifier names a script that exists, scenario IDs ↔ closure rows bijective |
| `check-error-catalog` | `docs/user-visible-errors.csv` shape (harm ranks 1..N, retryable vocab, verification non-empty) + bidirectional code-set parity vs the source enums (`emthin` `IpcErrorCode`, `voice.sh` FAIL codes, `modhost` error kinds) |
| `check-backup-sync` / `sync-workspace-backup` | byte-identity of both mirrors: `emthin/workspace-backup/` ↔ its root sources (refresh with `sync-workspace-backup`, then commit), and `unfer/scripts/{check-attribution,workspace-root.sh,tree-size}` ↔ the root copies (unfer's CI runs its own copy directly) |
| `testing-depths.md` | the R15 depth labels: each repo's named verification surfaces mapped onto rea's five lanes (module / composition / boundary / acceptance / conformance) with named skips — descriptive; the gates above enforce the *claims* around suites, this catalog says which lane a green run licenses |
| `check-book-sync` / `sync-book` | book ↔ site parity for the math corpus |
| `check-status` / `tree-size` | maintenance utilities |
| `check-shebangs` | every executable script under `scripts/` (root + checkouts) carries a shebang that can actually run on the host: `env`-mediated forms pass, an absolute interpreter that is **absent** fails (rc=126 on direct invocation), an absolute one that is present is reported but accepted — CI runners legitimately have `/bin/bash`. Proves the P2 fix (round 4) cannot regress; `check-all-gates` is blind to this because it dispatches through `bash` |
| `workspace-backup-files` | the mirror list both backup scripts consume (root docs/scripts ↔ `emthin/workspace-backup/`); a new root plan doc is added here in the same change |
| `waydroid-sudo-fixes.sh` | host utility, not a gate: the root-side waydroid checks for the Android (emthin-pad) lane |

Workspace `check-*` gates are auto-discovered by `check-all-gates` (the
runner skips itself and `check-status`, a maintenance utility) — drop a
new `check-*` here and the workspace pass picks it up.

### Scheduled runs (P4 — no CI target exists for the workspace root)

The workspace root is not a git repository and hosts no CI, so
`check-all-gates` runs from cron instead. Install with `crontab -e`:

```cron
# federation gates: weekly, Monday 06:00 local. Output to a dated log;
# grep for FAIL/gap after the run.
0 6 * * 1 /bin/bash -c 'cd /home/leo/Projects && bash scripts/check-all-gates > /tmp/check-all-gates-$(date +\%F).log 2>&1'
```The schedule exists to catch drift like F1 (a gate convention broken by a
well-intentioned edit) between sessions; the stage-boundary invariant
remains "run `check-all-gates` before claiming done", schedule or not.

**Host note (2026-10-14):** this machine has no `crontab`/`cron` binary
(NixOS, verified) — the line above cannot be installed here as written.
Owner decision on S4-5: **documented-only**, no scheduler installed; the
stage-boundary run is the safety net until a CI target or an explicit
scheduler (e.g. a systemd user timer) is chosen. Re-open S4-5 if and
when a mechanism actually runs.

## Per-repo gates (what `check-all-gates` invokes)

| repo | gates |
|---|---|
| `unfer` | `verify-invariants` (invariants, H11 real-entry-path smokes, coverage), `check-links`, `check-todo-ledger`, plus 10+ specialist scripts (`check-mass-gap-claims`, `check-non-claims`, `smoke_gate`, `coverage_gate`, `duplication`, …) |
| `australVM` | `verify-invariants` (symbol census, engine defaults), `check-links`, `check-todo-ledger` |
| `velysterm` | `verify-invariants` (VALID_OPS sourcing, kernel types), `check-links`, `check-op-catalog` (AGENT_OPS ↔ dispatch ↔ generated catalog) |
| `timepiece` | `verify-invariants` (gitbook drift incl. the pending-lemmas list, lake build), `check-links`, `check-todo-ledger` |
| `dynamic-arctic` | `verify-invariants` (toolchain pin, repro, honesty banner, SHA-pinned CI), `check-links` |
| `test` | `check_site.py` (prose integrity), `check-links` |
| `emthin` | `verify-invariants` (version lockstep, smithay rev pin), `check-links`, `check-ipc-docsync` (IPC catalog ↔ code) |

Generated artifacts that must never be hand-edited:
`velysterm/docs/AGENT_OP_CATALOG.md` (`check-op-catalog --write`),
`unfer`/`australVM` `CHANGELOG.md` (git-cliff).

## Redaction policy (logs and messages)

Transport credentials never ride along in output. Concretely:

- URLs echoed into progress lines, FAIL reasons, or logs are stripped of
  userinfo and query parameters (`unfer/voice/voice.sh`'s `LLM_URL_DISP`
  is the worked example — emthin splices FAIL reasons into the
  *document*, so a leaked token would land in the user's file);
- secrets arrive via environment or files with tight permissions and are
  never defaulted to empty strings that would silently authenticate as
  nobody (`unfer/logos/src/formalize/llm.rs`'s `api_key: Option<String>`
  is the model);
- an error path must be re-read as a future log line: if the message
  could contain a credential, sanitize at the producing boundary, not at
  the consumer.

## Adding a gate (admission criteria)

1. Put it in the owning repo's `scripts/` with the `check`/`gap` output
   convention (missing tool ⇒ `[gap]`, never a silent pass);
2. name it `check-<what>` or `verify-invariants` sections;
3. `check-all-gates` picks up `check-*` extras automatically for the
   repos that carry them — no runner edit needed for new `check-*`
   scripts;
4. document it in the table above and in the repo's AGENTS.md.

### Admission criteria for a new gate or feature (N3, adapted from REA)

A new gate or user-facing feature is admitted only when all of these
land **in the same change** — an item that skips one is not done:

- **Evidence command.** The README `status:` block (or the feature's doc)
  names the command whose run justifies the claim, and it is recounted
  through `check-status --recount REPO=LOG` (or `--run`) before the block
  is bumped. No count is written in prose that a gate could generate
  (see velysterm `check-op-catalog`'s F4 prose guard, unfer's H1 census).
- **Honest skip.** Anything the gate cannot check here states the missing
  authority as `[gap]`/exit-2/named skip — never a silent pass, never a
  fail that is indistinguishable from absent tooling.
- **Catalog + contract rows.** If the feature adds a wire method, IPC
  error code, or agent op, the corresponding catalog gate and doc row
  (`check-ipc-docsync`, `check-op-catalog`, `docs/error-contract.md`)
  are updated in the same change.
- **Error copy audited.** New user-visible error strings get a row in
  the root `ERROR_COPY_AUDIT.md` **and** the machine-checked
  `docs/user-visible-errors.csv` (harm rank, recovery guidance, no
  credential/internals leakage; `check-error-catalog` keeps the CSV's
  code sets in parity with the source enums) — producing boundary
  sanitised, not consumer-filtered (see the Redaction policy above).
- **Scenario row.** New E2E assertions get their row in the owning
  `SCENARIOS.md` with an honest status (verified / degraded /
  environment-gated).

## Commit subjects (P7, round 4 — observation, not a gate)

`australVM`, `timepiece` and `emthin` have drifted back to one-letter
commit subjects (`mm`, `a`). This is not enforced anywhere, but it is not
free either: **`australVM/AGENTS.md` documents that its `CHANGELOG.md` is
*generated* from conventional commits by git-cliff**, so every `mm` is a
change that ships with no changelog entry. Conventional subjects
(`fix(scope): …`, `docs(scope): …`) keep both the log readable and the
generated changelog honest. The workspace gates deliberately stay out of
commit-message territory — this is a note for whoever commits, not a rule
a machine should police.
