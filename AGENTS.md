# Design Philosophy

**The document is the layout authority.** emthin is a nested Wayland
compositor whose shell UI is a
[mathed](https://github.com/voxell-tech/velyst) document: Typst-flavoured
source text in a Loro CRDT, with hidden markers (`#1`, `#2`) and property
statements (`\app(#1, #2, 640, 400)`). Wayland applications appear as
**figures in that document** — like images in a PDF.

```
              ┌──────────────────────────────────────────────┐
              │   document  (mathed_core: MathDoc + markers) │
 winit window │   layout  (mathed_mini: Typst → CPU raster)   │
 ┌───────────┐│   ┌──────────────────────────────────────┐    │
 │  page 1   ││   │ #1 term #2 \app(#1,#2,640,400)     │    │
 │ ┌───────┐ ││   │ #3 chat #4 \app(#3,#4,320,240,"c") │    │
 │ │ fig 1 │◄┼───┤   └──────────────────────────────────────┘    │
 │ └───────┘ ││   geometry of each figure recovered from the  │
 │  fig 2    ││   laid-out frame (alt-text on a placeholder)   │
 └───────────┘│                                              │
              └──────────────────────────────────────────────┘
```

There is **no layout policy in the compositor**. No window manager, no
tiling, no placement rules, no Elisp. An `\app` statement reserves a
slot in the page flow; a bound app's live surface is composited exactly
over that slot. Editing the statement's numbers rewrites the document,
the page reflows, and the app is reconfigured.

**Consequences:**

- **The document is the single source of truth.** A figure's size lives
  in the `\app` arguments, not in compositor state. Anything that wants
  to change a figure's geometry edits the text.
- **Geometry flows document → compositor → client**, never the reverse.
  A client can request a resize (xdg_shell), but the document wins.
- **New features go in `docui/`**, not in a layout engine somewhere
  else. If a change needs compositor state that the document doesn't
  already express, that's a signal the document model is missing
  something.
- **Pages replace workspaces.** Typst's pagination is the only page
  model; there is no workspace stack, no per-workspace `Space`.

---

# emthin workspace

Cargo workspace, three crates:

```
crates/
├── emthin/            # compositor binary, docui/, handlers/, tests/
├── emthin-clipboard/  # smithay-free host clipboard proxy
└── emthin-dbus/       # DBus fcitx5 frontend + in-process broker
```

```
emthin      ──→  emthin-clipboard
       ├──→  emthin-dbus
       ├──→  mathed_core   (path dep: ../../../velysterm)
       └──→  mathed_mini   (path dep, no `gui` feature)
```

- `emthin-clipboard` **cannot** `use smithay` — it's a self-contained
  host clipboard proxy usable by any nested Wayland compositor. The
  smithay-aware glue lives in `src/clipboard_bridge.rs`.
- `mathed_mini` is built with `default-features = false`: emthin owns
  the window, so mathed_mini's winit/softbuffer/accesskit/arboard
  frontend is dead weight. The document *engine* is still used
  in-process — see "Rendering" below.

## Testing

```
cargo test -p emthin
```

- `tests/xwayland_satellite.rs` — pure pieces (socket pre-binding,
  spawn-command construction)
- `tests/xwayland_satellite_watch.rs` — calloop watch integration

The `docui`, `figure_render`, `keymap`, `session` and figure-binding
tests are plain unit tests: they are pure logic over the document and
need no compositor.

---

# Architecture

## Module map

```
crates/emthin/src/
├── main.rs              startup: event loop, IPC bind, winit, clipboard, dbus, spawn
├── lib.rs               module declarations + state re-exports
├── cli.rs               clap args + --spawn command-line splitting
├── util.rs              spawn_child, graceful_kill, logging
│
├── docui/               THE SHELL. Everything user-facing lives here.
│   ├── mod.rs           DocUi facade: load/save/tick, page access
│   ├── model.rs         DocModel: MathDoc + scan + segments + caret/selection
│   ├── layout.rs        DocLayoutCache: paged rasters + doc↔screen map
│   ├── figures.rs       FigureManager: one Figure per \app, app↔figure binding
│   ├── edit.rs          document edits the compositor makes (resize, append, clone)
│   └── keymap.rs        the global key bindings
│
├── doc_render.rs        the page raster's trip to the GPU, cached by revision
├── figure_render.rs     compositing: app surfaces over figure rects + overlays
│
├── winit.rs             winit backend, render_frame, post_render, host events
├── input.rs             figure-first input routing, document text editing
├── grabs.rs             MoveDialogGrab, ResizeGrab, FigureResizeGrab
├── tick.rs              per-event-loop-iteration work
├── element.rs           CustomElement enum + EmthinRenderer trait
├── mirror_render.rs     legacy xdg_toplevel-issued mirror views
├── session.rs           session.json: per-figure launch commands
│
├── ipc/                 JSON-RPC control protocol (docs/ipc.md)
│   ├── connection.rs    Content-Length framing
│   ├── jsonrpc.rs       JSON-RPC 2.0 envelope
│   ├── messages.rs      IncomingMessage / OutgoingMessage (hand-written conversions)
│   └── dispatch.rs      handlers: thin shims onto docui
│
├── handlers/
│   ├── xdg_shell.rs     toplevel/popup; `new_toplevel` defers classification one tick
│   ├── apps.rs          figure binding, app lifecycle, configure-to-figure
│   ├── dialogs.rs       floating dialogs (not figures)
│   ├── compositor.rs    wl_surface attach/detach
│   ├── seat.rs          wl_seat
│   ├── selection.rs     data_device / primary_selection
│   ├── dmabuf.rs        linux-dmabuf import
│   └── output.rs        wl_output
│
├── protocols/workspace.rs   ext-workspace-v1, one workspace per page
├── state/
│   ├── mod.rs           EmthinState, WaylandState, SelectionState, FocusState
│   ├── apps.rs          AppManager, AppWindow, SurfaceLayer, aspect-fit math
│   ├── page.rs          PageState (current_page + pending_toplevels) + ext-workspace
│   ├── host.rs          host window title/app_id + spawned child processes
│   ├── cursor.rs        cursor image + raw pointer tracking
│   ├── ime.rs           ImeBridge: text_input_v3 + fcitx DBus
│   ├── dbus.rs          DbusBridge (in-process broker)
│   ├── focus.rs         KeyboardFocusTarget + saved-focus slots
│   └── xwayland.rs      display cache + satellite supervisor
│
└── xwayland_satellite/     on-demand XWayland supervisor (niri pattern)
```

## State sub-structs

`EmthinState` groups its state; access via `self.<substruct>.<field>`.

- `wl: WaylandState` — 16 smithay protocol fields
- `apps: AppManager` — the app catalog, mirror table, pending-geometry timeouts
- `page: PageState` — the single `Space`, `current_page`, pending toplevels,
  ext-workspace handle
- `doc: DocUi` — document text, layout, figures
- `doc_page: DocPageTexture` — the page raster on the GPU
- `host: HostState` — host window title/app_id, spawned children
- `ime: ImeBridge` — text_input_v3 global, focused surface, deferred
  `ime_enabled`
- `cursor: CursorState` — cursor image + raw pointer location
- `focus: FocusState` — saved-focus slots (`FocusOverride::Host`) and
  `last_app_focus`
- `selection: SelectionState` — clipboard backend, origins, and the
  document's own `arboard` handle
- `dbus: DbusBridge` — the in-process broker

## The document model

`\app(#s, #f, W, H[, "id"])` is a property statement like any other:

- the span between `#s` and `#f` is the **caption** — ordinary
  document prose the user types and edits;
- `W`, `H` are the figure's size in logical px (== pt at zoom 1);
- the optional trailing `"id"` is the app binding key, a **glob**
  (`"foot*"` claims both `foot` and `footclient`).

`mathed_core::transform` hides the statement's tokens and splices a
block-level placeholder image (`app:fig/f<stmt-index>`, `fit: "stretch"`,
`alt: "app:fig:f<stmt-index>"`) at the caption's start. Typst lays that
out like any image, and `mathed_core::figures::figures_in_frame` walks
the resulting frame, reads the key back out of the image's `alt`, and
reports each figure's rect. **No IPC is involved in geometry.**

`fit: "stretch"` is load-bearing: typst's default `fit` is `"cover"`,
which preserves the placeholder's aspect ratio and only *clips* the
overflow — `FrameItem::Image`'s `size` is that pre-resize box, so a
covered placeholder would report a square rect.

## Rendering

`winit.rs::render_frame` composes, bottom-up:

1. **the document page** — `DocPageTexture::element`, a `TextureRenderElement`
   over the page raster, cached and re-imported only when
   `(page, doc revision)` changes;
2. **app surfaces over their figure rects** — `figure_render::build_figure_elements`,
   one group per bound figure on the visible page;
3. **document overlays** — `build_overlay_elements`: selection boxes, the
   focused figure's border, the caret;
4. **the software cursor** — topmost.

All of these are `CustomElement`s passed to `render_output`'s
`custom_elements` slot. App toplevels stay mapped in the `PageState`
`Space` **only** so they keep receiving configures and frame callbacks
(unmapped toplevels never commit); their pixels are composited by
`figure_render`, outside the `Space`'s z-order.

The doc raster's pixels are white text on transparent (mathed_mini's
`THEME_PRELUDE`), so a dark host shows the page over whatever is behind
it. `blit_over_bg`-style compositing lives in mathed_mini's frontend,
not here.

