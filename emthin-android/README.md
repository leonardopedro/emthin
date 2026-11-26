# emthin pad

The pad displays the rendered mathed document as its editing surface. Tap text
to open its editable source below the document; press **Esc** to close it.
The caret moves over the rendered text with the arrow keys and follows the
source caret while editing. Moving beyond the source closes its columns and
continues in the rendered document. Tap an empty document to start writing.
The shortcut guide can also be tapped on a touchscreen. Typing an
unescaped `#` creates a fresh marker named using mathed's RFC 1751 word table;
type `\#` for a literal hash. IME candidates stay in the source field until
they are committed, and rendered movement, selection and deletion follow
mathed's grapheme boundaries. Pages are laid out once on a background worker for
each document/marker/fold state: page placeholders keep the full document, but
only the visible page and one neighbour keep bitmaps; distant pages are
rehydrated from the retained layout on demand. Caret and search overlays only
repaint, and PNG decoding stays off the UI thread. **A−/A+** change the reading
size (8–32 pt) and **Zoom−/Zoom+** magnify the page from 1× to 4× with
horizontal panning; both settings persist. The selection's fixed end and the
reading position are stored as Loro cursor anchors, so they follow edits and
survive relaunch. Text and Loro marks are
saved together between launches. Press **Tab** to show or hide markers in the document and source;
the columns for the text at the caret open and close with them. The **Tab Show
markers** shortcut provides the same control on touchscreens.

