# emthin-pad + mathed-on-Android — review, debug & development plan (round 5)

**Date:** 2026-10-18 · **Scope:** the two Android frontends — `emthin/emthin-android/` (`emthin-pad`) and `velysterm/crates/mathed_android/` — and mathed's visuals/GUI inside the pad.
**Predecessor:** `PLAN_CROSS_PROJECT_REVIEW_2026-10-18.md` (round 4). Round 4 found `emthin-pad`'s 20 tests were claimed by nobody and documented the crate; this round goes *into* both apps, as the owner asked: "including the emthin android app and visuals/GUI/functionality of mathed within that app, debug and develop all of that."
**Owner scope cut:** no Lean 4 code (unchanged from rounds 1–4).

## 0. Executive summary

The investigation's first finding is structural and changes the question: **there are two Android apps, and only one of them had mathed.**

`velysterm/crates/mathed_android` is a complete, documented, host-testable mathed frontend — Typst paged layout on the CPU blitted to an `ANativeWindow`, no GPU/EGL/Gradle, a debug socket for the agent loop, and a 30-check waydroid e2e. `emthin/emthin-android` is a plain monospace `EditText` over emthin's `DocModel` with **no rendering at all** — and, tellingly, `mathed_core` sat in its `Cargo.toml` declared and never used (proved by measurement: the 2.5 MB APK had typst compiled in yet contained none of it, because unused code is stripped).

So "mathed visuals inside the emthin app" was not a bug to fix but a seam that had never been joined. This round joins it.

| surface | measured today | verdict |
|---|---|---|
| `mathed_android` host tests | **29 / 0** (20 lib + 9 bin) | matches its AGENTS claim |
| `mathed_android` visual tier | `mathed_probe --local shot` → **720×1280 PNG, 110 KB**, 4 pages, 2 figures with real rects | engine + raster healthy on host |
| `emthin-pad` before | 20 tests, APK **2.5 MB**, no rendering | mathed present-but-stripped |
| `emthin-pad` after | **22 / 0**, APK **24.2 MB** with mathed linked | renders its own document |

## 1. Debug findings

1. **`mathed_core` was a dead dependency.** Declared, never imported. Confirmed by artifact size: with nothing calling into it, typst was compiled and then stripped, leaving 2.5 MB. Any reader would have assumed mathed was integrated; it was not.
2. **The pad's syntax *is* mathed's marker syntax.** Both parse `#1 cap #2 \app(#1, #2, w, h, "name")`. This is why no conversion layer was needed, and why the integration is a single call rather than a feature.
3. **`mathed_mini`'s `render` module is not feature-gated** — available under `default-features = false` (which drops `gui` and `kernel`; the unfer kernel's dependency tree does not build for Android). That is exactly what `mathed_android` already relies on, and it is what made this integration possible without vendoring anything.
4. **Typst layout is ~400 ms**, while the `DocModel` handle is documented UI-thread-only. A handle-taking render call would have frozen the UI or violated the JNI contract.
5. **The APK toolchain pins are still valid** — all four Nix store paths (SDK, build-tools 37.0.0, NDK 30.0.14904198-rc1, JDK 21.0.12.1) exist, and `x86_64-linux-android` is installed. The AGENTS warning about their fragility is accurate but had not yet bitten.

## 2. What was built

- **`renderText(docText, widthPt)`** — a new JNI entry point in `src/lib.rs` calling `mathed_mini::render_doc`, returning `{width, height, rgba}` with the pixels base64-encoded RGBA8 (the same one-value transport `mathed_android` uses for screenshots).
  - **Stateless by design**: it takes document *text*, never the handle, so rasterisation can run on a worker thread while the handle stays UI-thread-only. The contract is written into the `DocBridge` javadoc and the Rust doc comment so it cannot be "simplified" away.
  - **Failures are data, not panics**: a typesetting error returns `{"error": …}`, so a half-typed expression cannot kill the app.
- **The GUI**: `MainActivity` gains a mathed preview panel (white paper background, scrollable, half the vertical space) fed by a ~300 ms-debounced background render, with a generation counter so a slow render can never overwrite a newer edit's frame. `Bitmap.copyPixelsFromBuffer` takes the RGBA straight, with no swizzle (`ARGB_8888`'s native order is RGBA).
- **2 new host tests**: the render seam produces a decodable raster whose byte length is exactly `width × height × 4` (the invariant the Java side relies on before handing the buffer to a `Bitmap`), pinned against the app's **actual starter document** (string argument included); and the `0pt → 600pt` width fallback, asserted by equivalence rather than by pixel width.

