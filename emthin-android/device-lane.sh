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
#   network-change  a live control lease whose host disappears (killed, which
#                is what an unreachable network looks like through `adb
#                reverse`), then returns: the pad keeps its document, offers
#                control again and reaches the restarted host on a fresh take
#                (used by e2e; requires the disposable test APK and python3)
#   pairing      paste a tailnet link -> pending -> approve through the TLS
#                fixture's admin socket -> row stored across a relaunch ->
#                Forget (LANE_WSS=1; requires the disposable test APK, python3
#                and openssl)
#   pairing-answers  the same screen's other answers: deny, lapse and a Core
#                mismatch, none of which may store a row (LANE_WSS=1)
#   sessions-hosts  pair two hosts with two Core IDs, then prove one Sessions
#                picker merges both catalogues and that Reconnect refreshes
#                only the host whose row was tapped (LANE_WSS=1)
#   enroll       a paired host's inventory, a proposal refused after its VM
#                rotated, then the current one enrolled and listed as a session
#                with the document untouched (LANE_WSS=1)
#   gateway-wss  TLS -> upgrade -> watch attach -> grid -> banner, then the
#                same without a generation so sessions.list supplies it
#                (LANE_WSS=1; requires the disposable test APK and python3)
#   proof-zenodo  real mm0web + mock_zenodo.py over adb reverse: invalid proof
#                rejected, publish confirm/cancel, DOI poll, note insert + Undo,
#                and missing-Zenodo-key refusal (used by e2e when mm0web exists)
#   phone-layout  360 dp portrait (1080x2340 @ 480 dpi) and landscape (2340x1080)
#                math-preview + compact two-row toolbar bounds check, then reset
#   wayland-gpu  app-wide GPU (GLES3 + Vulkan 1.1 compute + custom SPIR-V +
#                AHardwareBuffer interop + RGBA surface) & embedded Wayland
#                compositor smoke proof (used by e2e)
#   e2e          authorize, connect, build, install, launch, screencap,
#                swipe, screencap, logcat, persistence, new-reset,
#                scroll-anchor, search-replace, focus-stack, composing-ime,
#                accessibility, diagnostics, named-document, modifier-inspector,
#                session-pin, figure-a11y, terminal-scene, terminal-reflow,
#                terminal-pages, terminal-lease, network-change, math-preview,
#                proof-zenodo, pull + byte-identity
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

# Dump until PATTERN appears — activity transitions and popups animate, so a
# dump taken right after a tap can still be the previous screen. COUNT is in
# half-seconds (20 = 10 s). The caller dumps again for the file it asserts on.
wait_for_ui() {
    local pattern="$1" count="${2:-20}" out="${3:-$UI_XML}" tries=0
    UI_XML="$out"
    while [ "$tries" -lt "$count" ]; do
        if D shell uiautomator dump "$UI_DUMP_REMOTE" >/dev/null 2>&1 \
            && D exec-out cat "$UI_DUMP_REMOTE" 2>/dev/null | tr -d '\r' >"$out" \
            && grep -q "$pattern" "$out"; then
            return 0
        fi
        sleep 0.5
        tries=$((tries + 1))
    done
    return 1
}

