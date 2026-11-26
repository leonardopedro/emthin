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

<!-- status: verified | tests: 734 cargo passed, 2 ignored (workspace, nix develop, one build job, measured 2026-11-22) + 28/28 wtype (headless sway) and 36/36 xdotool (sway on Xvfb) run.sh E2E, including multi-page navigation and embedded-client fcitx5 composition + 136 emthin-pad Rust host tests and 17 Java host suites (583 counted checks) + 302 Waydroid device-lane E2E checks, 360 dp portrait/landscape phone-layout, proof-zenodo (mock + live sandbox.zenodo.org), and release v1->v2 upgrade/security check (measured 2026-11-25) | last_verified: 2026-11-25 -->

## Verification

```sh
nix develop -c bash -c 'cargo fmt --all --check && \
  cargo clippy --workspace --all-targets -- -D warnings && \
  cargo test --workspace'          # 734 passed, 2 ignored (2026-11-22)

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

### Testing the Android app locally

The Android pad (`emthin-android/`) has its own host tests, APK builder, and
ADB/Waydroid device lane:

```sh
# 1. Host Rust & Java tests (needs zig 0.16 + host cc + JDK on PATH)
(cd emthin-android && cargo test --locked --offline && bash test-java.sh)

# 2. Build APKs (shipping APK enforces manifest hygiene automatically)
(cd emthin-android && ./build-apk.sh)                                       # debug-signed shipping APK
(cd emthin-android && EMTHIN_TEST_PACKAGE=com.emthin.pad.test ./build-apk.sh) # disposable test APK

# Release-signed shipping build
(cd emthin-android && EMTHIN_RELEASE=1 \
  EMTHIN_RELEASE_KEYSTORE=/path/to/release.jks \
  EMTHIN_RELEASE_KEY_ALIAS=emthin \
  EMTHIN_RELEASE_KS_PASS='...' \
  ./build-apk.sh)