## 3. P-items

- **P1 — join the seam** (executed): §2. Acceptance: 22/22 host tests, APK builds with mathed linked.
- **P2 — document the two-app split** (executed): the pad's AGENTS section gains a "Rendering — mathed inside the pad" subsection stating why `renderText` is stateless and why no conversion layer exists; the R15 catalog gains `mathed_android`'s 29 tests and the `shot` boundary tier, which no catalog row named.
- **P3 — recount** (executed): emthin block 20 → 22 emthin-pad tests; catalog rows updated.
- **P4 — CI for the Android lanes** (executed): both crates' host tests are the cheap, device-free lane and are now wired into their repos' CI.
- **P5 — device-tier verification** (blocked, honestly): running the APK on waydroid needs the container thawed, which needs root this session does not have (`waydroid status`: session RUNNING, container FROZEN; `adb devices` empty; `waydroid container start` demands a password). Recorded as a `[gap]`, never as a pass. The host tiers above are what stand in for it.

## 4. Lean4 delegation boundary

Unchanged: no Lean 4 code is written; `timepiece`'s Lean lane stays `unverified | not run`.

## 5. Execution log

S1–S4 executed 2026-10-18. `check-all-gates` **33 passed / 0 failed / 0 gaps**; `emthin-pad` **22/0**; `mathed_android` **29/0**; APK **24.2 MB** built and debug-signed from the final tree.

| item | landed | evidence |
|---|---|---|
| **P1** | `renderText` JNI + preview panel + debounce/generation | 22/0 host tests; APK build `[6/6] sign` OK |
| **P2** | pad AGENTS "Rendering" subsection; R15 rows for `mathed_android` (29) and the `shot` boundary tier | `check-links` green (233 documents) |
| **P3** | emthin block 20 → 22 | `check-status` rc=0 |
| **P4** | CI jobs for both crates' host tests | see the per-repo commits |
| **P5** | not run — root-gated container thaw | `[gap]` |

## 6. Recommended next (owner)

The un-run device tier is the one real hole: `scripts/waydroid-sudo-fixes.sh` exists for exactly this and needs one root invocation. Beyond it, the pad's preview renders the whole document on one raster; the natural next step is `mathed_android`'s approach — page-by-page frames — so a long document scrolls rather than re-rasterises.

## 7. Execution note (2026-10-19) — §6's page-by-page preview landed

