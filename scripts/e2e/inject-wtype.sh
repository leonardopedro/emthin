#!/usr/bin/env bash
# Step 7, configuration `wtype`: zwp_virtual_keyboard_v1.
#
# The least invasive of the three: it speaks Wayland, needs no privileges, no X
# server, and works against a headless compositor — *if* the compositor
# implements the virtual-keyboard protocol. Mutter does not, which is why this
# is a separate configuration rather than the default: run it under a compositor
# that does implement it (sway, weston, wayfire, labwc) and the same checks
# apply unchanged.
#
# `wtype` takes text on argv and delivers it as a committed string, so it has
# no notion of modifiers or named keys — the two checks that need Ctrl are done
# by wtype's own key syntax where available, and otherwise skipped with a
# notice rather than silently passing.

# shellcheck source=lib.sh
. "$(dirname "$(readlink -f "$0")")/lib.sh"

inject_available() {
  need wtype
  [ -n "${WAYLAND_DISPLAY:-}" ] || return 1
  # Two *different* failures have to be caught, and missing either one is how
  # this gets reported wrong:
  #
  #   "Wayland connection failed"                        cannot reach the socket
  #   "does not support the virtual keyboard protocol"   reached it, no protocol
  #
  # The second is what Mutter says, and it is the answer for every Mutter
  # configuration, nested or not. Probing with a bare "did it fail" check misses
  # it and reports wtype as usable.
  local probe
  probe="$(wtype -k Return 2>&1 || true)"
  case "$probe" in
    *"Wayland connection failed"*|*"does not support"*) return 1 ;;
    "") return 0 ;;
    *) return 0 ;;
  esac
}

inject_note() {
  if [ -z "${WAYLAND_DISPLAY:-}" ]; then
    printf 'wtype UNAVAILABLE: no WAYLAND_DISPLAY\n'
  elif ! inject_available; then
    printf 'wtype UNAVAILABLE: the compositor does not implement\n'
    printf '            zwp_virtual_keyboard_v1 (Mutter does not). Try under sway,\n'
    printf '            weston or wayfire, or use --config %s.\n' "${E2E_INJECT_FALLBACK:-ydotool}"
  else
    printf 'wtype via zwp_virtual_keyboard_v1 on WAYLAND_DISPLAY=%s' "$WAYLAND_DISPLAY"
  fi
}

inject_type() {
  wtype -- "$1" >/dev/null 2>&1 || { warn "wtype could not deliver: $1"; return 1; }
  sleep "${INJECT_SETTLE:-0.2}"
}

inject_key() {
  case "$1" in
    Return)    wtype -k Return    >/dev/null 2>&1 ;;
    Escape)    wtype -k Escape    >/dev/null 2>&1 ;;
    BackSpace) wtype -k BackSpace >/dev/null 2>&1 ;;
    Page_Down) wtype -k Page_Down >/dev/null 2>&1 ;;
    ctrl+c)    wtype -k ctrl+c    >/dev/null 2>&1 ;;
    ctrl+v)    wtype -k ctrl+v    >/dev/null 2>&1 ;;
    ctrl+x)    wtype -k ctrl+x    >/dev/null 2>&1 ;;
    ctrl+a)    wtype -k ctrl+a    >/dev/null 2>&1 ;;
    ctrl+z)    wtype -k ctrl+z    >/dev/null 2>&1 ;;
    *) warn "wtype has no mapping for '$1'"; return 1 ;;
  esac
  sleep "${INJECT_SETTLE:-0.2}"
}