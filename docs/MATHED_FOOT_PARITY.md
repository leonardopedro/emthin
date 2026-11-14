# Mathed and foot feature inventory

This inventory compares the local `../velysterm` mathed sources and plans with
`../foot/foot.ini`, `../foot/doc/foot.ini.5.scd`, and the current emthin
frontends. A feature being available in `mathed_core` does not mean an emthin
frontend exposes it. The Android pad and desktop compositor are separate
frontends and need separate verification.

| Capability | Android pad | Desktop document UI | Remaining work |
|---|---|---|---|
| Loro text, undo/redo, persistent state | Text and marks, caret and scroll saved; `.mathed` open/save and UTF-8 source import | Loro text model and editing | Broader cross-frontend file interchange and export |
| Typst rendering and pagination | Rendered page editor, markers, citations, figures as document content | Paged document and live `\app` figures | Broader rendered-object editing and navigation |
| Marker creation and visibility | RFC word names; Tab visibility | Marker scan and document figure editing | Bring user-facing marker and relationship editing to desktop |
| Loro modifier relationships | Marker-pair input, comparison columns, undo | Not exposed as the pad exposes it | Shared relationship UI and rendering on desktop; more modifier forms |
| Foot/mathed incremental search | Shared mathed search, match overlays, next/previous | Shared mathed search wired to the document: `Ctrl+F` opens a query bar, `F3`/`Shift+F3` step and select matches, `Escape` closes the bar (F3 keeps the last query); status as `n/m` or `no match`, with match highlights and the query/status panel rendered on the page | Larger-document performance and search in generated content |
| Foot/mathed word navigation | Ctrl+arrows through mathed word boundaries | Ctrl+arrows and selection through mathed word boundaries | Full selection semantics on the rendered pad |
| Mathed references and citation popups | Numbered labels, nested expandable document references, Hayagriva citations and full entries, author/year suggestions | Numbered labels, clickable reference details, `Ctrl+click` go-to-definition for document references, nested keyboard navigation and Hayagriva entries; `Ctrl+Shift+S` / `Ctrl+Shift+L` cycle the popup's bundled CSL style and locale and `Ctrl+Shift+R` resets them, as presentation-only overrides saved per document | Non-bundled CSL styles, locale-aware document defaults and citation editing |
| Math completion and ASCII interchange | Mathed completion hints and delimiter commit in math | ASCII→Unicode completion in math: caret glyph preview, delimiter commit, run-extension, and `Escape` cancel; ASCII-only Typst projection exported and imported over IPC (`export_ascii`/`import_ascii`, with non-ASCII imports rejected) | Cross-frontend file interchange and preservation of hidden structure across the projection |
| Template composition | Translator/template/base bodies fold and expand | Mathed's caret-driven code expansion | Mathed context, template/base execution and output/export pipeline |
| Notebook and computation | No notebook/kernel frontend | No general mathed notebook frontend | Kernel worker integration, per-block results, staleness, run log, run-all and notebook export |
| IME and accessibility | Android text fields and editing | Wayland text-input / fcitx bridge | Accessibility for rendered glyphs and equivalent editing paths |
| Foot selection and clipboard | Keyboard selection and clipboard on rendered text; native selection in source fields | Document selection and host clipboard; click-drag selects rendered text, double-click selects a word, triple-click selects a line, and touch drag with double/triple tap groups the same way; non-empty document selections are published to the host PRIMARY selection | Rendered-glyph accessibility and touch gestures beyond selection |
| Foot font/zoom and search modes | Native-resolution raster and search | Document letterboxing; `Ctrl++` / `Ctrl+-` / `Ctrl+0` change the 10–32pt reading size and reflow the document; `Ctrl+Shift++` / `Ctrl+Shift+-` / `Ctrl+Shift+0` zoom the presentation view without reflow; `Ctrl+wheel`/`Ctrl+two-finger scroll` zoom around the pointer, and middle-drag, `Alt+Arrows` or a two-finger touch drag pans the zoomed page; view, reading size, page and citation preferences persist per document | Smooth zoom animation and named per-document view presets |

Mathed's kernel-enabled frontend depends on the separate `unfer` worker and
its execution/grant protocol. The pad deliberately builds `mathed_mini`
without its `kernel` feature because that dependency tree does not currently
build for Android. Notebook parity therefore requires an Android-compatible
worker and output transport, not just a shortcut or a UI control.

Foot terminal emulation, escape-sequence handling, scrollback, PTY process
management and terminal-specific protocols belong to a terminal client placed
in an `\app` figure; they are not document layout policy. The transferable
behaviors are navigation, selection, search, clipboard, cursor presentation,
scaling and discoverable keyboard interaction.
