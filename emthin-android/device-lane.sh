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
#   pull         pull the installed base.apk, md5-compare to the built APK
#   screencap F  capture the screen to file F (prefix-stripped PNG)
#   logcat F     dump the pad's logcat to F
#   swipe        one drag inside the preview ScrollView
#   persistence  edit -> force-stop -> relaunch autosave proof (used by e2e)
#   new-reset    New confirmation -> starter reset -> Undo restore proof
#                (used by e2e)
#   scroll-anchor  cold-launch starter -> page-2 -> one-page collapse ->
#                undo reading-position proof (used by e2e)
#   e2e          authorize, connect, build, install, launch, screencap,
#                swipe, screencap, logcat, persistence, new-reset,
#                scroll-anchor, pull + byte-identity (the repeatable
#                device proof)
#
# Env overrides:
#   DEVICE       adb serial (default: waydroid status IP + :5555; an explicit
#                serial skips the waydroid-session prerequisite)
#   ADB          adb executable (default: adb on PATH)
#   APK          built apk path (default: <script dir>/build/emthin-pad.apk)
#   OUTDIR       screenshot/logcat output dir (default: /tmp/emthin-device-lane)
#   SKIP_BUILD   1 = e2e uses the existing APK instead of rebuilding it
#   SWIPE_X / SWIPE_FROM_Y / SWIPE_TO_Y / SWIPE_MS   drag (default: from wm size)
#
# Exit status: 0 = every step passed, 1 = a step failed, 2 = bad usage,
# 3 = a prerequisite (adb / a running waydroid session) is missing.

set -u
PATH=/run/current-system/sw/bin:$PATH
export PATH

PKG=com.emthin.pad
ACTIVITY=com.emthin.pad/.MainActivity
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
APK="${APK:-$SCRIPT_DIR/build/emthin-pad.apk}"
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
    local default_apk="$SCRIPT_DIR/build/emthin-pad.apk"
    if [ "$APK" != "$default_apk" ]; then
        say "info build: custom APK path $APK; not rebuilding"
        return 0
    fi
    if [ ! -x "$SCRIPT_DIR/build-apk.sh" ]; then
        fail "build-apk.sh is not executable"
        return 1
    fi
    # build-apk.sh asks adb for the device ABI when ANDROID_ABI is unset;
    # pass both the serial and the adb executable it should use.
    if DEVICE="$DEVICE" ADB="$ADB" "$SCRIPT_DIR/build-apk.sh"; then
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
    if D shell am start -W -n "$ACTIVITY" >/dev/null 2>&1; then
        pass "am start -W $ACTIVITY"
    else
        fail "am start -W $ACTIVITY"
    fi
}

