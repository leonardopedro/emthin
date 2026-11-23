# Measurement surfaces — what each gate actually measures

A deviation table in the sense the reference-repo discipline uses (workspace
`ATTRIBUTION.md` rows 65–66): each surface states what it measures, what it
cannot see, and which seam it guards. The 2026-10-15 E2E regression is worked
through at the bottom as the classification example — the point of the table
is that the next seam regression can be placed without re-deriving it.

## The surfaces

| surface | measures | cannot see | seam guarded |
|---|---|---|---|
| `cargo test --locked --workspace -j1` (734 passed, 2 ignored on 2026-11-22) | doc-model invariants, caret geometry, keymap tables, session serde, figure binding, IPC protocol units, xwayland-supervisor timing | anything needing a live compositor: a real input path, a real clipboard, a real shutdown signal | `mathed_core` ↔ `emthin` model contract |
| `scripts/e2e/run.sh` (28 checks with `wtype`, 36 with XTEST, 0 skipped on 2026-11-21) | one real nested session end to end: synthetic keystrokes reaching the document, PgDn/PgUp multi-page navigation, clipboard both directions across the proxy, per-document preferences, touch panning, Find cancel and fcitx5/rime preedit/commit in the XTEST lane, the `list_state` document read, SIGTERM shutdown | rendering quality (performance is now sampled — see the live-session baseline below) | input path · IME host · clipboard proxy · control socket · shutdown — the seams *between* crates and processes |
| `scripts/e2e/process-kill.sh` (7 checks, exit 0 on 2026-11-21) | SIGKILL at a committed session-write boundary, then relaunch: the last verified snapshot pair recovers the edited note with no quarantine | rendering, input, the save path between checkpoints | the atomic session store's real-process crash recovery — D1/D8's process-kill case |
| `scripts/e2e/live-ipc.sh` (13 checks, exit 0 on 2026-11-22) | a real control client over a compositor-backed emthin: semantic/a11y/launcher/plugin/kernel/proof/inspect/history/descriptor notifications, plus a live rename reflected in the next outline | rendering, input injection, device | the control-socket contract of the semantic/a11y/parked-item surfaces — K1/K5/K8's live-client case |
| `scripts/e2e/live-mm0.sh` (5 checks, exit 0 on 2026-11-22) | loopback mm0web verifies the real Peano proof with mm0-rs + mm0-c, the document cites the minted id via `\formal(proof: "…")`, and that verdict travels through `proof_result` into `proof_query` | device, real-phone transport | the live verifier seam from a real backend into the document's advisory proof view — K9's live-run case |
| `emthin-android/device-lane.sh math-preview` (Waydroid arm64, 2026-11-22) | rendered prose and equation match their accessibility bounds, a tap edits the equation source, and Undo restores text and focus | physical-phone input and display, visual legibility by a human | bitmap placement shares page geometry with editor input and figures, and the source field keeps focus during a rebuild |
| what a user sees | all of it, eventually | — (they file the bug) | — |

Neither gate is a subset of the other, and neither can absorb the other: the
then-347-test cargo suite stayed green through the entire E2E regression below,
and the E2E's behavioral checks say nothing about the model invariants a
document edit can silently break.

## Classifying a regression — the 2026-10-15 example

The E2E disagreed with itself across two days at identical code: 10-13 green,
10-15 red four times. Symptoms and where each pointed:

| symptom | looked like | was |
|---|---|---|
| `list_state` returns an empty document | a doc-model/save bug | the harness talking to a dead control socket |
| keystrokes not landing (typed text check fails) | an input-path bug | emthin refusing to bind over the stale socket, `AlreadyInUse` |
| SIGTERM not honored within 30 s | a shutdown bug | the checks ran against a process that was not this run's |
| clipboard + transport checks pass | contradiction | those paths never touch the carried state — the split is the fingerprint |

The root cause was state the harness carried across runs (dead
`.e2e/run/emthin.ipc`, a leaked `Xvfb :99`); the fix is in
`reap_previous_run`/`cleanup` (see `scripts/e2e/README.md`, "Stale state").
The full post-mortem lives there; what belongs here is the rule it taught:

- **cargo green + E2E red → environment/harness/seams first**, document model
  last. The E2E runs in a workspace of processes; its failure modes are
  overwhelmingly *stale* rather than *wrong*.
- **cargo red + E2E green → the model.** The E2E only exercises paths the
  model tests also cover; a model break rarely stays E2E-green.
- **both red → the seam between them**, usually the session/save format —
  it is the one artifact cargo's serde tests and the E2E's session file
  genuinely share.

## Debug affordances on the E2E side

- `E2E_DUMP_STATE=1 ./scripts/e2e/run.sh ...` dumps the raw `list_state`
  frames to `.e2e/run/last_list_state.json` — this is what separates
  "document is empty" from "parser found no `state` notification", which
  read identically through `doc_from_state`.
- Harness logs land in `.e2e/` (`xvfb.log`, `sway.log`, `gnome-shell.log`,
  session log); the CI `e2e` job uploads the directory on failure.
- `./scripts/e2e/run.sh --list` reports which input configurations the
  machine can run before anything is started.

## Live-session timing baseline (2026-11-21)

D0 asked for a live-session baseline; this is it, from one `wtype` E2E run
with `RUST_LOG="info,emthin::timing=trace"` (28 checks, exit 0). Host:
aarch64, rustc 1.99.0, workspace `Cargo.lock`. Source: `.e2e/emthin.log`
from that run, aggregated by `stage` and field — this is instrumentation on
the real session, not a synthetic benchmark.

| stage / field | n | p50 | p95 | max | unit |
|---|---|---|---|---|---|
| `event_loop_tick` elapsed | 121,656 | 26 | 221 | 155,680 | µs |
| slow ticks (>1 ms) | 38 | — | — | — | 0.03% of ticks |
| `doc_relayout` elapsed | 11 | 10 | 1,330 | 1,330 | µs |
| `layout_worker` compile | 10 | 112.5 | 382.6 | 382.6 | ms |
| `layout_worker` glyph | 10 | 48.9 | 208.8 | 208.8 | ms |
| `layout_worker` raster | 10 | 124.6 | 156.7 | 156.7 | ms |
| `layout_worker` apply | 10 | 0 | 0 | 0 | ms |
| `session_save` elapsed | 11 | 14,267 | 75,430 | 75,430 | µs |
| `session_save` snapshot | 11 | 1,393 | 2,239 | 2,239 | µs |
| `session_save` write | 11 | 11,295 | 73,273 | 73,273 | µs |
| `autosave` elapsed | 8 | 64,123 | 75,500 | 75,500 | µs |

Reading it: the steady-state event-loop tick is tens of µs with a 221 µs
p95; only 38 of ~122k ticks crossed the 1 ms debug threshold (mostly startup
and the first render). Layout work happens off-loop — compile/glyph/raster
are separately visible in the worker's result event — and a document save's
dominant cost is the disk write, not the snapshot. The 1,200-line pure
layout measurement from D3.2 remains the page-work stress figure; this table
is the ordinary-session counterpart.

## What no gate covers yet

Rendering quality ("make it look nice") is still not machine-checkable; the
§9 debuggability workstream picks it up as its acceptance lane. The live
timing baseline above now samples performance during the E2E, but it is a
single session, not a gate — there is no p95 threshold a regression fails
against. The IME commit path is covered behaviourally by both E2E configs
(fcitx5/rime under XTEST, and the device lane's composing-IME check); what
remains unmeasured is timing *through* the host IME, which the trace does not
attribute.
