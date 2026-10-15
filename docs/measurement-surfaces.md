# Measurement surfaces — what each gate actually measures

A deviation table in the sense the reference-repo discipline uses (workspace
`ATTRIBUTION.md` rows 65–66): each surface states what it measures, what it
cannot see, and which seam it guards. The 2026-10-15 E2E regression is worked
through at the bottom as the classification example — the point of the table
is that the next seam regression can be placed without re-deriving it.

## The surfaces

| surface | measures | cannot see | seam guarded |
|---|---|---|---|
| `cargo test --workspace` (347) | doc-model invariants, keymap tables, session serde, figure binding, IPC protocol units, xwayland-supervisor timing | anything needing a live compositor: a real input path, a real clipboard, a real shutdown signal | `mathed_core` ↔ `emthin` model contract |
| `scripts/e2e/run.sh` (5 checks) | one real nested session end to end: XTEST keystrokes reaching the document, clipboard both directions across the proxy, the `list_state` document read, SIGTERM shutdown | rendering quality, IME commit (needs the host's fcitx engaged), multi-page flows, performance | input path · clipboard proxy · control socket · shutdown — the seams *between* crates and processes |
| what a user sees | all of it, eventually | — (they file the bug) | — |

Neither gate is a subset of the other, and neither can absorb the other: the
cargo suite's 347 stayed green through the entire E2E regression below, and
the E2E's five checks say nothing about the model invariants a document edit
can silently break.

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

## What no gate covers yet

The IME commit path (needs the host's fcitx engaged, not merely installed)
and rendering quality ("make it look nice") are user-visible and unmeasured.
The §9 debuggability workstream in the workspace plan picks rendering up as
its acceptance lane.