# 3. Device checks on Waydroid or a connected Android device (com.emthin.pad.test)
(cd emthin-android && ./device-lane.sh math-preview)  # equation ink + tap-to-edit + Undo (needs Pillow)
(cd emthin-android && ./device-lane.sh phone-layout)  # 360 dp portrait & landscape phone checks
(cd emthin-android && ./device-lane.sh proof-zenodo)  # MM0 verify/reject + mock Zenodo DOI + Undo
(cd emthin-android && LANE_WSS=1 ./device-lane.sh e2e) # full device suite
```

For manual testing against `mm0web` + `https://sandbox.zenodo.org/api` via
`adb reverse` (and details on `DEVICE` / `ADB` / `TALKBACK_APK` overrides), see
[emthin-android/README.md](emthin-android/README.md#testing-locally).

## Installing on Android for a teenager (F-Droid + parental lockdown)

This step-by-step guide walks a parent and teenager through setting up the app
from a standard Android phone/tablet (assuming the app is published on
F-Droid), turning on automatic updates, password-protecting both app stores,
and blocking sideloading of any extra `.apk` files outside Google Play Store and
F-Droid.

> [!NOTE]
> The parent should set the PINs/passwords below and keep them private.

### Before you start

- Start with a standard Android phone or tablet signed in to the teenager's
  account and Google Play Store.
- If a test/debug build of the app is already installed, uninstall it first (an
  F-Droid build uses F-Droid's signing key, so Android will reject updating a
  differently signed APK over it).

### Step 1 — Switch to Developer mode (Developer options)

1. Open **Settings → About phone** (on Samsung: **Settings → About phone → Software information**).
2. Find **Build number** and tap it **7 times** in a row until Android prompts for the device lock screen PIN, pattern, or password.
3. Enter the screen lock PIN to confirm. You will see **"You are now a developer!"**.
4. Open **Settings → System → Developer options** (on Samsung/some phones: **Settings → Developer options**):
   - **Android 14+ stability tip (optional):** if **Disable child process restrictions** appears, toggle it **ON** so Android's phantom process killer does not terminate background shells inside the terminal.
   - Keep **USB debugging** turned **OFF** unless you explicitly need `adb` from a computer (leaving USB debugging OFF prevents bypassing store locks via a PC later).

### Step 2 — Install F-Droid

1. Open the phone's web browser (for example Chrome) and go to `https://f-droid.org`.
2. Tap **Download F-Droid** to download `F-Droid.apk`.
3. Open the downloaded `F-Droid.apk` (from the browser notification or the **Files / Downloads** app).
4. Android will warn that your browser is not allowed to install unknown apps. Tap **Settings** on that prompt, temporarily switch **Allow from this source** **ON**, press **Back**, and tap **Install**.
5. Open **F-Droid**, wait ~30 seconds for its package index to update, and when prompted to allow F-Droid to install apps, grant **Allow from this source** for **F-Droid**.

### Step 3 — Install this app from F-Droid

1. Open **F-Droid** and tap the **Search** button.
2. Search for **emthin** (once published in the F-Droid repository), open its app page, and tap **Install**.
3. Wait for the download and tap **Install** when Android asks to confirm.
4. Tap **Open** once to let initial setup/bootstrap complete cleanly.

### Step 4 — Turn on automatic updates on Google Play Store and F-Droid

1. **Google Play Store auto-updates:**
   - Open **Google Play Store** → tap the profile icon (top-right) → **Settings → Network preferences → Auto-update apps**.
   - Choose **Over Wi-Fi only** (or **Over any network** if unlimited mobile data is preferred).
2. **F-Droid auto-updates:**
   - Open **F-Droid → Settings → Updates**.
   - Set **Automatic update interval** (for example **Daily** or **Every 12 hours**).
   - Enable **Automatically fetch updates** (under **Over Wi-Fi**).
   - Enable **Automatically install updates** (supported unattended on Android 12+).
   - Also open **Settings → Apps → F-Droid → Battery** and choose **Unrestricted** so Android does not put F-Droid to sleep before scheduled update checks run.

### Step 5 — Block installing `.apk` files outside Google Play Store and F-Droid

1. Open **Settings → Apps → Special app access → Install unknown apps** (or search `Install unknown apps` in the Settings search bar).
2. Check **every** app in the list (especially browsers such as Chrome, file managers such as Files, messaging apps such as WhatsApp/Telegram/Messages, email apps, and cloud drives) and set **Allow from this source** to **OFF**.
3. Leave **F-Droid** set to **ON** so F-Droid can keep downloading and installing updates automatically. *(Google Play Store does not appear/need this toggle because Android treats it as a built-in store.)*
4. Open the **Files / Downloads** folder and delete `F-Droid.apk` (and any other downloaded `.apk` files).
5. Return to **Settings → System → Developer options** and verify **USB debugging** is **OFF**. Unless you need Android 14+'s *Disable child process restrictions*, you can also switch **Use developer options** **OFF** completely now.

### Step 6 — Password-protect Google Play Store, F-Droid, and Settings

> [!IMPORTANT]
> F-Droid does **not** have a built-in launch password, and anyone who can open **Settings** can turn *Install unknown apps* back on. Password-locking **Settings** alongside **Google Play Store** and **F-Droid** is what keeps Step 5 locked in.

1. **Turn on Google Play Store parental controls & purchase lock:**
   - Open **Google Play Store** → profile icon → **Settings → Family → Parental controls**.
   - Toggle **Parental controls** **ON**, create a **parent-only PIN**, and filter **Apps & games** / **Movies** to the teenager's age rating.
   - Under **Settings → Authentication → Require authentication for purchases**, select **For all purchases through Google Play on this device** (or **Require biometrics/password**).
   - If the phone is supervised by **Google Family Link**, open the parent's **Family Link** app → **Controls → Google Play → Require approval for → All content** (so every new Play Store install requires parent approval, while auto-updates still run automatically).
2. **Password-lock Google Play Store, F-Droid, and Settings:**
   - **Option A — Built-in Phone App Lock (Samsung Secure Folder / Xiaomi, OnePlus, Oppo, Realme, Huawei App Lock):**
     1. Open **Settings → Security / Privacy / Apps → App Lock**.
     2. Set a **parent-only PIN/password** (do **not** enroll the teenager's fingerprint or face into App Lock).
     3. Lock **Google Play Store**, **F-Droid**, and **Settings** (plus any pre-installed manufacturer app store if your device has one, e.g. Galaxy Store).
   - **Option B — Google Family Link + App Locker (stock Android / Pixel without built-in App Lock):**
     - **With Family Link alone:** turn **Apps from unknown sources** **OFF** in the Family Link parent app, require approval for **All content** in Google Play, and set an app limit / block on **F-Droid** between parent check-ins *(note: when F-Droid is blocked or unknown sources is disabled by Family Link, F-Droid background auto-updates pause until the parent temporarily re-enables them)*.
     - **With a PIN App Locker (if not using built-in App Lock):** install a reputable **App Lock** app from Google Play Store *before* locking Play Store, grant it **Usage access** and **Display over other apps**, disable battery optimization for it, give it a **parent-only PIN**, and lock **Google Play Store**, **F-Droid**, and **Settings** (and prevent uninstalling the locker via **Device admin** if offered).

### Final check

- Try opening an `.apk` link in the browser or **Files** — Android must refuse installation unless enabled in locked **Settings**.
- Tap **F-Droid**, **Google Play Store**, and **Settings** — each should ask for the parent PIN/approval.
- Open **emthin** — it should launch normally for everyday use without prompting for any parent PIN.

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