## Invariants (every session)

1. **Layer-shell changes reflow the page.** The document is centred in
   the non-exclusive zone; `relayout_doc` recomputes the letterbox and
   reconfigures every figure whose size moved.
2. **`crates/emthin/Cargo.toml` keeps literal `version`/`edition`**
   because cargo-aur 0.x doesn't support `version.workspace = true`.
   Both it and `[workspace.package]` must bump together.

---

# emthin-clipboard

Self-contained host clipboard proxy for nested Wayland compositors. Zero dependency on smithay — the sibling `emthin` crate does the smithay-aware glue in `src/clipboard_bridge.rs`.

## What this crate exports

```
ClipboardBackend    trait — host-facing clipboard proxy
ClipboardEvent      enum — HostSelectionChanged / HostSendRequest / SourceCancelled
SelectionKind       enum — Clipboard / Primary (crate-independent of smithay)
Driver<'a>          enum — OwnedFd(BorrowedFd) or Piggyback
AsyncCompletion     struct — X11-only pipe-drain completion token
BackendHint         enum — DataControl / WlDataDevice{display_ptr} / X11
init(&[BackendHint])  factory that walks the fallback chain
```

## Backend fallback chain

| Variant | Transport | Needs focus? | Notes |
|---|---|---|---|
| `DataControl` | `ext_data_control_v1` or `zwlr_data_control_v1` on a fresh `$WAYLAND_DISPLAY` connection | No | Preferred path; mirrors wlroots / KDE ≥ 6.2 behavior. |
| `WlDataDevice { display_ptr }` | `wl_data_device` on a **foreign** wl_display (caller-owned, e.g. winit's) via `Backend::from_foreign_display` | Yes | Only works while the parent surface has host keyboard focus. Primary selection not implemented here. |
| `X11` | X11 selection via `$DISPLAY`, XFixes-watched | — | For X11 hosts (Xorg / Xvfb). Supports INCR for large payloads. |

`init(&hints)` tries each hint in order and returns the first backend that handshakes. Caller decides the order.

## Driving the backend

```rust
match backend.driver() {
    Driver::OwnedFd(fd) => {
        // Register fd with event loop (READ, level-triggered).
        // Call backend.dispatch() on readable.
    }
    Driver::Piggyback => {
        // No owned fd — the connection is drained elsewhere.
        // Call backend.dispatch() every tick.
    }
}
// After dispatch, drain events:
for event in backend.take_events() {
    match event { ... }
}
```

## Key principles

1. **No smithay**: this crate is reusable by any nested compositor. `SelectionKind` is our own enum; the host maps it to smithay's `SelectionTarget` at the boundary.
2. **`Driver` expresses the fd contract, not a hidden one**. `WlDataDeviceProxy` returns `Piggyback` because it genuinely has no owned fd; we don't manufacture a dummy fd to fit a unified shape.
3. **`HostSendRequest::completion` is the only X11-specific API surface in an otherwise uniform event**. Wayland backends always set it to `None`; X11 emits `Some(AsyncCompletion { id, read_fd })` and the caller must drain `read_fd` then call `ClipboardBackend::complete_outgoing(id, data)`. The default `complete_outgoing` impl is a no-op so Wayland backends stay silent.
4. **Anti-loop via suppress counters**: when we set a host selection, the host will echo back the change as `HostSelectionChanged`. Each backend has `suppress_clipboard` / `suppress_primary` counters (not booleans — Firefox sets selection twice in quick succession) that eat the echo.
5. **`BackendHint::WlDataDevice` is the unsafe surface**: it holds a raw `*mut wl_display` and the caller must guarantee lifetime via `unsafe BackendHint::wl_data_device(ptr)`. Everything else in the public API is safe.

## Deps

- `wayland-client` (+ `wayland-backend` with `client_system` feature for `Backend::from_foreign_display`)
- `wayland-protocols` + `wayland-protocols-wlr` for the data-control definitions
- `x11rb` with `xfixes` for the X11 backend
- `libc` for `pipe2` in the X11 backend's outgoing request path

No smithay, no calloop, no tokio — the crate is runtime-agnostic.

---

# emthin-dbus — DBus session-bus protocol primitives + in-process broker

Zero smithay deps. Provides the SASL handshake scanner, DBus v1 frame
parser + encoder, per-connection byte-stream state machine, fcitx5
frontend classifier / reply synthesis, **and** the full in-process
broker IO loop (listener, upstream dialing, per-connection pumps with
`SCM_RIGHTS` fd passing, fcitx5 signal emitters).

History: started out as a subprocess (`emthin-dbus-proxy` binary) +
JSON ctl socket for cursor-coord rewrite. M1 pulled the broker
in-process under `emthin/src/dbus_broker/`. M2 replaced the
cursor-rewrite hack with a full fcitx5 DBus frontend intercept (B1).
M3 added `SCM_RIGHTS` fd passing so portal.Secret / portal.FileChooser
clients work (Feishu's `RetrieveSecret` was the canary). M4 moved the
broker out of `emthin/` and into this crate's `proxy/` module, since
it has no emthin / smithay deps — just the wire primitives in this
same crate plus libc.

## Module layout

```
src/
├── lib.rs       # crate root + ergonomic re-exports
├── wire/        # DBus wire format (zero-cost over `zvariant`)
│   ├── mod.rs
│   ├── frame.rs # Frame, FrameBuilder, BodyBuilder, Headers, MessageKind,
│   │           # FieldCode, SerialCounter, FrameError
│   └── sasl.rs  # SASL handshake scanner (find_begin_end)
├── broker/      # per-connection byte-stream state machine
│   ├── mod.rs
│   └── state.rs # ConnectionState, FeedOutcome, BrokerError
├── fcitx.rs     # fcitx5 frontend: predicates + classify + IC allocator
│                # + build_reply, all in one ~700-line module since the
│                # surface is small and single-purpose.
└── proxy/       # in-process broker IO loop (listener, upstream dial,
    ├── mod.rs   # per-connection pumps, fcitx5 intercept + signal emit)
    ├── cmsg.rs  # recvmsg/sendmsg + SCM_RIGHTS fd passing
    └── signals.rs # build_preedit_chunks (UpdateFormattedPreedit chunks)
```

## Scope matrix

| Feature | Done | Future |
|---|---|---|
| SASL handshake scanner (`wire/sasl.rs`) | ✅ | |
| DBus v1 frame parser + encoder (`wire/frame.rs`) | ✅ | |
| Per-connection state machine (`broker/state.rs`) | ✅ | |
| Fcitx5 method_call classifier (`fcitx/classify.rs`) | ✅ | |
| Per-connection fcitx5 IC registry (`fcitx/ic.rs`) | ✅ | |
| Fcitx5 method_return synthesis (`fcitx/reply.rs`) | ✅ | |
| In-process broker IO loop (`proxy/mod.rs`) | ✅ | |
| `SCM_RIGHTS` fd passing (`proxy/cmsg.rs`) | ✅ | |
| `RequestName` local-own interception → closes emthin#60 | | ✅ |
| `ListNames` / `NameOwnerChanged` merging for policy | | ✅ |

## Architecture

```
embedded app (WeChat / Electron / GTK / Feishu)
       │
       │ DBus (bus.sock injected via DBUS_SESSION_BUS_ADDRESS)
       ▼
┌──────────── emthin-dbus::proxy ─────────────┐
│  DbusBroker (recvmsg/sendmsg + SCM_RIGHTS)  │
│    ├─ ConnectionState (wire/sasl + frames)  │
│    ├─ fcitx::classify (InputMethod1 /       │
│    │                   InputContext1)       │
│    ├─ fcitx::build_reply (method_return)    │
│    └─ FrameBuilder::signal                  │
│        (CommitString / UpdateFormattedPreedit)│
└──────┬──────────────────────────────────────┘
       │ non-fcitx5 methods pass through, fds round-trip
       ▼
  upstream host session bus (real fcitx5 stays untouched)
                  ↑
       OR a private `dbus-daemon` child when emthin is run with
       `--dbus-isolated` — the upstream socket is whatever path the
       consumer hands to `DbusBroker::bind`. From this crate's
       perspective there's no difference: it's still a Unix socket
       speaking DBus. The fcitx5 path keeps working because emthin's
       host fcitx5 reaches the winit window over Wayland (text_input_v3),
       not over this DBus bridge.
```

The consumer crate (e.g. `emthin`) wires the broker's listener fd and
each accepted connection's two fds (`client`, `upstream`) into its
event loop. From this crate's perspective those fds are just data —
calloop / mio / tokio all work the same. Tests use plain
`std::os::unix::net::socketpair` and step the pumps manually.

## Invariants

- **Parser is append-only.** `ConnectionState::feed_from_client(chunk)`
  must be called with successive socket reads; internally buffers
  partial messages. The returned `FeedOutcome.outbound` is the *exact*
  byte sequence to write to the other side — intercept sites filter
  it, not mutate it in place.
- **Encoder is little-endian only.** The parser still accepts
  big-endian input for messages the broker forwards verbatim; anything
  the broker synthesizes itself is LE because every modern Linux DBus
  client is LE and there's no value in the extra path.
- **Signals need a unique-name sender.** `fcitx::build_reply` does not
  set sender — the broker owns that (the caller tracks the real
  fcitx5 unique name via GetNameOwner-reply parsing +
  NameOwnerChanged refresh) and stamps it on the signal frame before
  encoding.
- **IC paths are opaque, not state.** `InputContextAllocator::allocate`
  hands out `(path, uuid)` for the `CreateInputContext` reply and
  forgets immediately — no per-IC state lives in the broker. emthin's
  IME state lives in `winit` + `ImeBridge`, driven by the FcitxEvent
  stream from `dbus_broker::emit_fcitx_event`. Ids are per-connection
  and monotonic so client-side stale references can't collide.
- **Serials are non-zero.** `SerialCounter::bump` skips zero on wrap;
  `next_serial == 0` violates the DBus spec and lockstep clients
  reject the frame.
- **Preedit format flags** (per fcitx5's `FcitxTextFormatFlag`,
  `fcitx-utils/textformatflags.h`): `Underline = 1 << 3`,
  `HighLight = 1 << 4`. `UpdateFormattedPreedit` chunks MUST include
  `Underline` or GTK fcitx-gtk renders the preedit as plain inline
  text (no visual distinction from committed content). The active
  segment (from winit's `(begin, end)` cursor range) gets
  `Underline | HighLight` for the inverted-color "currently composing"
  rendering — see `proxy::signals::build_preedit_chunks`.
- **`BareSignature`, not `Value::Signature`, encodes the SIGNATURE
  header.** zvariant 5 wraps multi-element signatures in `()` (it
  models them as an implicit struct); GDBus / fcitx5 reject signal
  bodies whose declared SIGNATURE includes those parens — IM signals
  silently drop. Regression test:
  `wire::frame::tests::signature_field_does_not_wrap_in_parens`.
- **`SCM_RIGHTS` rides one packet at a time.** The proxy's IO uses
  `recvmsg(MSG_CMSG_CLOEXEC)` / `sendmsg`; outbound queues are
  `VecDeque<OutPacket>` where one packet = one DBus message
  (post-SASL) and its declared `unix_fds` ride alongside that
  packet's first byte. On partial write the fds are gone — they were
  delivered with the first byte — so retry sends the remaining bytes
  with no ancillary. Pre-SASL bytes go through as one fd-less packet.

## Non-goals

- No high-level `Proxy` / `ObjectServer` API. This is raw-byte
  primitives for a broker, not a DBus service library.
- No activation fork-exec logic — all activation stays on the host bus.
- No policy / sandbox filtering. xdg-dbus-proxy's security model is
  out of scope; we use the same DBus-parsing techniques but the
  "what's allowed" question is fully answered by "emthin only
  intercepts fcitx5 interfaces, forwards everything else verbatim".

---


---

# Key Gotchas

The compositor substrate survived the rewrite unchanged, so almost
everything in this list still applies verbatim. The ones that changed
are marked **rewritten**.

## smithay / winit

- `Space::map_element` has a hidden re-stack side effect: even with
  `activate = false` it internally removes + re-appends to
  `elements.len()`, pushing the element to the top. **Figures must stay
  out of the `Space`'s z-order entirely** — they are composited as
  custom elements (see `figure_render`), and the `Space` only keeps
  them alive for configures/commits.
- smithay's winit backend defaults to a 10-10-10-2 pixel format
  (2-bit alpha) — breaks GTK semi-transparent UI. Fixed by
  prioritizing 8-bit in the smithay fork's `backend/winit/mod.rs`.
- GPU readback on the winit backend: `ExportMem::copy_framebuffer` must
  run **inside** the `backend.bind()` block while the EGL surface is
  still current, but `map_texture` must run **after**
  `backend.submit()` — `map_texture` internally `make_current`s without
  a draw surface, detaching the winit EGL surface and breaking the next
  `eglSwapBuffers` with `BAD_SURFACE`.
- Trust `backend.window_size()` (not `output.current_mode().size`) for
  physical framebuffer dimensions — on fractional-scale resizes winit
  resizes its EGL surface immediately but the output's mode is re-synced
  on the next render tick, so reading `mode.size` at capture time gives
  a lagged size and stride-mismatched pixel data.
- `winit::scale_factor()` returns 1.0 at init; the real scale arrives
  via `WinitEvent::Resized { scale_factor }`.
- Use `Scale::Fractional(scale_factor)` (not `Integer(ceil)`) to match
  the host's actual DPI.
- `render_scale` in `render_output()` is actually the **alpha**
  parameter and should be 1.0 (smallvil pattern); smithay handles a
  client's `buffer_scale` internally.
- `Transform::Flipped180` is required for correct orientation with the
  winit EGL backend.
- Use smithay's typed geometry: `size.to_f64().to_logical(scale).to_i32_round()`
  rather than manual arithmetic.
- GTK4/GTK3 send `unmaximize_request`/`unfullscreen_request`
  immediately on connect if those states were in the initial configure —
  don't set them for a figure (which is sized by the document).
- Host keyboard layout: smithay's winit backend does **not** expose the
  host keymap. `main.rs` connects a separate `wayland-client`, receives
  `wl_keyboard.keymap`, and calls
  `KeyboardHandle::set_keymap_from_string()`. Env vars
  (`XKB_DEFAULT_*`) are unreliable on KDE Wayland.

## Custom elements and textures

- `render_elements!` cannot parse associated-type bounds
  (`Renderer<TextureId = GlesTexture>`) — hence the blanket
  `EmthinRenderer` trait in `crate::element`.
- `render_output`'s **second** type parameter is the *custom* element
  type, not the space element type. Its arguments are
  `(output, renderer, framebuffer, alpha, 0, spaces, custom_elements,
  damage_tracker, clear_color)`.
- Mirror/figure `TextureRenderElement` positions are **physical** —
  convert with `output.current_scale().fractional_scale()`, never a
  hardcoded 1.0. `SolidColorRenderElement::from_buffer` wants a
  *logical* size and a physical location.
- Walk the **full subsurface tree** with `with_surface_tree_downward`;
  GTK/Firefox paint onto subsurface children, so reading only the
  toplevel yields an empty figure.
- `buffer_scale`, `buffer_transform` and the viewport `src`/`dst` must
  come from `RendererSurfaceState`, or the surface is wrong under
  fractional scaling.
- Subtract `window.geometry().loc` (and `popup.geometry().loc` for
  popups) when placing a surface tree: GTK/Chrome put CSD shadow
  padding in the buffer and use `xdg_surface.set_window_geometry` to
  mark the visible start. `SurfaceLayer::render_offset` already cancels
  it (it matches `Space::render_location()`).
- Element `Id`s must be namespaced **per figure** — the same surface
  in two figures (a mirror) needs distinct ids or the damage tracker
  collapses them and one goes blank. `figure_render::namespace` is
  FNV-1a over the figure key so it is stable across runs (a
  per-process counter would leak ids as the document is edited).
- `render_elements_from_surface_tree` can't be used for figures: its
  `Id` is hardcoded to `from_wayland_resource(surface)` with no
  namespace hook.
- Mirror scaling: aspect-fit with top-left alignment; the mapping uses
  `rel.downscale(ratio)`. `AppManager::aspect_fit_ratio` returns `None`
  for a zero-size box, to prevent NaN.

## Embedded toplevels

- An unmapped toplevel never commits, so **every** app toplevel must
  stay `map_element`'d in the `Space` — that's the whole reason figures
  are out of the `Space` for rendering but in it for lifecycle.
- Do **not** configure `(1, 1)` at `new_toplevel`: xwayland-satellite
  forwards the configure to the X client and clobbers its natural size.
  Leave the pending size unset (the initial configure goes out as
  `(0, 0)`, "client choose") and configure the real figure size once the
  toplevel is classified.
- `configure_to_figure` must set **all four** `Tiled*` states, not just
  `size`: terminal emulators (foot, alacritty) only hit an exact pixel
  size when every tiled edge is set, otherwise they pad to a cell
  boundary and the figure is a few pixels short.
- Pointer constraints are not auto-activated by smithay: call
  `PointerConstraintRef::activate` in `new_constraint` **and** on
  pointer-enter.
- Relative-pointer deltas are synthesized from successive absolute
  positions (`CursorState::consume_raw_location`) because the winit
  backend never emits relative motion.
- `surface_under` must check **figures before the Space**: a figure's
  app is not laid out by `Space::element_under`, and the Space may hold
  a *different* app mapped there.

## IME

- `set_ime_allowed(true)` **must** be called **before**
  `set_ime_cursor_area`; per spec `enable` resets text_input state to
  defaults.
- Registering `TextInputManagerState` makes fcitx5-gtk clients switch
  from DBus to `text_input_v3`, so `set_ime_allowed` must be toggled per
  focused client — `ImeBridge::on_focus_changed` probes
  `with_focused_text_input`.
- smithay's keyboard gates `text_input.enter()/leave()` behind
  `input_method.has_instance()`, which is always false here, so
  enter/leave are called **manually** with a temporary focus swap (the
  fork removes the `has_instance()` guard anyway).
- `focus_changed` cannot reach the winit backend, so the
  `set_ime_allowed` decision is stored in `ImeBridge::ime_enabled` and
  drained by `apply_pending_state` via `take_ime_enabled()` — the same
  deferred pattern as `pending_fullscreen`/`pending_maximize`.
- `TextInputHandle::cursor_rectangle()` is **per-seat**, not per-surface,
  and persists across client focus changes. `ImeBridge` keeps a
  `cursor_cache: HashMap<CursorCacheKey, Rectangle>` of the last
  fresh value per owner.
- Client-reported caret rects are in the client's own surface-local
  frame. Origin translation:
  - a **figure**'s origin is its figure rect on the visible page
    (`doc.figure_rect_on_page`), because `figure_render` places it;
  - anything else falls back to `Space::element_location` minus
    `window.geometry().loc`.
- IME has ONE owner at a time: `ImeOwner::{None, Tip{surface},
  Dbus{conn, ic_path, origin}}`.

## Clipboard

- Two clipboards coexist and must not fight:
  `state.selection.clipboard` (`emthin-clipboard`) is a **proxy** for
  client↔host sync; `state.selection.doc_clipboard` (`arboard`) is a
  **direct synchronous** handle for the document's keystroke path,
  which can't afford a round trip.
- Copying from the document sets the host selection, which the proxy
  sees as a `HostSelectionChanged` echo — that's *desired* (text copied
  out of the document must be pasteable in an app). The origin is
  recorded as `SelectionOrigin::Host` so `forward_client_selection`
  doesn't bounce it back.
- Each backend has **suppress counters** (not booleans — Firefox sets
  the selection twice in quick succession) that eat the echo.
- `IpcServer::send` drains its write buffer synchronously on EAGAIN
  (temporarily switching the fd to blocking). The calloop IPC source is
  **read-only**; `tick` pumps `flush`.
- xclip: `-loops=0` means it **never exits** — spawn it and don't
  wait. Use `Stdio::null()` for stderr.
- `BackendHint::WlDataDevice` is the only unsafe constructor (it takes
  a foreign `*mut wl_display`). Default field-drop order on
  `EmthinState` guarantees the backend drops before the display.

## calloop

- `LoopSignal::stop()` only sets a flag — always pair it with
  `wakeup()`.
- `std::process::Child::kill()` sends SIGKILL. Use
  `util::graceful_kill` (SIGTERM + 1.5s wait + SIGKILL) on the exit
  path so apps can flush their own session files.

## XWayland

- xwayland-satellite is an **external process** managed by
  `crates/emthin/src/xwayland_satellite/` with a niri-style on-demand
  supervisor: emthin pre-binds `/tmp/.X11-unix/X<N>` and the abstract
  socket, arms calloop `Generic` sources, spawns the satellite on first
  X client connect, and re-arms via `calloop::channel` +
  `ToMain::Rearm` after a crash.
- Socket/lock cleanup is owned by `Unlink` RAII guards in
  `xwayland_satellite::sockets::X11Sockets`.
- X clients arrive as ordinary Wayland clients through the satellite;
  all X-specific focus, cursor, clipboard and fullscreen policy lives
  inside satellite, not here.

## DBus

- The broker must bind **before any child spawn**: `inject_env` stamps
  `DBUS_SESSION_BUS_ADDRESS` on every `spawn_child` call.
- `--dbus-isolated` uses a private `dbus-daemon` child with
  `PR_SET_PDEATHSIG(SIGTERM)` in `pre_exec`.
- Signal `sender` is critical: DBus clients' match rules filter on the
  well-known's resolved unique name (`:N.M`), so the broker learns the
  real fcitx5 unique name from the `GetNameOwner` reply, with a
  fallback from intercepted `destination` fields; `NameOwnerChanged`
  signals refresh the cache.
- `BareSignature`, not `Value::Signature`, encodes the SIGNATURE
  header — zvariant wraps multi-element signatures in `()` and fcitx5
  drops the signal.
- fcitx5 `FcitxTextFormatFlag`: `Underline = 1 << 3`,
  `HighLight = 1 << 4`. Preedit chunks MUST include `Underline` or
  GTK renders preedit as plain inline text.

## Toolchain / release

- emthin pins `rust-toolchain.toml`; rustup honours the pin **per
  directory**, so a sibling checkout can pin a different version
  without interfering. Run that checkout's cargo commands from its own
  root.
- `crates/emthin/Cargo.toml` keeps literal `version`/`edition` (the
  cargo-aur limitation) — keep in sync with `[workspace.package]`.
- Conventional Commits; `chore:`/`style:` are stripped by `cliff.toml`.
- Never `git push` without explicit user approval.

## Wayland protocols implemented

xdg_shell · xdg-decoration (force `ServerSide`, no decorations drawn) ·
xdg_activation_v1 (**client-side only**, for host startup notification) ·
wl_seat (keyboard + pointer) · wl_data_device · wlr_data_control_v1 +
ext_data_control_v1 · fractional_scale · viewporter · text_input_v3 ·
wp_cursor_shape_v1 · linux-dmabuf · wlr-layer-shell ·
ext-workspace-v1 · wp-pointer-constraints-v1 · wp-relative-pointer-v1 ·
wm_base · shm.

---

# emthin developer patterns

Patterns derived from git history. Use these as defaults when
working in this repo; override only with explicit reason.

## Commit conventions

This repo uses **Conventional Commits**, filtered by `cliff.toml` into the
changelog.

**Scopes observed:** `release`, `ci`, `focus`, `docui`, `cli`,
`readme`, `emthin`. Scopes are optional but preferred when
the change is localized — e.g. `refactor(focus): …`.

`chore:`, `style:`, merge, and revert commits are stripped by `cliff.toml`.
Pick a different type if the change deserves a changelog line.

## Co-change patterns

These files tend to move together. When you touch one, check the others:

### IPC change → three sides in lockstep

1. `crates/emthin/src/ipc/messages.rs` — add the `IncomingMessage` /
   `OutgoingMessage` variant **and** its `from_jsonrpc` /
   `method_name` / `into_params_value` arm (conversions are
   hand-written, not derived).
2. `crates/emthin/src/ipc/dispatch.rs` — handle it.
3. `docs/ipc.md` — document it. This is not optional: the protocol's
   only consumers are external, and nothing else tells them a message
   exists.

`OutgoingMessage::method_name()` maps `XWaylandReady` →
`"x_wayland_ready"` (manual snake_case; no serde derives). A test
asserts every name is snake_case.

### Document-model change → velysterm is a separate workspace

A change to `PropKind`, the marker scanner, or the transform pipeline
lands in `../velysterm` (the sibling mathed checkout, path-depended).
Verify it there and here:

```sh
cd ../velysterm && cargo test -p mathed_core -p mathed_mini
cd -          && cargo test -p emthin
```

Adding a `PropKind` variant breaks every **exhaustive** `match` on it —
`mathed_core::accessibility::describe_segment` is the one that matters;
`mathed_mini::a11y`'s `AccessRole → Role` mapping is the other.

### Figure geometry change → document, not compositor

Anything that changes where or how big a figure is must go through
`docui::edit` (which rewrites `\app` args) and then `DocUi::relayout`.
Reconfiguring an app from anywhere else leaves the document and the
client disagreeing, and the next reflow will undo it silently.

## Versioning & release

- Workspace version lives in `[workspace.package]` in root `Cargo.toml`.
- `crates/emthin/Cargo.toml` **also** keeps a literal `version = "x.y.z"`
  because cargo-aur 0.x doesn't support `version.workspace = true`. Both
  sites must stay in sync — `cargo release` bumps them together via
  `release.toml` pre-release-replacements anchored by
  `# x-release-please-version`.
- Release is `cargo release patch --execute` (or `minor` / explicit).
  That runs `git-cliff` → updates `CHANGELOG.md` → single `chore: release`
  commit → tag → tag push.
- Historical note: the repo migrated from release-please to
  cargo-release + git-cliff; don't re-introduce release-please config.
  `.github/workflows/release.yml` was removed when the fork moved
  to a separate upstream; release automation is manual from here on.

## Local verification

Before pushing, run:

```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test -p emthin
```

If the change touches the document model (anything under `docui/`, or
`markers.rs`/`transform.rs` in velysterm), verify velysterm too — see
"Document-model change" above and `docs/build-notes.md` for the NixOS
`pkg-config` / `libxkbcommon` setup this build needs.

## Code-style defaults

- **Comments / logs / docs in Rust source: English only.** No Chinese in
  `.rs` files.
- **Never `git push` without explicit user approval.** `git commit` does not
  include push. Same for creating releases.

## smithay is a fork

When reading smithay source to trace behavior, use the checkout cargo
fetched at `~/.cargo/git/checkouts/smithay-*/<commit>/` — that revision
carries the emthin patches (`backend/winit/mod.rs`,
`text_input/text_input_handle.rs`, `selection/seat_data.rs`). A clean
upstream clone won't match what the compositor actually links against,
and these three patches each hide a bug described under "Key Gotchas".

Two API shapes worth knowing before you grep upstream smithay for an
example, because they differ from older versions:

- `KeyboardHandle` has **no** standalone keysym lookup. The only public
  way to see a key with the layout applied is inside
  `input_intercept`'s callback, which receives a `KeysymHandle`. That's
  why `input.rs` classifies the global keymap *inside* the intercept
  and stashes the result in a local.
- `PointerInnerHandle::current_focus()` returns
  `Option<(WlSurface, Point<f64, Logical>)>`, not a `WlSurface`.
- `Transform` is an enum with `Normal`/`Flipped180`/…, not a struct with
  `identity()`.