# The pad now autosaves. Device checks that assert the starter document
# must clear that saved text first or they inherit the previous run.
cmd_clear_data() {
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

# Tap below the starter's two source lines so the caret lands at the
# document end. Tapping the text itself would put it at the tap's char.
tap_editor_end() {
    local b x1 y1 x2 y2
    b=$(node_bounds 'class="android.widget.EditText"') || return 1
    read -r x1 y1 x2 y2 <<EOF
$b
EOF
    D shell input tap "$(( (x1 + x2) / 2 ))" "$(( y1 + (y2 - y1) * 2 / 3 ))" >/dev/null 2>&1
}

keyboard_shown() {
    D shell dumpsys input_method 2>/dev/null | tr -d '\r' \
        | grep -q 'mInputShown=true'
}

wait_for_log() {
    local out="$1" pattern="$2" tries=0
    while [ "$tries" -lt 25 ]; do
        D logcat -d -s emthin-pad:D 2>/dev/null | tr -d '\r' >"$out"
        if grep -q "$pattern" "$out"; then
            return 0
        fi
        sleep 0.2
        tries=$((tries + 1))
    done
    return 1
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
    wait_for_log "$log" 'applyChunk total 2 pages 2' \
        || { fail "starter document did not render two pages"; return; }
    wait_for_log "$log" 'captionScroll' \
        || { fail "starter document did not open at the first caption"; return; }
    pass "cold-launched the starter"

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
    wait_for_log "$log" 'applyState setCaret false .*textLen 90' \
        || { fail "typed probe did not reach the model"; return; }

    # Move one grapheme back from the document end. The first-paint
    # fallback uses text.length() (90); a restored caret is 89, so the
    # log line distinguishes a real restore from the fallback.
    D logcat -c >/dev/null 2>&1 || true
    D shell input keyevent KEYCODE_DPAD_LEFT >/dev/null 2>&1
    wait_for_log "$log" 'startRender .*textLen 90 caret 89' \
        || { fail "caret probe did not move to 89"; return; }
    wait_for_log "$log" 'applyChunk total 2 pages 2' \
        || { fail "caret probe did not render two pages"; return; }
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
    wait_for_log "$log" 'applyState setCaret true .*textLen 90' \
        || { fail "relaunch lost the saved 90-byte document"; return; }
    pass "relaunch restored the edited document (textLen 90)"
    wait_for_log "$log" 'startRender .*textLen 90 caret 89' \
        || { fail "relaunch did not restore the caret at 89"; return; }
    pass "relaunch restored the caret at 89"
    wait_for_log "$log" 'applyChunk total 2 pages 2' \
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
    local b focus shot

    say "==> new document: reset -> undo"
    mkdir -p "$OUTDIR"
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    cmd_launch || return
    wait_for_log "$log" 'applyChunk total 2 pages 2' \
        || { fail "starter document did not render two pages"; return; }
    wait_for_log "$log" 'captionScroll' \
        || { fail "starter document did not open at the first caption"; return; }
    pass "cold-launched the starter for New"

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
    # has a step to restore. The starter is 89 bytes; X makes it 90.
    D logcat -c >/dev/null 2>&1 || true
    D shell input text X >/dev/null 2>&1
    wait_for_log "$log" 'applyState setCaret false .*textLen 90' \
        || { fail "New probe did not reach the model"; return; }
    wait_for_log "$log" 'startRender .*textLen 90' \
        || { fail "New probe did not render"; return; }
    wait_for_log "$log" 'applyChunk total 2 pages 2' \
        || { fail "New probe did not render two pages"; return; }
    pass "typed the 90-byte probe"

    # Open the confirmation through the actual toolbar button, assert its
    # title, and capture the dark dialog before confirming it.
    D logcat -c >/dev/null 2>&1 || true
    cmd_uidump "$ui" || return
    b=$(node_bounds 'text="New"') \
        || { fail "New not found in UI hierarchy"; return; }
    tap_center "$b"
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
    wait_for_log "$log" 'applyState setCaret true .*textLen 89 canUndo true' \
        || { fail "New did not reset the document to 89 bytes"; return; }
    pass "New reset the document to 89 bytes"
    wait_for_log "$log" 'startRender .*textLen 89' \
        || { fail "New reset did not render"; return; }
    wait_for_log "$log" 'applyChunk total 2 pages 2' \
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
    wait_for_log "$log" 'applyState setCaret true .*textLen 90' \
        || { fail "Undo did not restore the 90-byte document"; return; }
    pass "Undo restored the 90-byte edited document"
    wait_for_log "$log" 'startRender .*textLen 90' \
        || { fail "Undo restore did not render"; return; }
    wait_for_log "$log" 'applyChunk total 2 pages 2' \
        || { fail "Undo restore did not render two pages"; return; }
    sleep 0.5
    if shot=$(cmd_screencap "$OUTDIR/new-reset-after-undo.png"); then
        say "    captured $shot"
    fi
}

cmd_scroll_anchor() {
    local log="$OUTDIR/scroll-anchor-logcat.txt"
    local ui="$OUTDIR/scroll-anchor-ui.xml"
    local target collapse_actual undo_y undo_actual
    local b focus i shot

    say "==> scroll anchor: page 2 -> one-page collapse -> undo"
    mkdir -p "$OUTDIR"

    # Self-contained start: a previous run or the persistence check may
    # have left text behind, so clear app data and cold-launch the
    # 89-byte starter instead of inheriting whatever state the app is in.
    cmd_clear_data || return
    D logcat -c >/dev/null 2>&1 || true
    cmd_launch || return
    wait_for_log "$log" 'applyChunk total 2 pages 2' \
        || { fail "starter document did not render two pages"; return; }
    wait_for_log "$log" 'captionScroll' \
        || { fail "starter document did not open at the first caption"; return; }
    pass "cold-launched two-page starter at the first caption"

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
    if ! grep -q 'startRender .*caret 90' "$log"; then
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
    wait_for_log "$log" 'applyChunk total 2 pages 2' \
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
    # clears app data itself, so it still cold-launches the 89-byte starter.
    cmd_persistence
    cmd_new_reset
    cmd_scroll_anchor

    # Byte-identity of the installed APK, the launch proof's companion.
    cmd_pull
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
    persistence) cmd_persistence ;;
    new-reset) cmd_new_reset ;;
    scroll-anchor) cmd_scroll_anchor ;;
    e2e)       cmd_e2e ;;
    *) say "unknown command: $cmd"; usage >&2; exit 2 ;;
esac

if [ "$FAILURES" -eq 0 ]; then
    say "device lane: all steps passed"
    exit 0
fi
say "device lane: $FAILURES step(s) FAILED"
exit 1
