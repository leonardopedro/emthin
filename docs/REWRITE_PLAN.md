# emthin rewrite: Emacs → mathed, apps as figures in a rendered document

> **Scope of this document: plan only.** All implementation is handed to
> another engineer/LLM — no implementation code is committed alongside this
> plan (a short W1 code sketch lives in Appendix A purely as design reference,
> tested neither compiled nor run; treat it as a starting draft, not finished
> work). Part 1 is the emthin/mathed rewrite; Part 2 is the ProofFlow
> adaptation (NL → logos/ControlledNaturalLanguage).

**Status:** handover plan (written for another engineer/LLM to execute).
**License constraint:** emthin stays **GPL-3.0** (keep `emthin/LICENSE` as-is).
All new code in `emthin/` is GPL-3.0. Code adapted from `driftwm/` is GPL-3.0
(same license — fine). `velysterm/` crates are MIT OR Apache-2.0 and are used
as *dependencies* (their licenses stay unchanged; the combined binary is
distributed under GPL-3.0, which MIT/Apache explicitly permits).

---

## 1. Mission

Rewrite the `emthin/` fork (currently "embeds Wayland apps inside **Emacs**
windows", driven by an Elisp layout engine over JSON-RPC IPC) so that:

1. **Emacs is gone entirely.** The shell UI is **mathed** — the document
   editor from `velysterm/` (crates `mathed_core` + `mathed_mini`).
2. **No window management "like in Emacs".** Wayland app toplevels appear as
   **images in a rendered document, like figures in a PDF**: the document is
   laid out and rasterized (Typst → CPU raster), and each app's live surface is
   composited exactly over a *figure slot* in the document flow.
3. **Paged, PDF-like document** (Typst pagination). Old "workspaces" become
   **pages**.
4. **No Bevy by default.** The default build uses only `mathed_core` +
   `mathed_mini` (CPU `typst_imaging` rasterizer, winit-free render core).
   The **Bevy `mathed` editor must keep working**: it shares the same document
   model (`\app` statements live in `mathed_core`, so the Bevy editor renders
   and edits the same documents), and the control-plane IPC is designed so the
   Bevy editor can later connect as an alternative external shell.

The compositor substrate (smithay, nested-in-winit) is **kept**. The document
rendering is done **in-process** (no shell client process): emthin itself lays
out the mathed document and composites app surfaces onto figure rects, which
it gets from **Typst frame introspection** (no IPC needed for geometry).

### What "inspired by driftwm" means here

`driftwm/` (GPL-3.0, smithay, infinite-canvas compositor) is the reference for
**window-as-canvas-content** mechanics, not for the document metaphor:

- `driftwm/src/canvas.rs` — `ScreenPos`/`CanvasPos` + `screen_to_canvas` /
  `canvas_to_screen` coordinate model (adapt to *doc-space ↔ output-space* with
  scroll instead of camera+zoom).
- `driftwm/src/grabs/{move_grab,resize_grab}.rs` — interactive move/resize grab
  patterns (adapt for figure resize that rewrites the `\app` size args).
- `driftwm/src/render/suspended.rs` + `docs/session.md` — "suspend window"
  placeholders and dormant session restore (reuse this UX for restoring
  figures whose apps are not running).
- `driftwm/src/layout/snap.rs` — optional snap behavior for figure resize.
- `driftwm/src/protocols/` — ext-workspace export patterns (`driftwm` exports
  bookmarks as workspaces; emthin will export **pages** as workspaces).

---

## 2. Locked decisions (do not re-litigate)

| # | Decision |
|---|----------|
| D1 | **In-process renderer.** emthin links `mathed_core` + `mathed_mini` (via relative path deps into `../velysterm/crates/*`), renders the doc with `typst_imaging` (CPU), composites app surfaces over figure rects. No shell process for the default UI. |
| D2 | **Figure syntax = `\app(#1, #2, W, H)` marker/property statement** (optionally `\app(#1, #2, W, H, "id")`). The span `#1..#2` is the **caption**. `W`,`H` are logical px (= pt at zoom 1). Consistent with `\bold`/`\prob`/`\cite` statement style. |
| D3 | **Paged document, PDF-like.** Typst pagination (`typst::compile::<PagedDocument>`). One page visible at a time (PgUp/PgDn / scroll). Pages replace workspaces. |
| D4 | **Full parity:** mirrors (one app shown in several figures), session restore, ext-workspace-v1 (pages as workspaces), clipboard bridge, IME bridge (text_input_v3 + DBus fcitx5), xwayland-satellite, layer-shell, pointer constraints — all kept from the current emthin. |
| D5 | **Bevy compatibility:** `\app` parsing/transform lives in `mathed_core`, so the Bevy `mathed` editor understands the same documents. The control IPC keeps a figure-rect message shape so a future external-shell mode (Bevy frontend driving emthin) is a small step, not a redesign. |

---

## 3. Landscape (verified facts)

### 3.1 `emthin/` (GPL-3.0) — the rewrite target

Cargo workspace, 3 crates: `emthin` (compositor binary, 55 `.rs` files,
~14k LOC), `emthin-clipboard` (smithay-free host clipboard proxy),
`emthin-dbus` (DBus fcitx5 frontend + in-process broker). `elisp/` (11 files)
is embedded via `include_dir!`.

Key files (current):

| File | Role | Fate |
|---|---|---|
| `crates/emthin/src/main.rs` | startup: event loop, IPC bind, winit init, clipboard, dbus, xwayland-satellite, spawn child | keep, rework spawn |
| `crates/emthin/src/winit.rs` | winit backend event loop + `render_frame` (smithay `render_output` over `active_space` + custom extras) | keep, rework render |
| `crates/emthin/src/element.rs` | `CustomElement` enum (Surface/Mirror/Solid/Label) + `EmthinRenderer` trait (render_elements! macro workaround) | extend with Doc/figure elements |
| `crates/emthin/src/mirror_render.rs` | mirror texture elements (surface-tree walk, aspect-fit) | adapt → figure compositing |
| `crates/emthin/src/state/mod.rs` | `EmthinState` + `WaylandState` (16 smithay fields) | rework fields |
| `crates/emthin/src/state/apps.rs` | `AppManager`/`AppWindow`/`MirrorView`/`SurfaceLayer`, `mirror_under`, `aspect_fit_ratio` | keep core, rework geometry source |
| `crates/emthin/src/state/workspace.rs` | `WorkspaceState` (active `Space` + inactive map, ext-workspace-v1) | replace with **pages** |
| `crates/emthin/src/state/emacs.rs` (526 ln) | Emacs child/surface/mailboxes | **DELETE** |
| `crates/emthin/src/state/migration.rs` | Emacs-driven migration policy | **DELETE** |
| `crates/emthin/src/input.rs` | winit input dispatch + Emacs prefix-chord interception | rewrite (doc caret + figure focus) |
| `crates/emthin/src/grabs.rs` | placeholder for move/resize | implement figure resize grab |
| `crates/emthin/src/ipc/*` | JSON-RPC 2.0 (`Content-Length` framing), Emacs messages | keep transport, replace message set |
| `crates/emthin/src/handlers/xdg_shell.rs` | toplevel/popup handling; "first toplevel = Emacs" heuristic | rework: all toplevels = figures |
| `crates/emthin/src/tick.rs` | per-tick work (fcitx drain, ipc pump…) | keep, rework |
| `elisp/*` | Elisp client | **DELETE** (all 11 files) |

Crucially preserved invariants/gotchas from `emthin/AGENTS.md` (49KB — read it
before touching the compositor; §8 below lists the ones that bite hardest).

### 3.2 `velysterm/` — mathed (MIT OR Apache-2.0)

- `crates/mathed_core` — doc model, **pure + typst types only** (no GUI):
  - `doc::MathDoc` — Loro CRDT text: `insert/delete/replace/undo/redo/
    snapshot()/from_snapshot()/commit()`.
  - `markers::scan(text) -> MarkerScan { markers: Vec<Marker{id,range}>,
    stmts: Vec<PropertyStmt{name,args: Vec<Arg>,range}> }`;
    `Arg::{MarkerRef{id,range}, Literal{text,range}}`;
    `resolve_segments(&MarkerScan) -> Vec<Segment{prop,kind,start_id,end_id,
    span: Option<Range>,stmt: usize,extra_args: Vec<Arg>}>`;
    `PropKind::of(name)` / `PropKind::resolve(name, args)`; marker-id helpers
    `auto_marker_id`, `next_marker_id`, `lowest_free_marker_numbers`.
  - `transform::to_render_text(doc_text, &scan, &segments, &TransformOptions)
    -> RenderOutput { text: String, map: OffsetMap }` — hides marker/statement
    tokens, applies visual properties, supports **splice points**
    (`template_splices`, `block_splices`, `annotations`) that inject caller
    markup at pinned doc positions. `OffsetMap::doc_to_render/render_to_doc`.
  - `glyphs::build_glyph_index(frame: &typst::layout::Frame, source: &Source,
    map: &OffsetMap, prelude_len: usize) -> GlyphIndex` with
    `caret_for_byte(usize) -> Option<CaretGeom>`, `byte_for_point(V2) ->
    Option<(usize,bool)>`, `rects_for_range(Range<usize>) -> Vec<RectF>`.
  - Also: `wordnav`, `search`, `completion`, `blocks` (block splitting),
    `semantics` (SemanticIndex), `accessibility`.
- `crates/mathed_mini` — **Bevy-free** frontend + headless render core
  (`default = ["gui"]`; `gui` = winit/softbuffer/accesskit/arboard):
  - `render::layout_doc(doc_text, width_pt) -> DocLayout { image: RgbaImage,
    glyphs: GlyphIndex, width, height }` (1px == 1pt); `layout_doc_with(opts)`;
    `render_world(&MiniWorld, width_pt) -> RgbaImage`;
    `rasterize_frame(&Frame)`; `render_paged(&MiniWorld) -> Vec<RgbaImage>`
    (typst-native pagination — the model for D3).
  - `world::MiniWorld` — minimal `typst::World`: shared fonts/library once per
    process; `World::file` resolves **`data:` URLs** only (see
    `decode_data_url` / `data_url_encode_payload`, both `pub(crate)` today —
    the figure-placeholder resolution (§5.2) extends this seam).
  - `render.rs` `THEME_PRELUDE` (white text, DejaVu Sans Mono 17pt, no
    ligatures/kerning, descender bottom-edge) — keep for caret fidelity.
  - Deps to note: `kernel_client`, `unfer_ffi`, `unfer_protocol`
    (path deps into `../unfer/`) are **unconditional** today. Optional
    hardening: feature-gate them behind `kernel` (see W2.5).
- `crates/mathed` — the **Bevy** editor (bevy 0.19 + velyst). Must keep
  building and must understand `\app` docs. Do not make it a dependency of
  emthin.

### 3.3 `driftwm/` (GPL-3.0) — reference for canvas/window mechanics

See §1. Files to read when implementing grabs/session/coordinate mapping:
`src/canvas.rs`, `src/grabs/resize_grab.rs`, `src/grabs/move_grab.rs`,
`src/render/suspended.rs`, `src/session.rs`, `docs/session.md`, `docs/ipc.md`.

### 3.4 `unfer/` — probability kernel (behind mathed's `\prob`)

No direct work needed. Relevant only if the `kernel` feature-gating (W2.5) is
done; `kernel_client`/`unfer_ffi`/`unfer_protocol` live here.

---

## 4. Target architecture

```
                        ┌────────────────────────────────────────────┐
                        │                emthin (GPL)                │
 winit window (host)    │  smithay nested compositor (kept)          │
 ┌───────────────────┐  │  ┌──────────────┐   ┌──────────────────┐   │
 │ rendered document │◄─┼──┤ docui (new)  │   │ FigureManager    │   │
 │  with live app    │  │  │ MathDoc      │   │  app ↔ figure    │   │
 │  figures (PDF-    │  │  │ transform    │   │  bindings, rects │   │
 │  like images)     │  │  │ typst layout │   │  mirrors         │   │
 └───────────────────┘  │  │ CPU raster   │   └───────┬──────────┘   │
                        │  └──────┬───────┘           │              │
                        │         │ figure rects      │ surface at   │
                        │         ▼                   ▼ rect         │
                        │  render: doc raster bg + app surfaces +    │
                        │          caret/cursor overlays             │
                        │  input: hit-test figures → apps, else doc  │
                        │  kept: clipboard, IME (tiv3+fcitx DBus),   │
                        │        xwayland-satellite, layer-shell,    │
                        │        pointer constraints, ext-workspace  │
                        └────────────────────────────────────────────┘
```

One `EmthinState`, no Emacs. The **document is the layout authority**:
editing the doc (including `\app` args) reflows figures, which reconfigures
app toplevels.

---

## 5. Core design

### 5.1 Document & figure model

Source syntax (D2):

```
#1 Terminal demo #2 \app(#1, #2, 640, 400)          ; caption = "Terminal demo"
#3 #4 \app(#3, #4, 320, 240, "chat")                ; explicit app id "chat"
```

- New `PropKind::App` in `mathed_core::markers`:
  - `PropKind::of("app" | "figure") -> Self::App`; keep it **non-visual,
    non-kernel**; add `is_app()` helper.
  - `Segment.extra_args` carries `Literal` args: `w`, `h` (integers, px/pt),
    optional trailing `Literal` `"id"` (app binding key), optional
    `scale:`/`fit:` literals later. Parsing helper
    `app_figure_spec(&Segment) -> Option<FigureSpec { w: i32, h: i32,
    id: Option<String> }>` in `mathed_core` (shared with the Bevy editor).
  - **Unbound** `\app` (no matching app) renders as an empty framed box with
    the caption (the "dormant figure", §5.10).
- Rendering (in `transform.rs`): the `\app` statement's tokens and markers are
  hidden like other statements; the span (caption) renders *under* the figure.
  At the span start, **splice** (existing splice machinery) the figure markup:

  ```typst
  #box(width: <w>pt, height: <h>pt)[#image("app:fig/<key>", width: 100%, height: 100%)]
  ```

  where `<key>` uniquely identifies the figure **for this render pass**
  (recommended: `"<stmt-index>-<doc-revision>"`). The caption follows on its
  own line (smaller text). The figure block is block-level (its own paragraph).

### 5.2 Figure rect extraction (frame introspection)

Mechanism: the `app:fig/<key>` image resolves (in the layout world) to a
**unique placeholder PNG**; after layout we walk the resulting
`typst::layout::Frame` tree and match `FrameItem::Image(img, size, ..)` items
by their byte payload to recover each figure's rect in page coordinates.

Concretely:

1. In `mathed_mini::world` (or a new `mathed_core::figures` module used by
   both frontends): a `FigureResolver` registry
   (`key -> Bytes`) where placeholder bytes are **deterministic 1×1 PNGs
   encoding the key** (e.g. RGBA pixel = 4 bytes of an index; or simply keep
   `HashMap<u64 /*fxhash of bytes*/, String /*key*/>` at generation time and
   compare `img.data()` hashes). `MiniWorld::file` resolves the `app:fig/`
   scheme through this registry next to the existing `data:` handling
   (note the leading-slash quirk already handled there:
   `id.vpath().get_with_slash().trim_start_matches('/')`).
2. `pub fn figures_in_frame(frame: &Frame) -> Vec<(String /*key*/, RectF)>` —
   recursive walk of `frame.items()` (`(Point, FrameItem)`) descending into
   `FrameItem::Group`; collect `FrameItem::Image(img, size, ..)` whose
   `img.data()` maps to a registered key; rect = `(point, size)` in pt
   (== px at the 1px/pt raster scale).
3. Layout entry point (new in `mathed_mini::render`):
   `layout_doc_paged(doc_text, opts, resolver) -> Vec<PageLayout>` where
   `PageLayout { image: RgbaImage, glyphs: GlyphIndex, figures: Vec<FigureRect>,
   }` built on `typst::compile::<typst_layout::PagedDocument>` (mirroring
   `render_paged`) + `build_glyph_index` per page frame + `figures_in_frame`.
   Keep `layout_doc` (continuous) working for tests/headless.

### 5.3 Layout & pages

- **Page size:** default A4-ish `595×842pt` scaled to the window width; page
  breaks come from Typst's page model (`render_paged` path), never pixel
  slicing.
- **Viewport:** the winit window shows exactly one page (letterboxed, centered)
  — the "PDF viewer" look. `current_page: usize` in docui state.
- **Doc coordinates:** page-local pt + page origin in "doc space"
  (`page_origin_y = page_index * (page_h + gap)`). Figure rect in doc-space =
  page rect + page origin. Output coords = doc coords − scroll + letterbox
  offset. (Adapt `driftwm/src/canvas.rs` naming: `DocPos`/`ScreenPos`,
  `doc_to_screen`/`screen_to_doc` with `scroll` instead of `camera+zoom`.
  Keep zoom at 1.0 in v1; the map makes adding it trivial.)
- **Reflow:** any doc edit → re-layout (cache invalidation on `MathDoc`
  revision). Reconfigure app toplevels whose figure rect changed
  (size args or reflow) using the existing **pending-geometry timeout**
  pattern (`AppManager::collect_timed_out`) so clients that never commit
  don't wedge.

### 5.4 Compositing

Render order per frame (`winit.rs::render_frame`):
1. clear;
2. **doc raster** (page image) — uploaded to a `GlesTexture`
   (`ImportMem::import_memory`) and cached until layout changes;
3. **app surfaces** at their figure rects (one element per bound figure,
   including mirror figures — same surface, multiple elements; reuse
   `mirror_render.rs` surface-tree walk + `TextureRenderElement` lessons);
4. dormant-figure placeholders (framed box + caption stand-in — already part
   of the doc raster; optionally an overlay "▶ click to start" like
   `driftwm/src/render/suspended.rs`);
5. caret/selection overlay (from `GlyphIndex::caret_for_byte`/`rects_for_range`);
6. software cursor (existing code).

**Z-order pitfall (verified in AGENTS.md):** smithay's `render_output` draws
custom elements *above* space elements, and `Space::map_element` re-stacks.
Recommended approach: keep app windows **out of the `Space` for rendering**
and composite them as custom `TextureRenderElement`s (like mirrors today)
at figure rects, while still keeping a `Space` for input/focus bookkeeping
(if kept, always `lower_element` non-figure elements per the AGENTS.md rule).
Alternative worth checking when implementing: background pass via
`renderer.render()` before `render_output` with transparent clear — see how
`driftwm/src/render/` composites its canvas background under windows and
follow that pattern (`render_output`'s 5th arg is the clear color; the old
code passed `[0.0,0.0,0.0,0.0]`).

Texture notes (from AGENTS.md, still true): mirror/figure `TextureRenderElement`
positions are **physical**; use `output.current_scale().fractional_scale()` for
logical→physical; subtract `window.geometry().loc` (CSD shadow padding) when
placing surface trees; walk the **full subsurface tree** with
`with_surface_tree_downward`; set `buffer_scale`, `buffer_transform`,
viewport `src` from `RendererSurfaceState`; `Id`s must be namespaced per
figure (`Id::from_wayland_resource(surface).namespaced(figure_id)`) or the
damage tracker collapses them.

**Aspect:** figures stretch to the figure rect by default (true "image in a
PDF" semantics); `fit: "aspect"` extra-arg opt-in to letterboxed aspect-fit
(`AppManager::aspect_fit_ratio` already exists).

### 5.5 Input routing

Focus model replaces Emacs chords entirely (delete `input_intercept` prefix
machinery):

- **Pointer:** hit-test figure rects first (topmost figure wins), map
  pointer coords into surface coords (mirror-input math exists in
  `AppManager::mirror_under` — generalize it: `figure_under(pos)`), forward
  via smithay pointer focus. Outside figures → doc: click places caret
  (`GlyphIndex::byte_for_point`), drag selects. Figure **edges** (6px border)
  start a resize grab (§5.7).
- **Keyboard:** if a figure is focused → forward to its window
  (`KeyboardFocusTarget::Window`). Else → doc editing against `MathDoc`:
  - printable → `insert`; Backspace/Delete; arrows + Shift selection
    (`wordnav` for Ctrl+Left/Right); Ctrl+Z/Ctrl+Y (and Ctrl+Shift+Z)
    undo/redo (`MathDoc` has both); Ctrl+A; Ctrl+C/X/V via **arboard**
    (same as `mathed_mini`'s gui path; the `emthin-clipboard` bridge stays
    for *client* clipboard sync — they coexist).
  - `Escape` / `Ctrl+G`: unfocus figure → doc caret. Click on figure focuses
    it. A visible border/tint marks the focused figure.
  - `Ctrl+Shift+P`-style command palette is out of scope; provide a minimal
    keymap table in `docui/keymap.rs` (document it in README):
    `Mod+Return`-less; suggest: `Ctrl+Shift+Return` spawn-app (opens the CLI
    launcher), `PgUp/PgDn` pages, `Ctrl+Shift+M` toggle mirror-of-focused…
- **IME:** keep `ImeBridge`, but the **origin translation** changes: caret
  rects are now `figure_rect.loc + (surface-local caret)` for figures, and
  `glyph_caret_rect_in_doc_space → screen` for the doc caret
  (AGENTS.md: "Emacs main surface IS the emthin winit window, origin (0,0)" —
  replace with the doc/screen mapping).
- **Prefix gating:** the old `prefix_active` IME gate disappears; gate IME on
  figure-focus vs doc-focus instead.

### 5.6 App lifecycle & binding

- **New toplevel** (`handlers/xdg_shell.rs::new_toplevel` — remove the
  "first toplevel is Emacs" heuristic and the 1×1 mapping hack):
  1. Compute preferred size = `window.geometry().size` (or 640×400 fallback).
  2. Bind to a figure:
     - an **unbound** `\app(..., "id")` statement whose id matches the
       toplevel's `app_id`/title (glob, like driftwm window rules) claims it;
     - else the first unbound `\app` statement in doc order;
     - else **auto-append** a figure at end of document:
       `#<a> <title> #<b> \app(#<a>, #<b>, <w>, <h>, "<appid>")` — allocate
       marker ids via `mathed_core::markers::lowest_free_marker_numbers` /
       `auto_marker_id`, insert via `MathDoc::insert`, commit.
  3. Configure the toplevel to the figure's `w×h` with all four
     `Tiled*` states (AGENTS.md: terminals need this to hit exact sizes).
- **Destroy:** unbind; the `\app` statement stays (becomes dormant, §5.10).
- **Mirrors (D4):** several `\app` statements may share one id → all bound
  figures render the same surface; input works in each (mirror-input math).
  `add_mirror`/`promote_mirror` IPC verbs become figure ops
  (`clone_figure` = copy the statement text elsewhere in the doc).
- **Visibility:** figures on non-visible pages still exist; send frame
  callbacks only for figures on the current page (saves work; apps on other
  pages stay idle — document this behavior).

### 5.7 Figure move/resize grabs (driftwm-inspired)

`grabs.rs` (currently a placeholder):

- **Resize:** Alt+drag on figure edge (or plain drag on the edge — pick one;
  recommend plain drag on edge, Alt+drag anywhere = move per driftwm parity)
  starts `FigureResizeGrab` which live-previews the rect and, **on release,
  rewrites the `\app` args** (`MathDoc::replace` on the `Literal` arg ranges —
  `Arg::Literal.range` gives exact doc byte ranges) and commits. Reflow then
  reconfigures the app. Snap to grid of 8px optional (driftwm
  `layout/snap.rs` pattern).
- **Move:** Alt+drag inside the figure = reorder in the flow: move the whole
  `\app` statement + caption span text to the drop position (between blocks;
  `mathed_core::blocks::split_blocks` gives block boundaries). v1 acceptable:
  move only reorders statement blocks; free 2D placement is *out of scope*
  (PDF-like flow is the point).
- Keyboard: focused figure + Alt+arrows resize by 8px (rewrites args).

### 5.8 Spawn & CLI

`cli.rs` changes:

```
emthin [OPTIONS]
  --spawn <CMD> [--spawn-arg <ARG>]...   launch an app (repeatable)
  --doc <PATH>                           open document (default: session/new)
  --standalone                           (removed with elisp; or repurposed
                                         to mean "load bundled demo doc")
  --fullscreen / --wayland-socket / --xkb-* / --log-file / --dbus-isolated
  --xwayland-display / --xwayland-satellite-bin   (kept)
  --session-file <PATH>                  nested-session state (driftwm convention)
```

Default command no longer "emacs"; default is **no auto-spawn** (document
starts empty or from session). `--command/--arg` may remain as aliases of
`--spawn` for compat.

Env plumbing (kept): `WAYLAND_DISPLAY` set to `state.socket_name` for
children; `DBUS_SESSION_BUS_ADDRESS` injected by the broker; xwayland-satellite
supervisor unchanged.

### 5.9 Control-plane IPC

Keep `ipc/{connection,jsonrpc}.rs` transport verbatim. Replace the Emacs
message set (`ipc/messages.rs`, `ipc/dispatch.rs`) with driftwm-`msg`-style
control (documented in a new `docs/ipc.md`):

```
→ compositor:  spawn {cmd, args}, close {figure}, focus {figure},
               set_figure_size {figure, w, h}, clone_figure {figure},
               goto_page {page}, open_doc {path}, save_doc {path},
               list_state {}
← compositor:  state {page, figures: [{key, app_id, title, rect, bound}]},
               figure_created/figure_destroyed, app_title_changed,
               doc_saved {path}, error {...}
```

This is also the seam for the future **external-shell mode** (Bevy mathed
driving emthin): `list_state`/figure rect reporting is exactly what a remote
frontend needs. ext-workspace-v1 (`protocols/workspace.rs`) is re-pointed at
**pages** (diff-based refresh + action queue pattern is reusable as-is).

### 5.10 Session restore (driftwm `docs/session.md` UX)

- State dir: `$XDG_STATE_HOME/emthin/` (or `--session-file`):
  - `doc.loro` — `MathDoc::snapshot()` bytes;
  - `session.json` — `{ version, current_page, figures: [{key, app_id,
    spawn: {cmd,args}, bound_size}], xwayland_display }`.
- Save on graceful exit + periodic autosave (driftwm saves as-you-work; do
  the same — snapshot is cheap).
- Restore: load doc (figures come back with it via `\app` statements), match
  `session.json` entries by figure key/id, **spawn nothing automatically** —
  each figure renders as a dormant stand-in (framed placeholder + app name +
  "Enter to launch", driftwm `render/suspended.rs` UX). Enter/click on a
  dormant figure runs its saved `spawn` command and binds on toplevel map.
- Nested sessions don't save unless `--session-file` given (driftwm rule).

### 5.11 Pages ↔ workspaces (ext-workspace-v1)

- `WorkspaceState` (active Space + inactive HashMap swap) is **deleted**;
  replaced by `docui::pages` + `current_page`.
- `protocols/workspace.rs`: export one workspace per page (name = page
  number or first heading), `activate` = `goto_page`. The
  "Compositor is the single source of truth; IPC and protocol operate on the
  same state" invariant is kept — page state now *is* doc state.

### 5.12 Infra adjustments (kept subsystems)

- **Clipboard** (`emthin-clipboard`, `clipboard_bridge.rs`): unchanged for
  clients. Doc editing clipboard goes through arboard (W2 note: two clipboard
  users must not fight — doc copy sets host selection via arboard; the bridge's
  suppress counters already handle echo).
- **IME**: §5.5. Keep `state/ime.rs` design (ImeOwner enum, deferred
  `ime_enabled`, ordering rules) — only the origin translation and the
  prefix gate change.
- **XWayland**: `xwayland_satellite/` untouched (it's just another Wayland
  client whose toplevels become figures like any other).
- **Layer-shell** (`handlers/` + `LayerMap`): kept; non-exclusive zone still
  shrinks `usable_area()`; `relayout_emacs()` becomes `relayout_doc()` (page
  letterbox recomputed against the usable zone).
- **Pointer constraints / relative pointer / cursor shape / dmabuf /
  fractional-scale / viewporter / data-control**: untouched.

### 5.13 Bevy `mathed` compatibility (D5)

- `PropKind::App` + `app_figure_spec` + `figures_in_frame` + placeholder
  resolution live in **`mathed_core` / `mathed_mini`**, so the Bevy editor
  renders figures in its own view with no emthin dependency (it already
  renders via velyst/typst; add the same `app:fig/` file resolution to its
  world — one small PR in `velysterm/crates/mathed`).
- Verify after the mathed_core change: `cargo test -p mathed_core -p
  mathed_mini -p mathed` in `velysterm/` (bevy still builds).
- Optional (nice-to-have, not blocking): `--external-shell` mode where emthin
  skips in-process doc rendering and accepts figure rects over IPC (§5.9)
  from a connected frontend. Design leaves the door open; do not build it in
  the first pass.

---

## 6. Work breakdown

Order matters: velysterm-side model first (testable in isolation), then the
emthin surgery, then parity features. Each step lists files and a check.

### W1 — `mathed_core`: `\app` statement (velysterm)

1. `markers.rs`: `PropKind::App` variant + `of("app"|"figure")` +
   `is_app()`; make sure `resolve_segments` keeps `extra_args` (it already
   does). Parse helper `app_figure_spec(seg: &Segment) -> Option<FigureSpec>`
   (`w`,`h` from the first two `Literal` args as i32; optional third Literal =
   id, accept quotes or bare).
2. `transform.rs`: emit the figure splice for `App` segments (§5.1): hide
   statement + marker tokens (existing behavior), splice
   `#box(width:Wpt,height:Hpt)[#image("app:fig/<key>",…)]` + caption line at
   the span start via the existing splice-point machinery (mirrors
   `template_splices` handling; respect the grapheme-boundary splice
   contract). Decide key format `<stmt_index>-<revision>`; thread the
   revision through `TransformOptions` (new field `doc_revision: u64`,
   default 0 keeps old tests green).
3. Tests (module tests in `markers.rs`/`transform.rs`, follow existing style):
   scan/resolve of `\app(#1,#2,640,400,"x")`; segment extraction; render text
   contains the `app:fig/` image and hides tokens; unbound figure still
   renders a box; caption text preserved.
4. Check: `cargo test -p mathed_core` (in `velysterm/`).

### W2 — `mathed_mini`: placeholders + paged layout + figure rects

1. `world.rs`: `FigureResolver` (key → placeholder PNG bytes; deterministic
   1×1 PNG generator keyed by index) and resolution of the `app:fig/` scheme
   in `World::file` next to `data:` (mind the leading-slash strip).
2. New `figures.rs` (mathed_core, if it needs only `typst::layout::Frame` —
   prefer mathed_core so the Bevy editor reuses it; else mathed_mini):
   `figures_in_frame(&Frame) -> Vec<(String, RectF)>` recursive walk matching
   `FrameItem::Image` payloads against the resolver's bytes.
3. `render.rs`: `layout_doc_paged(doc_text, opts, resolver) -> Vec<PageLayout>`
   on `typst::compile::<typst_layout::PagedDocument>` (pattern copy from
   `render_paged`), building per page: raster (`rasterize_frame`),
   `build_glyph_index` (per-page source mapping — recheck `walk_records`
   span bounds for paged), `figures_in_frame`.
4. Tests: figure rect extraction round-trip (markup with two `app:fig` images
   at known positions → rects); paged layout splits pages; no-figure docs
   unchanged.
5. W2.5 (optional hardening): feature `kernel` gating
   `kernel_client`/`unfer_ffi`/`unfer_protocol` and the kernel modules; add
   `features = ["kernel"]` where `mathed`/`mathed_mini` binaries need it.
   Only if the unconditional unfer deps make emthin's build heavy or
   fragile; otherwise skip.
6. Check: `cargo test -p mathed_mini` + `cargo test -p mathed` in `velysterm/`
   (Bevy mathed must still compile!).

### W3 — emthin: remove Emacs/Elisp

1. Delete `elisp/`, `crates/emthin/src/state/emacs.rs`,
   `crates/emthin/src/state/migration.rs`; strip `include_dir!` embed
   (`lib.rs`/`util.rs`), `elisp_dir`, `EmthinState.emacs`,
   `migration_policy`, spawn-standalone elisp extraction.
2. Delete the Emacs IPC message set (`ipc/messages.rs` variants,
   `ipc/dispatch.rs` handlers) — will be replaced in W7.
3. Delete the prefix-chord interception in `input.rs` (three-way dispatch),
   `FocusState`'s `prefix_saved_focus` (keep `layer_saved_focus`),
   `ime` prefix gating.
4. `handlers/xdg_shell.rs`: drop "first toplevel = Emacs" heuristic
   (`EmacsState::should_claim_main` etc.), drop child-frame detection and
   `pending_emacs_toplevels`.
5. `state/workspace.rs` gutted (W8 replaces it); keep only `active_space`
   temporarily so the tree compiles between steps.
6. Check after W3: `cargo check -p emthin` compiles with stubs; run
   `cargo clippy --workspace -- -D warnings` at the end of each subsequent W.

### W4 — emthin: docui (document model + layout) — new module `src/docui/`

- `docui/mod.rs`, `docui/model.rs`: wraps `MathDoc` + scan + segments +
  `TransformOptions` (caret reveal via `reveal` ranges — mirror mathed_mini
  `app.rs` behavior) + undo/redo + `dirty` flag.
- `docui/layout.rs`: drives `layout_doc_paged`; caches `Vec<PageLayout>`;
  `doc_to_screen`/`screen_to_doc` (§5.3); `current_page`, `scroll`.
- `docui/figures.rs` (or `state/figures.rs`): `FigureManager` —
  `Figure { key, stmt_idx, arg_ranges, page, rect_doc: Rectangle<i32,Logical>,
  spec: FigureSpec, app_id: Option<u64>, dormant: bool }`; binding rules
  (§5.6); `figure_under(pos)`; mirror groups by id.
- `docui/edit.rs`: text-editing operations on `MathDoc` used by input
  (insert/delete/replace + commit; marker-id allocation for auto-append).
- Check: unit tests for binding + arg rewriting (pure logic, no compositor).

### W5 — emthin: rendering

1. `winit.rs::render_frame`: compose doc raster + figure surfaces + caret +
   cursor (§5.4). Doc texture cached (`dirty` from W4).
2. `element.rs`: add `CustomElement::Doc(TextureRenderElement<GlesTexture>)`
   and `Figure=TextureRenderElement<GlesTexture>` variants; keep the
   `EmthinRenderer` trait workaround (render_elements! macro limitation).
3. Port `mirror_render.rs` → `figure_render.rs`: one element per figure
   (namespaced Ids), surface-tree walk, geometry-offset compensation,
   buffer_scale/transform/src plumbing (all in AGENTS.md gotchas).
4. Keep `render_output` for popups/layer-shell (or fold into custom pass —
   follow driftwm's `render/` layering if the custom-element ordering fights
   you).
5. Check: run nested (`emthin` inside a host Wayland session or
   `WinitBackend`), verify a doc renders and a spawned app lands in its box.

### W6 — emthin: input + grabs

1. `input.rs` rewrite (§5.5): figure hit-test first, then doc caret;
   keyboard routing; Esc/Ctrl+G; arboard clipboard for the doc.
2. `grabs.rs`: `FigureResizeGrab` (§5.7) writing back `w`/`h` args via
   `Arg::Literal.range` byte ranges; optional `FigureMoveGrab` (block
   reorder). Reference driftwm `resize_grab.rs` for grab state-machine shape
   (`PointerGrab` impl, `motion`/`button`/`unset`).
3. `docui/keymap.rs`: the small global keymap (documented).
4. Check: manual nested run — click places caret, type, click figure, keys go
   to app, resize figure reconfigures app.

### W7 — emthin: spawn, IPC, CLI

1. §5.8 CLI; §5.9 IPC messages (`messages.rs`/`dispatch.rs` new set;
   `OutgoingMessage::method_name` manual snake_case note in AGENTS.md still
   applies).
2. Auto-append + binding on `new_toplevel` (§5.6) incl. marker-id allocation.
3. `docs/ipc.md` documenting the control protocol.
4. Check: `cargo test -p emthin` (existing xwayland-satellite tests still
   pass) + new tests for arg-rewriting/binding helpers.

### W8 — parity: pages + ext-workspace

1. Replace `state/workspace.rs` with page state (§5.11); `goto_page` op;
   PgUp/PgDn; page letterboxing in `relayout_doc()`.
2. Re-point `protocols/workspace.rs` to pages (diff refresh + action queue
   kept).
3. Check: `driftwm msg`-style `emthin ipc goto-page` (or a test client)
   switches pages; an ext-workspace bar lists pages.

### W9 — parity: session restore + dormant figures

1. §5.10 save/load (`doc.loro` + `session.json`), `--session-file`,
   autosave; dormant stand-in rendering + Enter/click launch.
2. Check: kill and relaunch — doc returns, figures dormant, Enter launches
   the saved app into the same figure.

### W10 — docs + cleanup

1. **Rewrite `emthin/AGENTS.md`** (the current one is Emacs-centric):
   preserve the "Key Gotchas" and infra sections verbatim where still true
   (smithay/winit/mirror/IME/clipboard/xwayland gotchas — §8), replace the
   design-philosophy part with this document's architecture, update module
   map and IPC tables.
2. `README.md` (+ `README_cn.md`): new usage — document, `\app` figures,
   keymap, CLI; drop Emacs/Elisp; keep GPL-3.0 badge; add attribution note:
   "document engine: mathed (velysterm, MIT/Apache); canvas mechanics
   adapted from driftwm (GPL-3.0)".
3. CHANGELOG entry (Conventional Commits: `feat!: replace Emacs shell with
   mathed document figures` — cliff.toml strips `chore:`/`style:`).
4. Delete stale docs (`docs/superpowers/plans/*` migration plan is
   Emacs-specific), keep `docs/{clipboard,input-method}-*.md`.

### W11 — verification gate

```bash
# velysterm (Bevy mathed must still build!)
cd velysterm && cargo test -p mathed_core -p mathed_mini -p mathed_biblio \
  && cargo build -p mathed_mini --features gui && cargo build -p mathed

# emthin
cd emthin && cargo fmt --all --check \
  && cargo clippy --workspace -- -D warnings \
  && cargo build --workspace \
  && cargo test -p emthin
```

Manual E2E (nested under any host compositor):
1. `emthin --spawn foot --spawn firefox` → document with two figures, apps
   inside, captions auto-typed.
2. Edit text around a figure → figure reflows, app reconfigures.
3. Resize figure via drag → `\app` args rewritten, app resizes.
4. `\app(#1,#2,640,400,"foot")` copy of one statement pasted elsewhere →
   mirror figure showing the same app.
5. PgDn → second page; ext-workspace bar lists pages.
6. Quit, relaunch → session restored, dormant figures, Enter relaunches.
7. IME commit into an app figure and into the doc caret; clipboard
   host↔client and doc-copy all work.

## 7. Deletions checklist

- `emthin/elisp/` (11 files) + `include_dir!` embed + standalone extraction
- `emthin/crates/emthin/src/state/emacs.rs`, `state/migration.rs`
- Emacs IPC message set + `prefix_*` messages + `input_intercept` chords
- `EmthinState.{emacs, migration_policy, elisp_dir}`, `FocusState.prefix_saved_focus`
- `--command "emacs"` defaults, `--standalone` elisp semantics
- Emacs-specific AGENTS.md sections (replaced), `docs/superpowers/plans/2026-06-28-migration-policy.md`
- Child-frame detection (`pending_emacs_toplevels`), `sync-frame`-driven
  workspace swap machinery

## 8. Gotchas that survive the rewrite (from emthin/AGENTS.md — read the full list)

- `Space::map_element` re-stacks to top even with `activate=false` — never let
  anything cover a figure (or keep figures out of the Space entirely, §5.4).
- GPU readback on winit backend: `ExportMem::copy_framebuffer` inside
  `backend.bind()`, `map_texture` **after** `backend.submit()`.
- Trust `backend.window_size()` (not `output.current_mode().size`) for
  physical framebuffer dims (fractional-scale lag).
- `Scale::Fractional(scale_factor)`, `Transform::Flipped180`, `render_scale`
  = 1.0 in `render_output`.
- Mirror/figure elements: physical coords, buffer_scale/transform/viewport
  `src`, subsurface-tree walk, `geometry().loc` subtraction, namespaced Ids.
- winit 10-10-10-2 pixel format fix lives in the **smithay fork**
  (`emskin/smithay` branch `emskin-patches`; the fork also carries
  `WinitEvent::Ime`, text_input patches, seat_data fix). Keep using it.
- `render_output` type params: `(output, renderer, framebuffer, alpha=1.0, 0,
  spaces, custom_elements, damage_tracker, clear_color)`; 2nd type param is
  the **custom** element type; `render_elements!` macro can't parse
  associated-type bounds → `EmthinRenderer` trait workaround.
- embedded toplevels must be mapped in a Space to get commits/configures
  (the old 1×1 hack) — if figures leave the Space, keep a hidden mapping
  strategy that still drives frame callbacks (send_frame per figure from
  `post_render`).
- IME: `set_ime_allowed(true)` **before** `set_ime_cursor_area`;
  `TextInputHandle::cursor_rectangle()` is per-seat; enter/leave must be
  manual (smithay `has_instance()` guard is patched out in the fork).
- DBus broker must bind before any child spawn (`DBUS_SESSION_BUS_ADDRESS`
  injection); `--dbus-isolated` uses a private dbus-daemon with
  `PR_SET_PDEATHSIG`.
- calloop gotchas: `LoopSignal::stop()` needs `wakeup()` too;
  `IpcServer::send` drains synchronously on EAGAIN (calloop IPC source is
  read-only); xclip `-loops=0` never exits (spawn, don't wait).
- xwayland-satellite on-demand supervisor: pre-bind `/tmp/.X11-unix/X<N>` +
  abstract socket, arm calloop `Generic` sources, spawn on first connect,
  `ToMain::Rearm` after crash; `Unlink` RAII in `xwayland_satellite::sockets`.
- Toolchain: emthin pins `rust-toolchain.toml` (Rust ≥ 1.89, edition 2024
  workspace); velysterm pins 1.97.1 — rustup honors each directory's pin;
  run velysterm commands from `velysterm/`.
- `crates/emthin/Cargo.toml` keeps literal `version`/`edition` values
  (cargo-aur limitation) — keep in sync with `[workspace.package]`.
- Conventional Commits; `chore:`/`style:` are stripped from the changelog.
- Never `git push` without explicit user approval.

## 9. Risks / open points (resolve during W2/W5, not before)

1. **Paged glyph indexing:** `build_glyph_index` was built for one continuous
   frame; per-page frames need span-bound revalidation (footer/`render_len`
   clamping comments in `glyphs.rs`). If paged proves brittle, fall back to
   continuous layout with page-*view* slicing for v1 (feature-flag it).
2. **Custom-element z-order vs popups:** xdg popups of figure apps must draw
   above their figure but below nothing else important — the existing
   popup rendering path (via `render_output` spaces) may need the figure
   elements in the Space after all; decide in W5 with the AGENTS.md stacking
   rule in hand.
3. **Heavy deps:** if `mathed_mini`'s unconditional `unfer_*`/`kernel_client`
   deps break emthin's build env, do W2.5 (feature `kernel`) early.
4. **Two clipboards (arboard vs emthin-clipboard):** verify no selection
   echo loops when the doc copies while a client holds selection
   (suppress counters exist per-backend).
5. **Keymap product decisions** (spawn launcher UX, mirror toggle) are
   placeholders — keep them centralized in `docui/keymap.rs` and documented.

## 10. Definition of done

- `emthin/` builds, tests, clippy-clean; no Emacs/Elisp strings remain in the
  Rust tree; LICENSE (GPL-3.0) untouched.
- Apps appear **only** as figures in the rendered paged document; no window
  chrome anywhere (CSD-less: xdg-decoration force SSD-off is already the
  emthin behavior — keep `force ServerSide` + no decorations drawn).
- velysterm: `cargo test -p mathed_core -p mathed_mini` green and
  `cargo build -p mathed` (Bevy) green — the Bevy editor renders `\app`
  figures in its own document view.
- AGENTS.md + README describe the new architecture; docs/ipc.md documents the
  control protocol.

---

# Part 2 — ProofFlow adaptation: NL → logos/ControlledNaturalLanguage

## 11. Mission (Part 2)

Adapt **ProofFlow** (`/home/leo/Projects/ProofFlow`, outside this tree — MIT,
Python) into this project. ProofFlow does *faithful proof autoformalization*:
NL proofs → Lean 4, via a dependency-graph pipeline. The adaptation changes
the target language and the verification substrate:

> **Instead of NL → Lean, it is NL → logos/ControlledNaturalLanguage.**

Natural-language statements/proofs (authored in the mathed document of Part 1)
are decomposed into a dependency DAG, formalized into **logos' L0 controlled
natural language**, compiled/verified by **`unfer/logos`** (CCG → CoreIR →
linearity → DeltaNet interaction nets → reduce → readback → UNF hash),
evaluated probabilistically by the **unfer probability kernel**, and packaged
and extended through the **australVM plugin system** — in particular the
**logos / DeltaNets / engram** subsystem.

Licenses: ProofFlow is MIT (keep its copyright header in every ported file);
`unfer/` and `australVM/` are Apache-2.0 (australVM files carry
Apache-2.0 WITH LLVM-exception) — MIT code ports cleanly into Apache-2.0
with attribution.

## 12. Source profile — ProofFlow (verified)

Python package (`proofflow/`, `prompts/`, `benchmark_results/`, MIT):

| Stage | File | Key API |
|---|---|---|
| 1. Graph builder | `proofflow/proof_graph.py` | pydantic models `BaseTheoremComponent`, `TheoremCondition`, `Definition`, `Lemma`, `TheoremStatement`, `ProofGraphItem`; `build_proof_graph`, `validate_proof_graph`, `check_DAG` (cycle check), `condition_on_all_previous_steps`, LLM-JSON extraction helpers |
| 2. Lemma formalizer | `proofflow/proof_formalize.py` | `run_formalizer_prompt`, `extract_code_validate(text_input, lean_server)` (extract code → validate → retry) |
| 3. Tactic completer | `proofflow/proof_prover.py` | fills proof bodies under compiler feedback |
| 4. Verification | `proofflow/lean_check.py` | **Lean 4 compiler** as oracle |
| 5. Scoring | `proofflow/proof_scorer.py` | **ProofScore**: AI dependency + semantic checks |
| 6. Visualization | `proofflow/vis.py` | interactive proof DAG |

Prompts (`prompts/*.md`): `proof_graph.md` (+`proof_graph_no_DAG.md`),
`lemma_formalizer.md` / `lemma_prover.md` (+ `_no_think` variants),
`full_proof_scorer.md`, `step_proof_scorer.md`,
`proofscore_dependency_check.md`, `proofscore_semantinc_check.md`.
LLM-agnostic (Claude/GPT/Gemini/vLLM) — the LLM is the only external service.

## 13. Target profile — logos / DeltaNets / engram / kernel / plugins (verified)

- **logos** = `unfer/logos/` — “CNL-to-Verified-Execution Compiler”:

  ```
  sentence → harper_gate → CCG parse → CoreIR compile → linearity →
             interaction net (deltanet) → reduce → readback → UNF hash
  ```

  Modules: `harper_gate`, `ccg`, `core_ir` (+`core_ir::linearity`),
  `deltanet/{compiler,reducer,readback,unf,symbolic,ted,types}`,
  `engram/{table,tiered,spill,l1keys,ablation}`, `l1`, `lexicon`,
  `austral_codegen`, `cli`, `translate.rs`. L0 grammar (Sentence := NP VP,
  Det N, RelClause, …) over `corpus/lexicon.tsv` (46 words → CCG categories +
  semantic templates). Confluence reference: `logos/lean/Confluence.lean`.
- **Kernel seam** (`unfer/prob_kernel/src/logos.rs`):
  `logos_compile(sentence) -> LogosReport`, `austral_unf(source) ->
  AustralReport`. Protocol op `logos_compile` (`unfer/docs/PROTOCOL.md`):
  `{"sentence"}` → `{"unf", "unf_hash", "verified"}` — “compile a CNL
  sentence with an embedded lexicon to a CoreIR interaction-net unique normal
  form (S31)”. Error **UK-4804 `AustralUnfFailed`**: not translated to a
  unique normal form through DeltaNets.
- **Probabilistic evaluation**: `prob_kernel::Session` through `kernel_client`
  (`unfer_agent` NDJSON) over `unfer_protocol::KernelStatement` — the
  `\model` / `\prob` / `\prior` / `\solver` family mathed's
  `semantics::SemanticIndex` already emits.
- **Plugin system** (`australVM/lib/Vm_plugin.mli`, S36): unified
  application/VM plugin registry — `compiler_service { name; compile }`,
  `register_compiler`, `run_compiler`, `boot`, `reset`; existing plugins:
  `deltanet_plugin.ml` (test `test/DeltanetPluginTest.ml`), `why3_plugin.ml`,
  `npu_dma_plugin.ml`. PROTOCOL.md: “Modules *are* the project's skills
  (australVM is the plugin engine, `module.toml` + `modhost` the plugin
  slots, `uk_*` the capability surface)” — hosted-module grants are
  deny-by-default (`MATHED_KERNEL_LANGS`, `MATHED_EXEC_GRANTS`).
- **engram** = `logos::engram` (conditional-memory lookup; see
  `australVM/docs/ENGRAM.md`: “Arm (b), UNF keys, is this repo's
  `logos::engram`”): O(1) UNF-keyed tables, `tiered`/`spill` storage,
  `l1keys` (`logos::l1::split_l1` / `aggregate_results`).
- **DeltaNets** = the interaction-net backend: canonical execution +
  content-addressable identity (`deltanet::unf::unf_hash` = SHA-256 over the
  canonical serialization).

## 14. Substitution table — the NL→Lean ⇒ NL→logos/CNL pivot

| ProofFlow (NL→Lean) | Adapted (NL→logos/CNL) |
|---|---|
| Target language Lean 4 | logos L0 ControlledNaturalLanguage (`logos/src/lexicon` + `corpus/lexicon.tsv`) |
| Lean 4 compiler (`lean_check.py`) | `prob_kernel::logos::logos_compile` → `LogosReport { unf, unf_hash, verified }` + `deltanet::reduce` |
| Tactic completer (`proof_prover.py`) | **CNL completer**: iteratively extend/repair the L0 sentence until `verified` (harper/CCG/linearity/UK-4804 error loop) |
| Lemma = theorem with tactics | Lemma = CNL sentence; probabilistic claims additionally become `\model`/`\prob` kernel statements evaluated numerically by `prob_kernel::Session` |
| Node identity = name | Node identity = **UNF hash** (content-addressed; also the engram key) |
| ProofScore (AI dependency + semantic) | **LogosScore**: dependency check (DAG faithfulness — port as-is) + semantic check (kernel numeric agreement + readback round-trip) |
| `vis.py` interactive graph | same vis, hosted in the mathed document as an `\app` **figure** (Part 1) |
| Python LLM calls | `LlmClient` trait (OpenAI/vLLM-compatible default); prompts ported 1:1 |

## 15. Architecture (Part 2)

New home: `unfer/logos/src/formalize/` (inside the logos crate — it already
owns lexicon/ccg/core_ir/deltanet/engram):

- `graph.rs` — port of `proof_graph.py` models + `build_proof_graph` (LLM),
  `validate_graph`, `check_DAG`, `condition_on_all_previous_steps`.
- `formalizer.rs` — per-node CNL generation (port of `run_formalizer_prompt`)
  + `extract_validate` retry loop: `harper_gate::lint` →
  `ccg::parse_sentence` → `compile_to_core_ir` → `insert_linearity` →
  `deltanet::compile_to_net` → `reduce` → `readback` → `unf_hash` (or the
  kernel op `logos_compile` end-to-end).
- `completer.rs` — the tactic-completer analogue: repair loop over typed
  errors (UK-4804, lint/parse/linearity failures).
- `score.rs` — LogosScore (dependency + semantic checks; prompts ported).
- `memory.rs` — engram-backed lemma store: `logos::engram` table keyed by UNF
  hash → (CNL text, readback, provenance); `l1keys` for weighted retrieval;
  `spill`/`tiered` for scale. Retrieval feeds few-shot examples into the
  formalizer prompts (retrieval-augmented autoformalization).
- `llm.rs` — `LlmClient` trait + OpenAI/vLLM-compatible HTTP default; keep
  the Python package as a **reference oracle** in `tools/` during the port.
- `prompts/` — ported `prompts/*.md` (`include_str!`).

Kernel/protocol: first path uses the **existing** `kernel_client` /
`unfer_agent` NDJSON `logos_compile` op per node (no protocol change); a batch
`cnl_formalize` op in `unfer_protocol` + `prob_kernel::logos` is an optional
optimization (UK-code surface stays `uk_*`).

australVM plugin: `australVM/lib/formalize_plugin.ml` (+`.mli`,
`test/FormalizePluginTest.ml`) modeled on `deltanet_plugin.ml`, registered via
the `Vm_plugin` registry (`register_compiler`/`boot` pattern), packaged as a
hosted module (`module.toml` + `modhost` slot, `uk_*` grants,
deny-by-default allowlists in the `MATHED_KERNEL_LANGS` / `MATHED_EXEC_GRANTS`
style).

## 16. Work breakdown (Part 2)

| # | Task | Files | Check |
|---|---|---|---|
| P1 | Graph model + DAG check (pure Rust port of `proof_graph.py`) | `logos/src/formalize/graph.rs` | unit tests from ProofFlow's JSON shapes; `check_DAG` cycle tests |
| P2 | Formalizer loop over `logos_compile` (verify-per-node; UK-4804 retry) | `formalizer.rs`, `llm.rs` | golden NL→CNL nodes; `verified: true` + stable `unf_hash` |
| P3 | Engram lemma memory (UNF-keyed insert/lookup, few-shot retrieval) | `memory.rs` + `logos::engram` | lookup round-trip; prompt assembly includes retrieved lemmas |
| P4 | Completer + LogosScore (prompts ported; numeric semantic check via `Session`) | `completer.rs`, `score.rs` | score report on a fixture DAG |
| P5 | australVM `formalize_plugin` + `module.toml` packaging | `australVM/lib/formalize_plugin.ml`, `test/` | `dune runtest` / `run_ocaml_tests.sh`; plugin boots via `Vm_plugin.boot` |
| P6 | CLI: `logos formalize <nl.md> --out report.json --vis out.html` | `logos/src/cli` | end-to-end on a sample proof; HTML DAG renders |
| P7 | mathed/emthin integration: NL blocks = lemma units (`mathed_core::blocks::split_blocks`); formalized CNL shown via existing `TransformOptions.{annotations, block_splices}` splice machinery; `\model`/`\prob` segments → `kernel_client`; new `\formal(#s,#f,...)` statement family in `mathed_core::markers` (same pattern as `\app`, Part 1); DAG viewer hosted as an `\app` **figure** | velysterm + emthin | document round-trip; figure hosts the vis app |
| P8 | Evaluation harness + docs: ProofFlowBench-style corpus → CNL targets; LogosScore vs human DAGs; `unfer/docs/FORMALIZE.md` | `tools/`, `docs/` | benchmark script runs; docs reviewed |

## 17. Risks / open points (Part 2)

1. **L0 coverage**: 46-word lexicon vs arbitrary NL proofs — stage the grammar
   extension; surface harper/CCG failures through the completer loop instead
   of silently dropping nodes.
2. **Multi-sentence lemmas**: L0 is one-sentence — split nodes at graph build
   time (the DAG is the natural seam).
3. **LLM nondeterminism vs content-addressed identity**: cache per
   (node text, prompt version, model) in engram memory; UNF hash is the
   canonical id, not the CNL text.
4. **Unique-normal-form failures** (UK-4804) block identity — follow
   `logos/lean/Confluence.lean` semantics; report as node errors, never hash a
   non-unique NF.
5. **Python↔Rust parity** during the port — keep ProofFlow runnable as an
   oracle; compare DAGs and scores on the fixture corpus.

---

# Appendix A — W1 code sketch (designed, NOT implemented)

Design reference for the executor of W1 (left unimplemented deliberately —
this is a draft, untested and uncompiled). Everything here is already
specified in §6/W1; this appendix pins the exact shapes.

**`velysterm/crates/mathed_core/src/figures.rs`** (new module; add
`pub mod figures;` + re-exports to `lib.rs`):

```rust
//! App figures: `\app(#s, #f, w, h[, id])` segments rendered as
//! image placeholders in the document flow.
use typst::layout::{Frame, FrameItem};
use crate::glyphs::{RectF, V2};
use crate::markers::Arg;

pub const FIGURE_URL_PREFIX: &str = "app:fig/";
pub const FIGURE_ALT_PREFIX: &str = "app:fig:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FigureSpec {
    pub w: i32,              // pt == logical px at zoom 1, always > 0
    pub h: i32,
    pub id: Option<String>,  // app binding key, quotes stripped
}

/// Parse extra args: `w, h[, id]`. `None` unless w,h are positive ints.
pub fn app_figure_spec(extra_args: &[Arg]) -> Option<FigureSpec> {
    let lit = |arg: &Arg| match arg {
        Arg::Literal { text, .. } => Some(text.trim()),
        Arg::MarkerRef { .. } => None,
    };
    let w = lit(extra_args.first()?)?.parse::<i32>().ok()?;
    let h = lit(extra_args.get(1)?)?.parse::<i32>().ok()?;
    if w <= 0 || h <= 0 { return None; }
    let id = extra_args.get(2).and_then(lit).map(|s| {
        s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(s).to_owned()
    });
    Some(FigureSpec { w, h, id: id.filter(|s| !s.is_empty()) })
}

/// Stable-per-layout key: statement index into `MarkerScan::stmts`.
pub fn figure_key(stmt_idx: usize) -> String { format!("f{stmt_idx}") }

/// Typst block spliced at the caption span's start (raw, trusted markup).
pub fn figure_markup(spec: &FigureSpec, key: &str) -> String {
    format!(
        "#block(breakable: false, inset: 0pt)[#image(\"{FIGURE_URL_PREFIX}{key}\", \
         alt: \"{FIGURE_ALT_PREFIX}{key}\", width: {}pt, height: {}pt)]",
        spec.w, spec.h
    )
}

pub fn figure_alt_key(alt: &str) -> Option<&str> {
    alt.strip_prefix(FIGURE_ALT_PREFIX)
}

#[derive(Debug, Clone, PartialEq)]
pub struct FigureRect {
    pub key: String,
    pub rect: RectF,   // frame points == raster px at 1px/pt
}

/// Walk a laid-out Frame (descending into `FrameItem::Group`) and
/// report each `FrameItem::Image(img, size, _)` whose `img.alt()`
/// carries `FIGURE_ALT_PREFIX`; rect = accumulated offset + size.
pub fn figures_in_frame(frame: &Frame) -> Vec<FigureRect> { /* see below */ }
```

Walker body (mirror `glyphs.rs::walk_records`'s offset accumulation):

```rust
fn walk_figures(frame: &Frame, offset: V2, out: &mut Vec<FigureRect>) {
    for (p, item) in frame.items() {
        let pos = offset + V2::new(p.x.to_pt() as f32, p.y.to_pt() as f32);
        match item {
            FrameItem::Image(img, size, _) => {
                if let Some(key) = img.alt().and_then(figure_alt_key) {
                    let w = size.x.to_pt() as f32;
                    let h = size.y.to_pt() as f32;
                    out.push(FigureRect {
                        key: key.to_owned(),
                        rect: RectF::new(pos.x, pos.y, pos.x + w, pos.y + h),
                    });
                }
            }
            FrameItem::Group(group) => walk_figures(&group.frame, pos, out),
            _ => {}
        }
    }
}
```

**`markers.rs`** — add `PropKind::App` variant (doc comment: app figure,
span = caption), map it in `PropKind::of` (`"app" | "figure" => Self::App`),
and add `pub fn is_app(self) -> bool { matches!(self, Self::App) }`.

**`transform.rs`** — three seams in `to_render_text_range`:

1. After `translator_title_points`, before `let template_points` (so
   `bounds` is in scope), build the splice points (figure always splices,
   even when the statement token is revealed — the placeholder is the app
   window's anchor):

   ```rust
   let app_figure_points: Vec<(usize, String)> = segments
       .iter()
       .filter(|seg| seg.kind.is_app())
       .filter_map(|seg| {
           let span = seg.span.as_ref()?;              // dangling stmts skip
           if span.start < range.start || span.end > range.end { return None; }
           let spec = crate::figures::app_figure_spec(&seg.extra_args)?;
           let key = crate::figures::figure_key(seg.stmt);
           debug_assert!(is_grapheme_boundary(doc_text, span.start));
           bounds.push(span.start);
           Some((span.start, crate::figures::figure_markup(&spec, &key)))
       })
       .collect();
   ```

2. In the emit loop, first of the splice loops (before `template_points`):

   ```rust
   for (pos, markup) in &app_figure_points {
       if *pos == start {
           pin_splice_point(start, &mut out, &mut map);
           out.push_str(markup);
       }
   }
   ```

3. Trailing-range loop (after the `template_points` one) for a figure at
   exactly `range.end`, same `pin_splice_point(range.end, …)` shape.

**Verified typst-0.15.1 API facts used above** (checked against
`~/.cargo/registry/src/index.crates.io-*/typst-library-0.15.1/`):
`FrameItem::Image(Image, Size, Span)` (`layout/frame.rs:494`),
`Image::new(kind, alt: Option<EcoString>, scaling: Smart<ImageScaling>)`
and `Image::alt() -> Option<&str>` (`visualize/image/mod.rs`),
`RasterImage::plain(Bytes, impl Into<RasterFormat>) -> StrResult`
(`ExchangeFormat::Png`), `Frame::new(Size, FrameKind::Hard)`,
`frame.push(Point, FrameItem)`, `frame.push_frame(Point, Frame)`,
`Group.frame` — matching `glyphs.rs::walk_records`'s usage.

**Planned tests** (executor writes them): `app_figure_spec` parsing (ids,
quotes, rejects), `figure_key`/`figure_alt_key` round-trip, `figure_markup`
contents, `figures_in_frame` on a hand-built frame (nested `push_frame` group
offset accumulation; non-alt images ignored) using the 1×1 PNG const from
`world.rs`'s test base64
(`iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ…` decoded to 70 bytes),
`scan`/`resolve_segments` of `\app(#1, #2, 640, 400, "term")` →
`PropKind::App` + span `2..7` for `#1 cap #2 …`, and a transform test:
render text contains the `#block(breakable: false, inset: 0pt)[#image(
"app:fig/f0" …` splice and the caption, hides `\app(` and the markers.
Remember `marker_index` is **first-occurrence-wins**, so in-statement refs
(`#1` inside `\app(#1, #2, …)`) never affect the caption span.

---

# Execution log

Status of the work breakdown in §6, as executed. Everything in the
verification gate (§6/W11) passes:

```
# velysterm
cargo fmt --all --check                                              ✓
cargo test -p mathed_core -p mathed_mini -p mathed_biblio            ✓ 439 tests
cargo build -p mathed_mini --features gui                            ✓
cargo build -p mathed                                               ✓  (Bevy editor)

# emthin
cargo fmt --all --check                                              ✓
cargo clippy --workspace --all-targets -- -D warnings                ✓
cargo build --workspace                                              ✓
cargo test --workspace                                               ✓ 198 tests
```

| # | Status | Notes |
|---|---|---|
| W1 | done | `PropKind::App`, `mathed_core::figures`, the transform splice. Appendix A's shapes used, with two deliberate deviations (below). |
| W2 | done | `app:fig/` resolution in `MiniWorld::file`, `layout_doc_paged` returning `PageLayout { image, glyphs, figures, … }`. The `FigureResolver` registry of §5.2 was **not** needed: the figure key travels in the image's `alt` text, so one shared 1×1 placeholder covers every figure and geometry is recovered from the frame directly. |
| W3 | done | `elisp/`, `state/emacs.rs`, `state/migration.rs` and the Emacs IPC set are gone. No Emacs/Elisp string remains in the Rust tree. |
| W4 | done | `docui/{mod,model,layout,figures,edit,keymap}.rs`. |
| W5 | done | `doc_render.rs` (page raster → GPU, cached by `(page, revision)`), `figure_render.rs` (app surfaces over figure rects + overlays), `winit.rs::render_frame` composes page → figures → overlays → cursor. |
| W6 | done | Figure-first input routing, document text editing, `FigureResizeGrab`, `docui/keymap.rs`. |
| W7 | done | New CLI and IPC set, auto-append figure binding with glob ids, `docs/ipc.md`. |
| W8 | done | `state/page.rs` replaces `state/workspace.rs`; ext-workspace-v1 re-pointed at pages (ids are page index + 1). |
| W9 | partial | Document snapshot + `session.json` (current page) save on graceful exit and autosave, and restore on start; a dormant figure is framed, labelled with the app name and both relaunch gestures, and `Return` or a click over it runs its `\app`'s `launch:` command. That is §5.10's stand-in. Still missing: the `spawn` prompt for typing an *arbitrary* command into a slot — the last item, and it needs a text input surface of its own. |
| W10 | done | `AGENTS.md` rewritten, `README.md` + `README_cn.md`, `docs/ipc.md`, `docs/build-notes.md`, CHANGELOG entry, stale migration-policy docs deleted. |
| W11 | done | Gate above. Manual E2E under a host compositor is **not** run — no nested-compositor session was available in this environment. |

## Deviations from this plan, and why

1. **`fit: "stretch"` on the figure placeholder.** Not in Appendix A.
   Typst's default `fit` is `"cover"`, which preserves the placeholder's
   aspect ratio and only *clips* the overflow; `FrameItem::Image`'s
   `size` is that pre-resize box, so a covered 1×1 placeholder reports a
   **square** rect and every figure's geometry would be wrong. Stretching
   makes the frame item's size *be* the figure rect, and it's what the
   figure wants anyway (a slot to paint into, not artwork to crop).
   Found by the `continuous_layout_reports_figure_rects` test.

2. **`figure_key` is not revision-stamped.** §6/W1 suggests
   `<stmt-index>-<revision>` threaded through `TransformOptions`.
   A key only has to be unique within **one** render pass — it is minted
   by `transform` and consumed by `figures_in_frame` on the frame that
   same pass produced — so the revision only churns the ids a frontend
   caches between edits. Appendix A's `f<stmt-index>` is used.

3. **Workspace → page conversion happened in W3, not W8.** The plan had
   W3 keep a single `active_space` "temporarily" for five work items
   and W8 replace the model. Since `WorkspaceState` was going away
   anyway, converting it once avoids writing a throwaway intermediate.
   Net result is the same; the diff is smaller.

4. **`Control IPC → document IPC` was one pass.** §6 had W3 delete the
   Emacs message set and W7 replace it. Doing that in one go avoids
   writing handlers for messages that are about to be removed.

5. **`AppWindow.workspace_id` → `page`.** Mirrors the page model; the
   field is only read to decide "is this app on the visible page".

6. **The caption renders at body size, not "smaller text".** §5.1 asks for
   a smaller caption line. Achieving that needs a `#text(...)[…]` bracket
   opened at the caption's start and closed at its end, spanning windows
   — and `emit_plain_text` does **not** escape `]`, so a user typing
   `]` in a caption would break the layout (the same fragility the
   existing visual-span openers have). Cosmetic gain, real risk: skipped.

7. **`\app` figures don't get a `FigureResolver` key→bytes registry.**
   See W2 above.

8. **A pre-existing `mathed_core` bug was fixed in passing.**
   `markers::parse_arg` required marker ids to start with an ASCII
   digit, but `auto_marker_id` emits RFC-1751 *words* — so every
   editor-generated marker (`#ad`, `#o`, …) parsed as a `Literal` and its
   statement silently lost its span. `\app` figure statements are
   created by the compositor with auto-named markers, so this blocked
   the whole design. `parse_arg` now accepts the same id shape
   `try_parse_marker` does; regression test
   `markers::tests::auto_named_marker_refs_resolve_like_numeric_ones`.

9. **`mathed_mini` dead code is now `#[cfg(feature = "gui")]`-gated.**
   emthin depends on mathed_mini with `default-features = false`, which
   exposes four unused-function warnings in a dependency's build. Not
   strictly emthin's business, but it made the build noisy for anyone
   who takes the workspace gate literally.

## Known gaps

- **No manual E2E run.** §6/W11's seven-step manual test (spawn, edit,
  resize, mirror, page switch, session restore, IME) needs a nested
  compositor session; none was available. The automated gate covers
  everything statically checkable, and the docui/figure/binding logic is
  unit-tested, but the render and input *wiring* has not been exercised
  against a real client.
- **Dormant-figure affordance** is complete: `Return` over the figure
  relaunches it, a left click on it relaunches it, the launcher prefers the
  pointed-at figure, and a dormant figure carries an inset border so an empty
  slot is not indistinguishable from a failed app. The launch command is a
  `launch:` argument on the `\app` statement itself, so nothing outside the
  document has to be kept in step with it. Still no `spawn` prompt for typing
  an arbitrary command, and no label on the figure (`6bd91c8` closed the
  session half of W9).
- **`session.json` no longer has a `figures` array.** §5.10 sketched
  `{key, app_id, spawn, bound_size}` per figure. Three of those four are now
  wrong rather than missing: `spawn` lives in the statement as `launch:`,
  `bound_size` is the `\app` width and height (the document owns geometry),
  and `app_id` is a Wayland id from a previous process, meaningless on
  restore. `xwayland_display` is still carried for diagnostics only.
- **The launcher** (`Ctrl+Shift+Return`) has no prompt. It relaunches the
  pointed-at dormant figure, which is unambiguous, but a `spawn` prompt needs
  a text input surface of its own — a bigger decision than a key binding.
- **`figure_key`'s doc comment** says "stable for the lifetime of the
  statement", but deleting a statement shifts later statements'
  indices. `FigureManager::sync` releases the deleted figure's app, so
  the behaviour is correct; the key's stability is *relative to edits
  that keep the statement*. A key derived from something other than the
  statement index (e.g. a hash of the caption) would be stable under
  deletion, at the cost of no longer being readable.

## Part 2

Part 2 (§11–17) is the ProofFlow → `logos`/CNL adaptation. It landed in
`unfer/logos` and is documented in full in `unfer/docs/FORMALIZE.md`; this log
records only what landed *here*, in emthin and velysterm, and what the plan's
P7 row actually asked for.

| step | where | commits |
|---|---|---|
| P1–P6, P8 | `unfer`, `australVM` | `e8a37c6`, `fdec6e0`, `0dd064d`, `2194f5b`, `cb9b521`, `08c8cd6`, `8234ecd`, `8af9052` |
| P5 (VM side) | `australVM` | `d719bc7d` |
| P7a/b (the statement) | velysterm | `4fd1fe4`, `df50318`, `5dae026`, `18dc43e` |
| P7c (the kernel seam) | emthin | `d401742` |

### P7, as built

The row asked for four things. Three of them turned out differently than the
row assumed, and the differences are the interesting part.

**NL blocks as lemma units.** Not built. `blocks::split_blocks` was not the
seam: a proof step in the document is already a `\formal` statement with a
caption, so the lemma unit *is* the step. Splitting paragraphs into blocks would
have added a layer between the reader's sentence and the claim, with nothing
checking whether the two agreed.

**`\model`/`\prob` → `kernel_client`.** Not built, for the same reason the
architectural note in `AGENTS.md` gives: geometry flows document → compositor →
client and the *document* is the authority. A `\formal` step states what should
be true; it does not get to decide it. `kernel_client` remains the compositor's
subprocess surface, and `docui/formals.rs` calls `logos unf` through it rather
than growing a second kernel path.

**`\formal` spliced via `annotations`/`block_splices`.** Built, with one
correction: the *declaration* uses `block_splices` (it is content the reader
typed and it belongs in the document flow) while the *verdict* uses
`annotations` (it is a result, and results have their own priority tier). Using
one tier for both made the verdict either overwrite the claim or hide beneath
it. A declared hash is not displayed at all — a document must not be able to
assert an identity it did not check.

**The DAG viewer as an `\app` figure.** Built, and it needed no new code. The
HTML from `logos formalize --vis` is self-contained — no external `src`, no
network — so it is an ordinary figure payload. `\formal` being non-visual is
what makes this work: the claim and the viewer of the whole proof can sit in
one document without either disturbing the other, which
`the_dag_viewer_and_the_claim_are_figures_in_the_same_document` now asserts.

### Verification

`cargo test -p emthin` gives 164 lib + 20 integration. Two tests are
`#[ignore]`d because they need a `logos` build, and both pass against one:
they are the only checks that the subprocess contract holds, including that the
kernel's own rejection reason ("words not in the lexicon: Euler") survives to
the document instead of degrading to a generic failure.

### Still open

- **No manual E2E run** — same limitation as Part 1 (no nested session), and it
  now also covers the `\formal` badge's rendered appearance in a real page.
- **Dormant-figure affordance** (W9 partial, above) also bounds P7d: the DAG
  figure reserves its slot, and both `Return` and a click over it relaunch the
  viewer.
