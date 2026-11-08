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
#   e2e          authorize, connect, build, install, launch, screencap,
#                swipe, screencap, logcat, pull + byte-identity (the
#                repeatable device proof)
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

FAILURES=0
DEVICE="${DEVICE:-}"
ADB="${ADB:-adb}"
SKIP_BUILD="${SKIP_BUILD:-0}"

say()  { printf '%s\n' "$*"; }
pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; FAILURES=$((FAILURES + 1)); }

usage() { sed -n '2,50p' "$0" | sed 's/^# \{0,1\}//'; }

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

    # Cold start so the first frame is this launch's.
    D shell am force-stop "$PKG" >/dev/null 2>&1 || true
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
    e2e)       cmd_e2e ;;
    *) say "unknown command: $cmd"; usage >&2; exit 2 ;;
esac

if [ "$FAILURES" -eq 0 ]; then
    say "device lane: all steps passed"
    exit 0
fi
say "device lane: $FAILURES step(s) FAILED"
exit 1