Press **Ctrl+F** (or foot's **Ctrl+Shift+R**) to search the document. The
scope button selects **Text** (prose and math, excluding markers and property
statements), **Source** (all source, including structure), or **Selection**
(frozen to the non-empty selection). Matches are highlighted in the rendered
page and listed as bounded 200-result pages with 80-grapheme snippets;
**Ctrl+S** and **Ctrl+R** select the next and previous match while Find is
open; the Save shortcut changes to **^S Find next (Save: close Find)** so the
context is visible. While Find is open, the main status repeats the current/total match
count. **Enter** accepts a match, and **Esc** or Cancel restores the original
selection, folds and viewport. Lowercase queries ignore case; queries containing uppercase letters
match case. **F3/Shift+F3** reuse the last accepted query.

**Ctrl+H** opens the Replace row. **Replace** applies one eligible match;
**Replace all…** first shows the replaceable/protected counts and up to 20
before/after snippets, then applies one checked plan. Marker and property
statement ranges are never replaced, plans over 10,000 edits are refused, and
one Undo reverses the whole batch. **Ctrl+Left/Right** moves by mathed's word
boundaries.
Inside `$...$`, mathed's ASCII-to-Unicode completions are shown as a hint;
typing a delimiter commits the pending completion. Ordinary prose is unchanged.
Tap a numbered citation to expand its referenced document text or its
Hayagriva-formatted citation and bibliography entry. Citation details show
the source snippet and a breadcrumb for nested document references; repeated
references stop with a source action, and nesting is limited to 16 levels.
Style and locale selectors use the shared CSL choices, apply only to detail
formatting, persist per document, and **Reset style and locale** returns to
the document/default choices. The References list shows resolved/unresolved
status and source snippets and filters by number, label, key or snippet.
**Ctrl+K** opens the **Navigate** panel on References; **Ctrl+G** opens its
Sections tab.
Definitions list their resolved use count, open their source, and expose
Go to definition/Find uses/Rename definition in the source inspector. Rename
validates one math identifier, rejects duplicate or unresolved names outside
its plan, and applies the definition plus resolved uses as one grouped Undo
step. **Ctrl+1** through **Ctrl+9** open numbered entries.
Accepted search, section/definition/use and reference jumps enter a bounded
30-place return history; use **Alt+Left/Right** or the toolbar Back/Forward
actions. New/Open clear that history. Document references may
contain further expandable citations. Tap a collapsed translator or template
title to expand its code; **Ctrl+L** folds or unfolds the block at the caret.
On the rendered page, **Shift+arrows** selects text and **Ctrl+A/C/X/V**
selects all, copies, cuts and pastes. Long-press selects a word through
mathed's word boundaries and dragging extends it; start/end handles remain
visible and can be dragged independently. A contextual
Copy/Cut/Paste/Select-all bar provides the same touch-only path. Hidden marker
names are omitted from a copy and retained when visible text is cut. Loaded
pages expose their text as virtual accessibility nodes in reading order, with
source-backed bounds and actions for citations, folds and source editing;
unloaded pages request only their own page when accessibility focus reaches
them.
**Ctrl+O** opens a Loro `.mathed` snapshot or UTF-8 Typst source. Outside
Find, **Ctrl+S** saves a Loro snapshot, including modifier marks;
**Ctrl+Shift+S** saves a copy
without changing the active file. An opened plain text file is saved as a new
`.mathed` file so its marks are preserved. Saving over a file that changed
externally offers a copy, an intentional replacement, or cancel.

To use marked text as a modifier, tap the corresponding rendered text. The
panel below the document shows the editable source text, modifiers affecting
it, and text it modifies. Each link row names both marker pairs, shows a
snippet and resolved state, and can jump to either endpoint or remove the
link in either direction. Only one comparison row expands at a time; its
right side renders the selected modifier removed. Six row results are cached
without touching the document.

In the right-hand field, enter pairs of marker names separated by commas,
such as `a at, be e`. `#` prefixes are optional, and the field suggests the
markers that can complete the pair being typed. A complete pair is applied
when the caret leaves it, when the field loses focus, or when the IME's **Done** action
is used; an active partial pair is preserved. Missing or reversed endpoints
report an error and leave existing links unchanged, while duplicate pairs
collapse. The links are Loro marks and follow the named markers as the text
changes; they do not insert property statements into the document. Clear the
list to remove these links. Existing selection-based links without matching
marker pairs remain listed separately and can be removed there. Applications
participate in Undo and Redo, can overlap, and may cover several ranges. At
600 dp and wider, source, relationships and comparisons use aligned columns;
below that they stack, and **Show document** keeps the editor reachable.

Modifier text follows Typst's content-function convention. `bold` is an alias
for `strong`, and `italic` is an alias for `emph`; Typst function expressions
such as `text.with(fill: red)` are also accepted. The expression is applied to
the selected content. A malformed expression produces a rendering diagnostic
while the source remains editable.

The status line names the document (or **Untitled**), shows whether it is saved,
unsaved or currently being written, and reports the latest operation or
diagnostic summary. Problems found during layout are counted there. Tap the
count to open the diagnostics list; each entry identifies whether it points at exact
copied source, at the modifier or statement that produced the text, or at the
document as a whole. Selecting an exact or owner entry opens that source and
**Back** returns to the document. Unresolved semantic names, incomplete marker
pairs and kernel/execution statements are reported here too; execution
statements say explicitly that execution is unavailable on this Android
frontend, and their source remains editable.

The Android pad does not yet expose every mathed feature; in particular, this
interface does not implement its kernel execution and notebook workflows.

## App-wide GPU runtime and embedded Wayland compositor

`emthin-android` provides a process-wide GPU runtime (`src/gpu.rs`,
`GpuContract.java`, `GpuRuntime.get()`, and `DocBridge.gpu*`) shared by both the
embedded Wayland compositor (`src/wayland.rs`, `WaylandSurfaceView.java`,
`DocBridge.wayland*`) and any other Android `Surface` or background compute job:

- **Graphics / surface compositing (`EGL 1.4` / `OpenGL ES 3.x`, with CPU fallback):**
  `GpuRuntime.renderRgba`, `renderPlaceholder`, `releaseSurface`, and
  `renderWaylandFigure` share one thread-safe EGL/GLES3 context over
  `ANativeWindow` surfaces (releasing `eglMakeCurrent` after every draw so any
  worker thread can drive GPU rendering safely). Zero-copy `zwp_linux_dmabuf_v1`
  (`EGL_LINUX_DMA_BUF_EXT` when advertised) and `AHardwareBuffer`
  (`EGL_ANDROID_image_native_buffer`) sit alongside `wl_shm` GPU texture
  compositing with UV viewport cropping. If EGL init, `eglCreateWindowSurface`,
  `eglMakeCurrent`, `eglSwapBuffers`, or `EGLImage` fails—or `gpuSetForceCpu(true)`
  is set—rendering falls back automatically via `ANativeWindow_lock` ->
  `cpu_blit_rgba` / `cpu_fill_placeholder` -> `ANativeWindow_unlockAndPost`
  (`"cpu-software"`), and `TerminalSurface` falls back to Canvas CPU painting.
- **Vulkan 1.1 compute & AHardwareBuffer interop:**
  `GpuRuntime.computeF32` exposes built-in SPIR-V 1.0 compute kernels
  (`square_plus_one`, `relu`) with deterministic CPU fallback when Vulkan is
  absent, while `GpuRuntime.dispatchSpirv` executes arbitrary caller-supplied
  SPIR-V binaries (`GpuContract.buildDoubleSpirvBytes()` demonstrates end-to-end
  custom SPIR-V dispatch from Java). Both manifests declare optional
  `glEsVersion="0x00030000"` and `vulkan.version="0x401000"` features plus
  `hardwareAccelerated="true"`.

## Testing locally

Run all commands below from `emthin-android/` (except where noted).

### 1. Host tests (no device required)

`ghostty-vt`'s build script needs `zig` 0.16 and a host C compiler (`cc`) on
`PATH`, and `test-java.sh` uses a JDK (`javac`, `java`, `keytool`):

```sh
cargo test --locked --offline
cargo fmt --check
bash test-java.sh
```

### 2. Building the APKs

On an `aarch64` Linux host, run `./setup-box64-toolchain.sh` once to provision
`.toolchain/`. Ensure the host `cc`, `zig` 0.16, and a Rust toolchain with the
`aarch64-linux-android` target (`rustup target add aarch64-linux-android`) are
on `PATH`. Do not invoke `./build-apk.sh` inside `nix develop -c`.

```sh
# Shipping APK (build/emthin-pad.apk) — runs the shipping manifest hygiene gate
ANDROID_ABI=arm64-v8a ./build-apk.sh

# Disposable test APK (build/test/emthin-pad-test.apk, id: com.emthin.pad.test)
EMTHIN_TEST_PACKAGE=com.emthin.pad.test ANDROID_ABI=arm64-v8a ./build-apk.sh

# Release-signed shipping APK (with optional version overrides)
EMTHIN_RELEASE=1 \
EMTHIN_RELEASE_KEYSTORE=/path/to/release.jks \
EMTHIN_RELEASE_KEY_ALIAS=emthin \
EMTHIN_RELEASE_KS_PASS='...' \
EMTHIN_VERSION_CODE=2 EMTHIN_VERSION_NAME=0.1.1 \
ANDROID_ABI=arm64-v8a ./build-apk.sh
```

### 3. Device lane (Waydroid or physical Android device)

`device-lane.sh` defaults to Waydroid (`waydroid session start`) and targets
the disposable `com.emthin.pad.test` package so app data clears never touch a
personal `com.emthin.pad` install. For a physical device or custom `adb` binary,
set `DEVICE=<serial>` and/or `ADB=/path/to/adb`.

```sh
# Focused checks
./device-lane.sh math-preview   # math prose/equation ink + tap-to-edit + Undo (needs Python Pillow)
./device-lane.sh phone-layout   # 360 dp portrait & landscape phone math + toolbar bounds
./device-lane.sh wayland-gpu    # app-wide GLES3/Vulkan compute, custom SPIR-V, AHB interop & Wayland compositor
./device-lane.sh proof-zenodo   # real mm0web + mock_zenodo.py over adb reverse

# Full end-to-end device suite (set SKIP_BUILD=1 if the test APK is already built)
LANE_WSS=1 ./device-lane.sh e2e
# Optional TalkBack automated utterance check:
# TALKBACK_APK=/path/to/talkback.apk LANE_WSS=1 ./device-lane.sh e2e
```

### 4. Manual testing with `mm0web` and `sandbox.zenodo.org`

The shipping APK (`com.emthin.pad`) enforces `https://` for the proof backend
and sets `ProofClientActivity` non-exported. When testing locally over loopback
`adb reverse` with `http://127.0.0.1:18919`, use the disposable test APK
(`com.emthin.pad.test`) or place an HTTPS reverse proxy in front of `mm0web`.

1. Start `mm0web` against `https://sandbox.zenodo.org/api` with your sandbox token
   in `ZENODO_API_KEY` (never commit or log the token) and `mm0-rs`/`mm0-c` on
   `PATH`:

   ```sh
   MM0WEB_ZENODO_BASE=https://sandbox.zenodo.org/api \
   ZENODO_API_KEY="$(cat ~/.config/mm0web/sandbox-token)" \
   ../unfer/target/debug/mm0web --bind 127.0.0.1:18919 --state /tmp/mm0web-state
   ```

2. Forward the port to the device and launch `com.emthin.pad.test`:

   ```sh
   adb reverse tcp:18919 tcp:18919
   adb shell am start -W -n com.emthin.pad.test/com.emthin.pad.MainActivity
   ```

3. In the pad, open **More → Sessions and sharing → MM0 proof**, pick a
   `.mm0`/`.mm1` pair (or set the server URL to `http://127.0.0.1:18919`), tap
   **Verify**, then **Publish to Zenodo** and confirm. Once the poll settles to a
   `10.5072/zenodo.*` DOI, tap **Insert into note** to insert the proof ID and
   DOI link at the caret.

The [Android development plan](DEVELOPMENT_PLAN.md) records the current review,
priorities, and scoped implementation tasks. The [feature specification](FEATURE_SPEC.md)
defines the selected future behavior; the [sibling study](SIBLING_STUDY.md)
records its source evidence and adaptation decisions.

The [native terminal figure specification](../docs/HERDR_INTEGRATION_SPEC.md)
and [implementation plan](../docs/HERDR_INTEGRATION_PLAN.md) define the next
Android integration with remote Herdr Core sessions and the OpenShell fleet.
