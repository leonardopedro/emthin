# Agent loop: debugging a document with an LLM

emthin documents are plain text with `\app` figure statements, so an
LLM can edit one directly. What an LLM lacks is *observation*: what
the edit did to the layout, and whether it looks right. This doc
defines the loop that supplies both, at the lowest token and RAM cost
that is honest.

The design follows the known-good recipe for LLM-driven GUI work — a
**semantic tier** (structured geometry, hundreds of tokens) consulted
first, and a **visual tier** (PNG, only when the semantics say the
numbers are right) consulted second. The semantic tier is the
load-bearing one: it answers "where will every figure land, and is
anything structurally wrong?" without a compositor, a display, or a
rasteriser.

## Tier 1 — headless preview (no display, no compositor)

```sh
./scripts/agent/preview.sh scripts/agent/demo.typ   # a sample doc with three figures
./scripts/agent/preview.sh notes.typ                # geometry + diagnostics JSON
./scripts/agent/preview.sh notes.typ --png out.png  # also render the page
```

stdout is exactly **one compact JSON line** — devshell banners and
errors go to stderr (velysterm's `flake.nix` shellHook writes there
for this reason). `preview.sh` runs the prebuilt
`velysterm/target/debug/mathed_mini` directly when it is up to date
(measured: **0.14 s warm**), and only drops into velysterm's devshell
(~6 s warm, minutes cold) to rebuild after Rust-source edits. It
rejects binary loro session snapshots (emthin's `.doc` session file)
with `exit 3` and an actionable message — the loop's documents are
text; `live.sh save` (IPC `save_doc`) exports text.

The JSON is one compact line from `mathed_mini --layout-json` (the
sibling `velysterm` checkout, same path dependency the emthin build
uses). It reports, per `\app` figure:

| field | meaning |
|---|---|
| `key` | `f<stmt-index>` — the per-pass key emthin also reports on IPC |
| `line` | 1-based source line of the `\app` statement |
| `caption` | the text between the marker pair |
| `w`, `h` | declared size from the statement |
| `id`, `launch` | binding glob and launch command, when present |
| `page`, `rect` | where it actually landed (`{x,y,w,h}` in page points) |

plus `page_count`, per-page sizes, and a `diagnostics` array:
statements that dangle (missing markers, unparseable size) so
`transform` silently places nothing, and figures that overflow their
page. A compile failure is an **error**, not an empty report — a bad
edit reads as "broke the compile", never as "deleted the figures".

Layout comes from Typst's own paged model — the same
`typst::compile::<PagedDocument>` pass emthin's docui lays out with,
so tier-1 geometry is what tier 2 will composite against. Nothing is
rasterised unless `--png` is passed; a ten-page document costs a
compile and a frame walk.

**The loop:** edit `notes.typ` → `preview.sh` → read JSON → fix
diagnostics or adjust sizes → repeat. Reach for `--png` (or tier 2)
only when the diagnostics are clean and the question is aesthetic —
spacing taste, colour, whether a caption crowds its figure.

### Tier 1b — the document's meaning, no compile

```sh
./scripts/agent/preview.sh notes.typ --semantic   # defs, uses, dangling ids — one line
```

Same contract (one compact JSON line), one layer deeper:
`defs` (name, 1-based line, `uses` = the occurrences
`SemanticIndex::plan_rename` would edit), `unresolved` (free
identifiers that resolve to no definition — one row per name, and
application-argument identifiers are excluded, so `$norm(x)$`
reports `norm`'s referents, not its dummy `x`), `models` (the bare
name when the statement carries one, else the body's leading
identifier), `statements` (counts by kind), `translators`,
`templates`. Every key is present even when empty — absence must be
readable as a claim.

The difference that matters: **no Typst compile.** Typst eval
rejects unknown multi-letter math variables, so a document
containing `$foo$` fails tier 1 with `layout failed: Eval` —
honestly, because a user cannot render it either — but `--semantic`
still reports `unresolved: [{"name": "foo", "line": …}]`, which is
exactly the fact needed to fix it. (Measured: the same document's
tier-1 call exits 1; tier 1b exits 0.)

### What changed — the snapshot delta

```sh
./scripts/agent/preview.sh --diff notes.before.typ notes.typ
```

One line describing what one edit changed: figures added / removed /
changed (caption, size, id, launch, page, rect — only the fields
that actually moved), page-count and diagnostic deltas, and the
semantic delta (defs added/removed, uses moved, unresolved added/
removed, models, statement counts, translators, templates).
Figures are matched by caption first — the identity an editor
keeps when an insertion renumbers every later `figure_key` (the
caveat REWRITE_PLAN records) — then by key, which catches a caption
edit as one change instead of a removal plus an addition. Neither
document compiling is an **error naming the side**, never a silent
removal: a broken edit reads as "broke the compile", not "the
figure disappeared".

## Tier 2 — live session (real compositor, real apps)

```sh
./scripts/agent/live.sh boot                 # sway on Xvfb + emthin inside
./scripts/agent/live.sh state                # semantic: bound apps + live rects
./scripts/agent/live.sh shot /tmp/s.png      # visual: grim over wlr-screencopy
./scripts/agent/live.sh spawn kgx            # launch an app into a figure (see caveat below)
./scripts/agent/live.sh size f0 500 350      # rewrite a figure's \app args
./scripts/agent/live.sh open notes.typ       # replace the document
./scripts/agent/live.sh save out.typ         # snapshot the document
./scripts/agent/live.sh down                 # graceful teardown (session saved)
```

The session machinery is the e2e's own (`scripts/e2e/lib.sh`): sway
with `WLR_BACKENDS=x11` under Xvfb — a nested compositor that needs
no logind session — and emthin inside it, driven over the control
socket with the same `ipc.py`. `state` is the live semantic tier
(bound window ids, titles, composited rects at the output's scale);
`shot` is the visual tier, grim capturing sway's output so emthin
needs no screencopy protocol of its own.

Live rects differ from tier-1 rects by design: the compositor
letterboxes and scales the page into the output, so a 400×300 figure
arrives as e.g. 367×275 on screen. Tier 1 reports document truth,
tier 2 reports screen truth.

Each subcommand is a fresh process that re-derives its environment,
so an agent never holds state between calls beyond "the session is
up".

### Resolved: the dmabuf crash (found and fixed by this loop)

Spawning a GUI client (`live.sh spawn kgx`) used to **segfault
emthin** a few seconds in: Mesa logged `dmabuf import failed to mmap:
Operation not permitted`, then the process died with no panic (exit
139, SIGSEGV inside llvmpipe's `dri_sw_displaytarget_unmap`). The
e2e never reached it because `step7.sh` only types into the document
caret — it never maps a GUI client into a figure.

This loop found and root-caused it. The debugging path, reusable as
a recipe:

1. `live.sh boot` → `live.sh spawn kgx` → `live.sh log` shows the
   Mesa import failure repeated per frame.
2. `coredumpctl info <pid>` (or the last crash) gives the stack:
   main thread drawing a `TextureRenderElement` →
   `GlesFrame::render_texture_from_to` → llvmpipe
   `dri_sw_displaytarget_unmap` → `__munmap` on a garbage pointer.
   Memory corruption, not a logic error.
3. The log's own startup lines name the cause:
   `DMA-BUF v3 initialized (no render node or feedback build
   failed)` — emthin advertised `zwp_linux_dmabuf` while EGL had
   fallen back to software GL (llvmpipe, no DRM render node). GTK4
   protocol-correctly chose dmabuf buffers; Mesa's software import
   of them fails (EPERM on the sealed memfds) and leaves a
   half-mapped texture behind.
4. **Fix** (`winit.rs::init_dmabuf`): the dmabuf global is only
   advertised when EGL has a real DRM render node. On software GL
   the log says `DMA-BUF disabled: no DRM render node (software GL);
   clients will use wl_shm` and clients take the wl_shm fallback,
   whose buffers are CPU-copied and always import. Real-GPU runs
   are unchanged (v4 feedback path as before).

Verified live through this loop: `spawn kgx` → figure binds
(`window=1 title=…`), its pixels render in `shot` (non-trivial
content in the figure's rect), `size` reconfigures the running
client, and `down` still shuts down gracefully. A compositor must
not advertise a capability it cannot honour — that invariant is now
enforced in code, not convention.

## Replay — scripted observe → apply → assert

```sh
./scripts/agent/replay.sh scripts/agent/scenarios/figure-resize.txt
```

A scenario file drives the whole loop headlessly: `doc` loads the
document, `apply` applies an edit script (a `.patch`/`.diff applied
with patch(1), or a full replacement document — an LLM's returned
edit script can be either), `expect` asserts on the current state.
The runner executes it in a throwaway sandbox and prints a
transcript: a label line per step, one JSON observation per state
change (filter `^{` for just the data), and the `--diff` delta after
every `apply`.

The assertion vocabulary is **closed** — pages/figures/diagnostics
counts, per-figure fields (`figure f0 w == 440`, captions with
spaces), defs and their use counts, unresolved/models/statement
counts — and an unknown verb fails the run rather than being
ignored, so a scenario cannot quietly skip what it was written to
check. Failures keep the sandbox (`REPLAY_KEEP=1` keeps a passing
one) and exit 1; a compile-breaking edit is reported as exactly
that. Paths resolve relative to the scenario file, so a run is
reproducible from any cwd.

Lanes (`scripts/testing-depths.md`): replay is the **boundary** lane
— filesystem plus a real compile, no compositor, no display. The
**acceptance** lane stays `scripts/e2e/run.sh` against a real nested
session, and CI runs the replay step *first* (the e2e job in
`.github/workflows/ci.yml`) so a document-engine regression fails
before the session lane starts.

## What each tier costs

Measured 2026-10-13 on the dev machine, warm caches:

| | tier 1 | tier 1 `--png` | tier 2 `state` | tier 2 `shot` |
|---|---|---|---|---|
| compositor | no | no | sway + emthin | sway + emthin |
| GPU / display | no | no (CPU raster) | yes (nested) | yes (nested) |
| wall clock | **0.14 s** | ~0.3 s | ~1 s | ~1 s |
| typical size | 562 B JSON (~150 tokens) | ~61 KB PNG | ~200 tokens | 5–50 KB PNG |
| after a Rust edit | ~5.6 s (auto-rebuild) | +render | — | — |
| first-ever run | ~6.8 min (cold build) | | | |

The default is the left column. Eyes are the expensive part; spend
them last. The distilled editing playbook (geometry rules for "make
it look nice", failure modes) lives in
[`.agents/skills/emthin-document-editing/SKILL.md`](../.agents/skills/emthin-document-editing/SKILL.md).

The three headless additions re-measured 2026-10-15 (warm, fast
path — the same machine, the prebuilt binary, no devshell entered):

| | `--semantic` | `--diff` | replay scenario |
|---|---|---|---|
| compositor / GPU | no | no | no |
| wall clock | **0.02 s** | **0.03 s** | **0.8 s** (13 asserts) |
| typical output | 88–142 B JSON | ~526 B JSON | 2.4 KB transcript, 10 JSON lines |
| Typst compile | none | both docs | per state change |
