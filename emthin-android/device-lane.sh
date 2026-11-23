#!/usr/bin/env bash
# device-lane.sh — the emthin pad's repeatable adb e2e lane. No root needed.
#
# The root-gated waydroid checks live in the workspace root's
# scripts/waydroid-sudo-fixes.sh (section D). This script is the
# *unprivileged* half: everything here runs as the session user, because
# the container's /data is a user-owned host directory and its adbd is
# reached over the adb TCP link the user's key authorizes.
#
# It encodes the fiddly bits found by the 2026-10-19 visual loop so the
# device checks are repeatable rather than re-derived each session:
#   * authorize — the container's adbd reads $HOME/.android/adbkey.pub
#     out of data/misc/adb/adb_keys, a user-owned file, so authorizing
#     is a rootless copy (this is why no RSA dialog ever appears: the key
#     is pre-seeded, not confirmed on-screen);
#   * screencap — `adb exec-out screencap -p` leaks a text warning line
#     (/vendor/... hwcomposer / Mesa) before the PNG magic; the capture
#     is re-sliced to the first \x89PNG so the result is a real image;
#   * swipe — a real touch drag inside the preview ScrollView (host XTEST
#     swipes move nothing; `input swipe` injects at the surface).
#
# Usage:
#   ./device-lane.sh              # full e2e lane (default)
#   ./device-lane.sh <command>    # one step
#
# Commands:
#   authorize    seed the container's adb_keys with the host adb pubkey
#   connect      adb connect + wait until state == device
#   build        build the APK for the connected device's ABI
#   install      adb install -r the built APK (implies connect)
#   launch       am start -W the pad activity
#   austral-rule  native Austral rule artifact on a worker thread (K9 arm64)
#   history-preview  read-only history snapshot preview (K5)
#   pull         pull the installed base.apk, md5-compare to the built APK
#   screencap F  capture the screen to file F (prefix-stripped PNG)
#   logcat F     dump the pad's logcat to F
#   swipe        one drag inside the preview ScrollView
#   math-preview  rendered prose and equation agree with the on-screen hit map
#                (requires the disposable test APK and Python Pillow)
#   enter        Return in the preview and keypad Enter in its source editor
#                insert newlines (requires the disposable test APK and python3)
#   persistence  edit -> force-stop -> relaunch autosave proof (used by e2e)
#   new-reset    New confirmation -> starter reset -> Undo restore proof
#                (used by e2e)
#   scroll-anchor  cold-launch starter -> page-2 -> one-page collapse ->
#                undo reading-position proof (used by e2e)
#   search-replace  Find -> Accept -> Back/Forward -> Replace current -> Undo
#                (used by e2e)
#   accessibility  TalkBack enabled + bound -> page spoken and pad registered
#                as a client. Installs TALKBACK_APK when available, otherwise
#                uses an existing install, otherwise skips explicitly
#                (used by e2e)
#   diagnostics   exact + owner banner -> diagnostics list -> source -> Back
#                (used by e2e)
#   named-document  open a real named source file -> status shows its name
#                (used by e2e)
#   focus-stack  Find/replace IME + dialog Back + inspector Escape
#                (used by e2e)
#   composing-ime  test-only IME -> composing CJK -> commit -> emoji commit
#                (used by e2e; requires the disposable test APK)
#   modifier-inspector  deterministic modifier/target fixture -> incoming and
#                outgoing rows -> expand one comparison (used by e2e)
#   session-pin  fixture pin -> Undo -> Redo -> force-stop/reopen -> persisted
#                terminal figure (used by e2e; requires the disposable test APK)
#   figure-a11y  fixture watched terminal figure -> accessibility node state
#                and Go/Rebind/Detach/Sessions actions through the provider
#                (used by e2e; requires the disposable test APK)
#   terminal-scene  native VT scene -> placed rect, animation, context loss,
#                resize, keys, long-press copy, mouse-mode tap and Back
#                releasing focus (used by e2e; requires the disposable test APK)
#   terminal-pages   multi-page stream -> leave page -> return -> still streaming
#                (used by e2e; requires the disposable test APK)
#   terminal-lease   scripted gateway peer -> watch attach -> take control ->
#                controller input over DATA -> operator revoke -> read-only
#                (used by e2e; requires the disposable test APK and python3)
#   terminal-reflow  streaming scene -> view zoom -> page rebuild -> grid
#                reflow -> continued animation after a pan
#                (used by e2e; requires the disposable test APK)
#   e2e          authorize, connect, build, install, launch, screencap,
#                swipe, screencap, logcat, persistence, new-reset,
#                scroll-anchor, search-replace, focus-stack, composing-ime,
#                accessibility, diagnostics, named-document, modifier-inspector,
#                session-pin, figure-a11y, terminal-scene, terminal-reflow,
#                terminal-pages, terminal-lease, pull + byte-identity
#                (the repeatable device proof)
#
# Env overrides:
#   DEVICE       adb serial (default: waydroid status IP + :5555; an explicit
#                serial skips the waydroid-session prerequisite)
#   ADB          adb executable (default: adb on PATH)
#   APK          built apk path (default: <script dir>/build/emthin-pad.apk)
#   OUTDIR       screenshot/logcat output dir (default: /tmp/emthin-device-lane)
#   SKIP_BUILD   1 = e2e uses the existing APK instead of rebuilding it
#   EMTHIN_LANE_PACKAGE  application id to install/drive (default:
#                com.emthin.pad.test). The test package is disposable; a normal
#                package additionally requires EMTHIN_LANE_DISPOSABLE=1.
#   EMTHIN_LANE_DISPOSABLE  1 = allow `pm clear` on a non-test package. Only
#                set it for an install with no working document.
#   TALKBACK_APK path to a TalkBack APK. When TalkBack is absent and this is
#                readable, the accessibility step installs it for the check and
#                uninstalls it afterwards. Without it an absent TalkBack skips.
#   SWIPE_X / SWIPE_FROM_Y / SWIPE_TO_Y / SWIPE_MS   drag (default: from wm size)
#
# Exit status: 0 = every step passed, 1 = a step failed, 2 = bad usage,
# 3 = a prerequisite (adb / a running waydroid session) is missing.

set -u
PATH=/run/current-system/sw/bin:$PATH
export PATH

TEST_PACKAGE=com.emthin.pad.test
PKG="${EMTHIN_LANE_PACKAGE:-$TEST_PACKAGE}"
ACTIVITY="$PKG/com.emthin.pad.MainActivity"
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
if [ "$PKG" = "$TEST_PACKAGE" ]; then
    APK="${APK:-$SCRIPT_DIR/build/test/emthin-pad-test.apk}"
else
    APK="${APK:-$SCRIPT_DIR/build/emthin-pad.apk}"
fi
OUTDIR="${OUTDIR:-/tmp/emthin-device-lane}"
XDG_DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
CONTAINER_ADB_KEYS="$XDG_DATA/waydroid/data/misc/adb/adb_keys"
HOST_ADB_PUB="$HOME/.android/adbkey.pub"
# Preview drag geometry: resolved from `wm size` when not overridden,
# because the preview's lower-half position changes with the window. A
# drag *up* (from > to) scrolls the preview content down.
SWIPE_X="${SWIPE_X:-}"
SWIPE_FROM_Y="${SWIPE_FROM_Y:-}"
SWIPE_TO_Y="${SWIPE_TO_Y:-}"
SWIPE_MS="${SWIPE_MS:-500}"
UI_DUMP_REMOTE=/data/local/tmp/emthin-ui.xml
UI_XML="$OUTDIR/ui.xml"

FAILURES=0
DEVICE="${DEVICE:-}"
ADB="${ADB:-adb}"
SKIP_BUILD="${SKIP_BUILD:-0}"
EMTHIN_LANE_DISPOSABLE="${EMTHIN_LANE_DISPOSABLE:-0}"

say()  { printf '%s\n' "$*"; }
pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; FAILURES=$((FAILURES + 1)); }

usage() {
    awk 'NR == 1 { next } /^#/ { sub(/^# ?/, ""); print; next } { exit }' "$0"
}

# Resolve DEVICE from `waydroid status` when not given (no root needed).
resolve_device() {
    if [ -n "$DEVICE" ]; then return 0; fi
    local ip
    ip=$(waydroid status 2>/dev/null | awk '/IP address:/{print $3; exit}')
    DEVICE="${ip:+$ip:5555}"
    DEVICE="${DEVICE:-192.168.240.112:5555}"
}

# adb wrapper bound to $DEVICE.
D() { "$ADB" -s "$DEVICE" "$@"; }

require_adb() {
    if ! command -v "$ADB" >/dev/null 2>&1; then
        say "die  adb not found: $ADB (set ADB, or add pkgs.android-tools to PATH)"
        exit 3
    fi
}

# true if the waydroid session is RUNNING (else the lane has no target).
session_running() {
    waydroid status 2>/dev/null | grep -q 'Session:.*RUNNING'
}

require_session() {
    # An explicit serial may be a physical/remote device, so a local
    # waydroid session is not a prerequisite for the lane.
    if [ -n "$DEVICE" ]; then return 0; fi
    if ! command -v waydroid >/dev/null 2>&1 || ! session_running; then
        say "die  waydroid session not RUNNING (start it: systemctl --user start waydroid-session)"
        exit 3
    fi
}

# ---- commands ---------------------------------------------------------

cmd_authorize() {
    # The container's adbd reads the authorized keys from a user-owned
    # host file; appending the host pubkey authorizes without any
    # on-device dialog. Idempotent.
    if [ ! -f "$HOST_ADB_PUB" ]; then
        mkdir -p "$HOME/.android"
        "$ADB" keygen "$HOME/.android/adbkey" 2>/dev/null || true
    fi
    if [ ! -f "$HOST_ADB_PUB" ]; then
        fail "host adb pubkey at $HOST_ADB_PUB"
        return
    fi
    mkdir -p "$(dirname "$CONTAINER_ADB_KEYS")"
    if [ -f "$CONTAINER_ADB_KEYS" ] && grep -qFf "$HOST_ADB_PUB" "$CONTAINER_ADB_KEYS" 2>/dev/null; then
        pass "container already authorizes this key ($CONTAINER_ADB_KEYS)"
    else
        cat "$HOST_ADB_PUB" >>"$CONTAINER_ADB_KEYS" && \
            pass "appended host pubkey -> $CONTAINER_ADB_KEYS" || \
            fail "write $CONTAINER_ADB_KEYS"
    fi
}

cmd_connect() {
    "$ADB" connect "$DEVICE" >/dev/null 2>&1 || true
    local tries=0 state
    while [ "$tries" -lt 15 ]; do
        state=$(D get-state 2>/dev/null || true)
        if [ "$state" = "device" ]; then
            pass "$DEVICE state=device"
            return
        fi
        sleep 1
        tries=$((tries + 1))
    done
    fail "$DEVICE did not reach state=device (state=${state:-none})"
    say "     hint: run './device-lane.sh authorize' then retry; 'adb connect $DEVICE'"
}

cmd_build() {
    local default_apk
    if [ "$PKG" = "$TEST_PACKAGE" ]; then
        default_apk="$SCRIPT_DIR/build/test/emthin-pad-test.apk"
    else
        default_apk="$SCRIPT_DIR/build/emthin-pad.apk"
    fi
    if [ "$APK" != "$default_apk" ]; then
        say "info build: custom APK path $APK; not rebuilding"
        return 0
    fi
    if [ ! -x "$SCRIPT_DIR/build-apk.sh" ]; then
        fail "build-apk.sh is not executable"
        return 1
    fi
    local test_env=()
    if [ "$PKG" = "$TEST_PACKAGE" ]; then
        test_env=(EMTHIN_TEST_PACKAGE="$TEST_PACKAGE")
    fi
    # build-apk.sh asks adb for the device ABI when ANDROID_ABI is unset;
    # pass both the serial and the adb executable it should use.
    # The devshell's nix rustc (1.98) has no Android target on this host; the
    # rustup 1.99 toolchain does. Prefer /usr/bin's rustup shims when present
    # so a devshell-invoked lane can cross-build, without forcing it on hosts
    # where cargo comes only from nix.
    local build_path="$PATH"
    if [ -x /usr/bin/rustup ] && /usr/bin/rustup target list --installed 2>/dev/null \
            | grep -qx aarch64-linux-android; then
        build_path="/usr/bin:$build_path"
    fi
    if env PATH="$build_path" DEVICE="$DEVICE" ADB="$ADB" "${test_env[@]}" \
            "$SCRIPT_DIR/build-apk.sh"; then
        pass "build-apk.sh (device ABI)"
    else
        fail "build-apk.sh"
        return 1
    fi
}

cmd_install() {
    if [ ! -f "$APK" ]; then
        fail "APK at $APK (build it: ./build-apk.sh)"
        return 1
    fi
    if D install -r "$APK" >/dev/null 2>&1; then
        pass "adb install -r $(basename "$APK")"
    else
        fail "adb install -r (is the device connected? see 'connect')"
        return 1
    fi
}

cmd_launch() {
    # The JIT roundtrip is diagnostic-only: the normal launcher path no
    # longer compiles at startup. This verification launch opts in so the
    # logcat proof below still exercises the australVM lane.
    if D shell am start -W -n "$ACTIVITY" --ez emthin.jitSmoke true >/dev/null 2>&1; then
        pass "am start -W $ACTIVITY (jit diagnostic)"
    else
        fail "am start -W $ACTIVITY (jit diagnostic)"
    fi
}

# The pad autosaves. Device checks that assert the starter document must
# clear that saved text first or they inherit the previous run. Refuse by
# default: this wipes the only copy of an unsaved working document.
cmd_clear_data() {
    if [ "$PKG" != "$TEST_PACKAGE" ] && [ "$EMTHIN_LANE_DISPOSABLE" != 1 ]; then
        fail "refusing pm clear $PKG: use $TEST_PACKAGE or set EMTHIN_LANE_DISPOSABLE=1 for a disposable install"
        return 1
    fi
    if D shell pm clear "$PKG" >/dev/null 2>&1; then
        pass "cleared pad app data"
    else
        fail "pm clear $PKG"
        return 1
    fi
}

cmd_pull() {
    local remote out lmd5 rmd5
    remote=$(D shell pm path "$PKG" 2>/dev/null | sed -n 's/^package://p' | tr -d '\r' | head -1)
    if [ -z "$remote" ]; then
        fail "$PKG not installed (pm path empty)"
        return
    fi
    mkdir -p "$OUTDIR"
    out="$OUTDIR/pulled-base.apk"
    if ! D pull "$remote" "$out" >/dev/null 2>&1; then
        fail "adb pull $remote"
        return
    fi
    if [ ! -f "$APK" ]; then
        fail "no built APK at $APK to compare against"
        return
    fi
    lmd5=$(md5sum "$APK" | awk '{print $1}')
    rmd5=$(md5sum "$out" | awk '{print $1}')
    if [ "$lmd5" = "$rmd5" ]; then
        pass "installed base.apk byte-identical to build (md5 $lmd5)"
    else
        fail "installed base.apk differs from build ($rmd5 != $lmd5)"
    fi
}

# Capture to a file, re-sliced to the PNG magic (screencap leaks a text
# warning line before it). Prints the output path on success.
cmd_screencap() {
    local out="${1:-$OUTDIR/screencap.png}" raw off
    mkdir -p "$(dirname "$out")"
    raw=$(mktemp "$OUTDIR/.raw.XXXXXX" 2>/dev/null || mktemp)
    if ! D exec-out screencap -p >"$raw" 2>/dev/null; then
        rm -f "$raw"
        fail "screencap (device unreachable?)"
        return 1
    fi
    off=$(grep -abo $'\x89PNG' "$raw" 2>/dev/null | head -1 | cut -d: -f1 || true)
    if [ -z "$off" ]; then
        rm -f "$raw"
        fail "no PNG magic in screencap output"
        return 1
    fi
    tail -c +"$((off + 1))" "$raw" >"$out"
    rm -f "$raw"
    # Hard check: the sliced output must actually be a PNG (magic +
    # plausible length), proving the capture+strip pipeline.
    if [ "$(head -c 4 "$out" | od -An -tx1 | tr -d ' \n')" != "89504e47" ] \
            || [ "$(wc -c <"$out")" -lt 1000 ]; then
        fail "screencap did not yield a valid PNG"
        return 1
    fi
    printf '%s' "$out"
}