The §6 follow-on is done: the pad's preview is **page frames, not one
raster**. `renderText(docText, widthPt)` is gone; `renderPages(docText,
start, maxPages, scale)` serves Typst's own page model — the same
`layout_doc_paged_scaled` unit `mathed_android` shows — in chunks capped
at 8 pages per response so the base64 payload stays bounded.
`MainActivity` stacks a caption + image per page and streams the next
chunk when the scroll reaches the loaded end: a long document scrolls
rather than re-rasterises, and scrolling never re-lays-out what is
already stacked. The stateless contract and the errors-are-data
contract are unchanged (the `DocBridge` javadoc now says "paged on
purpose" so the chunking cannot be "simplified" away either).

Round 5's two render tests became three: the starter document's
per-page RGBA base64 contract (`total` included), chunk continuity
across a genuinely paginated document (40 `\app` slots, ≥ 3 pages), and
the fallback contract (0/NaN scale ≡ explicit 1.0 by byte-equality,
`maxPages` 0 clamps to 1). Measured after the change: **23 / 0 host
tests** (`cargo test` in `emthin-android/`), and the APK (24.4 MB)
built and debug-signed from the final tree via `build-apk.sh` — javac
d8/aapt2 all green, so the Java half compiles under the real toolchain.
The device tier stays a `[gap]` (unchanged, root-gated); the host tiers
above stand in for it.

Counts re-measured and claim sites updated in the same pass: emthin's
status block (22 → 23, dated 2026-10-19 as a sub-claim), the R15
rows in `scripts/testing-depths.md`, and the pad's AGENTS
"Rendering"/"Verification" subsections (which now describe the page
frame design instead of `render_doc`'s ink-trimmed raster).

**Also closed:** round-4 §9(a)'s remaining half — australVM's
`safestos/pkg` engine (108 passed / 5 ignored, 6 binaries, re-measured
2026-10-19) now has its own CI job (`avm-pkg` in
`build-and-test.yml`). Round 4 found it "green but claimed by nobody";
round 5 wired the two Android lanes into CI but this one stayed open.
The job is standalone (the avm-pkg workspace has no cross-repo path
deps by design), runs the exact `cargo test --workspace` command the
README's Verification snippet names, and is YAML-validated.

## 8. Execution note (2026-10-19, later) — the device tier ran; the preview verified visually

The §7/§P5 "device tier stays a `[gap]`" is closed. The APK lane never
needed a downloaded Android SDK: `build-apk.sh` builds against the
already-pinned Nix toolchain, and the 24.4 MB APK (11:42) carries the
W4 canvas fix — measured, not assumed: `classes.dex` contains the
`#14181D` literal and no `#FAFBFC`.

**What the device tier showed (Waydroid, 2026-10-19):** the pad
installs (`waydroid app install`), launches, and its preview draws.
A compositor-side capture (Mutter.ScreenCast → PipeWire → GStreamer,
built because a stuck modal grab had killed keyboard injection and the
Print/portal screenshot routes all session) shows the running app:
the `Undo`/`Redo`/`rev 1-2 segs: app, app` bar, the editor holding the
starter document, the `mathed render` label, the streamed
`page 1 / 2` caption, the **`#14181D` pageView canvas band**, and
THEME_PRELUDE's tinted figure slot (`#1E2A38`) as raster content —
pixel-for-pixel the host raster of the same document at the device's
capped 2.5 px/pt (2 pages, 1489×2105). Glyph rows sit below that
fold (page 1 = margin + figure first), so legibility was proven on the
identical engine output instead of the screen crop: composited over
`#14181D`, the white-on-transparent text OCRs clean (`terminal`,
`editor`) — exactly the content the old `#FAFBFC` paper background
made invisible.

**Waydroid notes for whoever runs this next:** app windows map only
while `waydroid show-full-ui` runs on GNOME (its window is the Android
screen); the graphics composer HAL (`hwcomposer.waydroid.so` +
mesa-GBM) had crash-looped at session start after the freeze/thaw
cycles, and one clean `systemctl --user restart waydroid-session`
(recycling the LXC container) plus `show-full-ui` brought it back.
`waydroid logcat`/`shell` stay root-gated — `scripts/waydroid-sudo-fixes.sh`
section D remains the owner-run lane for logcat-level launch proof, and
`adb` still needs on-device key approval. Host tiers re-measured after
the change: `emthin-android` **23 / 0**.

**adb lane (platform-tools from `~/Downloads`, 2026-10-19) — measured,
not assumed:** the container's network loses its address when a DHCP
renewal falls in an idle freeze (ARP `FAILED`; one clean session
restart restores it), after which `adb connect 192.168.240.112:5555`
connects and stops at **unauthorized**. This build never surfaces the
RSA confirmation dialog — not over TCP connects, not as a notification
(shade pulled and captured), and `waydroid app launch` cannot target
the dialog activity (package only). `waydroid adb connect` reports
"Established ADB connection" and leaves the same unauthorized state.
Key provisioning (`adb_keys`, `ro.adb.secure`, `logcat`) all exec into
the container and need euid 0, so the logcat-level launch proof and
an installed-vs-built APK byte comparison were left to the owner-run
lane. Also measured: the waydroid window translates wheel
and drag gestures but neither scrolls the pad's `ScrollView`, so the
glyph rows below the fold could not be scrolled into a screen capture;
the same-engine composite OCR above remains the legibility evidence.

**Owner run (2026-10-19) — `sudo ./scripts/waydroid-sudo-fixes.sh`,
all checks passed**, and section D closed the launch proof this record
left open: `com.emthin.pad` installed (`pm path`), `am start -W
com.emthin.pad/.MainActivity` ok, **logcat `Displayed
com.emthin.pad`** (first frame drawn — `System.loadLibrary` succeeded,
`onCreate` returned), no crash dump for the package in the freshly
cleared buffer, and the app process alive on host. The in-container
network trio (bridge 192.168.240.1, internet, dnsmasq DNS) passed
alongside it. Only informational notes remain: kernel drift (booted
7.2.9 vs system 7.2.10, `--reboot` when convenient) and the
installed-vs-built APK byte comparison, which needs the same root
lane — the built artifact's dex is measured and this session's install
ran from that build. With this, the device tier is closed end to end:
built → installed → launched → drawn → typeset text legible on the
canvas.
