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
./scripts/agent/preview.sh notes.typ                # geometry + diagnostics JSON
./scripts/agent/preview.sh notes.typ --png out.png  # also render the page
```

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

## What each tier costs

| | tier 1 | tier 1 `--png` | tier 2 `state` | tier 2 `shot` |
|---|---|---|---|---|
| compositor | no | no | sway + emthin | sway + emthin |
| GPU / display | no | no (CPU raster) | yes (nested) | yes (nested) |
| typical size | ~300 tokens JSON | ~10 KB PNG | ~200 tokens | ~5–50 KB PNG |

The default is the left column. Eyes are the expensive part; spend
them last.