# Fill in any missing drag coordinates from the live screen size. The
# previous fixed 1920x1168 defaults put the drag outside the preview on a
# 2944x1840 device, making a "successful" e2e prove nothing.
resolve_swipe_geometry() {
    local size w h
    if [ -n "$SWIPE_X" ] && [ -n "$SWIPE_FROM_Y" ] && [ -n "$SWIPE_TO_Y" ]; then
        return 0
    fi
    size=$(D shell wm size 2>/dev/null | awk '/size:/{s=$NF} END{print s}' | tr -d '\r')
    case "$size" in
        *x*) w=${size%x*}; h=${size#*x} ;;
        *) w=1920; h=1168 ;;
    esac
    case "$w$h" in
        *[!0-9]*|'') w=1920; h=1168 ;;
    esac
    SWIPE_X=${SWIPE_X:-$((w / 2))}
    SWIPE_FROM_Y=${SWIPE_FROM_Y:-$((h * 3 / 4))}
    SWIPE_TO_Y=${SWIPE_TO_Y:-$((h * 11 / 20))}
}

cmd_swipe() {
    resolve_swipe_geometry
    if D shell input swipe "$SWIPE_X" "$SWIPE_FROM_Y" "$SWIPE_X" "$SWIPE_TO_Y" "$SWIPE_MS" \
            >/dev/null 2>&1; then
        pass "input swipe ($SWIPE_X $SWIPE_FROM_Y -> $SWIPE_X $SWIPE_TO_Y)"
    else
        fail "input swipe"
    fi
}

# ---- reading-position proof ------------------------------------------

# Dump the live hierarchy so the check taps the actual editor and Undo
# button instead of hard-coded pixels. /sdcard is unreliable on this
# image; /data/local/tmp is not.
cmd_uidump() {
    local out="${1:-$UI_XML}"
    UI_XML="$out"
    mkdir -p "$(dirname "$out")"
    if ! D shell uiautomator dump "$UI_DUMP_REMOTE" >/dev/null 2>&1; then
        fail "uiautomator dump"
        return 1
    fi
    if ! D exec-out cat "$UI_DUMP_REMOTE" >"$out" 2>/dev/null; then
        fail "read $UI_DUMP_REMOTE"
        return 1
    fi
    tr -d '\r' <"$out" >"$out.tmp" && mv "$out.tmp" "$out"
    if ! grep -q '<hierarchy' "$out"; then
        fail "uiautomator dump missing hierarchy"
        return 1
    fi
    pass "uiautomator hierarchy -> $out"
}

# Bounds of the first node whose opening tag contains PATTERN, as
# "x1 y1 x2 y2".
node_bounds() {
    local pattern="$1" node
    node=$(grep -o "<node[^>]*${pattern}[^>]*>" "$UI_XML" 2>/dev/null | head -1 || true)
    [ -n "$node" ] || return 1
    printf '%s\n' "$node" | sed -n \
        's/.*bounds="\[\([0-9]*\),\([0-9]*\)\]\[\([0-9]*\),\([0-9]*\)\]".*/\1 \2 \3 \4/p'
}

tap_center() {
    local b="$1" x1 y1 x2 y2
    read -r x1 y1 x2 y2 <<EOF
$b
EOF
    D shell input tap "$(( (x1 + x2) / 2 ))" "$(( (y1 + y2) / 2 ))" >/dev/null 2>&1
}

tap_toolbar_menu() {
    local group="$1" action="$2" bounds
    bounds=$(node_bounds 'content-desc="More actions"') || return 1
    tap_center "$bounds" || return 1
    cmd_uidump "$UI_XML" || return 1
    bounds=$(node_bounds "text=\"$group\"") || return 1
    tap_center "$bounds" || return 1
    cmd_uidump "$UI_XML" || return 1
    bounds=$(node_bounds "text=\"$action\"") || return 1
    tap_center "$bounds"
}

# First paint focuses the rendered page at the document end. Tapping the
# page would move the caret to the tapped glyph and may open the source field,
# so the lane verifies the rendered surface and leaves focus where it is.
tap_editor_end() {
    node_bounds 'class="android.widget.ImageView"' >/dev/null || return 1
    D shell input keyevent KEYCODE_MOVE_END >/dev/null 2>&1
}

keyboard_shown() {
    D shell dumpsys input_method 2>/dev/null | tr -d '\r' \
        | grep -q 'mInputShown=true'
}

# `relationship_drag <dump>` answers "x1 y1 x2 y2 ms" for a drag inside the
# scrollable pane that holds the modifier inspector's relationship headings, or
# nothing when the dump shows no such pane. The pane's y range follows the
# layout, not the screen, so a pixel constant tuned to one device lands outside
# it on another and silently scrolls the document instead.
relationship_drag() {
    python3 - "$1" <<'PY'
import re, sys, xml.etree.ElementTree as ET

try:
    root = ET.parse(sys.argv[1]).getroot()
except Exception:
    sys.exit(0)
box = re.compile(r'\[(\d+),(\d+)\]\[(\d+),(\d+)\]')
headings = {"Outgoing targets", "Incoming modifiers"}

def bounds(node):
    m = box.match(node.get("bounds", ""))
    return tuple(int(v) for v in m.groups()) if m else None

class Found(Exception):
    def __init__(self, text):
        self.text = text

def walk(node, scrollable):
    if node.get("scrollable") == "true":
        scrollable = node
    if node.get("text") in headings and scrollable is not None:
        raise Found(scrollable)
    for child in node:
        walk(child, scrollable)

try:
    walk(root, None)
except Found as found:
    b = bounds(found.text)
    if b and b[3] - b[1] > 40:
        x = (b[0] + b[2]) // 2
        span = max(40, (b[3] - b[1]) // 2)
        print(x, b[3] - 12, x, b[3] - 12 - span, 300)
PY
}

# `relationship_scroll <dump> <up|down>` drags inside that pane. The pane's own
# headings scroll out of view once the list moves, so the first geometry found is
# kept in REL_DRAG and reused when the current dump no longer describes it.
relationship_scroll() {
    local found
    found="$(relationship_drag "$1")"
    [ -n "$found" ] && REL_DRAG="$found"
    [ -n "${REL_DRAG:-}" ] || return 0
    local x1 y1 x2 y2 ms
    read -r x1 y1 x2 y2 ms <<< "$REL_DRAG"
    if [ "$2" = up ]; then
        D shell input swipe "$x1" "$y1" "$x2" "$y2" "$ms" >/dev/null 2>&1
    else
        D shell input swipe "$x2" "$y2" "$x1" "$y1" "$ms" >/dev/null 2>&1
    fi
}

wait_for_log() {
    local out="$1" pattern="$2" tries=0
    while [ "$tries" -lt 50 ]; do
        D logcat -d -s emthin-pad:D 2>/dev/null | tr -d '\r' >"$out"
        if grep -q "$pattern" "$out"; then
            return 0
        fi
        sleep 0.2
        tries=$((tries + 1))
    done
    return 1
}

# Last textLen reported by any trace line. Device checks derive edit
# expectations from the actual starter instead of hard-coding its length.
last_text_len() {
    awk '
        {
            for (i = 1; i <= NF; i++) {
                if ($i == "textLen") { n = $(i + 1) + 0; found = 1; }
            }
        }
        END { if (found) print n }
    ' "$1"
}

# Last restoreScroll's "y actual" pair.
last_restore() {
    awk '
        /restoreScroll/ {
            y = -1; a = -1;
            for (i = 1; i <= NF; i++) {
                if ($i == "y") y = $(i + 1) + 0;
                else if ($i == "actual") a = $(i + 1) + 0;
            }
        }
        END { if (y < 0) y = 0; if (a < 0) a = 0; print y, a }
    ' "$1"
}

cmd_persistence() {
    local log="$OUTDIR/persistence-logcat.txt"
    local ui="$OUTDIR/persistence-ui.xml"
    local focus shot

    say "==> persistence: edit -> force-stop -> relaunch"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    cmd_launch || return
    wait_for_log "$log" 'applyChunk total 2 pages [12]' \
        || { fail "starter document did not render two pages"; return; }
    wait_for_log "$log" 'captionScroll' \
        || { fail "starter document did not open at the first caption"; return; }
    pass "cold-launched the starter"
    local base_len
    base_len=$(last_text_len "$log")
    if [ -z "$base_len" ] || [ "$base_len" -le 0 ]; then
        fail "cold launch did not report the starter text length"
        return
    fi

    cmd_uidump "$ui" || return
    tap_editor_end || { fail "editor not found in UI hierarchy"; return; }
    sleep 1.0
    if keyboard_shown; then
        D shell input keyevent KEYCODE_BACK >/dev/null 2>&1
        sleep 0.4
        pass "closed the IME"
    fi
    focus=$(D shell dumpsys window 2>/dev/null | tr -d '\r' | grep -m1 'mCurrentFocus' || true)
    case "$focus" in
        *com.emthin.pad*) ;;
        *) fail "activity left the foreground before typing: ${focus:-no focus}"; return ;;
    esac

    D logcat -c >/dev/null 2>&1 || true
    D shell input text X >/dev/null 2>&1
    wait_for_log "$log" "applyState setCaret false .*textLen $((base_len + 1))" \
        || { fail "typed probe did not reach the model"; return; }

    # Move one grapheme back from the document end. The first-paint
    # fallback uses text.length() (90); a restored caret is 89, so the
    # log line distinguishes a real restore from the fallback. Caret motion
    # is overlay-only and must not start a render.
    D logcat -c >/dev/null 2>&1 || true
    D shell input keyevent KEYCODE_DPAD_LEFT >/dev/null 2>&1
    wait_for_log "$log" "caretMove offset $base_len" \
        || { fail "caret probe did not move to $base_len"; return; }
    if grep -q 'startRender' "$log"; then
        fail "caret movement triggered a layout render"
        return
    fi
    sleep 0.5

    # Move the preview into page 2's content, then background the app so
    # onPause saves the caret and reading offset. A plain force-stop
    # would not run onPause.
    for i in 1 2 3 4 5 6 7 8 9 10; do
        cmd_swipe
        sleep 0.25
    done
    D shell input keyevent KEYCODE_HOME >/dev/null 2>&1
    sleep 1.2
    D shell am force-stop "$PKG" >/dev/null 2>&1 || true
    sleep 0.3
    D logcat -c >/dev/null 2>&1 || true
    cmd_launch || return
    wait_for_log "$log" "applyState setCaret true .*textLen $((base_len + 1))" \
        || { fail "relaunch lost the saved edited document"; return; }
    pass "relaunch restored the edited document (textLen $((base_len + 1)))"
    wait_for_log "$log" "startRender .*textLen $((base_len + 1)) caret $base_len" \
        || { fail "relaunch did not restore the caret at $base_len"; return; }
    pass "relaunch restored the caret at $base_len"
    wait_for_log "$log" 'applyChunk total 2 pages [12]' \
        || { fail "restored document did not render two pages"; return; }
    wait_for_log "$log" 'restoreScroll' \
        || { fail "relaunch did not restore the preview reading position"; return; }
    read -r restored_y restored_actual < <(last_restore "$log")
    if [ "$restored_y" -le 0 ]; then
        fail "relaunch did not save a positive reading offset"
        return
    fi
    if [ "$restored_y" -ne "$restored_actual" ]; then
        fail "relaunch restored y $restored_y but landed at $restored_actual"
        return
    fi
    pass "relaunch restored reading y $restored_y"
    sleep 0.6
    pass "restored document rendered"

    if shot=$(cmd_screencap "$OUTDIR/persistence-after-restart.png"); then
        say "    captured $shot"
    fi
}

cmd_new_reset() {
    local log="$OUTDIR/new-reset-logcat.txt"
    local ui="$OUTDIR/new-reset-ui.xml"
    local b focus shot base_len

    say "==> new document: reset -> undo"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    cmd_launch || return
    wait_for_log "$log" 'applyChunk total 2 pages [12]' \
        || { fail "starter document did not render two pages"; return; }
    wait_for_log "$log" 'captionScroll' \
        || { fail "starter document did not open at the first caption"; return; }
    pass "cold-launched the starter for New"
    base_len=$(last_text_len "$log")
    if [ -z "$base_len" ] || [ "$base_len" -le 0 ]; then
        fail "cold launch did not report the starter text length"
        return
    fi

    cmd_uidump "$ui" || return
    tap_editor_end || { fail "editor not found in UI hierarchy"; return; }
    sleep 1.0
    if keyboard_shown; then
        D shell input keyevent KEYCODE_BACK >/dev/null 2>&1
        sleep 0.4
        pass "closed the IME"
    else
        say "info IME was already hidden"
    fi
    focus=$(D shell dumpsys window 2>/dev/null | tr -d '\r' | grep -m1 'mCurrentFocus' || true)
    case "$focus" in
        *com.emthin.pad*) pass "activity still focused after closing the IME" ;;
        *) fail "activity left the foreground after closing the IME: ${focus:-no focus}"; return ;;
    esac

    # Change the document so New has something real to discard and Undo
    # has a step to restore. X adds one byte to the observed starter.
    D logcat -c >/dev/null 2>&1 || true
    D shell input text X >/dev/null 2>&1
    wait_for_log "$log" "applyState setCaret false .*textLen $((base_len + 1))" \
        || { fail "New probe did not reach the model"; return; }
    wait_for_log "$log" "startRender .*textLen $((base_len + 1))" \
        || { fail "New probe did not render"; return; }
    wait_for_log "$log" 'applyChunk total 2 pages [12]' \
        || { fail "New probe did not render two pages"; return; }
    pass "typed the $((base_len + 1))-byte probe"

    # Open the confirmation through the actual toolbar button, assert its
    # title, and capture the dark dialog before confirming it.
    D logcat -c >/dev/null 2>&1 || true
    cmd_uidump "$ui" || return
    tap_toolbar_menu Document New \
        || { fail "New not found in UI hierarchy"; return; }
    sleep 0.8
    cmd_uidump "$ui" || return
    if ! grep -q 'text="New document"' "$ui"; then
        fail "New confirmation did not open"
        return
    fi
    pass "New confirmation opened"
    if shot=$(cmd_screencap "$OUTDIR/new-dialog.png"); then
        say "    captured $shot"
    fi

    b=$(node_bounds 'resource-id="android:id/button1"') \
        || { fail "New confirmation has no positive button"; return; }
    tap_center "$b"
    wait_for_log "$log" "applyState setCaret true .*textLen $base_len canUndo true" \
        || { fail "New did not reset the document to $base_len bytes"; return; }
    pass "New reset the document to $base_len bytes"
    wait_for_log "$log" "startRender .*textLen $base_len" \
        || { fail "New reset did not render"; return; }
    wait_for_log "$log" 'applyChunk total 2 pages [12]' \
        || { fail "New reset did not render two pages"; return; }
    wait_for_log "$log" 'captionScroll' \
        || { fail "New reset did not reopen at the first caption"; return; }
    sleep 0.5

    # The confirmation is only useful if the reset is undoable.
    cmd_uidump "$ui" || return
    if ! grep -q 'text="Undo"[^>]*enabled="true"' "$ui"; then
        fail "Undo was not enabled after New"
        return
    fi
    pass "Undo is enabled after New"
    if shot=$(cmd_screencap "$OUTDIR/new-reset-after-new.png"); then
        say "    captured $shot"
    fi

    D logcat -c >/dev/null 2>&1 || true
    b=$(node_bounds 'text="Undo"') \
        || { fail "Undo not found in UI hierarchy"; return; }
    tap_center "$b"
    wait_for_log "$log" "applyState setCaret true .*textLen $((base_len + 1))" \
        || { fail "Undo did not restore the edited document"; return; }
    pass "Undo restored the $((base_len + 1))-byte edited document"
    wait_for_log "$log" "startRender .*textLen $((base_len + 1))" \
        || { fail "Undo restore did not render"; return; }
    wait_for_log "$log" 'applyChunk total 2 pages [12]' \
        || { fail "Undo restore did not render two pages"; return; }
    sleep 0.5
    if shot=$(cmd_screencap "$OUTDIR/new-reset-after-undo.png"); then
        say "    captured $shot"
    fi
}

