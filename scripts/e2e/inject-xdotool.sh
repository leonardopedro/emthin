#!/usr/bin/env bash
# Step 7, configuration `xdotool`: XTEST into a nested compositor's X window.
#
# The only configuration that needs nothing privileged — but it needs the
# compositor to have an X window, and that is the whole difficulty.
#
# Mutter run *nested* (not `--headless`) maps a window onto its X display, and
# XTEST events sent to that window become input on the nested seat. So this
# configuration wants:
#
#   * Xvfb (or a real X server) on DISPLAY, and
#   * gnome-shell able to take the session, which it cannot do from inside
#     another desktop session: it fails `Failed to take control of the session:
#     EBUSY`. So run this from a TTY or a plain login, not from inside GNOME.
#
# Reachable: on a machine whose seat is not already owned, or with a different
# nested compositor that does map an X window (Xephyr, Xvfb + a nested
# wayland compositor that is not Mutter).

# shellcheck source=lib.sh
. "$(dirname "$(readlink -f "$0")")/lib.sh"

inject_available() {
  need xdotool
  [ -n "${DISPLAY:-}" ] || return 1
  # There has to be a window to aim at.
  xdotool search --onlyvisible "" >/dev/null 2>&1 || return 1
  xdotool search --onlyvisible --class '.' >/dev/null 2>&1
}

inject_note() {
  if [ -z "${DISPLAY:-}" ]; then
    printf 'xdotool UNAVAILABLE: no DISPLAY. Start the compositor as `nested`\n'
    printf '            (run.sh --config xdotool does that) — which needs the logind\n'
    printf '            session to be free, so not from inside another desktop.\n'
  elif ! inject_available; then
    printf 'xdotool UNAVAILABLE: DISPLAY=%s has no visible window\n' "$DISPLAY"
  else
    printf 'xdotool via XTEST on DISPLAY=%s' "$DISPLAY"
  fi
}

# The window to type into: emthin's own toplevel, which under a nested Mutter
# on Xvfb is the shell's window. Fall back to whatever is focused.
_e2e_window() {
  local w
  w="$(xdotool search --onlyvisible --class 'gnome-shell' 2>/dev/null | tail -1)" || true
  [ -n "$w" ] || w="$(xdotool search --onlyvisible --name '.' 2>/dev/null | tail -1)" || true
  printf '%s' "${w:-}"
}

inject_focus() {
  local w; w="$(_e2e_window)"
  [ -n "$w" ] || return 1
  xdotool windowactivate --sync "$w" 2>/dev/null || xdotool windowfocus "$w" 2>/dev/null || true
  xdotool windowraise "$w" 2>/dev/null || true
}

inject_key() {
  inject_focus
  xdotool key --clearmodifiers --window "$(_e2e_window)" -- "$1" >/dev/null 2>&1 \
    || xdotool key --clearmodifiers -- "$1" >/dev/null 2>&1 \
    || { warn "xdotool could not deliver: $1"; return 1; }
  sleep "${INJECT_SETTLE:-0.12}"
}

inject_type() {
  inject_focus
  xdotool type --clearmodifiers --delay 20 -- "$1" >/dev/null 2>&1 \
    || { warn "xdotool could not type: $1"; return 1; }
  sleep "${INJECT_SETTLE:-0.12}"
}