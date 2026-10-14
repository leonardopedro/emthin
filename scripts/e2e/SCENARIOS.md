# E2E scenario contract — emthin

The checkable contract for `scripts/e2e/` (scenario-contract discipline
adapted from REA's browser-scenario contracts: each scenario states its
precondition, its steps, its expected observation, and — explicitly —
whether it is *verified*, *structurally broken*, or *not exercised*.
A harness that cannot say "did not run" cannot say "passed", which is
why exit code 2 exists).

`README.md` carries the measured evidence; this file is the contract
that evidence is measured against.

## Harness contract

| item | contract |
|---|---|
| entry point | `run.sh` picks a configuration, brings up compositor + emthin, runs `step7.sh` |
| seam | the four inject functions (`inject_available`, `inject_note`, `inject_type`, `inject_key`) — `step7.sh` is written once and runs unchanged under any backend |
| exit 0 | every executed check passed |
| exit 1 | at least one check failed |
| exit 2 | **nothing was verified** — every check skipped (declined ≠ passed) |
| shutdown | `run.sh` stops emthin with SIGTERM; the log must say `emthin exited cleanly`; SIGKILL is a failure, and "exited without logging a clean shutdown" is distinguishable from both |

## Configurations (the input axis)

| `--config` | mechanism | privilege | nested? |
|---|---|---|---|
| `ydotool` | uinput, kernel level | root for the daemon only | **no** — a nested compositor never opens `/dev/input`; host owns evdev (measured: nested 0 fds, host 14 fds) |
| `xdotool` | XTEST | none | **yes** — compositor non-headless with a real X window; needs the logind session unowned (Mutter → EBUSY; log out or use a VT) |
| `wtype` | `zwp_virtual_keyboard_v1` | none | yes, if the compositor implements it (not Mutter) |
| `none` | — | none | clipboard checks only; keyboard skipped |

## Scenarios

| id | scenario | steps | expected | status |
|---|---|---|---|---|
| E1 | keys reach the document | type a string via the seam | text appears in the document | **verified** |
| E2 | document copy round-trips | Ctrl+A / Ctrl+C / Ctrl+V inside emthin | marker appears 3 times | **verified** (`arboard` path `1a79b38`, live) |
| E3 | host clipboard → nested client | copy on host, paste inside | pasted content | **structurally broken** (nested clipboard hop, measured; cause documented in README §"the remaining hop") |
| E4 | nested client → host clipboard | copy inside, paste on host | pasted content | **structurally broken** (same hop) |
| E5 | graceful shutdown | SIGTERM to emthin | `emthin exited cleanly` in the log | **verified** (regressed once via a calloop self-pipe that never dispatched; now an atomic flag — see `src/shutdown.rs`) |
| E6 | IME commit path | fcitx engaged on host, type via IME | committed text | **not exercised** — needs the host's fcitx engaged, which `fcitx5` being installed does not arrange |

E3/E4 failing is a *true* result: the harness reports the broken state
rather than skipping, and the README records why it is structural
(nesting boundary sits below the input/clipboard stack, not above).

## Obligation closure (REA reconstruction-ledger discipline)

Closure discipline adapted from REA's reconstruction obligation ledgers
(`rea/docs/reconstruction-obligation-ledgers.md`, MIT): a claim closes only
when a verifier of **comparable authority** runs and its result is recorded.
A lower-tier run never closes a higher-tier row — a `unit` pass cannot close a
`harness` obligation any more than a green unit test closes REA's
`packaged-process` one. Dispositions (`blocked`, `gated`, `open`) are never
counted as verified.

Authority tiers (closed vocabulary): `real-session` > `harness` > `headless` > `unit`.
Closure values (closed vocabulary): `closed`, `open`, `blocked`, `gated`.

| id | authority | verifier | required case kinds | closure |
|---|---|---|---|---|
| E1 | harness | `bash scripts/e2e/run.sh` | positive | closed |
| E2 | harness | `bash scripts/e2e/run.sh` | positive | closed |
| E3 | harness | `bash scripts/e2e/run.sh` | positive | blocked |
| E4 | harness | `bash scripts/e2e/run.sh` | positive | blocked |
| E5 | harness | `bash scripts/e2e/run.sh` | positive, teardown | closed |
| E6 | real-session | `bash scripts/e2e/run.sh` | positive | gated |

E2 additionally has live-session evidence (copy round-trip measured in a real
session, `arboard` path `1a79b38`) — that is *stronger* than its closing
verifier and is recorded in `README.md`, not claimed here. E3/E4 are
`blocked` by the measured nested-clipboard hop (cause in `README.md`); a row
that is blocked stays open forever unless the hop is fixed and the verifier
then covers the positive case. E6 is `gated`: its verifier exists but the
host's fcitx engagement is not arranged by the harness, so no run closes it.

## Change rule

Adding a check to `step7.sh` means adding its row here in the same
change, with an honest status. Downgrading a status (verified →
broken/not-exercised) requires the measurement to move with it.
