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
| Foot/mathed incremental search | Shared mathed search, match overlays, next/previous | Not exposed | Desktop search UI; larger-document performance and search in generated content |
| Foot/mathed word navigation | Ctrl+arrows through mathed word boundaries | Ctrl+arrows and selection through mathed word boundaries | Full selection semantics on the rendered pad |
| Mathed references and citation popups | Numbered labels, nested expandable document references, Hayagriva citations and full entries, author/year suggestions | Numbered labels, clickable reference details, nested keyboard navigation and Hayagriva entries | Desktop go-to-definition; locale/style selection |
| Math completion and ASCII interchange | Mathed completion hints and delimiter commit in math | Not exposed | Desktop completion and ASCII import/export |
| Template composition | Translator/template/base bodies fold and expand | Mathed's caret-driven code expansion | Mathed context, template/base execution and output/export pipeline |
| Notebook and computation | No notebook/kernel frontend | No general mathed notebook frontend | Kernel worker integration, per-block results, staleness, run log, run-all and notebook export |
| IME and accessibility | Android text fields and editing | Wayland text-input / fcitx bridge | Accessibility for rendered glyphs and equivalent editing paths |
| Foot selection and clipboard | Keyboard selection and clipboard on rendered text; native selection in source fields | Document selection and host clipboard | Rendered mouse/touch selection, word/line selection and primary selection behavior |
| Foot font/zoom and search modes | Native-resolution raster and search | Document letterboxing | User-controlled text scaling and selection/search feedback |

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