cmd_scroll_anchor() {
    local log="$OUTDIR/scroll-anchor-logcat.txt"
    local ui="$OUTDIR/scroll-anchor-ui.xml"
    local target collapse_actual undo_y undo_actual base_len
    local b focus i shot

    say "==> scroll anchor: page 2 -> one-page collapse -> undo"
    mkdir -p "$OUTDIR"

    # Self-contained start: a previous run or the persistence check may
    # have left text behind, so clear app data and cold-launch the
    # starter instead of inheriting whatever state the app is in.
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    cmd_launch || return
    wait_for_log "$log" 'applyChunk total 2 pages [12]' \
        || { fail "starter document did not render two pages"; return; }
    wait_for_log "$log" 'captionScroll' \
        || { fail "starter document did not open at the first caption"; return; }
    pass "cold-launched two-page starter at the first caption"

    base_len=$(last_text_len "$log")
    if [ -z "$base_len" ] || [ "$base_len" -le 0 ]; then
        fail "cold launch did not report the starter text length"
        return
    fi

    # Put the caret at the document end and close the IME *before*
    # scrolling. The space then inserts at the end without a keyboard
    # resize changing the preview height or its clamped maximum.
    cmd_uidump "$ui" || return
    tap_editor_end || { fail "editor not found in UI hierarchy"; return; }
    sleep 1.0
    if keyboard_shown; then
        D shell input keyevent KEYCODE_BACK >/dev/null 2>&1
        sleep 0.4
        pass "closed the IME"
    else
        say "info IME was already hidden"
    fi
    focus=$(D shell dumpsys window 2>/dev/null | tr -d '\r' | grep -m1 'mCurrentFocus' || true)
    case "$focus" in
        *com.emthin.pad*) pass "activity still focused after closing the IME" ;;
        *) fail "activity left the foreground after closing the IME: ${focus:-no focus}"; return ;;
    esac

    # Eight drags put the reading position past the height of a one-page
    # document; further drags at the end of the range are harmless.
    for i in 1 2 3 4 5 6 7 8; do
        cmd_swipe
        sleep 0.25
    done

    # Clear the log here: the collapse's own startRender captures the
    # pre-edit position, and its restoreScroll reports both the retained
    # target and the shorter document's clamped maximum.
    D logcat -c >/dev/null 2>&1 || true
    D shell input keyevent KEYCODE_SPACE >/dev/null 2>&1
    wait_for_log "$log" 'applyChunk total 1 pages 1' \
        || { fail "space did not collapse the document to one page"; return; }
    sleep 0.6
    D logcat -d -s emthin-pad:D 2>/dev/null | tr -d '\r' >"$log"
    read -r target collapse_actual < <(last_restore "$log")
    if [ "$target" -le 0 ]; then
        fail "collapse did not report a pre-edit scroll target"
        return
    fi
    if [ "$collapse_actual" -ge "$target" ]; then
        fail "collapse did not clamp (actual=$collapse_actual target=$target)"
        return
    fi
    if ! grep -q "startRender .*caret $((base_len + 1))" "$log"; then
        fail "space was not inserted at the document end"
        return
    fi
    pass "collapse retained y $target (clamped to $collapse_actual)"
    cp "$log" "$OUTDIR/scroll-anchor-collapse-logcat.txt"

    # Start a fresh log for the undo, so the post-undo assertions cannot
    # match the pre-collapse two-page render.
    D logcat -c >/dev/null 2>&1 || true
    cmd_uidump "$ui" || return
    b=$(node_bounds 'text="Undo"') \
        || { fail "Undo not found in UI hierarchy"; return; }
    tap_center "$b"
    wait_for_log "$log" 'applyChunk total 2 pages [12]' \
        || { fail "undo did not restore the two-page document"; return; }
    sleep 0.6
    D logcat -d -s emthin-pad:D 2>/dev/null | tr -d '\r' >"$log"
    read -r undo_y undo_actual < <(last_restore "$log")
    if [ "$undo_y" -ne "$target" ] || [ "$undo_actual" -ne "$target" ]; then
        fail "undo did not return to y $target (y=$undo_y actual=$undo_actual)"
        return
    fi
    pass "undo restored y $target"

    if shot=$(cmd_screencap "$OUTDIR/scroll-anchor-after.png"); then
        say "    captured $shot"
    fi
}

