---
name: emthin-document-editing
description: Edit and visually refine emthin documents — text with hidden markers and \app figures that embed live Wayland app windows — using the headless preview loop before any screenshot.
---

# emthin document editing

An emthin document is plain UTF-8 text. **The document is the layout
authority**: figure geometry lives in `\app` statements, never in
compositor state, so editing text is the only edit surface you need.
Deeper background: [AGENTS.md](../../../AGENTS.md) (document model,
rendering) and [docs/AGENT_LOOP.md](../../../docs/AGENT_LOOP.md)
(the observation loop this skill drives).

## Document cheat-sheet

```
#1 Terminal #2 \app(#1, #2, 400, 300, "foot", launch: "foot")
Prose flows between figures...
```

- `#1` … `#2` — hidden marker pair wrapping a figure's **caption**,
  ordinary prose you can rewrite freely.
- `\app(#s, #f, W, H[, "id-glob"][, launch: "cmd"])` — reserves a
  W×H slot (logical px == pt at zoom 1) in the page flow. The `"id"`
  glob binds a Wayland/X app by app_id (`"kgx*"` claims `kgx` and
  anything starting with `kgx`); `launch:` runs when the slot is
  dormant.
- Figure rect = what Typst actually gives it after pagination — never
  assume W/H equals the landed rect; read it from preview JSON.
- A runnable sample lives at `scripts/agent/demo.typ`.

## The loop — cheapest observation first

1. **Edit** the `.typ` text.
2. **Preview** (geometry tier, one JSON line on stdout):

   ```sh
   ./scripts/agent/preview.sh scripts/agent/demo.typ
   ```

   Fields: `key`/`line`/`caption`/`w`/`h`/`id`/`launch` per figure,
   plus `page`, `rect{x,y,w,h}` where it landed, `page_count`, and
   `diagnostics[]` (dangling markers, unparseable sizes, page
   overflow). Empty diagnostics = structurally sound.
3. **Ask what it means** with `--semantic` when the question is
   definitions, uses or dangling identifiers — or when the document
   does not compile at all (tier 1b runs no Typst eval, so it still
   reports the `$foo$` that broke the render):

   ```sh
   ./scripts/agent/preview.sh scripts/agent/demo.typ --semantic
   ```

   `defs{name,line,uses}`, `unresolved`, `models`, `statements`,
   `translators`, `templates` — all keys present even when empty.
4. **Fix** any diagnostics; re-preview. A compile failure exits
   non-zero with the Typst error on stderr — never an empty report.
5. **Read the delta, not two dumps**: after an edit,
   `./scripts/agent/preview.sh --diff before.typ after.typ` reports
   exactly what moved (figures added/removed/rekeyed/changed,
   pages, diagnostics, semantics) in one ~500 B line. A broken
   edit is an error naming the side — never a silent removal.
6. **Judge geometry** against the aesthetics rules below. If the
   numbers are right and the question is *taste*, render once:
   `--png /tmp/out.png` (~60 KB — spend eyes last).
7. **Regression-proof it**: encode the expectations as a scenario
   (`doc` / `apply` / `expect`, closed vocabulary) and run
   `./scripts/agent/replay.sh <scenario>` — 13 asserts in ~0.8 s,
   sandboxed, exit 1 on any miss. Scenario example:
   `scripts/agent/scenarios/figure-resize.txt`.
8. **Live session** (`./scripts/agent/live.sh`) only when real apps
   matter: binding, focus, live pixels. `state` ≈ 200 tokens;
   `shot` is a screenshot — last resort.

## Making it look nice (from tier-1 numbers alone)

| rule | target |
|---|---|
| page | A4 = 595×842 pt; text starts at x=71 → usable width ≈ **453 pt**. `W > 453` overflows (tier-1 flags it). |
| width rhythm | give sibling figures the **same width** — a column of equal widths reads as a grid; mixed widths read as chaos. |
| aspect ratio | keep neighbouring figures on the same ratio (4:3 → 400×300, 440×330). |
| page balance | `page_count` jumping after an edit means spillover — trim a figure or move prose rather than leaving a lonely figure on a near-empty page. |
| captions | one short line; a caption longer than its figure crowds the layout. |
| app-realistic sizes | terminals ≈400×300, editors ≈410×310, chat ≈350×250 — apps reflow content, but tiny slots clip toolbars. |

After every size change: re-preview and read `rect` — spacing,
alignment, and spillover are all visible in geometry without pixels.

## Failure modes

- **`exit 3 … binary emthin session snapshot`** — you passed a loro
  CRDT session file (`.e2e/run/*.doc`). Export text first:
  `live.sh save out.typ` (IPC `save_doc` writes source text).
- **slow first run** — the cached `mathed_mini` is built on demand
  inside velysterm's devshell (minutes cold); every later run uses
  the direct fast path (~0.1 s). Rust edits under
  `../velysterm/crates/{mathed_core,mathed_mini}` re-arm the rebuild
  automatically.
- **`FAIL sibling checkout ../velysterm not found`** — preview needs
  the sibling velysterm checkout (same path dependency emthin builds
  against).
- stdout must stay **pure JSON**: devshell banners go to stderr
  (velysterm `flake.nix` shellHook). If you add shell wrappers, keep
  that contract.

## Measured costs (2026-10-13, warm cache)

| step | wall | output |
|---|---|---|
| `preview.sh` fast path | **0.14 s** | 562 B JSON (~150 tokens) |
| `preview.sh` after Rust edit (rebuild) | ~5.6 s | same |
| first-ever build (cold target) | ~6.8 min | — |
| `preview.sh --png` | ~0.3 s | ~61 KB PNG |
| `live.sh state` | ~1 s | ~200 tokens |
| `live.sh shot` | ~1 s | 5–50 KB PNG |
| `preview.sh --semantic` (2026-10-15) | **0.02 s** | 88–142 B JSON |
| `preview.sh --diff` (2026-10-15) | **0.03 s** | ~526 B JSON |
| `replay.sh` 13-assert scenario (2026-10-15) | **0.8 s** | 2.4 KB transcript |

Eyes and compositor are the expensive parts; geometry answers most
questions for pennies.
