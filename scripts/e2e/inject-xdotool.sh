#!/usr/bin/env bash
# Step 7, configuration `xdotool`: XTEST into a compositor that has an X window.
#
# The keys go in with no window search at all. My first version insisted on
# finding the compositor's window with `xdotool search --class ...` and so found
# nothing: wlroots does not set WM_NAME, and even `xdotool search --name "."`
# returns an empty list on a bare Xvfb. XTEST does not need it — the events go to
# the X server's focus window, which for a compositor hosting itself on X is the
# compositor's own window, which then routes them to its focused Wayland client.
# So: activate the display, send the key, done.
#
# This is the only configuration that reaches a *nested* compositor, and it is
# the reason `start_compositor sway` exists: a wlroots compositor with
# WLR_BACKENDS=x11 maps a real X window under Xvfb and never asks logind for
# anything. Mutter run non-headless wants to take the session and fails EBUSY
# from inside a desktop session.

# shellcheck source=lib.sh
. "$(dirname "$(readlink -f "$0")")/lib.sh"

inject_available() {
  need xdotool
  [ -n "${DISPLAY:-}" ] || return 1
  # A reachable X display is the whole requirement.
  xdotool getdisplaygeometry >/dev/null 2>&1 || return 1
  [ -S "${E2E_XDG:-$PWD/.e2e/xdg}/${E2E_DISPLAY:-e2e}" ] || return 1
}

inject_note() {
  if [ -z "${DISPLAY:-}" ]; then
    printf 'xdotool UNAVAILABLE: no DISPLAY. run.sh --config xdotool starts one.\n'
  elif ! inject_available; then
    printf 'xdotool UNAVAILABLE: DISPLAY=%s is not reachable, or no compositor socket\n' \
      "${DISPLAY:-unset}"
  else
    printf 'xdotool via XTEST on DISPLAY=%s (X focus -> compositor -> focused client)' \
      "$DISPLAY"
  fi
}

# Point the X input focus at the root, which is where a compositor hosting itself
# on X takes its clients' focus from. Harmless when it is already correct, and it
# avoids depending on a window manager being present to set focus.
inject_focus() {
  xdotool windowfocus "$(xdotool getactivewindow 2>/dev/null || echo 0)" \
    >/dev/null 2>&1 || true
}

inject_key() {
  xdotool key --clearmodifiers --delay 20 -- "$1" >/dev/null 2>&1 \
    || { warn "xdotool could not deliver: $1"; return 1; }
  sleep "${INJECT_SETTLE:-0.2}"
}

inject_type() {
  xdotool type --clearmodifiers --delay 25 -- "$1" >/dev/null 2>&1 \
    || { warn "xdotool could not type: $1"; return 1; }
  sleep "${INJECT_SETTLE:-0.25}"
}