# Bounds of the Nth (1-based) node whose opening tag contains PATTERN.
node_bounds_nth() {
    local pattern="$1" index="$2" node
    node=$(grep -o "<node[^>]*${pattern}[^>]*>" "$UI_XML" 2>/dev/null \
        | sed -n "${index}p" || true)
    [ -n "$node" ] || return 1
    printf '%s\n' "$node" | sed -n \
        's/.*bounds="\[\([0-9]*\),\([0-9]*\)\]\[\([0-9]*\),\([0-9]*\)\]".*/\1 \2 \3 \4/p'
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

# PopupMenus animate in, so a dump taken immediately after a tap can still be
# the previous frame — and a nested group opens a second popup, which is the
# slowest case. Wait for each popup, and if one does not appear retry the whole
# sequence with the menu closed again.
tap_toolbar_menu() {
    local group="$1" action="$2" attempt=0 rc=0
    while [ "$attempt" -lt 4 ]; do
        rc=0
        tap_toolbar_menu_once "$group" "$action" || rc=$?
        [ "$rc" -eq 0 ] && return 0
        # 2 means a popup was open: Back puts the document back where it was so
        # the retry starts from the toolbar. 1 means nothing opened at all.
        if [ "$rc" -eq 2 ]; then
            D shell input keyevent KEYCODE_BACK >/dev/null 2>&1 || true
            sleep 0.5
        fi
        attempt=$((attempt + 1))
    done
    return 1
}

# 0 = action tapped, 1 = no menu button on screen, 2 = a popup was open but the
# entry for the next step was not in it.
tap_toolbar_menu_once() {
    local group="$1" action="$2" bounds
    cmd_uidump "$UI_XML" >/dev/null || return 1
    bounds=$(node_bounds 'content-desc="More actions"') || return 1
    tap_center "$bounds" || return 1
    sleep 0.8
    cmd_uidump "$UI_XML" >/dev/null || return 2
    bounds=$(node_bounds "text=\"$group\"") || return 2
    tap_center "$bounds" || return 2
    sleep 0.8
    cmd_uidump "$UI_XML" >/dev/null || return 2
    bounds=$(node_bounds "text=\"$action\"") || return 2
    tap_center "$bounds" || return 2
    sleep 0.5
    # Leave a dump of whatever the action opened, since callers read the last
    # dump to assert on it.
    cmd_uidump "$UI_XML" >/dev/null || true
    return 0
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

# A scripted fixture that exits mid-step turns every later wait into a 10 s
# timeout whose failure blames the app. Check the process and the tail of its
# log at the wait instead, so the failure names the thing that actually went
# away. Returns 0 when a started fixture is gone, 1 when it is alive or was
# never started by this step.
fixture_gone() {
    local pid="$1" dir="$2" label="$3" want="$4" line
    [ -n "$pid" ] || return 1
    kill -0 "$pid" 2>/dev/null && return 1
    say "    info the $label process exited; '$want' cannot arrive"
    if [ -n "$dir" ] && [ -f "$dir/server.log" ]; then
        while IFS= read -r line; do
            say "    info $label: $line"
        done < <(tail -n 5 "$dir/server.log")
    fi
    return 0
}

wait_for_log() {
    local out="$1" pattern="$2" tries=0
    while [ "$tries" -lt 50 ]; do
        D logcat -d -s emthin-pad:D 2>/dev/null | tr -d '\r' >"$out"
        if grep -q "$pattern" "$out"; then
            return 0
        fi
        if fixture_gone "${LEASE_PID:-}" "${LEASE_DIR:-}" \
                "gateway lease fixture" "$pattern" \
            || fixture_gone "${WSS_PID:-}" "${WSS_DIR:-}" "WSS fixture" "$pattern"; then
            return 1
        fi
        sleep 0.2
        tries=$((tries + 1))
    done
    return 1
}

# Wait for PATTERN in a line that appears after this call. A step that repeats
# a phase (take, lose the host, take again) needs this: wait_for_log would match
# the first one and pass without the second ever happening.
wait_for_new_log() {
    local out="$1" pattern="$2" tries=0 mark lines
    mark=$(D logcat -d -s emthin-pad:D 2>/dev/null | tr -d '\r' | wc -l)
    mark=${mark:-0}
    while [ "$tries" -lt 50 ]; do
        D logcat -d -s emthin-pad:D 2>/dev/null | tr -d '\r' >"$out"
        lines=$(wc -l <"$out" | tr -d ' ')
        if [ "${lines:-0}" -gt "$mark" ] \
            && tail -n "+$((mark + 1))" "$out" | grep -q "$pattern"; then
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
    local log="$1" fixture="$2" placed x y w h cx cy status first_lease second_lease
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
    first_lease=$(sed -n 's/.*gatewayLease phase ready mode control lease \([^ ]*\).*/\1/p' "$log" | tail -1)
    if [ -z "$first_lease" ] || [ "$first_lease" = none ]; then
        fail "the gateway controller lease was not named in the log"
        return
    fi
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
    second_lease=$(sed -n 's/.*gatewayLease phase ready mode control lease \([^ ]*\).*/\1/p' "$log" | tail -1)
    if [ -z "$second_lease" ] || [ "$second_lease" = none ]; then
        fail "the second Take did not name its lease"
        return
    fi
    if [ "$second_lease" = "$first_lease" ]; then
        fail "the second Take reused the revoked lease '$first_lease'"
        return
    fi
    pass "the second Take acquired a lease the gateway named differently"
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
# The raw client `paired-lease` uses to prove the gateway refuses a revoked
# controller's bytes; cleared whenever the fixture stops.
WSS_PROBE_PID=""
# The scripted Core's ID (lease_fixture.CORE_ID): the fixture only approves a
# pairing request that names it.
PAIR_CORE=6ab09a20fd1288983dfbbac2bb349231
# The fixture's second identity (`--second-core`): the two-host step pairs one
# row per Core ID, so the picker's merge spans two real catalogues.
SECOND_CORE=b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0

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
        --second-core "$SECOND_CORE" \
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
    if [ -n "$WSS_PROBE_PID" ]; then
        kill "$WSS_PROBE_PID" >/dev/null 2>&1 || true
        wait "$WSS_PROBE_PID" 2>/dev/null || true
    fi
    WSS_PROBE_PID=""
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

wss_admin() {
    (exec 3<>/dev/tcp/127.0.0.1/$WSS_ADMIN_PORT
     printf '%s\n' "$1" >&3
     cat <&3) 2>/dev/null
}

# An admin command the step cannot continue without. The fixture answers every
# command with JSON, so silence means the socket is gone — worth naming here,
# where it is the cause, instead of letting the step wait out the request's
# pending timeout and report the answer that never came.
wss_admin_checked() {
    local reply
    if fixture_gone "${WSS_PID:-}" "${WSS_DIR:-}" "WSS fixture" "admin $1"; then
        fail "the WSS fixture process died mid-step"
        return 1
    fi
    reply=$(wss_admin "$1")
    if [ -z "$reply" ]; then
        fail "the WSS fixture admin socket answered nothing to '$1'"
        return 1
    fi
    printf '%s\n' "$reply"
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

# Cold-start the pad against the TLS fixture with the pairing origin remapped to
# loopback, then open the pairing screen. The extras are read in onCreate, so a
# warm activity would ignore them.
# `am force-stop` returns before the process is gone, and a start that races
# the teardown can land on the dying activity, which drops the extras.
cold_stop() {
    local tries=0
    D shell am force-stop "$PKG" >/dev/null 2>&1 || true
    while [ "$tries" -lt 25 ]; do
        D shell pidof "$PKG" >/dev/null 2>&1 || return 0
        sleep 0.2
        tries=$((tries + 1))
    done
    return 0
}

pair_launch() {
    local ui="$1"
    UI_XML="$ui"
    cold_stop
    D logcat -c >/dev/null 2>&1 || true
    D shell am start -W -n "$ACTIVITY" \
        --es emthin.fixturePairBase "https://127.0.0.1:$WSS_PORT" \
        --es emthin.fixtureConnect "https://127.0.0.1:$WSS_PORT" \
        --es emthin.fixtureGatewayCaPem "$WSS_CA_B64" >/dev/null 2>&1 || return 1
    sleep 3
    tap_toolbar_menu "Sessions and sharing" "Pair gateway" >/dev/null
}

# The same cold start, on the Sessions picker instead of the Pair screen.
sessions_launch() {
    local ui="$1"
    UI_XML="$ui"
    cold_stop
    D logcat -c >/dev/null 2>&1 || true
    D shell am start -W -n "$ACTIVITY" \
        --es emthin.fixturePairBase "https://127.0.0.1:$WSS_PORT" \
        --es emthin.fixtureConnect "https://127.0.0.1:$WSS_PORT" \
        --es emthin.fixtureGatewayCaPem "$WSS_CA_B64" >/dev/null 2>&1 || return 1
    sleep 3
    tap_toolbar_menu "Sessions and sharing" "Sessions" >/dev/null
}

# Withdrawing a *granted* runtime permission kills the app (the platform drops
# the process so nothing keeps using it), so the Pair screen the scan came from
# is gone with it. Reopen it rather than asserting against a dead screen.
ensure_pair_screen() {
    local ui="$1" tries=0
    wait_for_ui 'text="SCAN QR"' 4 "$ui" && return 0
    pair_launch "$ui" >/dev/null || return 1
    wait_for_ui 'text="SCAN QR"' 20 "$ui"
}

# Tap PASTE LINK, type LINK into the field, and press PAIR, leaving the pairing
# screen (which PAIR reopens) as the last dump.
pair_paste() {
    local link="$1" expect="${2:-fleet.leo.ts.net}" ui bounds
    ui=$UI_XML
    bounds=$(node_bounds 'text="PASTE LINK"') \
        || { fail "Paste link is missing"; return 1; }
    tap_center "$bounds"
    cmd_uidump "$ui" >/dev/null || return 1
    bounds=$(node_bounds 'class="android.widget.EditText"') \
        || { fail "the link field is missing"; return 1; }
    tap_center "$bounds"
    sleep 0.5
    D shell "input text '$link'" >/dev/null 2>&1 || { fail "type the pairing link"; return 1; }
    sleep 0.5
    cmd_uidump "$ui" >/dev/null || return 1
    if ! grep -q "$expect/pair?v=1" "$ui"; then
        fail "the typed link did not reach the field"
        return 1
    fi
    bounds=$(node_bounds 'text="PAIR"') \
        || { fail "Pair is missing"; return 1; }
    tap_center "$bounds"
    sleep 0.5
}

# Answer the system camera-permission prompt with "Don't allow". The button's
# resource id is the platform's, so the tap does not depend on the dialog's
# wording or locale; a package the platform has already stopped asking about
# gets the "don't ask again" variant of the same button. A package that has
# been denied outright is answered without a prompt, so a missing dialog is not
# a failure by itself.
deny_camera_prompt() {
    local ui="$OUTDIR/pairing-permission.xml" tries=0 bounds pattern
    UI_XML="$ui"
    mkdir -p "$OUTDIR" || return 1
    while [ "$tries" -lt 20 ]; do
        if D shell uiautomator dump "$UI_DUMP_REMOTE" >/dev/null 2>&1 \
            && D exec-out cat "$UI_DUMP_REMOTE" 2>/dev/null | tr -d '\r' >"$ui"; then
            for pattern in permission_deny_button permission_deny_and_dont_ask_again_button; do
                if bounds=$(node_bounds "$pattern"); then
                    tap_center "$bounds"
                    return 0
                fi
            done
        fi
        sleep 0.5
        tries=$((tries + 1))
    done
    return 1
}

# T16's inventory half: a paired host proposes the sessions it already runs, and
# enrolling one is the operator's explicit act. The host owns the refusal, so a
# proposal read before its VM was rotated must not become a session.
cmd_enroll() {
    local ui="$OUTDIR/enroll-ui.xml"
    local log="$OUTDIR/enroll-logcat.txt"
    local link="https://fleet.leo.ts.net/pair?v=1&core=$PAIR_CORE"
    local bounds status
    say "==> enrol: inventory -> stale vm refused -> enrol -> listed"
    mkdir -p "$OUTDIR" || { fail "create $OUTDIR"; return; }
    cmd_clear_data || return
    if ! start_wss_fixture; then stop_wss_fixture; return 1; fi
    if ! pair_launch "$ui"; then
        fail "open the pairing screen"
        stop_wss_fixture
        return 1
    fi
    pair_paste "$link" || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase request host fleet.leo.ts.net'; then
        fail "the link did not open a pairing request"
        stop_wss_fixture
        return 1
    fi
    wss_admin_checked "pair approve" >/dev/null || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase approved host fleet.leo.ts.net'; then
        fail "the approval did not arrive"
        stop_wss_fixture
        return 1
    fi

    # The picker lists the host, and its Inventory action reads what runs there.
    sessions_launch "$ui" || { fail "open Sessions against the paired host"; stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'sessions phase host fleet.leo.ts.net rows'; then
        fail "the paired host did not answer its catalogue"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    bounds=$(node_bounds 'text="Inventory"') \
        || { fail "the host row has no Inventory action"; stop_wss_fixture; return 1; }
    tap_center "$bounds"
    if ! wait_for_log "$log" 'enrollment phase inventory host fleet.leo.ts.net'; then
        fail "the inventory was never read"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'fb-7' "$ui" || ! grep -q 'vm-fixture-1' "$ui"; then
        fail "the proposals are not listed with the VM they run in"
        stop_wss_fixture
        return 1
    fi
    pass "a paired host proposed its running sessions with their VM"

    # The VM the list named is replaced underneath the open dialog: the enrol
    # the operator started must be refused rather than create a session.
    wss_admin_checked "enrollment rotate-vm" >/dev/null || { stop_wss_fixture; return 1; }
    bounds=$(node_bounds 'text="Enrol"') \
        || { fail "a proposal has no Enrol action"; stop_wss_fixture; return 1; }
    tap_center "$bounds"
    if ! wait_for_log "$log" 'enrollment phase enrol host fleet.leo.ts.net address fb-7 failed'; then
        fail "a stale proposal was not refused by the host"
        stop_wss_fixture
        return 1
    fi
    status=$(wss_admin status)
    if printf '%s' "$status" | grep -q '"enrolled":\["'; then
        fail "a refused enrolment created a session"
        stop_wss_fixture
        return 1
    fi
    pass "a proposal whose VM was rotated is refused and creates nothing"

    # Reading again names the current VM, so the same row now enrols.
    D shell input keyevent KEYCODE_BACK >/dev/null 2>&1 || true
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    bounds=$(node_bounds 'text="Inventory"') \
        || { fail "the host row lost its Inventory action"; stop_wss_fixture; return 1; }
    tap_center "$bounds"
    if ! wait_for_log "$log" 'enrollment phase inventory host fleet.leo.ts.net vm vm-fixture-2'; then
        fail "the second read did not name the current VM"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    bounds=$(node_bounds 'text="Enrol"') \
        || { fail "the fresh proposal has no Enrol action"; stop_wss_fixture; return 1; }
    tap_center "$bounds"
    if ! wait_for_log "$log" 'enrollment phase enrolled host fleet.leo.ts.net address fb-7'; then
        fail "a current proposal was not enrolled"
        stop_wss_fixture
        return 1
    fi
    status=$(wss_admin status)
    if ! printf '%s' "$status" | grep -q '"enrolled":\["fb-7"\]'; then
        fail "the host does not record the enrolment"
        stop_wss_fixture
        return 1
    fi
    pass "the current proposal was enrolled at the host"

    # The enrolled session is a session now: the picker lists it, with no
    # document edit and nothing started for it. Enrolling re-reads that host.
    D shell input keyevent KEYCODE_BACK >/dev/null 2>&1 || true
    if ! wait_for_ui 'Fleet build'; then
        fail "the enrolled session did not join the picker"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'Fleet build' "$ui"; then
        fail "the enrolled session is not listed by the picker"
        stop_wss_fixture
        return 1
    fi
    pass "the enrolled session appeared in the picker without a document edit"
    stop_wss_fixture
}

# The pairing screen end to end against the TLS fixture. The link is a real
# tailnet link — production parse and validation — and only its origin is
# remapped to loopback by `emthin.fixturePairBase`.
cmd_pairing() {
    local ui="$OUTDIR/pairing-ui.xml"
    local log="$OUTDIR/pairing-logcat.txt"
    local link="https://fleet.leo.ts.net/pair?v=1&core=$PAIR_CORE"
    local bounds
    say "==> pairing: paste link -> pending -> approve -> stored -> forget"
    mkdir -p "$OUTDIR" || { fail "create $OUTDIR"; return; }
    cmd_clear_data || return
    if ! start_wss_fixture; then stop_wss_fixture; return 1; fi

    if ! pair_launch "$ui"; then
        fail "open the pairing screen"
        stop_wss_fixture
        return 1
    fi
    if ! grep -q 'text="No gateways paired yet."' "$ui"; then
        fail "a fresh registry does not report itself empty"
        stop_wss_fixture
        return 1
    fi
    pass "the pairing screen starts with an empty registry"

    # Scanning is the paste path with a camera in front: the decoded text is the
    # same link. This container exposes no camera, so the check that matters
    # here is that the scanner says so and leaves pairing usable — the decode
    # itself is the host suite's (QrScanTest).
    #
    # A denial is its own answer, and it arrives before the scanner can open
    # anything: the preview surface is laid out while the prompt is still up.
    # Opening there would report the denial as "no camera" and dismiss the
    # screen underneath the prompt. The scanner instead keeps its state — the
    # permission can still be granted in Settings — and only reports the denial
    # when the user gives up.
    local scanner
    local doc_before
    D logcat -d -s emthin-pad:D >"$log" 2>/dev/null || true
    doc_before=$(last_text_len "$log")
    D shell pm revoke "$PKG" android.permission.CAMERA >/dev/null 2>&1 \
        || { fail "revoke $PKG's camera permission"; stop_wss_fixture; return 1; }
    scanner=$(node_bounds 'text="SCAN QR"') \
        || { fail "Scan QR is missing from the pairing screen"; stop_wss_fixture; return 1; }
    tap_center "$scanner"
    deny_camera_prompt || true
    wait_for_ui 'text="Open settings"' || { fail "a denial did not leave the scanner waiting"; stop_wss_fixture; return 1; }
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'Camera permission denied' "$ui" || ! grep -q 'text="Open settings"' "$ui"; then
        fail "a denial did not leave the scanner waiting on a grant"
        stop_wss_fixture
        return 1
    fi
    pass "a denied permission keeps the scanner up, with the way to grant it"

    # Granting mid-scan is the user leaving for Settings and coming back. That
    # delivers no prompt result, so only the resume can pick the grant up.
    scanner=$(node_bounds 'text="Open settings"') \
        || { fail "the scanner has no way into Settings"; stop_wss_fixture; return 1; }
    tap_center "$scanner"
    sleep 2
    if ! D shell dumpsys activity activities 2>/dev/null | grep -q 'topResumedActivity=.*com.android.settings'; then
        fail "Open settings did not reach this app's system settings page"
        stop_wss_fixture
        return 1
    fi
    pass "the scanner opens this app's settings page"
    D shell pm grant "$PKG" android.permission.CAMERA >/dev/null 2>&1 \
        || { fail "grant the camera permission from settings"; stop_wss_fixture; return 1; }
    D shell input keyevent KEYCODE_BACK >/dev/null 2>&1 || true
    if ! wait_for_log "$log" 'qr permission granted on return'; then
        fail "returning with the grant did not reach the waiting scanner"
        stop_wss_fixture
        return 1
    fi
    if ! wait_for_log "$log" 'gatewayPair phase scan-failed reason no camera'; then
        fail "the scanner kept the grant but never opened the camera"
        stop_wss_fixture
        return 1
    fi
    pass "a grant made in settings is picked up by the scanner that was waiting"

    # Giving up instead of granting reports the denial, and pairing stays usable.
    wait_for_ui 'text="SCAN QR"' || { fail "the Pair screen did not come back after the scan"; stop_wss_fixture; return 1; }
    D shell pm revoke "$PKG" android.permission.CAMERA >/dev/null 2>&1 || true
    ensure_pair_screen "$ui" || { fail "the Pair screen did not come back after the permission was withdrawn"; stop_wss_fixture; return 1; }
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    scanner=$(node_bounds 'text="SCAN QR"') \
        || { fail "Scan QR is gone after the grant round trip"; stop_wss_fixture; return 1; }
    tap_center "$scanner"
    deny_camera_prompt || true
    sleep 1
    # A package the platform has already denied twice is answered without a
    # prompt, so the denial can arrive before any screen to cancel from.
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if scanner=$(node_bounds 'text="Cancel"'); then
        tap_center "$scanner"
    fi
    if ! wait_for_log "$log" 'gatewayPair phase scan-failed reason camera permission denied'; then
        fail "a denied camera permission was not reported as a denial"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'Scanner unavailable' "$ui" || ! grep -q 'camera permission denied' "$ui"; then
        fail "the denial is not reported on the pairing screen"
        stop_wss_fixture
        return 1
    fi
    if ! grep -q 'text="PASTE LINK"' "$ui"; then
        fail "a denied permission left no way to pair by link"
        stop_wss_fixture
        return 1
    fi
    if [ "$(last_text_len "$log")" != "$doc_before" ]; then
        fail "the denial changed the document"
        stop_wss_fixture
        return 1
    fi
    pass "giving up on the permission reports the denial and leaves the document untouched"

    # Grant what the app would ask for, so the next check sees the scanner's own
    # answer instead of a permission prompt.
    D shell pm grant "$PKG" android.permission.CAMERA >/dev/null 2>&1 || true
    local scan
    scan=$(node_bounds 'text="SCAN QR"') \
        || { fail "Scan QR is missing from the pairing screen"; stop_wss_fixture; return 1; }
    tap_center "$scan"
    if ! wait_for_log "$log" 'gatewayPair phase scan-failed'; then
        fail "the scanner neither returned a code nor reported why"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'Scanner unavailable' "$ui"; then
        fail "the scanner failure is not reported on the pairing screen"
        stop_wss_fixture
        return 1
    fi
    if ! grep -q 'text="PASTE LINK"' "$ui"; then
        fail "a failed scan left no way to pair by link"
        stop_wss_fixture
        return 1
    fi
    pass "a scan with no camera is reported and pairing stays usable"

    pair_paste "$link" || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase request host fleet.leo.ts.net'; then
        fail "the pasted link did not open a pairing request"
        stop_wss_fixture
        return 1
    fi
    pass "the pasted link opened a pairing request"

    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'Pending on fleet' "$ui"; then
        fail "the screen does not show the request as pending"
        stop_wss_fixture
        return 1
    fi
    pass "the screen shows the request waiting for the host"

    wss_admin_checked "pair approve" >/dev/null || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase approved host fleet.leo.ts.net'; then
        fail "the approved answer did not arrive"
        stop_wss_fixture
        return 1
    fi
    pass "the approval arrived through the poll"
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'content-desc="fleet.leo, fleet.leo.ts.net"' "$ui"; then
        fail "the approved endpoint is not listed"
        stop_wss_fixture
        return 1
    fi
    pass "the approved endpoint is listed"

    # The Sessions picker is the live catalogue: one `sessions.list` per paired
    # host over the same TLS/WebSocket transport a lease uses, reached through
    # the row the screen just stored (`fixtureConnect` remaps only its origin).
    D shell input keyevent KEYCODE_BACK >/dev/null 2>&1 || true
    sleep 0.5
    if ! tap_toolbar_menu "Sessions and sharing" "Sessions"; then
        fail "open the sessions picker"
        stop_wss_fixture
        return 1
    fi
    if ! wait_for_log "$log" 'sessions phase catalogue'; then
        fail "the picker never fetched a catalogue"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'content-desc="fleet.leo, fleet.leo.ts.net, answered"' "$ui"; then
        fail "the paired host is not shown as answered"
        stop_wss_fixture
        return 1
    fi
    if ! grep -q 'Lease fixture' "$ui"; then
        fail "the host's sessions.list row is missing from the picker"
        stop_wss_fixture
        return 1
    fi
    pass "the picker listed the paired host's sessions from sessions.list"

    # The registry is SharedPreferences state, so it has to survive a launch
    # with no fixture extras at all.
    cold_stop
    D logcat -c >/dev/null 2>&1 || true
    if ! D shell am start -W -n "$ACTIVITY" >/dev/null 2>&1; then
        fail "relaunch the pad"
        stop_wss_fixture
        return 1
    fi
    sleep 3
    if ! tap_toolbar_menu "Sessions and sharing" "Pair gateway"; then
        fail "reopen the pairing screen"
        stop_wss_fixture
        return 1
    fi
    if ! grep -q 'content-desc="fleet.leo, fleet.leo.ts.net"' "$ui"; then
        fail "the paired endpoint did not survive a relaunch"
        stop_wss_fixture
        return 1
    fi
    pass "the paired endpoint survived a relaunch"

    # With no fixture extras the row still points at its tailnet name, which
    # this host cannot resolve: the picker must show that as a host row with
    # Reconnect rather than refusing to list anything.
    D shell input keyevent KEYCODE_BACK >/dev/null 2>&1 || true
    sleep 0.5
    if ! tap_toolbar_menu "Sessions and sharing" "Sessions"; then
        fail "reopen the sessions picker"
        stop_wss_fixture
        return 1
    fi
    if ! wait_for_log "$log" 'sessions phase catalogue'; then
        fail "the picker never answered for the unreachable host"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'content-desc="fleet.leo, fleet.leo.ts.net, ' "$ui"; then
        fail "the unreachable host has no row"
        stop_wss_fixture
        return 1
    fi
    if ! grep -q 'text="Reconnect"' "$ui"; then
        fail "the unreachable host offers no Reconnect"
        stop_wss_fixture
        return 1
    fi
    pass "an unreachable host is a row with Reconnect, not an error"
    D shell input keyevent KEYCODE_BACK >/dev/null 2>&1 || true
    sleep 0.5
    if ! tap_toolbar_menu "Sessions and sharing" "Pair gateway"; then
        fail "reopen the pairing screen for Forget"
        stop_wss_fixture
        return 1
    fi

    bounds=$(node_bounds 'text="Forget"') \
        || { fail "Forget is missing"; stop_wss_fixture; return 1; }
    tap_center "$bounds"
    if ! wait_for_log "$log" 'gatewayPair phase forget host fleet.leo.ts.net'; then
        fail "Forget did not remove the row"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'text="No gateways paired yet."' "$ui"; then
        fail "the registry still lists the forgotten endpoint"
        stop_wss_fixture
        return 1
    fi
    pass "Forget removed the local row"
    stop_wss_fixture
}

# Two paired hosts, each with its own Core ID and its own `sessions.list`
# catalogue, in one picker — and Reconnect that stays per host.
cmd_sessions_hosts() {
    local ui="$OUTDIR/sessions-hosts-ui.xml"
    local log="$OUTDIR/sessions-hosts-logcat.txt"
    local first="https://fleet.leo.ts.net/pair?v=1&core=$PAIR_CORE"
    local second="https://loft.leo.ts.net/pair?v=1&core=$SECOND_CORE"
    local bounds

    say "==> two hosts: pair both -> one merged picker -> Reconnect one"
    mkdir -p "$OUTDIR" || { fail "create $OUTDIR"; return; }
    cmd_clear_data || return
    if ! start_wss_fixture; then stop_wss_fixture; return 1; fi
    if ! pair_launch "$ui"; then
        fail "open the pairing screen"
        stop_wss_fixture
        return 1
    fi

    # Host one, under the fixture's primary identity.
    pair_paste "$first" || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase request host fleet.leo.ts.net'; then
        fail "the first link opened no pairing request"
        stop_wss_fixture
        return 1
    fi
    wss_admin_checked "pair approve" >/dev/null || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase approved host fleet.leo.ts.net'; then
        fail "the first host never approved"
        stop_wss_fixture
        return 1
    fi
    # The approval re-renders the pairing screen in place; node_bounds reads
    # the last dump, so take a fresh one before the second link.
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    pass "the first host paired"

    # Host two, under the fixture's second identity: same process, different
    # Core ID and catalog. The fixture refuses any other ID, so reaching here
    # means the row really carries `SECOND_CORE`.
    pair_paste "$second" loft.leo.ts.net || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase request host loft.leo.ts.net'; then
        fail "the second link opened no pairing request"
        stop_wss_fixture
        return 1
    fi
    wss_admin_checked "pair approve" >/dev/null || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase approved host loft.leo.ts.net'; then
        fail "the second host never approved"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'content-desc="fleet.leo, fleet.leo.ts.net"' "$ui" \
            || ! grep -q 'content-desc="loft.leo, loft.leo.ts.net"' "$ui"; then
        fail "the second row did not land beside the first"
        stop_wss_fixture
        return 1
    fi
    pass "both hosts are paired under their own Core IDs"

    # One picker, both catalogues.
    D shell input keyevent KEYCODE_BACK >/dev/null 2>&1 || true
    sleep 0.5
    if ! tap_toolbar_menu "Sessions and sharing" "Sessions"; then
        fail "open the sessions picker"
        stop_wss_fixture
        return 1
    fi
    if ! wait_for_log "$log" 'sessions phase catalogue hosts 2 rows 2'; then
        fail "the picker did not merge two hosts' catalogues"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'content-desc="fleet.leo, fleet.leo.ts.net, answered"' "$ui" \
            || ! grep -q 'content-desc="loft.leo, loft.leo.ts.net, answered"' "$ui"; then
        fail "one of the two hosts is not shown as answered"
        stop_wss_fixture
        return 1
    fi
    if ! grep -q 'text="Lease fixture ·' "$ui" || ! grep -q 'text="Loft fixture ·' "$ui"; then
        fail "the merged list is missing one host's sessions.list row"
        stop_wss_fixture
        return 1
    fi
    pass "one picker listed both hosts' sessions.list rows"

    # Reconnect the second host only. Its own fetch is traced; the first host
    # must not be dialled again, which would be a fleet-wide refresh.
    bounds=$(node_bounds_nth 'text="Reconnect"' 2) \
        || { fail "the second host has no Reconnect"; stop_wss_fixture; return 1; }
    tap_center "$bounds"
    if ! wait_for_log "$log" 'sessions phase reconnect host loft.leo.ts.net'; then
        fail "tapping Reconnect did not start a per-host fetch"
        stop_wss_fixture
        return 1
    fi
    # The first catalogue fetch logged the same host line, so the assertion is
    # on what comes after the reconnect marker, not on the file as a whole.
    local after="$OUTDIR/sessions-hosts-after-reconnect.txt" tries=0
    while [ "$tries" -lt 40 ]; do
        # A fresh capture each round: $log is a snapshot, not a stream, so
        # reading it again would never see the worker's line.
        D logcat -d -s emthin-pad:D 2>/dev/null | tr -d '\r' > "$log"
        sed -n '/sessions phase reconnect host loft/,$p' "$log" > "$after"
        grep -q 'sessions phase host loft.leo.ts.net rows 1' "$after" && break
        sleep 0.5
        tries=$((tries + 1))
    done
    if ! grep -q 'sessions phase host loft.leo.ts.net rows 1' "$after"; then
        fail "the tapped host's own catalogue was not fetched"
        stop_wss_fixture
        return 1
    fi
    if grep -q 'sessions phase host fleet.leo.ts.net' "$after"; then
        fail "Reconnect refreshed the host that was not tapped"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'text="Lease fixture ·' "$ui" || ! grep -q 'text="Loft fixture ·' "$ui"; then
        fail "the per-host refresh lost one of the two catalogues"
        stop_wss_fixture
        return 1
    fi
    pass "Reconnect refreshed only the tapped host and kept both catalogues"
    stop_wss_fixture
}

# The answers the screen has to report besides approval: a denial, a lapse and
# a Core mismatch. Every round starts from the same empty registry and has to
# leave it empty, and the mismatch is the one the fixture refuses outright
# (409) rather than deciding later.
cmd_pairing_answers() {
    local ui="$OUTDIR/pairing-answers-ui.xml"
    local log="$OUTDIR/pairing-answers-logcat.txt"
    local link="https://fleet.leo.ts.net/pair?v=1&core=$PAIR_CORE"
    local other="https://fleet.leo.ts.net/pair?v=1&core=ffffffffffffffffffffffffffffffff"
    say "==> pairing answers: deny -> expire -> core mismatch"
    mkdir -p "$OUTDIR" || { fail "create $OUTDIR"; return; }
    cmd_clear_data || return
    if ! start_wss_fixture; then stop_wss_fixture; return 1; fi
    if ! pair_launch "$ui"; then
        fail "open the pairing screen"
        stop_wss_fixture
        return 1
    fi
    pass "opened the pairing screen against the fixture"

    pair_paste "$link" || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase request host fleet.leo.ts.net'; then
        fail "the link to deny did not open a request"
        stop_wss_fixture
        return 1
    fi
    wss_admin_checked "pair deny" >/dev/null || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase denied host fleet.leo.ts.net'; then
        fail "a denial was not reported"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'text="Denied' "$ui"; then
        fail "the screen does not say the host refused the device"
        stop_wss_fixture
        return 1
    fi
    if ! grep -q 'text="No gateways paired yet."' "$ui"; then
        fail "a denial stored an endpoint"
        stop_wss_fixture
        return 1
    fi
    pass "a denial is reported and stores nothing"

    # A lapsed request: the fixture forgets it, so the next poll answers 404.
    pair_paste "$link" || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase request host fleet.leo.ts.net'; then
        fail "the link to lapse did not open a request"
        stop_wss_fixture
        return 1
    fi
    wss_admin_checked "pair expire" >/dev/null || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase expired host fleet.leo.ts.net'; then
        fail "a lapsed request was not reported"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'text="Expired' "$ui"; then
        fail "the screen does not say the request lapsed"
        stop_wss_fixture
        return 1
    fi
    if ! grep -q 'text="No gateways paired yet."' "$ui"; then
        fail "a lapsed request stored an endpoint"
        stop_wss_fixture
        return 1
    fi
    pass "a lapsed request is reported and stores nothing"

    # A link naming a different Core than the host approved is refused by the
    # create itself, so no request ever reaches polling.
    pair_paste "$other" || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase core_mismatch host fleet.leo.ts.net'; then
        fail "a Core mismatch was not reported"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'text="Mismatched' "$ui"; then
        fail "the screen does not say the host approved a different Core"
        stop_wss_fixture
        return 1
    fi
    if ! grep -q 'text="No gateways paired yet."' "$ui"; then
        fail "a mismatched Core stored an endpoint"
        stop_wss_fixture
        return 1
    fi
    pass "a Core mismatch is reported and stores nothing"
    stop_wss_fixture
}

# A row approved through the screen is then dialled by a figure. Nothing points
# the lease at the fixture: only the row's origin is remapped to loopback, so
# the registry lookup, the Core ID in `hello` and the generation discovery are
# all the paired path's.
cmd_paired_lease() {
    local ui="$OUTDIR/paired-lease-ui.xml"
    local actions_ui="$OUTDIR/paired-lease-actions-ui.xml"
    local log="$OUTDIR/paired-lease-logcat.txt"
    local link="https://fleet.leo.ts.net/pair?v=1&core=$PAIR_CORE"
    local fixture placed x y w h bounds status first_lease second_lease
    say "==> paired lease: approve a row -> lease -> take -> operator revoke"
    mkdir -p "$OUTDIR" || { fail "create $OUTDIR"; return; }
    cmd_clear_data || return
    if ! start_wss_fixture; then stop_wss_fixture; return 1; fi
    if ! pair_launch "$ui"; then
        fail "open the pairing screen"
        stop_wss_fixture
        return 1
    fi
    pair_paste "$link" || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase request host fleet.leo.ts.net'; then
        fail "the link did not open a pairing request"
        stop_wss_fixture
        return 1
    fi
    wss_admin_checked "pair approve" >/dev/null || { stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayPair phase approved host fleet.leo.ts.net'; then
        fail "the approval did not arrive"
        stop_wss_fixture
        return 1
    fi
    cmd_uidump "$ui" || { stop_wss_fixture; return 1; }
    if ! grep -q 'content-desc="fleet.leo, fleet.leo.ts.net"' "$ui"; then
        fail "the approval did not store a row"
        stop_wss_fixture
        return 1
    fi
    pass "the pairing screen stored an approved row"

    fixture=$(printf '%s\n' \
        "#1 Build log #2 \\app(#1, #2, 640, 400, terminal: \"hc://$PAIR_CORE/t7\")")
    cold_stop
    D logcat -c >/dev/null 2>&1 || true
    if ! D shell am start -W -n "$ACTIVITY" \
            --ez emthin.fixtureVtScene true \
            --es emthin.fixtureConnect "wss://127.0.0.1:$WSS_PORT" \
            --es emthin.fixtureGatewayCaPem "$WSS_CA_B64" \
            --ef emthin.fixtureViewZoom 1.0 \
            --es emthin.fixtureB64 "$(printf '%s' "$fixture" | base64 -w0)" \
            >/dev/null 2>&1; then
        fail "launch the figure against the paired row"
        stop_wss_fixture
        return 1
    fi
    pass "launched the figure with only the paired row to dial"
    wait_for_log "$log" 'vtScene phase ready' \
        || { fail "the paired-lease scene did not start"; stop_wss_fixture; return 1; }
    if ! wait_for_log "$log" 'gatewayLease phase connect .*secure true'; then
        fail "the paired row did not connect over TLS"
        stop_wss_fixture
        return 1
    fi
    if ! wait_for_log "$log" 'gatewayLease phase connect .*generation discover'; then
        fail "the paired row did not discover its generation"
        stop_wss_fixture
        return 1
    fi
    wait_for_log "$log" 'gatewayLease phase ready mode watch' \
        || { fail "the paired row did not become a lease"; stop_wss_fixture; return 1; }
    wait_for_log "$log" 'gatewayLease resize cols 80 rows 24' \
        || { fail "the paired lease did not publish its grid"; stop_wss_fixture; return 1; }
    pass "the figure leased through the paired row"
    if ! wait_for_log "$log" 'gatewayLease data .*gateway lease'; then
        fail "the paired lease sent no DATA"
        stop_wss_fixture
        return 1
    fi
    pass "DATA arrived over the paired row"

    # Watch is only the row's first half: the same row has to carry control,
    # and the host still owns the revoke that ends it.
    wait_for_log "$log" 'vtScene phase placed' \
        || { fail "the paired-lease scene did not publish its placed rect"; stop_wss_fixture; return 1; }
    placed=$(sed -n 's/.*vtScene phase placed x \([-0-9]*\) y \([-0-9]*\) w \([0-9]*\) h \([0-9]*\).*/\1 \2 \3 \4/p' "$log" | tail -1)
    read -r x y w h <<EOF
$placed
EOF
    if [ -z "$x" ] || [ -z "$y" ] || [ -z "$w" ] || [ -z "$h" ]; then
        fail "could not parse the paired-lease scene rect from $log"
        stop_wss_fixture
        return 1
    fi
    D shell input tap "$((x + w / 2))" "$((y + h / 2))" >/dev/null 2>&1
    wait_for_log "$log" 'terminalInput phase focus' \
        || { fail "tapping the paired scene did not focus it"; stop_wss_fixture; return 1; }

    if ! cmd_uidump "$actions_ui" >/dev/null 2>&1 || ! bounds=$(node_bounds 'text="Take"'); then
        fail "the paired lease has no Take action"
        stop_wss_fixture
        return 1
    fi
    tap_center "$bounds"
    wait_for_log "$log" 'gatewayLease phase request control' \
        || { fail "Take on the paired row did not request control"; stop_wss_fixture; return 1; }
    wait_for_log "$log" 'gatewayLease phase ready mode control' \
        || { fail "Take on the paired row did not become a controller"; stop_wss_fixture; return 1; }
    first_lease=$(sed -n 's/.*gatewayLease phase ready mode control lease \([^ ]*\).*/\1/p' "$log" | tail -1)
    if [ -z "$first_lease" ] || [ "$first_lease" = none ]; then
        fail "the paired controller lease was not named in the log"
        stop_wss_fixture
        return 1
    fi
    pass "Take on the paired row acquired a controller lease"

    D shell input text 'pairedtake' >/dev/null 2>&1
    wait_for_log "$log" 'gatewayLease input bytes' \
        || { fail "controller input did not cross the paired row"; stop_wss_fixture; return 1; }
    wait_for_log "$log" 'gatewayLease data .*echo:' \
        || { fail "the paired row did not echo the controller input"; stop_wss_fixture; return 1; }
    status=$(wss_admin status)
    if ! printf '%s' "$status" | grep -q 'pairedtake'; then
        fail "the paired gateway did not receive the complete controller input"
        stop_wss_fixture
        return 1
    fi
    pass "controller input crossed the paired row and returned as DATA"

    wss_admin_checked revoke >/dev/null || { stop_wss_fixture; return 1; }
    wait_for_log "$log" 'gatewayLease closed reason .*revoked' \
        || { fail "operator revoke did not close the paired lease"; stop_wss_fixture; return 1; }
    pass "operator revoke closed the paired lease"

    D logcat -c >/dev/null 2>&1 || true
    D shell input text 'after-revoke' >/dev/null 2>&1
    wait_for_log "$log" 'terminalInput blocked conflict' \
        || { fail "the revoked paired controller still accepted input"; stop_wss_fixture; return 1; }
    status=$(wss_admin status)
    if printf '%s' "$status" | grep -q 'after-revoke'; then
        fail "revoked input reached the paired gateway"
        stop_wss_fixture
        return 1
    fi
    pass "revoked paired lease rejected input before the transport"

    # Revocation ends the generation, not the row. A second Take must attach
    # again and come back with a lease the gateway names differently; a reused
    # generation would keep the old name.
    D logcat -c >/dev/null 2>&1 || true
    if ! cmd_uidump "$actions_ui" >/dev/null 2>&1 || ! bounds=$(node_bounds 'text="Take"'); then
        fail "the paired lease has no Take action after the revoke"
        stop_wss_fixture
        return 1
    fi
    tap_center "$bounds"
    wait_for_log "$log" 'gatewayLease phase ready mode control' \
        || { fail "a second Take did not acquire a fresh lease"; stop_wss_fixture; return 1; }
    second_lease=$(sed -n 's/.*gatewayLease phase ready mode control lease \([^ ]*\).*/\1/p' "$log" | tail -1)
    if [ -z "$second_lease" ] || [ "$second_lease" = none ]; then
        fail "the second Take did not name its lease"
        stop_wss_fixture
        return 1
    fi
    if [ "$second_lease" = "$first_lease" ]; then
        fail "the second Take reused the revoked lease '$first_lease'"
        stop_wss_fixture
        return 1
    fi
    pass "the second Take acquired a lease the gateway named differently"
    D shell input text 'retakepair' >/dev/null 2>&1
    wait_for_log "$log" 'gatewayLease data .*echo:' \
        || { fail "the fresh lease did not echo its input"; stop_wss_fixture; return 1; }
    status=$(wss_admin status)
    if ! printf '%s' "$status" | grep -q 'retakepair'; then
        fail "the fresh lease did not reach the gateway"
        stop_wss_fixture
        return 1
    fi
    pass "the fresh lease carried input through the paired row"

    # The app refuses revoked input before the transport, so none of the
    # checks above can say what the gateway does with a controller's bytes.
    # A raw client that keeps a revoked controller channel can: the gateway
    # must answer those bytes with not_controller and must not echo them.
    local probe_log="$OUTDIR/paired-lease-probe.txt" attempts rc
    if [ ! -f "$WSS_DIR/cert.pem" ]; then
        fail "the paired-lease probe has no fixture certificate to trust"
        stop_wss_fixture
        return 1
    fi
    wss_admin_checked revoke >/dev/null || { stop_wss_fixture; return 1; }
    python3 "$SCRIPT_DIR/../fixtures/herdr-gateway-v1/stale_lease_probe.py" \
        --listen "127.0.0.1:$WSS_PORT" --ca "$WSS_DIR/cert.pem" \
        >"$probe_log" 2>&1 &
    WSS_PROBE_PID=$!
    attempts=0
    while [ "$attempts" -lt 150 ]; do
        grep -q '^probe: control channel' "$probe_log" 2>/dev/null && break
        kill -0 "$WSS_PROBE_PID" 2>/dev/null || break
        sleep 0.1
        attempts=$((attempts + 1))
    done
    if ! grep -q '^probe: control channel' "$probe_log" 2>/dev/null; then
        say "    probe output: $(tail -5 "$probe_log" 2>/dev/null | tr '\n' ' | ')"
        fail "the raw client did not get a controller lease"
        stop_wss_fixture
        return 1
    fi
    pass "a raw client took the lease after the operator revoke"

    wss_admin_checked revoke >/dev/null || { stop_wss_fixture; return 1; }
    attempts=0
    while [ "$attempts" -lt 200 ]; do
        kill -0 "$WSS_PROBE_PID" 2>/dev/null || break
        sleep 0.1
        attempts=$((attempts + 1))
    done
    if kill -0 "$WSS_PROBE_PID" 2>/dev/null; then
        say "    probe output: $(tail -5 "$probe_log" 2>/dev/null | tr '\n' ' | ')"
        fail "the raw client hung after the revoke"
        stop_wss_fixture
        return 1
    fi
    if wait "$WSS_PROBE_PID"; then rc=0; else rc=$?; fi
    WSS_PROBE_PID=""
    if [ "$rc" -ne 0 ]; then
        say "    probe output: $(tail -5 "$probe_log" 2>/dev/null | tr '\n' ' | ')"
        fail "the gateway did not refuse the revoked controller's bytes"
        stop_wss_fixture
        return 1
    fi
    if ! grep -q '^probe: stale controller refused not_controller, no echo' "$probe_log"; then
        say "    probe output: $(tail -5 "$probe_log" 2>/dev/null | tr '\n' ' | ')"
        fail "the probe did not report the gateway's refusal"
        stop_wss_fixture
        return 1
    fi
    status=$(wss_admin status)
    if printf '%s' "$status" | grep -q 'stalecontroller'; then
        fail "the gateway recorded bytes from the revoked controller"
        stop_wss_fixture
        return 1
    fi
    if ! printf '%s' "$status" | grep -q '"active":null'; then
        fail "the gateway still reports an active lease after the raw revoke"
        stop_wss_fixture
        return 1
    fi
    pass "the gateway refused the revoked controller's bytes and kept none"
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

# A network change during a live lease, as the user meets one: the host goes
# unreachable while the pad is controlling it, and comes back. The scripted
# gateway is reached through `adb reverse`, so killing its process is what an
# airplane-mode toggle looks like to the app — established sockets break and
# new connects are refused — without touching this lane's own adb transport
# (which rides the same container network, so the real toggle is not drivable
# here). The pad must keep the document and its figure, report the loss instead
# of pretending to still control, and reach the host again on a fresh take.
cmd_network_change() {
    local log="$OUTDIR/network-change-logcat.txt"
    local ui="$OUTDIR/network-change-ui.xml"
    local fixture placed x y w h bounds first_lease status before after
    say "==> network change: control -> host lost -> host back -> take again"
    mkdir -p "$OUTDIR" || { fail "create $OUTDIR"; return; }
    cmd_clear_data || return
    fixture=$(printf '%s\n' \
        '#1 Build log #2 \app(#1, #2, 640, 400, terminal: "hc://0123456789abcdef0123456789abcdef/t7")')
    if ! start_lease_fixture; then
        stop_lease_fixture
        return
    fi
    if ! lease_launch "$fixture"; then
        fail "launch the lease fixture for the network change"
        stop_lease_fixture
        return
    fi
    wait_for_log "$log" 'gatewayLease phase ready mode watch' \
        || { fail "the lease never became a watch"; stop_lease_fixture; return; }
    wait_for_log "$log" 'gatewayLease data .*gateway lease' \
        || { fail "the lease sent no DATA"; stop_lease_fixture; return; }
    wait_for_log "$log" 'vtScene phase placed' \
        || { fail "the lease scene published no rect"; stop_lease_fixture; return; }
    # The first rect arrives while the page is still scrolling to its caption,
    # so the settled one is read from the same history a moment later.
    sleep 2
    D logcat -d -s emthin-pad:D 2>/dev/null | tr -d '\r' >"$log"
    placed=$(sed -n 's/.*vtScene phase placed x \([-0-9]*\) y \([-0-9]*\) w \([0-9]*\) h \([0-9]*\).*/\1 \2 \3 \4/p' "$log" | tail -1)
    read -r x y w h <<EOF
$placed
EOF
    if [ -z "$x" ] || [ -z "$w" ]; then
        fail "could not parse the lease scene rect from $log"
        stop_lease_fixture
        return
    fi
    D shell input tap "$((x + w / 2))" "$((y + h / 2))" >/dev/null 2>&1
    wait_for_log "$log" 'terminalInput phase focus' \
        || { fail "the lease scene did not focus"; stop_lease_fixture; return; }
    if ! cmd_uidump "$ui" >/dev/null 2>&1 || ! bounds=$(node_bounds 'text="Take"'); then
        fail "the lease offers no Take action"
        stop_lease_fixture
        return
    fi
    tap_center "$bounds"
    wait_for_log "$log" 'gatewayLease phase ready mode control' \
        || { fail "Take did not become a controller"; stop_lease_fixture; return; }
    first_lease=$(sed -n 's/.*gatewayLease phase ready mode control lease \([^ ]*\).*/\1/p' "$log" | tail -1)
    if [ -z "$first_lease" ] || [ "$first_lease" = none ]; then
        fail "the controller lease was not named in the log"
        stop_lease_fixture
        return
    fi
    D shell input text 'netbefore' >/dev/null 2>&1
    wait_for_log "$log" 'gatewayLease data .*echo:' \
        || { fail "the controller's input did not come back as DATA"; stop_lease_fixture; return; }
    status=$(lease_admin status)
    printf '%s' "$status" | grep -q 'netbefore' \
        || { fail "the gateway never received the controller input"; stop_lease_fixture; return; }
    pass "a live control lease carried input before the network changed"

    # wait_for_log rewrites $log from the live buffer, so the length below and
    # the loss below are read from the same history.
    D logcat -d -s emthin-pad:D 2>/dev/null | tr -d '\r' >"$log"
    before=$(last_text_len "$log")
    # The host disappears: its process dies, so its sockets break and its port
    # refuses. The reverse rule stays, exactly as the network would come back.
    kill "$LEASE_PID" >/dev/null 2>&1 || true
    wait "$LEASE_PID" 2>/dev/null || true
    LEASE_PID=""
    if ! wait_for_log "$log" 'gatewayLease error kind'; then
        fail "losing the host was not reported to the lease"
        stop_lease_fixture
        return
    fi
    if ! wait_for_log "$log" 'gatewayLease phase retry host .* failed'; then
        fail "the lost lease did not consult the registry once"
        stop_lease_fixture
        return
    fi
    pass "losing the host is reported, after one registry retry"

    if ! D shell pidof "$PKG" >/dev/null 2>&1; then
        fail "the pad died with the lease instead of reporting it"
        stop_lease_fixture
        return
    fi
    after=$(last_text_len "$log")
    if [ -n "$before" ] && [ "$after" != "$before" ]; then
        fail "the network change altered the document (textLen $before -> $after)"
        stop_lease_fixture
        return
    fi
    if ! cmd_uidump "$ui" >/dev/null 2>&1 || ! grep -q 'text="Take"' "$ui"; then
        fail "a lost lease does not offer control again"
        stop_lease_fixture
        return
    fi
    pass "the pad kept the document and offers control again after the loss"

    # The host comes back, as it does when the network does: a new process with
    # no memory of the old generation.
    if ! start_lease_fixture; then
        stop_lease_fixture
        return
    fi
    if ! cmd_uidump "$ui" >/dev/null 2>&1 || ! bounds=$(node_bounds 'text="Take"'); then
        fail "no Take action after the host came back"
        stop_lease_fixture
        return
    fi
    tap_center "$bounds"
    if ! wait_for_new_log "$log" 'gatewayLease phase ready mode control'; then
        fail "control did not come back when the host did"
        stop_lease_fixture
        return
    fi
    # The restarted fixture counts its leases from one again, so a lease name
    # cannot say whether this generation is new. Only the new process's own
    # record can: its input log starts empty.
    status=$(lease_admin status)
    if printf '%s' "$status" | grep -q 'netbefore'; then
        fail "the restarted host remembers the old generation's input"
        stop_lease_fixture
        return
    fi
    pass "control came back against a host with no memory of the old lease"
    # Losing the lease released the terminal's keyboard focus, so the retaken
    # scene is tapped again before its input is expected to arrive.
    D logcat -d -s emthin-pad:D 2>/dev/null | tr -d '\r' >"$log"
    placed=$(sed -n 's/.*vtScene phase placed x \([-0-9]*\) y \([-0-9]*\) w \([0-9]*\) h \([0-9]*\).*/\1 \2 \3 \4/p' "$log" | tail -1)
    read -r x y w h <<EOF
$placed
EOF
    D shell input tap "$((x + w / 2))" "$((y + h / 2))" >/dev/null 2>&1
    sleep 1
    D shell input text 'netafter' >/dev/null 2>&1
    status=""
    local tries=0
    while [ "$tries" -lt 30 ]; do
        status=$(lease_admin status)
        printf '%s' "$status" | grep -q 'netafter' && break
        sleep 0.2
        tries=$((tries + 1))
    done
    if ! printf '%s' "$status" | grep -q 'netafter'; then
        fail "input did not reach the host once it came back"
        stop_lease_fixture
        return
    fi
    if printf '%s' "$status" | grep -q 'netbefore'; then
        fail "the restarted host received the old generation's input"
        stop_lease_fixture
        return
    fi
    pass "input reached the restarted host and none of the old generation's did"
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
    cmd_wayland_gpu
    cmd_network_change
    if python3 -c 'from PIL import Image' >/dev/null 2>&1; then
        cmd_math_preview
    else
        say "info math-preview: Python Pillow unavailable; skipped"
    fi
    local default_mm0web="$SCRIPT_DIR/../../unfer/target/debug/mm0web"
    local default_mock="$SCRIPT_DIR/../../unfer/mm0web/tests/mock_zenodo.py"
    if [ "${LANE_PROOF:-0}" = "1" ] \
        || { [ -x "${MM0WEB_BIN:-$default_mm0web}" ] \
             && [ -r "${MOCK_ZENODO:-$default_mock}" ] \
             && command -v mm0-rs >/dev/null 2>&1 \
             && command -v mm0-c >/dev/null 2>&1; }; then
        cmd_proof_zenodo
    else
        say "info proof-zenodo: mm0web/mock_zenodo/mm0 checkers unavailable; skipped"
    fi
    # Opt in with LANE_WSS=1: these steps need an APK built from the transport
    # sources plus python3 for the fixtures, so a default run against a plain
    # build would only fail on a binary that predates them.
    if [ "${LANE_WSS:-0}" = "1" ]; then
        cmd_pairing
        cmd_sessions_hosts
        cmd_pairing_answers
        cmd_paired_lease
        cmd_enroll
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

cmd_wayland_gpu() {
    local log="$OUTDIR/wayland-gpu-logcat.txt"
    say "==> wayland-gpu: app-wide GLES3/Vulkan compute & embedded Wayland compositor"
    if [ "$PKG" != "$TEST_PACKAGE" ]; then
        fail "wayland-gpu requires $TEST_PACKAGE"
        return
    fi
    mkdir -p "$OUTDIR" || { fail "create wayland-gpu output directory"; return; }
    D shell am force-stop "$PKG" >/dev/null 2>&1 || true
    D logcat -c >/dev/null 2>&1 || true
    local fixture_b64
    fixture_b64=$(printf '#1 GPU demo #2 \\app(#1, #2, 320, 200, "gpu-demo")' | base64 -w0)
    if D shell am start -W -n "$ACTIVITY" \
            --ez emthin.waylandGpuSmoke true \
            --es emthin.fixtureB64 "$fixture_b64" >/dev/null 2>&1; then
        pass "launched wayland-gpu smoke fixture"
    else
        fail "launch wayland-gpu smoke fixture"
        return
    fi
    wait_for_log "$log" 'waylandGpu gpuRecovered: ' \
        || { fail "wayland-gpu smoke did not finish CPU fallback / GPU recovery"; return; }
    if grep -q 'waylandGpu smoke failed' "$log"; then
        fail "wayland-gpu smoke reported an exception"
        return
    fi
    if python3 - "$log" <<'PY'
import sys
log = open(sys.argv[1]).read()
checks = [
    ("capabilities", "waylandGpu capabilities:", ['"hasGles3":true', '"hasEglAndroidImageNativeBuffer":true', '"hasVulkan":true']),
    ("compute", "waylandGpu compute:", ['"ok":true', '"sampleOk":true', '"ahbEglOk":true', '"ahbVulkanOk":true']),
    ("appCompute", "waylandGpu appCompute:", ['"ok":true', '"usedGpu":true', '"reluOk":true', '"spirvOk":true']),
    ("appRender", "waylandGpu appRender:", ['"rendered":true', '"backend":"egl-gles3"']),
    ("client", "waylandGpu client:", ['"ok":true']),
    ("frame", "waylandGpu frame:", ['"rendered":true']),
    ("cpuFallbackRender", "waylandGpu cpuFallbackRender:", ['"rendered":true', '"mode":"cpu-shm"', '"backend":"cpu-software"']),
    ("cpuFallbackWayland", "waylandGpu cpuFallbackWayland:", ['"rendered":true', '"backend":"cpu-software"']),
    ("cpuFallbackCompute", "waylandGpu cpuFallbackCompute:", ['"ok":true', '"usedGpu":false', '"backend":"cpu-fallback"']),
    ("gpuRecovered", "waylandGpu gpuRecovered:", ['"rendered":true', '"backend":"egl-gles3"']),
]
missing = []
for name, prefix, needles in checks:
    line = next((l for l in log.splitlines() if prefix in l), None)
    if not line:
        missing.append(f"{name}: line not found")
        continue
    for needle in needles:
        if needle not in line:
            missing.append(f"{name}: missing {needle} in {line}")
if missing:
    sys.exit("\n".join(missing))
PY
    then
        pass "app-wide GPU compute/render, CPU fallback, and GPU recovery verified on device"
    else
        fail "wayland-gpu assertions failed (see $log)"
    fi
}

cmd_proof_zenodo() {
    say "==> proof-zenodo: MM0 verify/reject, Zenodo DOI poll, insert + Undo, no-key refusal"
    if [ "$PKG" != "$TEST_PACKAGE" ]; then
        fail "proof-zenodo requires $TEST_PACKAGE"
        return
    fi
    local mm0web="${MM0WEB_BIN:-$SCRIPT_DIR/../../unfer/target/debug/mm0web}"
    local mock="${MOCK_ZENODO:-$SCRIPT_DIR/../../unfer/mm0web/tests/mock_zenodo.py}"
    if [ ! -x "$mm0web" ]; then
        fail "mm0web binary not executable at $mm0web"
        return
    fi
    if [ ! -r "$mock" ]; then
        fail "mock_zenodo.py not readable at $mock"
        return
    fi
    if ! command -v mm0-rs >/dev/null 2>&1 || ! command -v mm0-c >/dev/null 2>&1; then
        fail "mm0-rs and mm0-c must be on PATH for proof-zenodo"
        return
    fi
    if ! command -v curl >/dev/null 2>&1; then
        fail "curl is required for proof-zenodo"
        return
    fi
    mkdir -p "$OUTDIR" || { fail "create proof-zenodo output directory"; return; }

    local work mock_port=18931 web_port=18932
    work=$(mktemp -d "${TMPDIR:-/tmp}/emthin-proof-zenodo.XXXXXX")
    local mock_pid="" web_pid=""
    local log="$OUTDIR/proof-zenodo-logcat.txt"
    local ui="$OUTDIR/proof-zenodo-ui.xml"

    cleanup_proof_zenodo() {
        [ -n "${web_pid:-}" ] && kill "$web_pid" 2>/dev/null || true
        [ -n "${mock_pid:-}" ] && kill "$mock_pid" 2>/dev/null || true
        wait "${web_pid:-}" 2>/dev/null || true
        wait "${mock_pid:-}" 2>/dev/null || true
        D reverse --remove "tcp:$web_port" >/dev/null 2>&1 || true
        rm -rf "$work"
    }

    start_mm0web_for_proof() {
        local api_key="$1"
        [ -n "${web_pid:-}" ] && kill "$web_pid" 2>/dev/null || true
        [ -n "${web_pid:-}" ] && wait "$web_pid" 2>/dev/null || true
        web_pid=""
        env \
            MM0WEB_ZENODO_BASE="http://127.0.0.1:$mock_port/api" \
            ZENODO_API_KEY="$api_key" \
            MM0WEB_PUBLIC_URL="http://127.0.0.1:$web_port" \
            MM0WEB_RATE=200 \
            MM0WEB_DAILY_CAP=50 \
            "$mm0web" --bind "127.0.0.1:$web_port" --state "$work/state" >"$work/web.log" 2>&1 &
        web_pid=$!
        local i=0
        while [ "$i" -lt 40 ]; do
            if curl -sS -m 2 "http://127.0.0.1:$web_port/healthz" 2>/dev/null \
                    | grep -q 'checker: ready'; then
                return 0
            fi
            sleep 0.25
            i=$((i + 1))
        done
        return 1
    }

    mock_stat() {
        python3 - "$mock_port" "$1" <<'PY'
import json, sys, urllib.request
with urllib.request.urlopen(f"http://127.0.0.1:{sys.argv[1]}/__stats", timeout=3) as r:
    data = json.load(r)
print(data.get(sys.argv[2], ""))
PY
    }

    env MOCK_PORT="$mock_port" python3 "$mock" >"$work/mock.log" 2>&1 &
    mock_pid=$!
    local tries=0
    while [ "$tries" -lt 30 ]; do
        if curl -sS -m 2 -o /dev/null "http://127.0.0.1:$mock_port/__stats" 2>/dev/null; then
            break
        fi
        sleep 0.2
        tries=$((tries + 1))
    done
    if [ "$tries" -ge 30 ]; then
        fail "mock_zenodo.py did not come up"
        cleanup_proof_zenodo
        return
    fi
    if ! start_mm0web_for_proof "mock-key"; then
        fail "mm0web did not become ready (/healthz)"
        cleanup_proof_zenodo
        return
    fi
    if ! D reverse "tcp:$web_port" "tcp:$web_port" >/dev/null 2>&1; then
        fail "adb reverse tcp:$web_port failed"
        cleanup_proof_zenodo
        return
    fi
    pass "started mock_zenodo + mm0web over adb reverse"

    # Case 1: launch a math note, then reject an invalid MM0 pair in ProofClientActivity.
    cmd_clear_data || { cleanup_proof_zenodo; return; }
    local note_b64 spec_b64 bad_b64 nat_b64 before_len after_len bounds
    note_b64=$(printf 'Proof note\n\n$x^2 + y^2 = z^2$\n\nStatus below:\n' | base64 -w0)
    spec_b64=$(printf 'sort Prop;\n' | base64 -w0)
    bad_b64=$(printf 'sort Prop;\nsort Nope;\n' | base64 -w0)
    nat_b64=$(printf 'sort Nat;\n' | base64 -w0)

    D logcat -c >/dev/null 2>&1 || true
    if ! D shell am start -W -n "$ACTIVITY" --es emthin.fixtureB64 "$note_b64" >/dev/null 2>&1; then
        fail "launch math+proof note"
        cleanup_proof_zenodo
        return
    fi
    wait_for_log "$log" 'applyChunk total 1 pages 1 start 0' \
        || { fail "math+proof note did not render"; cleanup_proof_zenodo; return; }
    before_len=$(last_text_len "$log")
    if [ -z "$before_len" ] || [ "$before_len" -le 0 ]; then
        fail "could not read initial note length"
        cleanup_proof_zenodo
        return
    fi
    pass "rendered math note (textLen $before_len)"

    D logcat -c >/dev/null 2>&1 || true
    if ! D shell am start -W -n "$PKG/com.emthin.pad.ProofClientActivity" \
        --ez emthin.proof_reset true \
        --es emthin.proof_base "http://127.0.0.1:$web_port" \
        --es emthin.proof_spec_b64 "$spec_b64" \
        --es emthin.proof_text_b64 "$bad_b64" >/dev/null 2>&1; then
        fail "launch ProofClientActivity with invalid pair"
        cleanup_proof_zenodo
        return
    fi
    wait_for_ui 'text="Verify"' 20 "$ui" \
        || { fail "Verify button did not appear"; cleanup_proof_zenodo; return; }
    bounds=$(node_bounds 'text="Verify"')
    tap_center "$bounds"
    wait_for_log "$log" 'proof phase result kind REJECTED verified false' \
        || { fail "invalid pair was not rejected"; cleanup_proof_zenodo; return; }
    pass "invalid MM0 pair rejected with verified=false"
    cmd_uidump "$ui" >/dev/null || { cleanup_proof_zenodo; return; }
    if grep -q 'rejected at stage mm0-' "$ui" \
        && ! grep -q '\$ mm0-rs compile' "$ui" \
        && grep -o '<node[^>]*text="Insert into note"[^>]*>' "$ui" | grep -q 'enabled="false"'; then
        pass "rejection diagnostic shown (without command banner) and Insert stays disabled"
    else
        fail "rejection UI state or disabled Insert button missing"
        cleanup_proof_zenodo
        return
    fi
    if [ "$(mock_stat deposits)" = "0" ] && [ "$(mock_stat publishes)" = "0" ]; then
        pass "rejected proof made zero Zenodo deposits"
    else
        fail "rejected proof touched mock Zenodo"
        cleanup_proof_zenodo
        return
    fi

    # Case 2: valid pair verify, cancel confirmation, confirm via MainActivity toolbar launch,
    # poll to published + DOI, insert into math note, and Undo back to exact before_len.
    D logcat -c >/dev/null 2>&1 || true
    if ! D shell am start -W -n "$PKG/com.emthin.pad.ProofClientActivity" \
        --ez emthin.proof_reset true \
        --es emthin.proof_base "http://127.0.0.1:$web_port" \
        --es emthin.proof_spec_b64 "$spec_b64" \
        --es emthin.proof_text_b64 "$spec_b64" >/dev/null 2>&1; then
        fail "launch ProofClientActivity with valid pair"
        cleanup_proof_zenodo
        return
    fi
    wait_for_ui 'text="Verify"' 20 "$ui" \
        || { fail "Verify button did not appear for valid pair"; cleanup_proof_zenodo; return; }
    bounds=$(node_bounds 'text="Verify"')
    tap_center "$bounds"
    wait_for_log "$log" 'proof phase result kind ACCEPTED verified true' \
        || { fail "valid pair did not verify"; cleanup_proof_zenodo; return; }
    pass "valid MM0 pair verified"

    bounds=$(node_bounds 'text="Publish to Zenodo"') \
        || { fail "Publish to Zenodo button missing"; cleanup_proof_zenodo; return; }
    tap_center "$bounds"
    wait_for_ui 'text="Publish to Zenodo?"' 20 "$ui" \
        || { fail "publish confirmation dialog did not appear"; cleanup_proof_zenodo; return; }
    bounds=$(node_bounds 'text="Cancel"')
    tap_center "$bounds"
    wait_for_log "$log" 'proof phase publish-cancelled' \
        || { fail "cancelling publish was not logged"; cleanup_proof_zenodo; return; }
    if [ "$(mock_stat deposits)" = "0" ]; then
        pass "cancelled publish left mock Zenodo untouched"
    else
        fail "cancelled publish created a Zenodo deposit"
        cleanup_proof_zenodo
        return
    fi

    # Return to the existing MainActivity and re-open ProofClientActivity from
    # the toolbar so startActivityForResult + document binding + persisted
    # fixture pair are exercised.
    D shell am start -W --activity-clear-top --activity-single-top -n "$ACTIVITY" >/dev/null 2>&1 || true
    wait_for_ui 'content-desc="More actions"' 20 "$ui" \
        || { fail "did not return to MainActivity"; cleanup_proof_zenodo; return; }
    if ! tap_toolbar_menu "Sessions and sharing" "MM0 proof"; then
        fail "could not open MM0 proof from Sessions and sharing menu"
        cleanup_proof_zenodo
        return
    fi
    wait_for_ui 'text="Publish to Zenodo"' 20 "$ui" \
        || { fail "ProofClientActivity did not open from menu"; cleanup_proof_zenodo; return; }
    D logcat -c >/dev/null 2>&1 || true
    bounds=$(node_bounds 'text="Publish to Zenodo"')
    tap_center "$bounds"
    wait_for_ui 'text="Publish to Zenodo?"' 20 "$ui" \
        || { fail "publish confirmation dialog missing from toolbar launch"; cleanup_proof_zenodo; return; }
    bounds=$(node_bounds 'text="Publish"')
    tap_center "$bounds"
    wait_for_log "$log" 'proof phase result kind QUEUED' \
        || { fail "publish did not queue on mm0web"; cleanup_proof_zenodo; return; }
    wait_for_log "$log" 'proof phase status id .* state published doi 10\.[0-9][0-9]*/zenodo\.' \
        || { fail "background poll did not reach published DOI"; cleanup_proof_zenodo; return; }
    pass "published to mock Zenodo and polled to settled DOI"
    if [ "$(mock_stat deposits)" = "1" ] && [ "$(mock_stat publishes)" = "1" ]; then
        pass "mock Zenodo recorded exactly 1 deposit and 1 publish"
    else
        fail "unexpected mock Zenodo stats after publish"
        cleanup_proof_zenodo
        return
    fi

    cmd_uidump "$ui" >/dev/null || { cleanup_proof_zenodo; return; }
    if grep -Eq 'https://doi\.org/10\.[0-9]+/zenodo\.' "$ui"; then
        pass "DOI link visible in ProofClientActivity"
    else
        fail "DOI link missing from ProofClientActivity UI"
        cleanup_proof_zenodo
        return
    fi

    D logcat -c >/dev/null 2>&1 || true
    bounds=$(node_bounds 'text="Insert into note"') \
        || { fail "Insert into note button missing"; cleanup_proof_zenodo; return; }
    tap_center "$bounds"
    wait_for_log "$log" 'proof inserted .* state published' \
        || { fail "MainActivity did not insert proof result"; cleanup_proof_zenodo; return; }
    wait_for_log "$log" 'applyState .*textLen ' \
        || { fail "MainActivity did not sync inserted proof result"; cleanup_proof_zenodo; return; }
    after_len=$(last_text_len "$log")
    if [ -n "$after_len" ] && [ "$after_len" -gt "$before_len" ]; then
        pass "proof ID and DOI link inserted into math note ($before_len -> $after_len)"
    else
        fail "note length did not increase after proof insertion"
        cleanup_proof_zenodo
        return
    fi

    wait_for_ui 'text="Undo"' 20 "$ui" \
        || { fail "Undo button did not reappear on MainActivity"; cleanup_proof_zenodo; return; }
    if ! grep -o '<node[^>]*text="Undo"[^>]*>' "$ui" | grep -q 'enabled="true"'; then
        fail "Undo button is not enabled after proof insertion"
        cleanup_proof_zenodo
        return
    fi
    D logcat -c >/dev/null 2>&1 || true
    bounds=$(node_bounds 'text="Undo"') \
        || { fail "Undo button bounds missing"; cleanup_proof_zenodo; return; }
    tap_center "$bounds"
    wait_for_log "$log" "applyState .*textLen $before_len" \
        || { fail "Undo did not restore note length $before_len"; cleanup_proof_zenodo; return; }
    pass "Undo restored the math note to textLen $before_len"

    # Case 3: missing Zenodo key on backend is reported honestly (NOT_PUBLISHED).
    if ! start_mm0web_for_proof ""; then
        fail "mm0web restart without ZENODO_API_KEY failed"
        cleanup_proof_zenodo
        return
    fi
    D logcat -c >/dev/null 2>&1 || true
    if ! D shell am start -W -n "$PKG/com.emthin.pad.ProofClientActivity" \
        --ez emthin.proof_reset true \
        --es emthin.proof_base "http://127.0.0.1:$web_port" \
        --es emthin.proof_spec_b64 "$nat_b64" \
        --es emthin.proof_text_b64 "$nat_b64" >/dev/null 2>&1; then
        fail "launch ProofClientActivity for no-key test"
        cleanup_proof_zenodo
        return
    fi
    wait_for_ui 'text="Publish to Zenodo"' 20 "$ui" \
        || { fail "Publish button missing in no-key test"; cleanup_proof_zenodo; return; }
    bounds=$(node_bounds 'text="Publish to Zenodo"')
    tap_center "$bounds"
    wait_for_ui 'text="Publish to Zenodo?"' 20 "$ui" \
        || { fail "Publish confirm missing in no-key test"; cleanup_proof_zenodo; return; }
    bounds=$(node_bounds 'text="Publish"')
    tap_center "$bounds"
    wait_for_log "$log" 'proof phase result kind NOT_PUBLISHED verified true' \
        || { fail "missing Zenodo key was not reported as NOT_PUBLISHED"; cleanup_proof_zenodo; return; }
    cmd_uidump "$ui" >/dev/null || { cleanup_proof_zenodo; return; }
    if grep -q 'ZENODO_API_KEY' "$ui" && [ "$(mock_stat deposits)" = "1" ]; then
        pass "unconfigured Zenodo refused honestly without touching mock Zenodo"
    else
        fail "no-key error message missing or mock stats changed"
    fi

    cleanup_proof_zenodo
}

cmd_phone_layout() {
    say "==> phone-layout: 360 dp portrait and landscape math-preview + toolbar bounds"
    if [ "$PKG" != "$TEST_PACKAGE" ]; then
        fail "phone-layout requires $TEST_PACKAGE"
        return
    fi
    if ! python3 -c 'from PIL import Image' >/dev/null 2>&1; then
        fail "phone-layout requires Python Pillow"
        return
    fi
    mkdir -p "$OUTDIR" || { fail "create phone-layout output directory"; return; }

    restore_display() {
        D shell wm size reset >/dev/null 2>&1 || true
        D shell wm density reset >/dev/null 2>&1 || true
    }
    trap restore_display RETURN

    # 360 x 780 dp portrait phone (1080x2340 at 480 dpi)
    if ! D shell wm size 1080x2340 >/dev/null 2>&1 \
        || ! D shell wm density 480 >/dev/null 2>&1; then
        fail "set 360 dp portrait phone size/density"
        return
    fi
    sleep 1
    local before_fails=$FAILURES
    cmd_math_preview
    if [ "$FAILURES" -eq "$before_fails" ]; then
        pass "360 dp portrait math-preview passed"
    fi

    # Verify compact toolbar controls sit inside the 1080 px portrait width.
    local ui="$OUTDIR/phone-portrait-ui.xml"
    cmd_uidump "$ui" || return
    if python3 - "$ui" <<'PY'
import re, sys, xml.etree.ElementTree as ET

root = ET.parse(sys.argv[1]).getroot()
want = {
    "Open": 'text="Open"',
    "Save": 'text="Save"',
    "Undo": 'text="Undo"',
    "Find": 'text="Find"',
    "Edit": 'text="Edit"',
    "More actions": 'content-desc="More actions"',
}
nodes = list(root.iter("node"))
for label in ("Open", "Save", "Undo", "Find", "Edit"):
    node = next((n for n in nodes if n.get("text") == label), None)
    assert node is not None, f"missing toolbar button {label}"
    x1, y1, x2, y2 = map(int, re.findall(r"\d+", node.get("bounds", "")))
    assert 0 <= x1 < x2 <= 1080 and y1 < y2, f"out-of-bounds {label}: {(x1, y1, x2, y2)}"
more = next((n for n in nodes if n.get("content-desc") == "More actions"), None)
assert more is not None, "missing More actions button"
x1, y1, x2, y2 = map(int, re.findall(r"\d+", more.get("bounds", "")))
assert 0 <= x1 < x2 <= 1080 and y1 < y2, f"out-of-bounds More actions: {(x1, y1, x2, y2)}"
PY
    then
        pass "compact toolbar buttons fit inside 360 dp portrait width"
    else
        fail "compact toolbar clipped or missing at 360 dp portrait"
    fi

    # 780 x 360 dp landscape phone (2340x1080 at 480 dpi): scroll past the
    # scaled A4 top margin inside the 148 dp viewport so the equation is visible.
    if ! D shell wm size 2340x1080 >/dev/null 2>&1 \
        || ! D shell wm density 480 >/dev/null 2>&1; then
        fail "set 360 dp landscape phone size/density"
        return
    fi
    sleep 1
    cmd_clear_data || return
    local llog="$OUTDIR/phone-landscape-logcat.txt"
    local lui="$OUTDIR/phone-landscape-ui.xml"
    local lshot="$OUTDIR/phone-landscape.png"
    local ledited="$OUTDIR/phone-landscape-edit.xml"
    local lundone="$OUTDIR/phone-landscape-undo.xml"
    local lfixture
    lfixture=$(printf 'Preview ink\n\n$x^2 + y^2 = z^2$\n\nAfter equation\n' | base64 -w0)
    D logcat -c >/dev/null 2>&1 || true
    if ! D shell am start -W -n "$ACTIVITY" --es emthin.fixtureB64 "$lfixture" >/dev/null 2>&1; then
        fail "launch math fixture in 360 dp landscape"
        return
    fi
    wait_for_log "$llog" 'applyChunk total 1 pages 1 start 0' \
        || { fail "360 dp landscape math preview did not apply its page"; return; }
    wait_for_log "$llog" 'diagnostics .*count 0 errors 0 warnings 0 notes [1-9]' \
        || { fail "360 dp landscape math variables triggered non-note diagnostics"; return; }
    D shell input swipe 1170 760 1170 580 300 >/dev/null 2>&1 || true
    sleep 0.5
    cmd_uidump "$lui" || return
    cmd_screencap "$lshot" >/dev/null || return

    local lglyph
    if lglyph=$(python3 - "$lui" "$lshot" <<'PY'
import re, sys, xml.etree.ElementTree as ET
from PIL import Image

nodes = list(ET.parse(sys.argv[1]).iter("node"))
for label in ("Open", "Save", "Undo", "Find", "Edit"):
    node = next((n for n in nodes if n.get("text") == label), None)
    assert node is not None, f"missing toolbar button {label}"
    x1, y1, x2, y2 = map(int, re.findall(r"\d+", node.get("bounds", "")))
    assert 0 <= x1 < x2 <= 2340 and y1 < y2, f"out-of-bounds {label}: {(x1, y1, x2, y2)}"
more = next((n for n in nodes if n.get("content-desc") == "More actions"), None)
assert more is not None, "missing More actions button"
mx1, my1, mx2, my2 = map(int, re.findall(r"\d+", more.get("bounds", "")))
assert 0 <= mx1 < mx2 <= 2340 and my1 < my2, f"out-of-bounds More actions"

scroll = next((n for n in nodes if n.get("class") == "android.widget.ScrollView"), None)
assert scroll is not None, "missing ScrollView"
sx1, sy1, sx2, sy2 = map(int, re.findall(r"\d+", scroll.get("bounds", "")))
block = next((n for n in nodes if n.get("text", "").startswith("Preview ink")), None)
assert block is not None, "math preview accessibility bounds are absent"
bx1, by1, bx2, by2 = map(int, re.findall(r"\d+", block.get("bounds", "")))
crop_box = (max(sx1, bx1), max(sy1, by1), min(sx2, bx2), min(sy2, by2))
assert crop_box[2] > crop_box[0] and crop_box[3] > crop_box[1], f"empty visible crop: {crop_box}"

image = Image.open(sys.argv[2]).convert("RGB").crop(crop_box)
width, height = image.size
bands = []
for y in range(height):
    if sum(min(image.getpixel((x, y))) > 160 for x in range(width)) < 3:
        continue
    if not bands or y - bands[-1][-1] > max(2, height // 40):
        bands.append([])
    bands[-1].append(y)
bands = [b for b in bands if len(b) >= 3]
if len(bands) < 2:
    sys.exit(f"expected prose + equation bands inside visible ScrollView, got {len(bands)}")
equation = bands[1]
ey = equation[2 * len(equation) // 3]
ex = next(x for x in range(width) if min(image.getpixel((x, ey))) > 160)
print(ex + crop_box[0], ey + crop_box[1])
PY
    )
    then
        pass "360 dp landscape toolbar fits and equation is visible after scrolling"
    else
        fail "360 dp landscape math preview bitmap or toolbar check failed ($lshot)"
        return
    fi

    local lx ly
    read -r lx ly <<<"$lglyph"
    if ! D shell input tap "$lx" "$ly" >/dev/null 2>&1 ||
            ! D shell input text q >/dev/null 2>&1; then
        fail "tap and edit displayed equation in 360 dp landscape"
        return
    fi
    cmd_uidump "$ledited" || return
    if ! D shell input keycombination 113 54 >/dev/null 2>&1; then
        fail "send Ctrl+Z in 360 dp landscape"
        return
    fi
    cmd_uidump "$lundone" || return
    if python3 - "$ledited" "$lundone" <<'PY'
import sys, xml.etree.ElementTree as ET

def focused_source(path):
    for node in ET.parse(path).iter("node"):
        if (node.get("content-desc") == "Editable text without modifiers"
                and node.get("focused") == "true"):
            return node.get("text", "")
    return ""

original = "$x^2 + y^2 = z^2$"
edited = focused_source(sys.argv[1])
if not edited.startswith("$") or edited.count("q") != 1 or edited.replace("q", "") != original:
    sys.exit(f"360 dp landscape edit mismatch: {edited!r}")
if focused_source(sys.argv[2]) != original:
    sys.exit("360 dp landscape Undo did not restore equation with focus")
PY
    then
        pass "360 dp landscape equation tap-to-edit and Undo passed"
    else
        fail "360 dp landscape equation edit or Undo failed"
    fi

    restore_display
    trap - RETURN
    if D shell wm size 2>/dev/null | tr -d '\r' | grep -q 'Override size'; then
        fail "wm size override did not reset cleanly"
    else
        pass "restored native display size and density"
    fi
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
    network-change) cmd_network_change ;;
    gateway-wss) cmd_gateway_wss ;;
    pairing)   cmd_pairing ;;
    sessions-hosts) cmd_sessions_hosts ;;
    pairing-answers) cmd_pairing_answers ;;
    paired-lease)    cmd_paired_lease ;;
    enroll)          cmd_enroll ;;
    austral-rule) cmd_austral_rule ;;
    wayland-gpu) cmd_wayland_gpu ;;
    history-preview) cmd_history_preview ;;
    proof-zenodo) cmd_proof_zenodo ;;
    phone-layout) cmd_phone_layout ;;
    e2e)       cmd_e2e ;;
    *) say "unknown command: $cmd"; usage >&2; exit 2 ;;
esac

if [ "$FAILURES" -eq 0 ]; then
    say "device lane: all steps passed"
    exit 0
fi
say "device lane: $FAILURES step(s) FAILED"
exit 1
