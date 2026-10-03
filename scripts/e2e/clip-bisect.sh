#!/usr/bin/env bash
# Bisect the clipboard chain one hop at a time.
#
#   Xvfb (:99)  <->  sway (wlroots x11 backend)  <->  emthin
#
# Each hop is tested on its own, so "the clipboard does not work" becomes "hop N
# is broken", and emthin is either implicated or cleared.
#
# Run inside the flake:  nix develop --command bash scripts/e2e/clip-bisect.sh

set -uo pipefail

HOST_DISPLAY="${HOST_DISPLAY:-:99}"
XDG="${XDG:-/home/leo/Projects/emthin/.e2e/xdg}"
SWAY_DISPLAY="${SWAY_DISPLAY:-wayland-1}"
EMTHIN_DISPLAY="${EMTHIN_DISPLAY:-e2e-c}"

pass=0; fail=0
report() {
  local name="$1" got="$2" want="$3"
  if [ "$got" = "$want" ]; then
    printf '  \033[1;32mPASS\033[0m  %s\n' "$name"; pass=$((pass+1))
  else
    printf '  \033[1;31mFAIL\033[0m  %s\n        got:  %q\n        want: %q\n' \
      "$name" "$got" "$want"; fail=$((fail+1))
  fi
}

# Hold a selection on one display for the duration of a read on another.
# $1 = display label for messages, $2 = command that OWNS the selection.
own_for() {
  local log="$1"; shift
  "$@" </dev/null >/dev/null 2>&1 &
  OWN_PID=$!
  sleep 1.5
}

echo "clipboard chain bisect"
echo "  Xvfb      : $HOST_DISPLAY"
echo "  sway      : $XDG/$SWAY_DISPLAY"
echo "  emthin    : $XDG/$EMTHIN_DISPLAY"
echo

# ── hop 1: sway's x11 backend <-> the X server's selection ────────────────
echo "hop 1 — sway <-> Xvfb X selection (wlroots' x11 backend)"
M1="hop1-$$"
own_for 1 env DISPLAY="$HOST_DISPLAY" xclip -selection clipboard -i <<<"$M1"
GOT="$(XDG_RUNTIME_DIR="$XDG" WAYLAND_DISPLAY="$SWAY_DISPLAY" timeout 4 wl-paste --no-newline 2>/dev/null || true)"
kill "$OWN_PID" 2>/dev/null; rm -f "$OWN_PID"
report "X selection reaches sway's clients" "$GOT" "$M1"

own_for 1 env XDG_RUNTIME_DIR="$XDG" WAYLAND_DISPLAY="$SWAY_DISPLAY" \
  wl-copy --type text/plain -- "$M1"
GOT="$(DISPLAY="$HOST_DISPLAY" timeout 4 xclip -selection clipboard -o 2>/dev/null || true)"
kill "$OWN_PID" 2>/dev/null; rm -f "$OWN_PID"
report "sway's clients reach the X selection" "$GOT" "$M1"
echo

# ── hop 2: emthin <-> sway, which is emthin's own bridge ──────────────────
echo "hop 2 — emthin <-> sway (emthin's clipboard proxy)"
if [ ! -S "$XDG/$EMTHIN_DISPLAY" ]; then
  echo "  (skipped: emthin is not running, so $XDG/$EMTHIN_DISPLAY is absent)"
else
  M2="hop2-$$"
  own_for 2 env XDG_RUNTIME_DIR="$XDG" WAYLAND_DISPLAY="$SWAY_DISPLAY" \
    wl-copy --type text/plain -- "$M2"
  GOT="$(XDG_RUNTIME_DIR="$XDG" WAYLAND_DISPLAY="$EMTHIN_DISPLAY" timeout 4 wl-paste --no-newline 2>/dev/null || true)"
  kill "$OWN_PID" 2>/dev/null; rm -f "$OWN_PID"
  report "sway's selection reaches emthin's clients" "$GOT" "$M2"

  own_for 2 env XDG_RUNTIME_DIR="$XDG" WAYLAND_DISPLAY="$EMTHIN_DISPLAY" \
    wl-copy --type text/plain -- "$M2"
  GOT="$(XDG_RUNTIME_DIR="$XDG" WAYLAND_DISPLAY="$SWAY_DISPLAY" timeout 4 wl-paste --no-newline 2>/dev/null || true)"
  kill "$OWN_PID" 2>/dev/null; rm -f "$OWN_PID"
  report "emthin's clients reach sway" "$GOT" "$M2"
fi
echo

printf '%d passed, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]