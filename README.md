# emthin — nested Wayland compositor as a rendered document

> **Wayland applications as figures in a paged, rendered document.**

emthin is a nested Wayland compositor whose shell UI is a
[mathed](https://github.com/voxell-tech/velyst) **document**. Emacs is
gone; there is no Elisp layout engine and no window-management policy
anywhere in the compositor. Instead the screen shows a Typst-rendered
document, and each Wayland app appears **as a figure in that document** —
like an image in a PDF, sized and placed by the text.

```
#1 Terminal demo #2 \app(#1, #2, 640, 400, "foot", launch: "foot")
#3 Chat #4 \app(#3, #4, 320, 240, "chat")
```

An `\app` statement reserves a `640 × 400` slot in the page flow. When
an app is bound to that figure its live surface is composited exactly
over the slot. Editing the numbers rewrites the document, the page
reflows, and the app is reconfigured — the document is the layout
authority, not compositor state.

`launch:` names the command that fills an *empty* slot. A figure with no
app bound to it is drawn as a labelled placeholder, and `Return` over it —
or a click — runs `launch:` to start the app there. It lives in the
document rather than being remembered by the compositor for the same reason
the geometry does: the document is the authority on what an `\app` means.
Omit it and the slot still reserves space, it just has nothing to start.

---

## Status

The Emacs shell has been replaced (see `docs/REWRITE_PLAN.md` for the
design and the work breakdown). The compositor substrate — smithay,
the DBus fcitx5 bridge, xwayland-satellite, the clipboard proxy,
layer-shell, pointer constraints, ext-workspace-v1 — is unchanged.

**Not yet implemented** (planned, see the rewrite plan):

- an interactive app launcher (`Ctrl+Shift+Return` relaunches the pointed-at
  or first dormant figure's saved command, with no prompt);
- dormant-figure "click to start" UX (Enter-to-relaunch is wired to the
  launcher key, not to a per-figure prompt);
- free 2D figure placement — figures live in the document's flow, by
  design.

## Build

```sh
nix develop -c cargo build --release          # the emthin binary
nix develop -c cargo clippy --workspace -- -D warnings
nix develop -c cargo test -p emthin
```

The devshell supplies `pkg-config`, `libxkbcommon` and `glib` on the search
path (the `emthin-dbus` crate links `gio`) — outside it every one of these
dies at `glib-sys`. See
[`docs/build-notes.md`](docs/build-notes.md).

<!-- status: verified | tests: 511 cargo passed, 2 ignored (workspace, nix develop; re-measured 2026-11-18 on the current worktree) + 28/28 wtype (headless sway) and 36/36 xdotool (X11 window on Xvfb) run.sh E2E green, including multi-page navigation and, on xdotool, Find cancel and fcitx5/rime preedit/commit checks (last run 2026-11-18; the earlier five-check run.sh campaign was 2026-10-15 — harness hardened against stale state, see scripts/e2e/README "Stale state" post-mortem; the tier-1 replay scenario and the E2E both run in CI) + 96 emthin-pad Rust host tests and 197 Java host checks (re-measured 2026-11-18; see AGENTS.md #emthin-android) | last_verified: 2026-11-18 -->

## Verification

```sh
nix develop -c bash -c 'cargo fmt --all --check && \
  cargo clippy --workspace --all-targets -- -D warnings && \
  cargo test --workspace'          # 511 passed, 2 ignored (2026-11-18)

./scripts/e2e/run.sh --config wtype      # 28/28, exit 0 (headless sway)
./scripts/e2e/run.sh --config xdotool    # 36/36, exit 0 (sway on Xvfb + Find + fcitx5 IME)

nix-shell -p python3 --run "python3 scripts/check-links"   # relative markdown links (CI: linkcheck.yml)
```

The E2E number is the load-bearing one: it drives a real nested session (sway on
Xvfb, emthin inside it) and checks typed text, document copy/paste, both
clipboard directions, and that emthin opens no X11 socket. `E2E_NO_X=1` reruns it
with `DISPLAY` unset, so the clipboard is exercised with no X server reachable.
Since 2026-10-15 it also runs in CI (`.github/workflows/ci.yml`, the `e2e` job:
nix devshell, `--config xdotool`, harness logs uploaded on failure) — for two
days it ran only by hand and a stale-state regression went unnoticed; the
harness reaps carried state now, see `scripts/e2e/README.md`. What each gate
can and cannot see — and how to classify the next seam regression — is
[`docs/measurement-surfaces.md`](docs/measurement-surfaces.md).

What the count does not cover: `xwayland_satellite` supervisor tests are timing
sensitive and occasionally flake; the doc/ model is shared with the sibling
`velysterm` checkout, which must be verified separately when it changes.

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
| `Ctrl+Shift+Return` | relaunch the pointed-at (or first) dormant figure's saved command |
| `PgUp` / `PgDn` | previous / next document page |
| `Home` / `End` | start / end of the document |
| `Ctrl+Home` / `Ctrl+End` | start / end of the line |
| `Ctrl+Shift+M` | clone the focused figure (add a mirror) |
| `Ctrl+Shift+V` | voice turn for the `\voice` block at the caret |
| `Ctrl++` / `Ctrl+-` | larger / smaller reading size (reflows the document) |
| `Ctrl+0` | reset the reading size |
| `Ctrl+Shift++` / `Ctrl+Shift+-` | zoom the view in / out (no reflow) |
| `Ctrl+Shift+0` | reset the view zoom and pan |
| `Ctrl+wheel` / `Ctrl+two-finger scroll` | zoom the view around the pointer |
| `Alt+Left` / `Alt+Right` / `Alt+Up` / `Alt+Down` | pan the zoomed page |
| `Ctrl+Shift+S` / `Ctrl+Shift+L` | cycle the open citation's CSL style / locale |
| `Ctrl+Shift+R` | reset the open citation's style and locale |
| `Escape` / `Ctrl+G` | return focus from a figure to the document |
| `XF86WakeUp` | toggle focus between the focused figure and the document |

Editing is the ordinary set any editor has: printable keys insert,
`Backspace`/`Delete` delete a grapheme, arrows move (with `Shift` to
select, `Ctrl` by word), `Ctrl+A` selects all, `Ctrl+Z` undoes and
`Ctrl+Shift+Z` (or `Ctrl+Y`) redoes, and `Ctrl+C` / `Ctrl+X` / `Ctrl+V`
use the host clipboard. A shift chord arrives as the *uppercase*
keysym, so the redo arm accepts both cases — matching only lowercase
made `Ctrl+Shift+Z` unreachable from a real keyboard.

On the rendered page, click to place the caret, drag to select,
double-click to select a word and triple-click to select a line. A
formula is one word for this purpose. Touch works the same way: drag to
select, double-tap a word and triple-tap a line. Two fingers pan a
zoomed page, taking over from the one-finger selection gesture. The
selection feeds the same `Ctrl+C` / `Ctrl+X` / `Ctrl+V` path as keyboard
selection, and a non-empty selection is also published as the host
PRIMARY selection (middle-click paste in another app); clearing the
selection clears it.

View zoom is separate from reading size: it scales the letterboxed page
without reflowing the document. `Ctrl+wheel` (or `Ctrl` plus two-finger
trackpad scroll) zooms around the pointer, `Alt+Arrows` pan, and
middle-drag or a two-finger touch drag pans a zoomed page. Zoom, pan,
reading size, the visible page and the citation style/locale are stored per
document in the session, so switching files does not carry one document's
view or place into another.
Citation popups (`Ctrl+click` a numbered reference) render through
Hayagriva; while one is open, `Ctrl+Shift+S` and `Ctrl+Shift+L` cycle
its bundled CSL style and locale and `Ctrl+Shift+R` returns to the
document's defaults. These are presentation overrides, not document
edits.

### Figures

- **Click** a figure to focus its app; **drag its edge** (within 6px) to
  resize it — the `\app` arguments are rewritten and the app is
  reconfigured. Sizes snap to an 8px grid.
- **Mirrors** are two `\app` statements sharing one binding id; the same
  app renders in both. `Ctrl+Shift+M` clones a figure's statement.
- Figures on other pages keep their apps running but idle — they stop
  receiving frame callbacks until you turn back to their page.

## Voice

A `\voice` block in the document is a **live dialogue session** with a
local LLM — the same helper the `unfer` project's `voice/voice.sh`
drives interactively, used here as a plugin (the shape `\slint` blocks
have in `mathed`): the block marker names the helper process, the
helper owns everything heavy (model round trip, microphone, TTS), and
emthin degrades to a plain document when the helper is missing.

```text
\voice typeyou: what is a Krylov basis?
```

`Ctrl+Shift+V` (or the IPC `voice` method) asks about the block's
unanswered `you:` line — or, in a `listen`/`full` block, opens the
microphone and asks what it heard. The reply is spliced back into the
transcript as a `gemma:` line on a later tick, because the helper runs
on a worker thread: a turn is seconds, and the compositor must not
stall.

The transcript lives **in the document**, which is the point: it
persists with the session, it is editable and undoable like any other
text, and each turn feeds the helper its history read back out of the
block — so editing a past turn edits the model's context too. Modes
(`type`/`speak`/`listen`/`full`) select text vs. spoken replies and
typed vs. spoken questions; only `listen`/`full` ever touch the
microphone.

The helper is resolved from `$EMTHIN_VOICE_BIN`, else `voice.sh` on
`$PATH` (see `unfer/voice/` for the harness and its one-shot
`turn`/`dictate` contract). Without it, `Ctrl+Shift+V` is a no-op and
the document is unaffected — the `\formal`/`\slint` rule.

## Control protocol

A JSON-RPC 2.0 protocol over a Unix socket lets a bar, a script, or a
future external frontend observe and drive the document. See
[`docs/ipc.md`](docs/ipc.md).

## Design

- The [native terminal figure specification](docs/HERDR_INTEGRATION_SPEC.md)
  and [implementation plan](docs/HERDR_INTEGRATION_PLAN.md) cover Android and
  desktop, both Herdr forks, paired Tailscale access, and OpenShell's shared
  Nix fleet.
- [`DEVELOPMENT_PLAN.md`](DEVELOPMENT_PLAN.md) — the current desktop/shared
  work plan; the Android plan remains under `emthin-android/`.
- `docs/REWRITE_PLAN.md` — the rewrite design and work breakdown.
- `docs/collaboration.md` — the collaboration layers and why AT Protocol is
  reserved for session publication, not live delta transport.
- `docs/AGENT_LOOP.md` — the LLM debug loop: headless geometry JSON
  (`scripts/agent/preview.sh`), live session driving and screenshots
  (`scripts/agent/live.sh`). Semantic tier first, pixels second.
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
