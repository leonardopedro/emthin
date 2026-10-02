# emthin — nested Wayland compositor as a rendered document

> **Wayland applications as figures in a paged, rendered document.**

emthin is a nested Wayland compositor whose shell UI is a
[mathed](https://github.com/voxell-tech/velyst) **document**. Emacs is
gone; there is no Elisp layout engine and no window-management policy
anywhere in the compositor. Instead the screen shows a Typst-rendered
document, and each Wayland app appears **as a figure in that document** —
like an image in a PDF, sized and placed by the text.

```
#1 Terminal demo #2 \app(#1, #2, 640, 400)
#3 Chat #4 \app(#3, #4, 320, 240, "chat")
```

An `\app` statement reserves a `640 × 400` slot in the page flow. When
an app is bound to that figure its live surface is composited exactly
over the slot. Editing the numbers rewrites the document, the page
reflows, and the app is reconfigured — the document is the layout
authority, not compositor state.

---

## Status

The Emacs shell has been replaced (see `docs/REWRITE_PLAN.md` for the
design and the work breakdown). The compositor substrate — smithay,
the DBus fcitx5 bridge, xwayland-satellite, the clipboard proxy,
layer-shell, pointer constraints, ext-workspace-v1 — is unchanged.

**Not yet implemented** (planned, see the rewrite plan):

- an interactive app launcher (`Ctrl+Shift+Return` currently relaunches
  the first dormant figure's saved command);
- dormant-figure "click to start" UX (Enter-to-relaunch is wired to the
  launcher key, not to a per-figure prompt);
- free 2D figure placement — figures live in the document's flow, by
  design.

## Build

```sh
cargo build --release          # the emthin binary
cargo clippy --workspace -- -D warnings
cargo test -p emthin
```

On NixOS the build needs `pkg-config`, `libxkbcommon` and `glib` on the
search path (the `emthin-dbus` crate links `gio`). See
[`docs/build-notes.md`](docs/build-notes.md).

## Usage

```sh
# start with an empty document
emthin

# open a document and launch two apps into figures
emthin --doc notes.typ --spawn foot --spawn "firefox --new-window"

# nested session with a specific state file
emthin --session-file /tmp/emthin-demo.loro
```

Every `--spawn` is one full command line, Each occurrence is one full command line, split with quote and
backslash handling. Nothing is spawned automatically beyond what you
ask for: the document is the session, and its figures come back
**dormant** (an empty framed box with the caption) after a restart.
Launching an app the compositor has never seen appends a new `\app`
figure to the end of the document, captioned with the app's title.

### Keymap

| Binding | Action |
|---|---|
| `Ctrl+Shift+Return` | relaunch the first dormant figure's saved command |
| `PgUp` / `PgDn` | previous / next document page |
| `Home` / `End` | start / end of the document |
| `Ctrl+Home` / `Ctrl+End` | start / end of the line |
| `Ctrl+Shift+M` | clone the focused figure (add a mirror) |
| `Escape` / `Ctrl+G` | return focus from a figure to the document |
| `XF86WakeUp` | toggle focus between the focused figure and the document |

Editing is the ordinary set any editor has: printable keys insert,
`Backspace`/`Delete` delete a grapheme, arrows move (with `Shift` to
select, `Ctrl` by word), `Ctrl+A` selects all, `Ctrl+Z` / `Ctrl+Y` undo
and redo, and `Ctrl+C` / `Ctrl+X` / `Ctrl+V` use the host clipboard.

### Figures

- **Click** a figure to focus its app; **drag its edge** (within 6px) to
  resize it — the `\app` arguments are rewritten and the app is
  reconfigured. Sizes snap to an 8px grid.
- **Mirrors** are two `\app` statements sharing one binding id; the same
  app renders in both. `Ctrl+Shift+M` clones a figure's statement.
- Figures on other pages keep their apps running but idle — they stop
  receiving frame callbacks until you turn back to their page.

## Control protocol

A JSON-RPC 2.0 protocol over a Unix socket lets a bar, a script, or a
future external frontend observe and drive the document. See
[`docs/ipc.md`](docs/ipc.md).

## Design

- `docs/REWRITE_PLAN.md` — the rewrite design and work breakdown.
- `AGENTS.md` — architecture, module map, and the compositor gotchas
  that survive the rewrite.

## Licences and attribution

emthin is **GPL-3.0**; see [`LICENSE`](LICENSE).

- Document engine: [mathed](https://github.com/voxell-tech/velyst)
  (`mathed_core`, `mathed_mini`) — MIT OR Apache-2.0, used as
  dependencies.
- Canvas and grab mechanics adapted from
  [driftwm](https://github.com/) — GPL-3.0.

MIT- and Apache-licensed dependencies may be combined into a GPL-3.0
binary; their own licence terms are unchanged.