cmd_search_replace() {
    local log="$OUTDIR/search-replace-logcat.txt"
    local ui="$OUTDIR/search-replace-ui.xml"
    local b shot base_len query=terminal replacement=cursor

    say "==> search/replace: accept -> history -> replace current -> undo"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    cmd_launch || return
    wait_for_log "$log" 'applyChunk total 2 pages [12]' \
        || { fail "starter document did not render two pages"; return; }
    sleep 0.6
    base_len=$(last_text_len "$log")
    if [ -z "$base_len" ] || [ "$base_len" -le 0 ]; then
        fail "cold launch did not report the starter text length"
        return
    fi

    cmd_uidump "$ui" || return
    b=$(node_bounds 'text="Find"') \
        || { fail "Find not found in UI hierarchy"; return; }
    tap_center "$b"
    sleep 0.5
    cmd_uidump "$ui" || return
    if ! grep -q 'content-desc="Find in document"' "$ui"; then
        fail "Find bar did not open"
        return
    fi
    b=$(node_bounds 'content-desc="Find in document"') \
        || { fail "Find query field not found"; return; }
    tap_center "$b"
    sleep 0.3
    D shell input text "$query" >/dev/null 2>&1
    wait_for_log "$log" "findScope scope text query $query total 1" \
        || { fail "scoped search did not report the starter's terminal match"; return; }
    pass "Find reported one literal Text match"

    cmd_uidump "$ui" || return
    grep -Eq 'text="[^"]* · 1 / 1"' "$ui" \
        || { fail "main status did not carry the Find match count"; return; }
    pass "main status carried the Find match count"

    b=$(node_bounds 'text="Enter Accept"') \
        || { fail "Accept not found in UI hierarchy"; return; }
    D logcat -c >/dev/null 2>&1 || true
    tap_center "$b"
    wait_for_log "$log" 'navigate kind search' \
        || { fail "Accept did not record a search destination"; return; }
    pass "Accept pushed the search destination"
    if grep -q 'startRender' "$log"; then
        fail "Accept triggered a full render instead of a direct destination"
        return
    fi

    cmd_uidump "$ui" || return
    tap_toolbar_menu Navigation Back \
        || { fail "Back action not found in UI hierarchy"; return; }
    wait_for_log "$log" 'navigate kind current' \
        || { fail "Back did not restore the pre-search location"; return; }
    pass "Back restored the pre-search destination"

    cmd_uidump "$ui" || return
    tap_toolbar_menu Navigation Forward \
        || { fail "Forward action not found in UI hierarchy"; return; }
    wait_for_log "$log" 'navigate kind search' \
        || { fail "Forward did not return to the accepted search"; return; }
    pass "Forward returned to the accepted search"

    # Replace the current literal match, then one Undo restores the starter.
    # Find is reopened through the visible toolbar action.
    cmd_uidump "$ui" || return
    b=$(node_bounds 'text="Find"') \
        || { fail "Find not found for replacement"; return; }
    tap_center "$b"
    sleep 0.4
    cmd_uidump "$ui" || return
    b=$(node_bounds 'content-desc="Find in document"') \
        || { fail "Find query field not found for replacement"; return; }
    tap_center "$b"
    sleep 0.2
    D shell input text "$query" >/dev/null 2>&1
    sleep 0.7
    cmd_uidump "$ui" || return
    b=$(node_bounds 'text="Replace…"') \
        || { fail "Replace row action not found"; return; }
    tap_center "$b"
    sleep 0.4
    cmd_uidump "$ui" || return
    b=$(node_bounds 'content-desc="Replacement text"') \
        || { fail "Replacement field not found"; return; }
    tap_center "$b"
    D shell input text "$replacement" >/dev/null 2>&1
    sleep 0.3
    cmd_uidump "$ui" || return
    b=$(node_bounds 'text="Replace"') \
        || { fail "Replace current action not found"; return; }
    tap_center "$b"
    local replaced_len=$((base_len - ${#query} + ${#replacement}))
    wait_for_log "$log" "applyState setCaret true .*textLen $replaced_len" \
        || { fail "Replace current did not update the document to $replaced_len bytes"; return; }
    pass "Replace current replaced the literal match"

    cmd_uidump "$ui" || return
    b=$(node_bounds 'text="Undo"') \
        || { fail "Undo not found after replacement"; return; }
    tap_center "$b"
    wait_for_log "$log" "applyState setCaret true .*textLen $base_len" \
        || { fail "Undo did not restore the $base_len-byte original"; return; }
    pass "one Undo restored the original after Replace current"

    if shot=$(cmd_screencap "$OUTDIR/search-replace-after.png"); then
        say "    captured $shot"
    fi
}

# A15: the banner and status count open the same list; selecting an exact
# row opens whole-document source, selecting an owner row opens source at
# the producing modifier, and Back restores the earlier inspector scope.
cmd_diagnostics() {
    local a11y_was rc
    a11y_was=$(D shell settings get secure accessibility_enabled 2>/dev/null | tr -d '\r')
    if [ "$a11y_was" = "1" ]; then
        D shell settings put secure accessibility_enabled 0 >/dev/null 2>&1
        D shell am force-stop com.android.talkback >/dev/null 2>&1
        say "    paused TalkBack for deterministic diagnostic taps (restored afterwards)"
        sleep 1
    fi
    _diagnostics
    rc=$?
    if [ "$a11y_was" = "1" ]; then
        D shell settings put secure enabled_accessibility_services \
            com.android.talkback/com.google.android.marvin.talkback.TalkBackService >/dev/null 2>&1
        D shell settings put secure accessibility_enabled 1 >/dev/null 2>&1
    fi
    return "$rc"
}

_diagnostics() {
    local log="$OUTDIR/diagnostics-logcat.txt"
    local ui="$OUTDIR/diagnostics-ui.xml"
    local exact owner b
    exact=$(printf '%s\n' '#1 text #2 \app(#1, #2, 10, 10)' \
        '#3 thing #4 \prob(#3, #4)' '' 'Unresolved $z$.')
    owner=$(printf '%s\n' '#1 target #2' '#3 nope #4')

    say "==> diagnostics: banner -> list -> exact/owner source -> Back"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    if D shell am start -W -n "$ACTIVITY" \
            --es emthin.fixtureB64 "$(printf '%s' "$exact" | base64 -w0)" >/dev/null 2>&1; then
        pass "launched the exact-diagnostic fixture"
    else
        fail "launch the exact-diagnostic fixture"
        return
    fi
    wait_for_log "$log" 'diagnostics .*count 1 errors 0 warnings 1 notes 1' \
        || { fail "exact fixture did not publish a warning and a note"; return; }
    sleep 0.8
    cmd_uidump "$ui" || return
    grep -q 'text="1 diagnostic · 1 warning · tap for details"' "$ui" \
        || { fail "diagnostic banner is missing"; return; }
    grep -q 'text="Untitled · unsaved · Recovery copy saved on this device · 1 diagnostic"' "$ui" \
        || { fail "status line does not carry the name, state and diagnostic count"; return; }
    pass "banner and status line report one actionable diagnostic"

    b=$(node_bounds 'text="1 diagnostic · 1 warning · tap for details"') \
        || { fail "diagnostic banner is not tappable"; return; }
    tap_center "$b"
    sleep 0.6
    cmd_uidump "$ui" || return
    grep -q 'Open diagnostic source: execution-unavailable' "$ui" \
        || { fail "execution diagnostic is missing from the list"; return; }
    grep -q 'Open diagnostic source: unresolved-name' "$ui" \
        || { fail "unresolved-name diagnostic is missing from the list"; return; }
    grep -q 'NOTE · unresolved-name · exact' "$ui" \
        || { fail "unresolved-name diagnostic is not marked exact"; return; }
    pass "list shows exact execution and unresolved-name rows"

    b=$(node_bounds 'Open diagnostic source: unresolved-name') \
        || { fail "unresolved-name row is not clickable"; return; }
    tap_center "$b"
    wait_for_log "$log" 'navigate kind diagnostic start 71' \
        || { fail "unresolved name did not navigate to source offset 71"; return; }
    sleep 0.4
    cmd_uidump "$ui" || return
    grep -q 'text="Show document"' "$ui" \
        || { fail "diagnostic navigation did not open the source inspector"; return; }
    pass "exact diagnostic opened source at the unresolved name"

    tap_toolbar_menu Navigation Back \
        || { fail "Back action is missing after diagnostic navigation"; return; }
    sleep 0.8
    cmd_uidump "$ui" || return
    grep -q 'content-desc="Page 1 of 1"' "$ui" \
        || { fail "Back did not restore the rendered document"; return; }
    if grep -q 'text="Show document"' "$ui"; then
        fail "Back left the source inspector open"
        return
    fi
    pass "Back restored the document and closed the exact source panel"

    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    if D shell am start -W -n "$ACTIVITY" \
            --es emthin.fixtureB64 "$(printf '%s' "$owner" | base64 -w0)" \
            --es emthin.fixtureModifier '3,4' \
            --es emthin.fixtureTargetsB64 "$(printf '%s' '1 2' | base64 -w0)" \
            --es emthin.fixtureMarker '3' >/dev/null 2>&1; then
        pass "launched the modifier-owner fixture"
    else
        fail "launch the modifier-owner fixture"
        return
    fi
    wait_for_log "$log" 'fixture modifier 3,4 targets 1 2' \
        || { fail "owner fixture link was not applied"; return; }
    wait_for_log "$log" 'diagnostics .*count 2 errors 2 warnings 0' \
        || { fail "owner fixture did not publish two errors"; return; }
    sleep 0.8
    cmd_uidump "$ui" || return
    b=$(node_bounds 'text="2 diagnostics') \
        || { fail "diagnostic count entry point is missing"; return; }
    tap_center "$b"
    sleep 0.6
    cmd_uidump "$ui" || return
    grep -q 'ERROR · typst · owner · #3 #4' "$ui" \
        || { fail "owner-attributed Typst row is missing"; return; }
    grep -q 'ERROR · modifier-application · owner · #3 #4' "$ui" \
        || { fail "modifier-application owner row is missing"; return; }
    pass "list marks both failures with the #3 #4 modifier owner"

    b=$(node_bounds 'Open diagnostic source: modifier-application') \
        || { fail "modifier-application row is not clickable"; return; }
    tap_center "$b"
    wait_for_log "$log" 'navigate kind diagnostic start 15' \
        || { fail "owner diagnostic did not navigate to source offset 15"; return; }
    sleep 0.4
    cmd_uidump "$ui" || return
    grep -q 'text="Show document"' "$ui" \
        || { fail "owner diagnostic did not open the source inspector"; return; }
    pass "owner diagnostic opened source at the modifier pair"

    tap_toolbar_menu Navigation Back \
        || { fail "Back action is missing after owner navigation"; return; }
    sleep 0.8
    cmd_uidump "$ui" || return
    grep -q 'content-desc="Target marker pairs"' "$ui" \
        || { fail "Back did not restore the modifier-at-caret inspector"; return; }
    grep -q 'text="1 2"' "$ui" \
        || { fail "Back lost the modifier's target list"; return; }
    pass "Back restored the modifier inspector and its target list"
}

# A9: open a real named source file through the fixture hook and prove both
# the bytes and the visible document name reach the status line.
cmd_named_document() {
    local log="$OUTDIR/named-document-logcat.txt"
    local ui="$OUTDIR/named-document-ui.xml"
    local file="$OUTDIR/named-source.txt"
    local remote=/data/local/tmp/named-source.txt
    local expected

    say "==> named document: open source -> status shows the file name"
    mkdir -p "$OUTDIR"
    printf 'Named source fixture\n' >"$file"
    expected=$(wc -c <"$file" | tr -d ' ')
    if D push "$file" "$remote" >/dev/null 2>&1; then
        D shell chmod 644 "$remote" >/dev/null 2>&1
        pass "pushed the named source fixture"
    else
        fail "push the named source fixture"
        return
    fi
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    if D shell am start -W -n "$ACTIVITY" --es emthin.fixtureOpenPath "$remote" >/dev/null 2>&1; then
        pass "opened the named source fixture"
    else
        fail "open the named source fixture"
        return
    fi
    wait_for_log "$log" "applyState .*textLen $expected" \
        || { fail "named source bytes did not reach the model"; return; }
    sleep 0.6
    cmd_uidump "$ui" || return
    grep -Fq 'text="named-source.txt · unsaved ·' "$ui" \
        || { fail "status line does not name named-source.txt"; return; }
    pass "status names the opened document (named-source.txt)"
}

# A9: Back first dismisses an overlay IME, then closes the overlay; Escape
# closes the source inspector; dialog Back cancels without leaving the pad.
cmd_focus_stack() {
    local log="$OUTDIR/focus-stack-logcat.txt"
    local ui="$OUTDIR/focus-stack-ui.xml"
    local exact b

    say "==> focus stack: Find IME -> Back, dialog -> Back, inspector -> Esc"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    cmd_launch || return
    wait_for_log "$log" 'applyChunk total 2 pages [12]' \
        || { fail "starter did not render for the focus test"; return; }

    cmd_uidump "$ui" || return
    b=$(node_bounds 'text="Find"') \
        || { fail "Find is missing from the toolbar"; return; }
    tap_center "$b"
    sleep 0.5
    cmd_uidump "$ui" || return
    b=$(node_bounds 'text="Replace…"') \
        || { fail "Replace row action is missing"; return; }
    tap_center "$b"
    sleep 0.4
    cmd_uidump "$ui" || return
    b=$(node_bounds 'content-desc="Replacement text"') \
        || { fail "replacement field is missing"; return; }
    tap_center "$b"
    sleep 0.3
    D shell input text x >/dev/null 2>&1
    sleep 0.6
    if keyboard_shown; then
        pass "replacement field opened the IME"
    else
        fail "replacement field did not open the IME"
        return
    fi

    # Waydroid's IME consumes Back before the activity while the soft
    # keyboard owns focus, so prove the overlay Escape path here. Dialog
    # Back (no IME) and source-inspector Escape are checked below.
    # The first synthetic Escape can be consumed by the IME window; retry a
    # bounded number of times so the app-level handler is what closes Find.
    for i in 1 2 3; do
        D shell input keyevent KEYCODE_ESCAPE >/dev/null 2>&1
        sleep 0.9
        cmd_uidump "$ui" || return
        grep -q 'content-desc="Find in document"' "$ui" || break
    done
    if grep -q 'content-desc="Find in document"' "$ui"; then
        fail "Escape did not cancel Find while the IME was open"
        return
    fi
    if keyboard_shown; then
        fail "closing Find left the IME visible"
        return
    fi
    pass "Escape cancelled Find and dismissed the IME"

    cmd_uidump "$ui" || return
    tap_toolbar_menu Document New \
        || { fail "New is missing from the toolbar"; return; }
    sleep 0.5
    cmd_uidump "$ui" || return
    grep -q 'text="New document"' "$ui" \
        || { fail "New confirmation did not open"; return; }
    D shell input keyevent KEYCODE_BACK >/dev/null 2>&1
    sleep 0.5
    cmd_uidump "$ui" || return
    if grep -q 'text="New document"' "$ui"; then
        fail "Back did not cancel the New confirmation"
        return
    fi
    pass "Back cancelled the dialog and returned to the pad"

    exact=$(printf '%s\n' '#1 text #2 \app(#1, #2, 10, 10)' \
        '#3 thing #4 \prob(#3, #4)' '' 'Unresolved $z$.')
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    D shell am start -W -n "$ACTIVITY" \
        --es emthin.fixtureB64 "$(printf '%s' "$exact" | base64 -w0)" >/dev/null 2>&1
    wait_for_log "$log" 'diagnostics .*count 1 errors 0 warnings 1 notes 1' \
        || { fail "focus fixture did not publish diagnostics"; return; }
    sleep 0.8
    cmd_uidump "$ui" || return
    b=$(node_bounds 'text="1 diagnostic') \
        || { fail "diagnostic banner is missing"; return; }
    tap_center "$b"
    sleep 0.6
    cmd_uidump "$ui" || return
    b=$(node_bounds 'Open diagnostic source: unresolved-name') \
        || { fail "unresolved-name row is missing"; return; }
    tap_center "$b"
    wait_for_log "$log" 'navigate kind diagnostic start 71' \
        || { fail "diagnostic source did not open"; return; }
    sleep 0.4
    cmd_uidump "$ui" || return
    grep -q 'text="Show document"' "$ui" \
        || { fail "source inspector did not open for Escape"; return; }
    D shell input keyevent KEYCODE_ESCAPE >/dev/null 2>&1
    sleep 0.5
    cmd_uidump "$ui" || return
    if grep -q 'text="Show document"' "$ui"; then
        fail "Escape did not close the source inspector"
        return
    fi
    grep -q 'content-desc="Page 1 of 1"' "$ui" \
        || { fail "Escape did not return focus to the rendered document"; return; }
    pass "Escape closed the source inspector and restored the document"
}

# A14/A16: drive the app's real InputConnection path with a test-only IME in
# the disposable APK. CJK and emoji go through setComposingText/commitText
# instead of pasting text or synthesizing key events.
cmd_composing_ime() {
    local test_ime="$PKG/com.emthin.pad.TestComposingIme"
    local enabled_was default_was rc

    if ! D shell ime list -a -s 2>/dev/null | grep -qF "$test_ime"; then
        fail "$PKG does not expose TestComposingIme; rebuild the test APK"
        return 1
    fi
    enabled_was=$(D shell settings get secure enabled_input_methods 2>/dev/null | tr -d '\r')
    default_was=$(D shell settings get secure default_input_method 2>/dev/null | tr -d '\r')
    say "    selected the test composing IME (restored afterwards)"

    _composing_ime_check
    rc=$?

    D shell ime disable "$test_ime" >/dev/null 2>&1
    _restore_secure_setting enabled_input_methods "$enabled_was"
    if [ -n "$default_was" ] && [ "$default_was" != null ]; then
        D shell ime set "$default_was" >/dev/null 2>&1 || true
    else
        _restore_secure_setting default_input_method "$default_was"
    fi
    D shell am force-stop "$PKG" >/dev/null 2>&1 || true
    return "$rc"
}

_ime_action() {
    local action="$1" text="${2:-}"
    if [ -n "$text" ]; then
        D shell content call --uri content://com.emthin.pad.ime \
            --method "$action" --arg "$text" >/dev/null 2>&1
    else
        D shell content call --uri content://com.emthin.pad.ime \
            --method "$action" >/dev/null 2>&1
    fi
}

_wait_ime_log() {
    local out="$1" pattern="$2" tries=0
    while [ "$tries" -lt 20 ]; do
        D logcat -d -s emthin-test-ime:I 2>/dev/null | tr -d '\r' >"$out"
        grep -qF "$pattern" "$out" && return 0
        sleep 0.2
        tries=$((tries + 1))
    done
    return 1
}

_ime_field_node() {
    grep -o '<node[^>]*content-desc="Editable text without modifiers"[^>]*>' \
        "$1" | head -1
}

_composing_ime_check() {
    local log="$OUTDIR/composing-ime-logcat.txt"
    local ime_log="$OUTDIR/composing-ime-service-logcat.txt"
    local ui="$OUTDIR/composing-ime-ui.xml"
    local test_ime="$PKG/com.emthin.pad.TestComposingIme"
    local b field i

    say "==> composing IME: CJK composition -> commit -> emoji commit"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    cmd_launch || return
    wait_for_log "$log" 'applyChunk total 2 pages [12]' \
        || { fail "starter did not render for the composing IME check"; return; }

    # Selecting the IME before pm clear is not stable: clearing the app can
    # make the system fall back to the previous keyboard. Select it after the
    # cold launch, before the source field asks for an input connection.
    D shell ime enable "$test_ime" >/dev/null 2>&1
    D shell ime set "$test_ime" >/dev/null 2>&1
    for i in $(seq 1 10); do
        D shell dumpsys input_method 2>/dev/null | tr -d '\r' \
            | grep -qF "mCurMethodId=$test_ime" && break
        sleep 0.2
    done
    if ! D shell dumpsys input_method 2>/dev/null | tr -d '\r' \
            | grep -qF "mCurMethodId=$test_ime"; then
        fail "test composing IME did not become the active input method"
        return
    fi

    cmd_uidump "$ui" || return
    b=$(node_bounds 'text="terminal"') \
        || { fail "starter text is missing from the page"; return; }
    tap_center "$b"
    sleep 0.5
    cmd_uidump "$ui" || return
    b=$(node_bounds 'content-desc="Editable text without modifiers"') \
        || { fail "source field did not open"; return; }
    tap_center "$b"
    sleep 0.5
    D shell input keyevent KEYCODE_MOVE_END >/dev/null 2>&1
    if ! D shell dumpsys input_method 2>/dev/null | tr -d '\r' \
            | grep -qF "mCurMethodId=$PKG/com.emthin.pad.TestComposingIme"; then
        fail "test composing IME is not the active input method"
        return
    fi
    pass "source field is focused through the test composing IME"

    D logcat -c >/dev/null 2>&1 || true
    _ime_action compose 你好
    _wait_ime_log "$ime_log" 'compose 你好' \
        || { fail "test IME did not call setComposingText"; return; }
    sleep 0.3
    cmd_uidump "$ui" || return
    field=$(_ime_field_node "$ui")
    printf '%s' "$field" | grep -qF '你好' \
        || { fail "CJK composing text did not reach the source field"; return; }
    pass "CJK composing text reached the source field"

    D logcat -c >/dev/null 2>&1 || true
    _ime_action finish
    _wait_ime_log "$ime_log" 'finish ' \
        || { fail "test IME did not finish CJK composition"; return; }
    wait_for_log "$log" 'applyState .*textLen' \
        || { fail "finished CJK composition did not reach the model"; return; }
    pass "finished CJK composition committed to the model"

    # The render after a commit rebuilds the inspector; focus the field again
    # before the next input connection action.
    cmd_uidump "$ui" || return
    b=$(node_bounds 'content-desc="Editable text without modifiers"') \
        || { fail "source field disappeared after CJK commit"; return; }
    tap_center "$b"
    sleep 0.3
    D shell input keyevent KEYCODE_MOVE_END >/dev/null 2>&1

    D logcat -c >/dev/null 2>&1 || true
    _ime_action commit 😀
    _wait_ime_log "$ime_log" 'commit 😀' \
        || { fail "test IME did not commit emoji text"; return; }
    wait_for_log "$log" 'applyState .*textLen' \
        || { fail "emoji commit did not reach the model"; return; }
    sleep 0.3
    cmd_uidump "$ui" || return
    field=$(_ime_field_node "$ui")
    printf '%s' "$field" | grep -qF '&#128512;' \
        || { fail "emoji did not reach the source field"; return; }
    pass "emoji commit reached the source field and model"
}

# Restore a global/secure setting exactly, including the null default.
_restore_global_setting() {
    local key="$1" value="$2"
    if [ -z "$value" ] || [ "$value" = null ]; then
        D shell settings delete global "$key" >/dev/null 2>&1 || true
    else
        D shell settings put global "$key" "$value" >/dev/null 2>&1 || true
    fi
}

_restore_secure_setting() {
    local key="$1" value="$2"
    if [ -z "$value" ] || [ "$value" = null ]; then
        D shell settings delete secure "$key" >/dev/null 2>&1 || true
    else
        D shell settings put secure "$key" "$value" >/dev/null 2>&1 || true
    fi
}

# Play Protect can block an adb-side-load indefinitely when the image has no
# working verification backend. Pause the two verifier settings only for the
# install and restore them exactly afterwards.
_install_talkback() {
    local apk="$1" pv vv rc
    if [ ! -r "$apk" ]; then
        fail "TALKBACK_APK is not readable: $apk"
        return 1
    fi
    mkdir -p "$OUTDIR"
    pv=$(D shell settings get global package_verifier_enable 2>/dev/null | tr -d '\r')
    vv=$(D shell settings get global verifier_verify_adb_installs 2>/dev/null | tr -d '\r')
    D shell settings put global package_verifier_enable 0 >/dev/null 2>&1 || true
    D shell settings put global verifier_verify_adb_installs 0 >/dev/null 2>&1 || true
    say "    installing TalkBack from $apk (package verification paused for install)"
    if timeout 180 "$ADB" -s "$DEVICE" install --no-streaming -r "$apk" \
            >"$OUTDIR/talkback-install.txt" 2>&1; then
        rc=0
        pass "installed TalkBack for the accessibility check"
    else
        rc=1
        say "    $(tail -2 "$OUTDIR/talkback-install.txt" | tr '\n' ' ')"
        fail "install TalkBack from $apk"
    fi
    _restore_global_setting package_verifier_enable "$pv"
    _restore_global_setting verifier_verify_adb_installs "$vv"
    return "$rc"
}

# When a TalkBack build is installed, require a real bound service, prove the
# pad is a client, and capture the page utterance. If TalkBack is absent, use
# TALKBACK_APK when supplied; otherwise skip explicitly rather than claim
# coverage or fail an otherwise unrelated e2e run. Enable it only for this
# check and restore the prior package/settings state afterwards so synthetic-
# tap steps still run with it paused.
cmd_accessibility() {
    local tb='com.android.talkback/com.google.android.marvin.talkback.TalkBackService'
    local apk="${TALKBACK_APK:-}"
    local a11y_was services_was touch_was services_new rc
    local installed_here=0 bound=0 i

    if ! D shell pm path com.android.talkback 2>/dev/null | grep -q '^package:'; then
        if [ -z "$apk" ]; then
            say "info accessibility: TalkBack is not installed; set TALKBACK_APK to install it, check skipped"
            return 0
        fi
        if [ ! -r "$apk" ]; then
            fail "TALKBACK_APK is not readable: $apk"
            return 1
        fi
        _install_talkback "$apk" || return 1
        installed_here=1
    fi

    a11y_was=$(D shell settings get secure accessibility_enabled 2>/dev/null | tr -d '\r')
    services_was=$(D shell settings get secure enabled_accessibility_services 2>/dev/null | tr -d '\r')
    touch_was=$(D shell settings get secure touch_exploration_enabled 2>/dev/null | tr -d '\r')
    [ "$services_was" = "null" ] && services_was=""
    [ "$touch_was" = "null" ] && touch_was=""

    # Track the TTS engine separately: leave the first-run tutorial with TTS
    # working, then pause it only for the utterance-log capture.
    TTS_INSTALLED=0
    TTS_WAS_DISABLED=0
    TTS_DISABLED_HERE=0
    if D shell pm path com.google.android.tts 2>/dev/null | grep -q '^package:'; then
        TTS_INSTALLED=1
        D shell pm list packages -d 2>/dev/null | tr -d '\r' \
            | grep -qx 'package:com.google.android.tts' && TTS_WAS_DISABLED=1
    fi

    D shell settings put secure accessibility_enabled 0 >/dev/null 2>&1
    D shell am force-stop com.android.talkback >/dev/null 2>&1
    case ",$services_was," in
        *",$tb,"*) services_new="$services_was" ;;
        *) services_new="${services_was:+$services_was,}$tb" ;;
    esac
    D shell settings put secure enabled_accessibility_services "$services_new" >/dev/null 2>&1
    D shell settings put secure accessibility_enabled 1 >/dev/null 2>&1
    say "    enabled TalkBack for the accessibility check (restored afterwards)"

    # Bind before launching the fixture: if the app starts while TalkBack is
    # still coming up, the initial page announcement can be missed entirely.
    for i in $(seq 1 20); do
        D shell dumpsys accessibility 2>/dev/null | tr -d '\r' \
            | grep -qF 'Bound services:{Service[label=TalkBack' && bound=1 && break
        sleep 0.3
    done

    if [ "$bound" != 1 ]; then
        fail "TalkBack did not bind before the accessibility fixture launch"
        _restore_secure_setting enabled_accessibility_services "$services_was"
        _restore_secure_setting touch_exploration_enabled "$touch_was"
        _restore_secure_setting accessibility_enabled "$a11y_was"
        if [ "$TTS_DISABLED_HERE" = 1 ]; then
            D shell pm enable --user 0 com.google.android.tts >/dev/null 2>&1 || true
        fi
        if [ "$installed_here" = 1 ]; then
            D shell pm uninstall com.android.talkback >/dev/null 2>&1 || true
        fi
        return 1
    fi

    _accessibility_check "$services_new"
    rc=$?

    D shell settings put secure accessibility_enabled 0 >/dev/null 2>&1
    D shell am force-stop com.android.talkback >/dev/null 2>&1
    _restore_secure_setting enabled_accessibility_services "$services_was"
    _restore_secure_setting touch_exploration_enabled "$touch_was"
    _restore_secure_setting accessibility_enabled "$a11y_was"
    if [ "$TTS_DISABLED_HERE" = 1 ]; then
        D shell pm enable --user 0 com.google.android.tts >/dev/null 2>&1 || true
    fi
    if [ "$installed_here" = 1 ]; then
        if D shell pm uninstall com.android.talkback >/dev/null 2>&1; then
            pass "removed the TalkBack install used for the check"
        else
            fail "uninstall TalkBack after the accessibility check"
            rc=1
        fi
    fi
    return "$rc"
}

# Fresh TalkBack installs open a six-page tutorial that overlays the pad.
# Dismiss it before launching the fixture; otherwise the page node and its
# initial announcement are hidden behind the tutorial window.
_dismiss_talkback_tutorial() {
    local ui="$OUTDIR/talkback-tutorial-ui.xml" b i
    for i in $(seq 1 20); do
        D shell uiautomator dump "$UI_DUMP_REMOTE" >/dev/null 2>&1 || { sleep 0.4; continue; }
        D exec-out cat "$UI_DUMP_REMOTE" >"$ui" 2>/dev/null || { sleep 0.4; continue; }
        tr -d '\r' <"$ui" >"$ui.tmp" && mv "$ui.tmp" "$ui"
        UI_XML="$ui"
        if grep -qF 'text="TalkBack is off"' "$ui"; then
            # TrainingActivity can open before TalkBack has finished enabling.
            # Continue into the tutorial; the next screen offers Finish.
            b=$(node_bounds 'text="Continue anyways"') || { sleep 0.4; continue; }
            tap_center "$b"
            sleep 0.8
            continue
        fi
        if grep -qF 'text="Welcome to TalkBack"' "$ui"; then
            # TalkBack consumes synthetic taps; keyboard focus + Enter reliably
            # activates the tutorial's Finish button.
            D shell input keyevent KEYCODE_TAB >/dev/null 2>&1
            sleep 0.15
            D shell input keyevent KEYCODE_TAB >/dev/null 2>&1
            sleep 0.15
            D shell input keyevent KEYCODE_ENTER >/dev/null 2>&1
            sleep 0.8
            say "    dismissed the TalkBack first-run tutorial"
            return 0
        fi
        sleep 0.4
    done
    return 1
}

_accessibility_check() {
    local desired_services="$1"
    local log="$OUTDIR/accessibility-logcat.txt"
    local speech="$OUTDIR/accessibility-speech-logcat.txt"
    local dump="$OUTDIR/accessibility-dumpsys.txt"
    local ui="$OUTDIR/accessibility-ui.xml"
    local tb='com.android.talkback/com.google.android.marvin.talkback.TalkBackService'
    local fixture_b64 a11y services i bound=0 client=0 spoken=0

    say "==> accessibility: TalkBack bound, page spoken, pad registered as a client"
    mkdir -p "$OUTDIR"
    fixture_b64=$(printf '%s' '#1 Spoken reading #2' | base64 -w0)
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    if D shell am start -W -n "$ACTIVITY" \
            --es emthin.fixtureB64 "$fixture_b64" >/dev/null 2>&1; then
        pass "launched the pad for the accessibility check"
    else
        fail "launch the pad for the accessibility check"
        return
    fi
    wait_for_log "$log" 'applyChunk' \
        || { fail "pad did not render before the accessibility check"; return; }

    # The first-run tutorial can open after the activity starts. If it did,
    # relaunch the fixture after dismissing it so TalkBack gets a fresh
    # window/page announcement to speak.
    if _dismiss_talkback_tutorial; then
        # The tutorial's window must be gone before the service will rebind.
        for i in $(seq 1 15); do
            D shell dumpsys window 2>/dev/null | tr -d '\r' \
                | grep -q 'mCurrentFocus=.*TrainingActivity' || break
            sleep 0.3
        done
        # Force-stopping the app also tears down the tutorial task; a service
        # that is merely toggled while TrainingActivity lives will not rebind.
        D shell am force-stop com.android.talkback >/dev/null 2>&1
        sleep 0.5
        # Now that the tutorial is past, a missing TTS engine makes TalkBack
        # log the utterance instead of waiting on audio.
        if [ "$TTS_INSTALLED" = 1 ] && [ "$TTS_WAS_DISABLED" = 0 ]; then
            if D shell pm disable-user --user 0 com.google.android.tts >/dev/null 2>&1; then
                TTS_DISABLED_HERE=1
                say "    paused TTS for the utterance log (restored afterwards)"
            fi
        fi
        D shell settings put secure enabled_accessibility_services "$desired_services" >/dev/null 2>&1
        D shell settings put secure accessibility_enabled 0 >/dev/null 2>&1
        sleep 1
        D shell settings put secure accessibility_enabled 1 >/dev/null 2>&1
        bound=0
        for i in $(seq 1 30); do
            D shell dumpsys accessibility 2>/dev/null | tr -d '\r' \
                | grep -qF 'Bound services:{Service[label=TalkBack' && bound=1 && break
            sleep 0.3
        done
        # Let the restarted service finish initializing before the relaunch,
        # then clear the log so the assertion only sees the page utterance.
        sleep 2
        D logcat -c >/dev/null 2>&1 || true
        D shell am force-stop "$PKG" >/dev/null 2>&1
        D shell am start -W -n "$ACTIVITY" \
            --es emthin.fixtureB64 "$fixture_b64" >/dev/null 2>&1 \
            || { fail "relaunch the pad after the TalkBack tutorial"; return; }
        wait_for_log "$log" 'applyChunk' \
            || { fail "pad did not render after the TalkBack tutorial"; return; }
    fi

    a11y=$(D shell settings get secure accessibility_enabled 2>/dev/null | tr -d '\r')
    services=$(D shell settings get secure enabled_accessibility_services 2>/dev/null | tr -d '\r')
    if [ "$a11y" != "1" ]; then
        fail "accessibility_enabled is '$a11y', expected 1"
        return
    fi
    if ! printf '%s' "$services" | grep -qF "$tb"; then
        fail "TalkBack is not in enabled_accessibility_services"
        return
    fi
    pass "TalkBack is enabled in secure settings"

    # Binding and client registration happen after the activity starts.
    for i in $(seq 1 12); do
        D shell dumpsys accessibility >"$dump" 2>/dev/null || true
        grep -qF 'Bound services:{Service[label=TalkBack' "$dump" && bound=1
        grep -qF "[$PKG]" "$dump" && client=1
        [ "$bound" = 1 ] && [ "$client" = 1 ] && break
        sleep 0.5
    done
    if [ "$bound" != 1 ]; then
        say "    $(grep -F 'Bound services:' "$dump" | head -1)"
        fail "TalkBack is not bound"
        return
    fi
    pass "TalkBack is bound"
    if [ "$client" != 1 ]; then
        fail "$PKG is not in the accessibility client list"
        return
    fi
    pass "$PKG is registered as an accessibility client"

    cmd_uidump "$ui" || return
    grep -qF 'content-desc="Page 1 of 1"' "$ui" \
        || { fail "rendered page is not exposed as an accessibility node"; return; }
    grep -qF 'text="Spoken reading"' "$ui" \
        || { fail "rendered page text is not exposed as an accessibility node"; return; }
    pass "rendered page and text are exposed to TalkBack"

    # TTS is paused for this check, so TalkBack logs the utterance it would
    # speak. Wait for the page caption it announces on app launch.
    for i in $(seq 1 25); do
        D logcat -d -v brief >"$speech" 2>/dev/null || true
        if grep -qF 'fragments:[{text:Page 1 of 1' "$speech"; then
            spoken=1
            break
        fi
        sleep 0.3
    done
    if [ "$spoken" != 1 ]; then
        say "    spoken: $(grep -o 'fragments:.*' "$speech" 2>/dev/null | tail -2 | tr '\n' ' ')"
        fail "TalkBack did not attempt to speak the rendered page"
        return
    fi
    pass "TalkBack spoke the rendered page caption (Page 1 of 1)"
}

# Deterministic A14/A15 inspector fixture. The intent extras are the only
# supported way to supply it: no IME typing or shifting coordinates.
cmd_modifier_inspector() {
    local a11y_was rc
    # TalkBack consumes synthetic taps for focus, so pause it for this
    # deterministic interaction lane and restore the user's setting afterwards.
    a11y_was=$(D shell settings get secure accessibility_enabled 2>/dev/null | tr -d '\r')
    if [ "$a11y_was" = "1" ]; then
        D shell settings put secure accessibility_enabled 0 >/dev/null 2>&1
        D shell am force-stop com.android.talkback >/dev/null 2>&1
        say "    paused TalkBack for deterministic taps (restored afterwards)"
        sleep 1
    fi
    _modifier_inspector
    rc=$?
    if [ "$a11y_was" = "1" ]; then
        D shell settings put secure enabled_accessibility_services \
            com.android.talkback/com.google.android.marvin.talkback.TalkBackService >/dev/null 2>&1
        D shell settings put secure accessibility_enabled 1 >/dev/null 2>&1
    fi
    return "$rc"
}

_modifier_inspector() {
    local log="$OUTDIR/modifier-inspector-logcat.txt"
    local ui="$OUTDIR/modifier-inspector-ui.xml"
    local shot b expanded=0 i
    local fixture='#1 terminal #2 #3 editor #4 #5 bold #6 #7 italic #8'
    local fixture_b64 targets_b64 targets2_b64
    fixture_b64=$(printf '%s' "$fixture" | base64 -w0)
    targets_b64=$(printf '%s' '1 2, 3 4' | base64 -w0)
    targets2_b64=$(printf '%s' '5 6' | base64 -w0)

    say "==> modifier inspector: fixture -> rows -> expanded comparison"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    if D shell am start -W -n "$ACTIVITY" \
            --es emthin.fixtureB64 "$fixture_b64" \
            --es emthin.fixtureModifier '5,6' \
            --es emthin.fixtureTargetsB64 "$targets_b64" \
            --es emthin.fixtureModifier2 '7,8' \
            --es emthin.fixtureTargets2B64 "$targets2_b64" \
            --es emthin.fixtureMarker '5' >/dev/null 2>&1; then
        pass "launched the modifier-pair fixture"
    else
        fail "launch the modifier-pair fixture"
        return
    fi
    wait_for_log "$log" 'fixture modifier 5,6 targets 1 2, 3 4' \
        || { fail "fixture link was not applied"; return; }
    wait_for_log "$log" 'applyChunk' \
        || { fail "fixture document did not render"; return; }
    sleep 1.0

    cmd_uidump "$ui" || return
    grep -q 'content-desc="Target marker pairs"' "$ui" \
        || { fail "modifier inspector did not open on the fixture"; return; }
    grep -q 'text="1 2, 3 4"' "$ui" \
        || { fail "fixture target list is not shown verbatim"; return; }
    grep -q '#5 #6' "$ui" \
        || { fail "incoming relationship for #5/#6 is missing"; return; }

    # The relationship list is its own small scroll viewport. On a short
    # landscape tablet even the two outgoing rows do not fit in it at once,
    # so collect each row over successive scroll positions instead of
    # requiring one dump to contain both.
    #
    # The swipe has to land *inside* that viewport, and its y range depends on
    # the pane layout rather than the screen size: a fixed y=850 was below the
    # list's 604..826 box on a 2943x1638 tablet, so the drag missed the list and
    # scrolled the document instead, and both rows stayed unreachable. Read the
    # scrollable box around "Outgoing targets" from the dump and aim at it.
    local saw12=0 saw34=0
    for i in $(seq 1 8); do
        cmd_uidump "$ui" || return
        grep -q '#1 #2' "$ui" && saw12=1
        grep -q '#3 #4' "$ui" && saw34=1
        [ "$saw12" = 1 ] && [ "$saw34" = 1 ] && break
        relationship_scroll "$ui" up
        sleep 0.4
    done
    [ "$saw12" = 1 ] \
        || { fail "outgoing target relationship for #1/#2 is missing"; return; }
    [ "$saw34" = 1 ] \
        || { fail "outgoing target relationship for #3/#4 is missing"; return; }
    pass "fixture shows an incoming modifier and both outgoing targets"

    # Scroll back to the top of the relationship list before tapping the
    # incoming heading. A downward swipe inside the small viewport is the
    # only reliable way to return there without touching the fields above.
    for i in $(seq 1 12); do
        cmd_uidump "$ui" || return
        b=$(node_bounds '#5 #6') && break
        b=
        # Scroll back toward the top of the relationship viewport rather than
        # at a fixed screen y, which misses the pane on a taller device.
        relationship_scroll "$ui" down
        sleep 0.35
    done
    [ -n "$b" ] || { fail "incoming relationship heading not tappable"; return; }
    tap_center "$b"
    sleep 1.2
    # The preview renders below the sheet's fold. The exact number of swipes
    # depends on system-bar and sheet height, so scroll and collect both
    # labels and the rendered comparison; they need not share one viewport.
    local saw_with=0 saw_without=0 saw_rendered=0
    for i in $(seq 1 12); do
        cmd_uidump "$ui" || return
        grep -q 'text="With modifier"' "$ui" && saw_with=1
        grep -q 'text="Without modifier"' "$ui" && saw_without=1
        grep -q 'content-desc="Rendered comparison"' "$ui" && saw_rendered=1
        if [ "$saw_with" = 1 ] && [ "$saw_without" = 1 ] && [ "$saw_rendered" = 1 ]; then
            expanded=1
            break
        fi
        # Stay inside the relationship viewport: a swipe starting near the
        # source field scrolls the panel instead of the comparison list. Its
        # y range comes from the dump for the same reason.
        relationship_scroll "$ui" up
        sleep 0.5
    done
    if [ "$expanded" != 1 ]; then
        [ "$saw_with" = 1 ] \
            || { fail "expanded row is missing the With modifier preview"; return; }
        [ "$saw_without" = 1 ] \
            || { fail "expanded row is missing the Without modifier preview"; return; }
        fail "expanded row is missing the rendered comparison"
        return
    fi
    pass "one comparison row expanded with both rendered sides"

    if shot=$(cmd_screencap "$OUTDIR/modifier-inspector-after.png"); then
        say "    captured $shot"
    fi
}

# Dump the app's debug log (cold-start proof: JIT + render chunks).
cmd_logcat() {
    local out="${1:-$OUTDIR/logcat.txt}" raw
    mkdir -p "$(dirname "$out")"
    raw=$(mktemp "$OUTDIR/.logcat.XXXXXX" 2>/dev/null || mktemp)
    if ! D logcat -d -s emthin-pad:D >"$raw" 2>/dev/null; then
        rm -f "$raw"
        fail "logcat -d -s emthin-pad:D"
        return 1
    fi
    tr -d '\r' <"$raw" >"$out"
    rm -f "$raw"
    if ! grep -q 'emthin-pad' "$out"; then
        fail "no emthin-pad lines in $out"
        return 1
    fi
    pass "logcat -> $out"
}

# T05: pin a public terminal reference through the real Sessions action,
# prove the figure/Undo/Redo contract, then reopen and prove autosave kept it.
cmd_session_pin() {
    local log="$OUTDIR/session-pin-logcat.txt"
    local fixture shot

    say "==> session pin: pin -> Undo -> Redo -> reopen"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    fixture=$(printf '%s\n' 'Session pin fixture.')
    D logcat -c >/dev/null 2>&1 || true
    if D shell am start -W -n "$ACTIVITY" \
            --ez emthin.fixtureSessionPin true \
            --es emthin.fixtureB64 "$(printf '%s' "$fixture" | base64 -w0)" \
            >/dev/null 2>&1; then
        pass "launched the disposable session-pin fixture"
    else
        fail "launch the disposable session-pin fixture"
        return
    fi
    wait_for_log "$log" 'sessionPin phase pinned' \
        || { fail "fixture did not pin a terminal figure"; return; }
    pass "fixture pinned the public terminal reference"
    wait_for_log "$log" 'sessionPin phase undo' \
        || { fail "Undo did not remove the pinned figure"; return; }
    pass "Undo removed the pinned figure"
    wait_for_log "$log" 'sessionPin phase complete' \
        || { fail "Redo did not restore the pinned figure"; return; }
    pass "Redo restored the pinned figure"

    # Reopen through the normal recovery/autosave path with no fixture extra.
    sleep 1.2
    D shell input keyevent KEYCODE_HOME >/dev/null 2>&1
    sleep 1.2
    D shell am force-stop "$PKG" >/dev/null 2>&1 || true
    sleep 0.3
    D logcat -c >/dev/null 2>&1 || true
    cmd_launch || return
    wait_for_log "$log" 'figureMetadata .*terminal 1' \
        || { fail "reopened document did not restore the pinned terminal figure"; return; }
    pass "reopened document carries one terminal figure"

    if shot=$(cmd_screencap "$OUTDIR/session-pin-after-reopen.png"); then
        say "    captured $shot"
    fi
}

# T08/T09: native VT scene geometry, animation, context loss, resize, keys.
cmd_terminal_scene() {
    local log="$OUTDIR/terminal-scene-logcat.txt"
    local fixture placed x y w h cx cy
    local test_ime="$PKG/com.emthin.pad.TestComposingIme"
    local enabled_was default_was

    say "==> terminal scene: placed rect -> animate -> context loss -> resize -> keys"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    fixture=$(printf '%s\n' \
        '#1 Build log #2 \app(#1, #2, 640, 400, terminal: "hc://0123456789abcdef0123456789abcdef/t7")')
    D logcat -c >/dev/null 2>&1 || true
    if D shell am start -W -n "$ACTIVITY" \
            --ez emthin.fixtureVtScene true \
            --ef emthin.fixtureViewZoom 1.0 \
            --es emthin.fixtureB64 "$(printf '%s' "$fixture" | base64 -w0)" \
            >/dev/null 2>&1; then
        pass "launched the disposable VT-scene fixture"
    else
        fail "launch the VT-scene fixture"
        return
    fi
    wait_for_log "$log" 'vtScene phase ready' \
        || { fail "native scene did not start"; return; }
    pass "native scene started in a terminal figure"
    wait_for_log "$log" 'vtScene gl frame page 0' \
        || { fail "native scene was not submitted through a GLES window surface"; return; }
    pass "scene submitted through a GLES window surface"

    wait_for_log "$log" 'vtScene phase placed' \
        || { fail "scene did not publish its placed rect"; return; }
    placed=$(sed -n 's/.*vtScene phase placed x \([-0-9]*\) y \([-0-9]*\) w \([0-9]*\) h \([0-9]*\).*/\1 \2 \3 \4/p' "$log" | tail -1)
    x=$(printf '%s' "$placed" | cut -d' ' -f1)
    y=$(printf '%s' "$placed" | cut -d' ' -f2)
    w=$(printf '%s' "$placed" | cut -d' ' -f3)
    h=$(printf '%s' "$placed" | cut -d' ' -f4)
    if [ -z "$x" ] || [ -z "$y" ] || [ -z "$w" ] || [ -z "$h" ]; then
        fail "could not parse the placed scene rect from $log"
        return
    fi
    if [ "$w" -le 0 ] || [ "$h" -le 0 ]; then
        fail "placed scene rect is empty ($placed)"
        return
    fi
    pass "scene reports a placed rect ($w x $h)"

    enabled_was=$(D shell settings get secure enabled_input_methods 2>/dev/null | tr -d '\r')
    default_was=$(D shell settings get secure default_input_method 2>/dev/null | tr -d '\r')
    D shell ime enable "$test_ime" >/dev/null 2>&1
    D shell ime set "$test_ime" >/dev/null 2>&1
    sleep 0.5

    wait_for_log "$log" 'vtScene phase frame 5' \
        || { fail "scene did not animate"; return; }
    pass "scene animated without a page re-render"
    wait_for_log "$log" 'vtScene phase context-loss' \
        || { fail "scene did not survive context loss"; return; }
    pass "scene survived renderer context loss"
    wait_for_log "$log" 'vtScene phase resize' \
        || { fail "scene did not reflow on resize"; return; }
    pass "scene reflowed after resize"

    cx=$((x + w / 2))
    cy=$((y + h / 2))
    D shell input tap "$cx" "$cy" >/dev/null 2>&1
    wait_for_log "$log" 'terminalInput phase focus' \
        || { fail "tapping the scene did not take terminal focus"; return; }
    pass "tap focused the terminal figure"

    # The fixture starts watched, like a session that has not taken control.
    # Input is blocked until the explicit Take action.
    if cmd_uidump >/dev/null 2>&1; then
        if b=$(node_bounds 'text="Take"'); then
            tap_center "$b"
            wait_for_log "$log" 'terminalInput control take page' \
                || { fail "Take did not grant terminal input"; return; }
            pass "Take granted terminal input"
        else
            fail "terminal key row has no Take action"
            return
        fi
    fi

    _ime_action compose 你好
    _ime_action finish
    wait_for_log "$log" 'terminalInput text 你好' \
        || { fail "CJK composition did not commit once to the terminal"; return; }
    pass "CJK composition committed once to the terminal"
    _ime_action commit 😀
    wait_for_log "$log" 'terminalInput text 😀' \
        || { fail "emoji commit did not reach the terminal"; return; }
    pass "emoji commit reached the terminal"

    D shell input keyevent KEYCODE_ENTER >/dev/null 2>&1
    wait_for_log "$log" 'terminalInput key 66 ghostty 58' \
        || { fail "Enter did not reach the VT encoder"; return; }
    pass "Enter was encoded as a terminal key"
    D shell input keyevent KEYCODE_DEL >/dev/null 2>&1
    wait_for_log "$log" 'terminalInput key 67 ghostty 53' \
        || { fail "Backspace did not reach the VT encoder"; return; }
    pass "Backspace was encoded as a terminal key"

    D shell input keyevent KEYCODE_ESCAPE >/dev/null 2>&1
    wait_for_log "$log" 'terminalInput key 111 ghostty 120' \
        || { fail "Escape did not reach the VT encoder"; return; }
    pass "Escape was encoded as a terminal key"
    D shell input keyevent KEYCODE_DPAD_LEFT >/dev/null 2>&1
    wait_for_log "$log" 'terminalInput key 21 ghostty 76' \
        || { fail "the left arrow did not reach the VT encoder"; return; }
    pass "arrow key was encoded as a terminal key"

    # Long-press a word, copy it through the terminal selection bar, then
    # clear logcat and prove a tap is delivered as a mouse report when the
    # program enabled tracking.
    # Use the same on-screen centre as the focus tap: the page can be
    # wider than a rotated viewport, so the figure's top-left corner is
    # not necessarily reachable.
    D shell input swipe "$cx" "$cy" "$cx" "$cy" 800 >/dev/null 2>&1
    wait_for_log "$log" 'terminalSelection start' \
        || { fail "long-press did not select a terminal word"; return; }
    pass "long-press selected a terminal word"
    if cmd_uidump >/dev/null 2>&1; then
        if b=$(node_bounds 'text="Copy"'); then
            tap_center "$b"
            wait_for_log "$log" 'terminalSelection copy' \
                || { fail "terminal Copy action did not run"; return; }
            pass "terminal selection copied to the clipboard"
        else
            fail "terminal selection bar has no Copy action"
            return
        fi
    fi

    wait_for_log "$log" 'vtScene phase mouse-mode' \
        || { fail "terminal fixture did not enable mouse tracking"; return; }
    D logcat -c >/dev/null 2>&1 || true
    D shell input tap "$cx" "$cy" >/dev/null 2>&1
    wait_for_log "$log" 'terminalInput mouse .*press true' \
        || { fail "tap was not sent as a mouse press"; return; }
    wait_for_log "$log" 'terminalInput mouse .*press false' \
        || { fail "tap was not sent as a mouse release"; return; }
    pass "mouse-mode tap reached the terminal"

    # Watch is a real read-only state: keystrokes are rejected at the input
    # boundary instead of being dropped later by a transport.
    if cmd_uidump >/dev/null 2>&1; then
        if b=$(node_bounds 'text="Watch"'); then
            tap_center "$b"
            wait_for_log "$log" 'terminalInput control watch page' \
                || { fail "Watch did not enter read-only mode"; return; }
            D logcat -c >/dev/null 2>&1 || true
            _ime_action commit 只读
            wait_for_log "$log" 'terminalInput blocked watch' \
                || { fail "watched terminal did not reject input"; return; }
            if grep -q 'terminalInput text 只读' "$log"; then
                fail "watched terminal accepted text"
                return
            fi
            pass "Watch blocked terminal input"
            if b=$(node_bounds 'text="Take"'); then
                tap_center "$b"
                wait_for_log "$log" 'terminalInput control take page' \
                    || { fail "Take did not restore terminal input"; return; }
            fi
        else
            fail "terminal key row has no Watch action"
            return
        fi
    fi

    # A composition that finishes after focus release must reach neither the
    # terminal nor the document.
    _ime_action compose 迟到
    D shell input keyevent KEYCODE_BACK >/dev/null 2>&1
    wait_for_log "$log" 'terminalInput phase release' \
        || { fail "Back did not release terminal focus"; return; }
    D logcat -c >/dev/null 2>&1 || true
    _ime_action finish
    sleep 0.8
    if grep -q 'terminalInput text 迟到' "$log"; then
        fail "late composition reached the terminal after release"
        return
    fi
    pass "Back released focus and a late commit was rejected"

    D shell ime disable "$test_ime" >/dev/null 2>&1
    _restore_secure_setting enabled_input_methods "$enabled_was"
    if [ -n "$default_was" ] && [ "$default_was" != null ]; then
        D shell ime set "$default_was" >/dev/null 2>&1 || true
    else
        _restore_secure_setting default_input_method "$default_was"
    fi

    cmd_screencap "$OUTDIR/terminal-scene.png" >/dev/null \
        && say "    captured $OUTDIR/terminal-scene.png"
}

# T12 fixture: real gateway frames over adb reverse, not an in-process flag.
LEASE_PORT=47654
LEASE_ADMIN_PORT=47655
LEASE_PID=""
LEASE_DIR=""

start_lease_fixture() {
    command -v python3 >/dev/null 2>&1 || {
        fail "python3 is required for the gateway lease fixture"
        return 1
    }
    LEASE_DIR=$(mktemp -d /tmp/emthin-lease.XXXXXX) || return 1
    python3 "$SCRIPT_DIR/../fixtures/herdr-gateway-v1/lease_fixture.py" \
        --listen "127.0.0.1:$LEASE_PORT" \
        --admin "127.0.0.1:$LEASE_ADMIN_PORT" \
        >"$LEASE_DIR/server.log" 2>&1 &
    LEASE_PID=$!
    local tries=0
    while [ "$tries" -lt 30 ]; do
        if grep -q 'lease fixture listening' "$LEASE_DIR/server.log" 2>/dev/null; then
            break
        fi
        if ! kill -0 "$LEASE_PID" 2>/dev/null; then
            say "    lease fixture exited: $(cat "$LEASE_DIR/server.log" 2>/dev/null)"
            fail "gateway lease fixture exited before it listened"
            return 1
        fi
        sleep 0.1
        tries=$((tries + 1))
    done
    if ! grep -q 'lease fixture listening' "$LEASE_DIR/server.log" 2>/dev/null; then
        fail "gateway lease fixture did not start"
        return 1
    fi
    if ! D reverse "tcp:$LEASE_PORT" "tcp:$LEASE_PORT" >/dev/null 2>&1; then
        fail "adb reverse for the gateway lease fixture"
        return 1
    fi
    return 0
}

stop_lease_fixture() {
    D reverse --remove "tcp:$LEASE_PORT" >/dev/null 2>&1 || true
    if [ -n "$LEASE_PID" ]; then
        kill "$LEASE_PID" >/dev/null 2>&1 || true
        wait "$LEASE_PID" 2>/dev/null || true
    fi
    LEASE_PID=""
    if [ -n "$LEASE_DIR" ] && [ -f "$LEASE_DIR/server.log" ]; then
        cp "$LEASE_DIR/server.log" "$OUTDIR/terminal-lease-server.txt" 2>/dev/null || true
    fi
    rm -rf "$LEASE_DIR"
    LEASE_DIR=""
}

lease_admin() {
    (exec 3<>/dev/tcp/127.0.0.1/$LEASE_ADMIN_PORT
     printf '%s\n' "$1" >&3
     cat <&3) 2>/dev/null
}

# Launch the fixture scene against the scripted gateway. Clearing the log here
# makes a retry start from a clean history.
lease_launch() {
    local fixture="$1"
    D logcat -c >/dev/null 2>&1 || true
    D shell am start -W -n "$ACTIVITY" \
        --ez emthin.fixtureVtScene true \
        --es emthin.fixtureGateway "127.0.0.1:$LEASE_PORT" \
        --es emthin.fixtureGatewayGeneration 'g-fixture-1' \
        --ef emthin.fixtureViewZoom 1.0 \
        --es emthin.fixtureB64 "$(printf '%s' "$fixture" | base64 -w0)" \
        >/dev/null 2>&1
}

# A fresh install hands the package a new uid. A stale uid whose packets the
# container drops (seen on 2026-11-23: the app's connect sat in SYN_SENT and the
# host listener saw nothing, while the shell uid and an ordinary app reached the
# same port) fails this step forever, and without root reinstalling is the only
# lever.
lease_reinstall() {
    D uninstall "$PKG" >/dev/null 2>&1 || true
    D install -r "$APK" >/dev/null 2>&1
}

terminal_lease_check() {
    local log="$1" fixture="$2" placed x y w h cx cy status
    if lease_launch "$fixture"; then
        pass "launched the disposable gateway-lease fixture"
    else
        fail "launch the gateway-lease fixture"
        return
    fi
    wait_for_log "$log" 'vtScene phase ready' \
        || { fail "lease fixture scene did not start"; return; }
    if ! wait_for_log "$log" 'gatewayLease phase ready mode watch'; then
        if ! grep -q 'gatewayLease error kind transport' "$log"; then
            fail "gateway watch attach did not become ready"
            return
        fi
        say "    info the app's connect never reached the fixture; reinstalling for a fresh uid"
        if ! lease_reinstall; then
            fail "reinstall the pad after a dead transport"
            return
        fi
        if ! lease_launch "$fixture"; then
            fail "relaunch the gateway-lease fixture after a fresh install"
            return
        fi
        wait_for_log "$log" 'vtScene phase ready' \
            || { fail "lease fixture scene did not start after a fresh install"; return; }
        wait_for_log "$log" 'gatewayLease phase ready mode watch' \
            || { fail "gateway watch attach still failed after a fresh install"; return; }
        pass "a fresh install restored the app's network egress"
    fi
    pass "watch attach became a gateway lease"
    wait_for_log "$log" 'gatewayLease resize cols 80 rows 24' \
        || { fail "gateway did not publish its authoritative grid"; return; }
    wait_for_log "$log" 'gatewayLease data .*gateway lease' \
        || { fail "gateway DATA did not reach the native scene"; return; }
    pass "gateway DATA reached the scene"

    wait_for_log "$log" 'vtScene phase placed' \
        || { fail "lease scene did not publish its placed rect"; return; }
    placed=$(sed -n 's/.*vtScene phase placed x \([-0-9]*\) y \([-0-9]*\) w \([0-9]*\) h \([0-9]*\).*/\1 \2 \3 \4/p' "$log" | tail -1)
    read -r x y w h <<EOF
$placed
EOF
    if [ -z "$x" ] || [ -z "$y" ] || [ -z "$w" ] || [ -z "$h" ]; then
        fail "could not parse the lease scene rect from $log"
        return
    fi
    cx=$((x + w / 2))
    cy=$((y + h / 2))
    D shell input tap "$cx" "$cy" >/dev/null 2>&1
    wait_for_log "$log" 'terminalInput phase focus' \
        || { fail "tapping the lease scene did not focus it"; return; }

    if ! cmd_uidump >/dev/null 2>&1 || ! b=$(node_bounds 'text="Take"'); then
        fail "lease fixture has no Take action"
        return
    fi
    tap_center "$b"
    wait_for_log "$log" 'gatewayLease phase request control' \
        || { fail "Take did not request a control attach"; return; }
    wait_for_log "$log" 'gatewayLease phase ready mode control' \
        || { fail "control attach did not become ready"; return; }
    pass "Take acquired a gateway controller lease"

    D shell input text 'gatewayinput' >/dev/null 2>&1
    wait_for_log "$log" 'gatewayLease input bytes' \
        || { fail "controller input did not cross the gateway DATA channel"; return; }
    wait_for_log "$log" 'gatewayLease data .*echo:' \
        || { fail "gateway did not echo the controller input"; return; }
    status=$(lease_admin status)
    if ! printf '%s' "$status" | grep -q 'gatewayinput'; then
        fail "gateway did not receive the complete controller input"
        return
    fi
    pass "controller input crossed the gateway and returned as DATA"

    lease_admin revoke >/dev/null 2>&1
    wait_for_log "$log" 'gatewayLease closed reason .*revoked' \
        || { fail "operator revoke did not close the viewer generation"; return; }
    pass "operator revoke closed the viewer generation"

    D logcat -c >/dev/null 2>&1 || true
    D shell input text 'after-revoke' >/dev/null 2>&1
    wait_for_log "$log" 'terminalInput blocked conflict' \
        || { fail "revoked controller still accepted input"; return; }
    status=$(lease_admin status)
    if printf '%s' "$status" | grep -q 'after-revoke'; then
        fail "revoked input reached the gateway"
        return
    fi
    pass "revoked lease rejected input before the transport"

    # A revoked viewer may explicitly Take again; that is a new attach and a
    # new lease, not a reuse of the old generation.
    D logcat -c >/dev/null 2>&1 || true
    if ! cmd_uidump >/dev/null 2>&1 || ! b=$(node_bounds 'text="Take"'); then
        fail "lease fixture has no Take action after revocation"
        return
    fi
    tap_center "$b"
    wait_for_log "$log" 'gatewayLease phase ready mode control' \
        || { fail "a second Take did not acquire a fresh lease"; return; }
    D shell input text 'retakeinput' >/dev/null 2>&1
    wait_for_log "$log" 'gatewayLease data .*echo:' \
        || { fail "fresh lease did not carry input"; return; }
    status=$(lease_admin status)
    if ! printf '%s' "$status" | grep -q 'retakeinput'; then
        fail "fresh lease did not reach the gateway"
        return
    fi
    pass "second Take acquired a fresh working lease"

    # Leave terminal focus so the IME closes; the gateway lane's screenshot
    # should show the scene, not the keyboard overlay.
    D shell input keyevent KEYCODE_BACK >/dev/null 2>&1
    wait_for_log "$log" 'terminalInput phase release' \
        || { fail "Back did not release terminal focus"; return; }
    sleep 0.8
    cmd_screencap "$OUTDIR/terminal-lease.png" >/dev/null \
        && say "    captured $OUTDIR/terminal-lease.png"
}

# T14: the same scripted Core behind TLS and a WebSocket upgrade. The pad's
# transport differs end to end (certificate checks, upgrade, masked frames,
# one emthin frame per message), so this step asserts the stream it produces is
# the one the lease step already trusts: a watch attach, the authoritative grid
# and the banner.
WSS_PORT=${WSS_PORT:-47658}
WSS_ADMIN_PORT=${WSS_ADMIN_PORT:-47659}
WSS_DIR=""
WSS_PID=""

start_wss_fixture() {
    command -v openssl >/dev/null 2>&1 || {
        fail "openssl is required for the WSS fixture certificate"
        return 1
    }
    WSS_DIR=$(mktemp -d /tmp/emthin-wss.XXXXXX) || return 1
    # The pad dials the reversed port as 127.0.0.1, and its HTTPS endpoint
    # identification checks the name it dialled, so the certificate has to
    # name that address as well as `localhost`.
    if ! openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=localhost" \
            -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
            -keyout "$WSS_DIR/key.pem" -out "$WSS_DIR/cert.pem" >/dev/null 2>&1; then
        fail "generate the WSS fixture certificate"
        return 1
    fi
    python3 "$SCRIPT_DIR/../fixtures/herdr-gateway-v1/wss_fixture.py" \
        --listen "127.0.0.1:$WSS_PORT" \
        --admin "127.0.0.1:$WSS_ADMIN_PORT" \
        --cert "$WSS_DIR/cert.pem" --key "$WSS_DIR/key.pem" \
        >"$WSS_DIR/server.log" 2>&1 &
    WSS_PID=$!
    local tries=0
    while [ "$tries" -lt 30 ]; do
        grep -q 'wss fixture listening' "$WSS_DIR/server.log" 2>/dev/null && break
        if ! kill -0 "$WSS_PID" 2>/dev/null; then
            say "    wss fixture exited: $(cat "$WSS_DIR/server.log" 2>/dev/null)"
            fail "wss fixture exited before it listened"
            return 1
        fi
        sleep 0.1
        tries=$((tries + 1))
    done
    if ! grep -q 'wss fixture listening' "$WSS_DIR/server.log" 2>/dev/null; then
        fail "wss fixture did not start"
        return 1
    fi
    if ! D reverse "tcp:$WSS_PORT" "tcp:$WSS_PORT" >/dev/null 2>&1; then
        fail "adb reverse for the wss fixture"
        return 1
    fi
    # The pad trusts exactly this certificate. Getting it into the app's own
    # directory needs `run-as`, which this container refuses (the shell has no
    # CAP_SETGID), so the PEM travels as launch data and the app writes it into
    # its files dir itself.
    if ! WSS_CA_B64=$(base64 -w0 "$WSS_DIR/cert.pem"); then
        fail "encode the fixture certificate"
        return 1
    fi
    return 0
}

stop_wss_fixture() {
    D reverse --remove "tcp:$WSS_PORT" >/dev/null 2>&1 || true
    if [ -n "$WSS_PID" ]; then
        kill "$WSS_PID" >/dev/null 2>&1 || true
        wait "$WSS_PID" 2>/dev/null || true
    fi
    WSS_PID=""
    if [ -n "$WSS_DIR" ] && [ -f "$WSS_DIR/server.log" ]; then
        cp "$WSS_DIR/server.log" "$OUTDIR/gateway-wss-server.txt" 2>/dev/null || true
    fi
    rm -rf "$WSS_DIR"
    WSS_DIR=""
}

wss_lease_launch() {
    local fixture="$1"
    # An empty generation (the second argument, "" ) omits the extra, which
    # makes the pad ask the core for it with `sessions.list` — the path a
    # paired gateway takes, since a pairing link names no generation.
    local generation="${2-g-fixture-1}"
    local generation_args=()
    if [ -n "$generation" ]; then
        generation_args=(--es emthin.fixtureGatewayGeneration "$generation")
    fi
    # The fixture extras are read in onCreate, so a warm activity would ignore
    # them; every fixture launch has to be a cold start.
    D shell am force-stop "$PKG" >/dev/null 2>&1 || true
    D logcat -c >/dev/null 2>&1 || true
    D shell am start -W -n "$ACTIVITY" \
        --ez emthin.fixtureVtScene true \
        --es emthin.fixtureGateway "wss://127.0.0.1:$WSS_PORT" \
        --es emthin.fixtureGatewayCaPem "$WSS_CA_B64" \
        ${generation_args[@]+"${generation_args[@]}"} \
        --ef emthin.fixtureViewZoom 1.0 \
        --es emthin.fixtureB64 "$(printf '%s' "$fixture" | base64 -w0)" \
        >/dev/null 2>&1
}

cmd_gateway_wss() {
    local log="$OUTDIR/gateway-wss-logcat.txt"
    local fixture
    say "==> gateway wss: tls -> upgrade -> watch attach -> grid -> banner"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    fixture=$(printf '%s\n' \
        '#1 Build log #2 \app(#1, #2, 640, 400, terminal: "hc://0123456789abcdef0123456789abcdef/t7")')
    if ! start_wss_fixture; then
        stop_wss_fixture
        return 1
    fi
    if wss_lease_launch "$fixture"; then
        pass "launched the disposable WSS gateway fixture"
    else
        fail "launch the WSS gateway fixture"
        stop_wss_fixture
        return 1
    fi
    wait_for_log "$log" 'vtScene phase ready' \
        || { fail "WSS fixture scene did not start"; stop_wss_fixture; return 1; }
    wait_for_log "$log" 'gatewayLease phase connect .*secure true' \
        || { fail "the pad did not choose the WSS transport"; stop_wss_fixture; return 1; }
    wait_for_log "$log" 'gatewayLease phase ready mode watch' \
        || { fail "WSS watch attach did not become ready"; stop_wss_fixture; return 1; }
    pass "WSS watch attach became a gateway lease"
    wait_for_log "$log" 'gatewayLease resize cols 80 rows 24' \
        || { fail "WSS gateway did not publish its grid"; stop_wss_fixture; return 1; }
    wait_for_log "$log" 'gatewayLease data .*gateway lease' \
        || { fail "WSS gateway DATA did not reach the native scene"; stop_wss_fixture; return 1; }
    pass "WSS gateway DATA reached the scene"

    # A paired gateway supplies no generation, so the pad has to read it from
    # the core before attaching. Launching without the extra exercises exactly
    # that against the same fixture, which answers `sessions.list`.
    if wss_lease_launch "$fixture" ""; then
        pass "launched the fixture without a generation"
    else
        fail "relaunch the fixture without a generation"
        stop_wss_fixture
        return 1
    fi
    wait_for_log "$log" 'gatewayLease phase connect .*generation discover' \
        || { fail "the pad did not ask the core for the generation"; stop_wss_fixture; return 1; }
    wait_for_log "$log" 'gatewayLease phase ready mode watch' \
        || { fail "discovered generation did not lead to a lease"; stop_wss_fixture; return 1; }
    wait_for_log "$log" 'gatewayLease data .*gateway lease' \
        || { fail "DATA did not follow the discovered generation"; stop_wss_fixture; return 1; }
    pass "the pad discovered the generation through sessions.list"
    stop_wss_fixture
}

cmd_terminal_lease() {
    local log="$OUTDIR/terminal-lease-logcat.txt"
    local fixture
    say "==> terminal lease: watch -> take -> gateway input -> revoke -> retake"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    fixture=$(printf '%s\n' \
        '#1 Build log #2 \app(#1, #2, 640, 400, terminal: "hc://0123456789abcdef0123456789abcdef/t7")')
    if ! start_lease_fixture; then
        stop_lease_fixture
        return
    fi
    terminal_lease_check "$log" "$fixture"
    stop_lease_fixture
}

# T08: view zoom rebuilds the page stack without dropping the stream.
cmd_terminal_reflow() {
    local log="$OUTDIR/terminal-reflow-logcat.txt"
    local fixture zoom reflow

    say "==> terminal reflow: stream -> zoom -> rebuild -> reflow -> pan"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    fixture=$(printf '%s\n' \
        '#1 Build log #2 \app(#1, #2, 640, 400, terminal: "hc://0123456789abcdef0123456789abcdef/t7")')
    D logcat -c >/dev/null 2>&1 || true
    if D shell am start -W -n "$ACTIVITY" \
            --ez emthin.fixtureVtScene true \
            --ez emthin.fixtureVtZoom true \
            --ez emthin.fixtureVtA11y true \
            --ef emthin.fixtureViewZoom 1.0 \
            --es emthin.fixtureB64 "$(printf '%s' "$fixture" | base64 -w0)" \
            >/dev/null 2>&1; then
        pass "launched the disposable VT-zoom fixture"
    else
        fail "launch the VT-zoom fixture"
        return
    fi
    wait_for_log "$log" 'vtScene phase ready' \
        || { fail "native scene did not start"; return; }
    wait_for_log "$log" 'vtScene phase placed' \
        || { fail "scene did not publish its placed rect"; return; }
    wait_for_log "$log" 'vtScene phase zoom cols' \
        || { fail "fixture did not request view zoom"; return; }
    pass "view zoom started while the scene was streaming"
    wait_for_log "$log" 'terminalTransfer attach' \
        || { fail "page rebuild dropped the scene instead of transferring it"; return; }
    pass "page rebuild transferred the live scene"
    wait_for_log "$log" 'vtScene phase reflow' \
        || { fail "transferred scene did not reflow its grid"; return; }

    zoom=$(sed -n 's/.*vtScene phase zoom cols \([0-9]*\) rows \([0-9]*\).*/\1 \2/p' "$log" | tail -1)
    reflow=$(sed -n 's/.*vtScene phase reflow .*cols \([0-9]*\) rows \([0-9]*\).*/\1 \2/p' "$log" | tail -1)
    if [ -z "$zoom" ] || [ -z "$reflow" ] || [ "$zoom" = "$reflow" ]; then
        fail "grid did not change after zoom (before '$zoom', after '$reflow')"
        return
    fi
    pass "grid reflowed from $zoom to $reflow cells"

    wait_for_log "$log" 'vtA11y phase complete' \
        || { fail "terminal accessibility actions did not complete"; return; }
    wait_for_log "$log" 'terminalInput text a11y-paste' \
        || { fail "accessibility paste did not reach the terminal"; return; }
    wait_for_log "$log" 'terminalInput blocked conflict' \
        || { fail "a conflicting lease did not block terminal input"; return; }
    pass "terminal accessibility Focus/Copy/Paste and conflict gate ran"

    local before_lines tries=0
    before_lines=$(D logcat -d -s emthin-pad:D 2>/dev/null | wc -l)
    D shell input swipe 1200 1000 1200 940 300 >/dev/null 2>&1
    while [ "$tries" -lt 50 ]; do
        D logcat -d -s emthin-pad:D 2>/dev/null | tr -d '\r' >"$log"
        if tail -n +$((before_lines + 1)) "$log" | grep -q 'vtScene phase frame'; then
            pass "scene kept streaming after a pan"
            break
        fi
        if ! D shell pidof "$PKG" >/dev/null 2>&1; then
            D logcat -d >"$OUTDIR/terminal-reflow-crash.txt" 2>/dev/null || true
            fail "app died while panning the streaming scene (log: $OUTDIR/terminal-reflow-crash.txt)"
            return
        fi
        sleep 0.2
        tries=$((tries + 1))
    done
    if [ "$tries" -ge 50 ]; then
        fail "scene stopped streaming after a pan"
        return
    fi

    cmd_screencap "$OUTDIR/terminal-reflow.png" >/dev/null \
        && say "    captured $OUTDIR/terminal-reflow.png"
}

# T08: leave a streaming scene's page, return, and keep the same parser.
cmd_terminal_pages() {
    local log="$OUTDIR/terminal-pages-logcat.txt"
    local fixture ready_page cycle_page away back

    say "==> terminal pages: stream -> leave page -> return -> still streaming"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    fixture=$(
        printf '#1 Cover #2 \\\n'
        i=1
        while [ "$i" -le 80 ]; do
            printf 'cover line %s\n\n' "$i"
            i=$((i + 1))
        done
        printf '#1 Build log #2 \\app(#1, #2, 640, 400, terminal: "hc://0123456789abcdef0123456789abcdef/t7")\n'
    )
    D logcat -c >/dev/null 2>&1 || true
    if D shell am start -W -n "$ACTIVITY" \
            --ez emthin.fixtureVtScene true \
            --ez emthin.fixtureVtPages true \
            --ef emthin.fixtureViewZoom 1.0 \
            --es emthin.fixtureB64 "$(printf '%s' "$fixture" | base64 -w0)" \
            >/dev/null 2>&1; then
        pass "launched the multi-page VT fixture"
    else
        fail "launch the multi-page VT fixture"
        return
    fi
    wait_for_log "$log" 'vtScene phase ready page' \
        || { fail "multi-page fixture never started its terminal scene"; return; }
    ready_page=$(sed -n 's/.*vtScene phase ready page \([0-9]*\).*/\1/p' "$log" | tail -1)
    if [ -z "$ready_page" ]; then
        fail "could not parse the terminal scene page"
        return
    fi
    pass "terminal scene started on page $((ready_page + 1))"

    wait_for_log "$log" 'vtScene phase paging away' \
        || { fail "fixture did not leave the streaming page"; return; }
    wait_for_log "$log" 'pageRaster page 1' \
        || { fail "the next page never loaded during the switch"; return; }
    pass "switched to another page while the scene was streaming"
    wait_for_log "$log" 'vtScene phase paging back' \
        || { fail "fixture did not return to the streaming page"; return; }
    wait_for_log "$log" 'vtScene phase page-cycle' \
        || { fail "scene did not survive the page switch"; return; }
    wait_for_log "$log" 'vtScene gl repaint page .* reason page-cycle' \
        || { fail "scene surface did not repaint after returning"; return; }
    away=$(sed -n 's/.*vtScene phase paging away .*scroll \([0-9]*\).*/\1/p' "$log" | tail -1)
    back=$(sed -n 's/.*vtScene phase paging back .*scroll \([0-9]*\).*/\1/p' "$log" | tail -1)
    cycle_page=$(sed -n 's/.*vtScene phase page-cycle page \([0-9]*\).*/\1/p' "$log" | tail -1)
    if [ -z "$away" ] || [ -z "$back" ] || [ "$back" -ge "$away" ]; then
        fail "page switch did not move away and back (away '$away', back '$back')"
        return
    fi
    if [ "$cycle_page" != "$ready_page" ]; then
        fail "scene moved from page $ready_page to page $cycle_page"
        return
    fi
    pass "scene survived leaving page $((ready_page + 1)) and returning"
    wait_for_log "$log" 'vtScene phase frame 25' \
        || { fail "scene stopped streaming after the page cycle"; return; }
    pass "same parser kept streaming after the page cycle"

    cmd_screencap "$OUTDIR/terminal-pages.png" >/dev/null \
        && say "    captured $OUTDIR/terminal-pages.png"
}

# T05: query and activate a terminal figure through the accessibility provider.
cmd_figure_a11y() {
    local log="$OUTDIR/figure-a11y-logcat.txt"
    local fixture

    say "==> figure accessibility: node state + Go/Rebind/Detach/Sessions"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    fixture=$(printf '%s\n' \
        '#1 Build log #2 \app(#1, #2, 640, 400, terminal: "hc://0123456789abcdef0123456789abcdef/t7")')
    D logcat -c >/dev/null 2>&1 || true
    if D shell am start -W -n "$ACTIVITY" \
            --ez emthin.fixtureFigureA11y true \
            --es emthin.fixtureB64 "$(printf '%s' "$fixture" | base64 -w0)" \
            >/dev/null 2>&1; then
        pass "launched the disposable figure-accessibility fixture"
    else
        fail "launch the disposable figure-accessibility fixture"
        return
    fi
    wait_for_log "$log" 'figureA11y phase nodes .*Watching on studio' \
        || { fail "figure node did not expose its watched state"; return; }
    pass "figure node exposed the watched state and all four actions"
    wait_for_log "$log" 'figureA11y phase focus .*Watching on studio' \
        || { fail "accessibility focus did not resolve to the figure node"; return; }
    pass "accessibility focus resolved to the figure node"
    wait_for_log "$log" 'figureA11y phase go' \
        || { fail "Go to figure action did not run"; return; }
    pass "Go to figure action ran"
    wait_for_log "$log" 'figureA11y phase detach .*Live on studio' \
        || { fail "Detach did not transition the figure to live"; return; }
    pass "Detach transitioned the figure to live"
    wait_for_log "$log" 'figureA11y phase sessions' \
        || { fail "Sessions action did not run"; return; }
    pass "Sessions action ran"
    wait_for_log "$log" 'figureA11y phase complete' \
        || { fail "figure-accessibility fixture did not complete"; return; }
    pass "figure-accessibility fixture completed"
}

cmd_e2e() {
    say "=== emthin pad device lane ($DEVICE) ==="
    cmd_authorize
    cmd_connect
    # Without a live device nothing below can run; stop rather than
    # spray FAILs for a missing prerequisite.
    if [ "$(D get-state 2>/dev/null || true)" != "device" ]; then
        say "device not reachable; aborting e2e after the steps above"
        return
    fi

    # Build for the device's ABI, then install it. The build is the part
    # that used to be a manual prerequisite and silently installed a
    # host-ABI APK.
    if [ "$SKIP_BUILD" != "1" ]; then
        cmd_build || { say "build failed; aborting e2e"; return; }
    fi
    cmd_install || { say "install failed; aborting e2e"; return; }

    # Cold start so the first frame is this launch's. Autosave means the
    # previous session's text would otherwise survive this launch.
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    cmd_launch
    sleep 3

    local a b log
    a=$(cmd_screencap "$OUTDIR/e2e-before.png") || return
    say "    captured $a"
    cmd_swipe
    sleep 1
    b=$(cmd_screencap "$OUTDIR/e2e-after.png") || return
    say "    captured $b"

    # Whether the drag scrolls depends on the document's length (a
    # short doc fits one page), so the pixel delta is informational
    # here, not a hard gate. Identical frames only mean "nothing to
    # scroll"; the capture pipeline itself is asserted above.
    if cmp -s "$a" "$b"; then
        say "info swipe: frames identical (preview likely fits one page)"
    else
        pass "swipe moved pixels (before != after)"
    fi

    # The log is the visual loop's machine-readable half: JIT loaded,
    # and a render chunk actually reached the preview.
    cmd_logcat "$OUTDIR/e2e-logcat.txt" || return
    log="$OUTDIR/e2e-logcat.txt"
    say "    captured $log"
    if grep -q 'cranelift roundtrip: {"ok":42' "$log"; then
        pass "JIT roundtrip in logcat"
    else
        fail "JIT roundtrip missing from $log"
    fi
    if grep -q 'applyChunk' "$log"; then
        pass "render chunks in logcat"
    else
        fail "applyChunk missing from $log"
    fi
    if grep -q 'captionScroll' "$log"; then
        pass "first caption scroll in logcat"
    else
        fail "captionScroll missing from $log"
    fi

    # Autosave, New/Undo, then the reading-position proof. Each check
    # clears app data itself, so it still cold-launches the starter.
    cmd_persistence
    cmd_new_reset
    cmd_enter
    cmd_scroll_anchor
    cmd_search_replace
    cmd_accessibility
    cmd_diagnostics
    cmd_named_document
    cmd_focus_stack
    cmd_composing_ime
    cmd_modifier_inspector
    cmd_session_pin
    cmd_figure_a11y
    cmd_terminal_scene
    cmd_terminal_reflow
    cmd_terminal_pages
    cmd_terminal_lease
    # Opt in with LANE_WSS=1: the step needs an APK built from the transport
    # sources, and until the NDK link works again the installed APK predates
    # them, so enabling it by default would only fail on a stale binary.
    if [ "${LANE_WSS:-0}" = "1" ]; then
        cmd_gateway_wss
    fi

    # Byte-identity of the installed APK, the launch proof's companion.
    cmd_pull
}

cmd_enter() {
    say "==> Enter: rendered page -> source editor"
    if [ "$PKG" != "$TEST_PACKAGE" ]; then
        fail "enter requires $TEST_PACKAGE"
        return
    fi
    mkdir -p "$OUTDIR" || { fail "create Enter output directory"; return; }
    local log="$OUTDIR/enter-logcat.txt" ui="$OUTDIR/enter-ui.xml"
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    if ! D shell am start -W -n "$ACTIVITY" \
            --es emthin.fixtureB64 "$(printf 'First line' | base64 -w0)" >/dev/null 2>&1; then
        fail "launch the Enter fixture"
        return
    fi
    wait_for_log "$log" 'applyChunk total 1 pages 1 start 0' \
        || { fail "Enter fixture did not render"; return; }
    D shell input keyevent KEYCODE_MOVE_END KEYCODE_ENTER >/dev/null 2>&1
    wait_for_log "$log" 'applyState setCaret false .*textLen 11 ' \
        || { fail "Return in the rendered page did not insert a newline"; return; }
    D shell input keyevent KEYCODE_NUMPAD_ENTER >/dev/null 2>&1
    D shell input text Next >/dev/null 2>&1
    wait_for_log "$log" 'applyState setCaret false .*textLen 16 ' \
        || { fail "keypad Enter and typing did not reach the source editor"; return; }
    cmd_uidump "$ui" || return
    if python3 - "$ui" <<'PY'
import sys
import xml.etree.ElementTree as ET

for node in ET.parse(sys.argv[1]).iter("node"):
    if (node.get("focused") == "true"
            and node.get("content-desc") == "Editable text without modifiers"
            and node.get("text") == "First line\n\nNext"):
        sys.exit(0)
sys.exit("expected the two newlines and continued typing in the focused source field")
PY
    then
        pass "both Enter keys inserted newlines and typing continued in source"
    else
        fail "unexpected source text or focus after Enter"
    fi
}

cmd_math_preview() {
    say "==> math preview: verify on-screen prose and equation"
    if [ "$PKG" != "$TEST_PACKAGE" ]; then
        fail "math-preview requires $TEST_PACKAGE"
        return
    fi
    if ! python3 -c 'from PIL import Image' >/dev/null 2>&1; then
        fail "math-preview requires Python Pillow"
        return
    fi
    mkdir -p "$OUTDIR" || { fail "create math preview output directory"; return; }
    cmd_clear_data || return
    local log="$OUTDIR/math-preview-logcat.txt" shot="$OUTDIR/math-preview.png"
    local layout="$OUTDIR/math-preview-ui.xml" fixture
    fixture=$(printf 'Preview ink\n\n$x^2 + y^2 = z^2$\n\nAfter equation\n' | base64 -w0)
    D logcat -c >/dev/null 2>&1 || true
    if ! D shell am start -W -n "$ACTIVITY" --es emthin.fixtureB64 "$fixture" >/dev/null 2>&1; then
        fail "launch the math preview fixture"
        return
    fi
    wait_for_log "$log" 'applyChunk total 1 pages 1 start 0' \
        || { fail "math preview did not apply its page"; return; }
    wait_for_log "$log" 'diagnostics .*count 0 errors 0 warnings 0 notes [1-9]' \
        || { fail "ordinary math variables should produce notes only"; return; }
    cmd_uidump "$layout" || return
    if grep -q 'tap for details' "$layout"; then
        fail "ordinary math variables triggered a diagnostic banner"
        return
    fi
    cmd_screencap "$shot" >/dev/null || return

    local glyph
    if glyph=$(python3 - "$layout" "$shot" <<'PY'
import re
import sys
import xml.etree.ElementTree as ET
from PIL import Image

blocks = ET.parse(sys.argv[1]).iter("node")
block = next((b for b in blocks if b.get("text", "").startswith("Preview ink")), None)
if block is None:
    sys.exit("math preview accessibility bounds are absent")
bounds = tuple(map(int, re.findall(r"\d+", block.get("bounds", ""))))
image = Image.open(sys.argv[2]).convert("RGB").crop(bounds)
width, height = image.size
# Text is white. Ignore isolated pixels and group adjacent rows so the
# raised exponent counts with the equation below it.
bands = []
for y in range(height):
    if sum(min(image.getpixel((x, y))) > 160 for x in range(width)) < 3:
        continue
    if not bands or y - bands[-1][-1] > max(2, height // 40):
        bands.append([])
    bands[-1].append(y)
bands = [band for band in bands if len(band) >= 3]
if len(bands) < 3:
    sys.exit("expected three visible text bands inside the preview hit map")
equation = bands[1]
y = equation[2 * len(equation) // 3]
x = next(x for x in range(width) if min(image.getpixel((x, y))) > 160)
print(x + bounds[0], y + bounds[1])
PY
    )
    then
        pass "prose and equation are visible at the reported preview bounds"
    else
        fail "math preview bitmap is missing or misplaced ($shot)"
        return
    fi

    local x y edited="$OUTDIR/math-preview-edit.xml" undone="$OUTDIR/math-preview-undo.xml"
    read -r x y <<<"$glyph"
    if ! D shell input tap "$x" "$y" >/dev/null 2>&1 ||
            ! D shell input text q >/dev/null 2>&1; then
        fail "tap and edit the displayed equation"
        return
    fi
    cmd_uidump "$edited" || return
    if ! D shell input keycombination 113 54 >/dev/null 2>&1; then
        fail "send Ctrl+Z to undo the equation edit"
        return
    fi
    cmd_uidump "$undone" || return
    if python3 - "$edited" "$undone" <<'PY'
import sys
import xml.etree.ElementTree as ET

def focused_source(path):
    for node in ET.parse(path).iter("node"):
        if (node.get("content-desc") == "Editable text without modifiers"
                and node.get("focused") == "true"):
            return node.get("text", "")
    return ""

original = "$x^2 + y^2 = z^2$"
edited = focused_source(sys.argv[1])
if not edited.startswith("$") or edited.count("q") != 1 or edited.replace("q", "") != original:
    sys.exit("editing the tapped glyph did not update the focused math source")
if focused_source(sys.argv[2]) != original:
    sys.exit("Undo did not restore the equation and keep source focus")
PY
    then
        pass "tapping the equation edits its source and Undo restores it with focus"
    else
        fail "equation edit or Undo lost the source text or focus"
    fi
}

cmd_history_preview() {
    # K5: the pad retains a snapshot of the current note and previews it
    # read-only through the shared docui::history ring on the model owner.
    local log="$OUTDIR/history-preview-logcat.txt"
    say "==> history: retain -> preview a read-only snapshot"
    mkdir -p "$OUTDIR"
    D shell am force-stop "$PKG" >/dev/null 2>&1 || true
    D logcat -c >/dev/null 2>&1 || true
    if D shell am start -W -n "$ACTIVITY" --ez emthin.historySmoke true \
            >/dev/null 2>&1; then
        pass "launched the history smoke"
    else
        fail "launch the history smoke"
        return
    fi
    wait_for_log "$log" 'history smoke: .*readOnly=true' \
        || { fail "history preview did not reach logcat"; return; }
    if grep -q 'history smoke failed' "$log"; then
        fail "history preview reported a failure"
        return
    fi
    pass "history preview left live editor state unchanged"
}

cmd_austral_rule() {
    # K9: run the embedded Austral rule artifact on a worker thread (the JNI
    # entry spawns one, and MainActivity calls it from a Java background
    # thread). Assert the real native result reaches logcat on this arm64
    # device; a fingerprint/artifact mismatch or a capability violation
    # would surface as `austral rule failed` instead.
    local log="$OUTDIR/austral-rule-logcat.txt"
    say "==> austral rule: native artifact off the UI thread"
    mkdir -p "$OUTDIR"
    D shell am force-stop "$PKG" >/dev/null 2>&1 || true
    D logcat -c >/dev/null 2>&1 || true
    if D shell am start -W -n "$ACTIVITY" --ez emthin.australRuleSmoke true \
            >/dev/null 2>&1; then
        pass "launched the austral-rule smoke"
    else
        fail "launch the austral-rule smoke"
        return
    fi
    wait_for_log "$log" 'austral rule: .*"result":"complete".*"facts":2' \
        || { fail "native Austral rule did not return complete/2 on device"; return; }
    if grep -q 'austral rule failed' "$log"; then
        fail "native Austral rule reported a failure on device"
        return
    fi
    pass "native Austral rule returned complete/2 off the UI thread on arm64"
}

# ---- dispatch ---------------------------------------------------------

cmd="${1:-e2e}"
case "$cmd" in
    -h|--help|help) usage; exit 0 ;;
esac

require_adb
require_session
resolve_device

case "$cmd" in
    authorize) cmd_authorize ;;
    connect)   cmd_connect ;;
    build)     cmd_build ;;
    install)   cmd_install ;;
    launch)    cmd_launch ;;
    pull)      cmd_pull ;;
    screencap) cmd_screencap "${2:-}"; echo ;;
    logcat)    cmd_logcat "${2:-}" ;;
    swipe)     cmd_swipe ;;
    math-preview) cmd_math_preview ;;
    enter) cmd_enter ;;
    persistence) cmd_persistence ;;
    new-reset) cmd_new_reset ;;
    scroll-anchor) cmd_scroll_anchor ;;
    search-replace) cmd_search_replace ;;
    accessibility) cmd_accessibility ;;
    diagnostics) cmd_diagnostics ;;
    named-document) cmd_named_document ;;
    focus-stack) cmd_focus_stack ;;
    composing-ime) cmd_composing_ime ;;
    modifier-inspector) cmd_modifier_inspector ;;
    session-pin) cmd_session_pin ;;
    figure-a11y) cmd_figure_a11y ;;
    terminal-scene) cmd_terminal_scene ;;
    terminal-reflow) cmd_terminal_reflow ;;
    terminal-pages) cmd_terminal_pages ;;
    terminal-lease) cmd_terminal_lease ;;
    gateway-wss) cmd_gateway_wss ;;
    austral-rule) cmd_austral_rule ;;
    history-preview) cmd_history_preview ;;
    e2e)       cmd_e2e ;;
    *) say "unknown command: $cmd"; usage >&2; exit 2 ;;
esac

if [ "$FAILURES" -eq 0 ]; then
    say "device lane: all steps passed"
    exit 0
fi
say "device lane: $FAILURES step(s) FAILED"
exit 1
