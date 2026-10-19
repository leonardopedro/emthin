#!/usr/bin/env bash
# Step 7, configuration `wtype`: zwp_virtual_keyboard_v1.
#
# The least invasive of the three: it speaks Wayland, needs no privileges and
# no X server. It works against a compositor that implements the
# virtual-keyboard protocol — wlroots ones do (sway, weston, wayfire, labwc),
# Mutter does not. Since 2026-10-19 the harness therefore pairs this config
# with its own headless sway (`COMPOSITOR_MODE=sway-headless`), so it runs on a
# Wayland machine with nothing but the devshell — no X server, no root, no
# host compositor support required.
#
# Every wtype invocation installs its own synthetic keymap on its own virtual
# keyboard, which makes the host swap the active keyboard (Leave + Enter on the
# focused surface). winit drops keys that arrive in that gap — the first press
# of an invocation is lost — so every invocation here leads with `-s <ms>` to
# let the swap settle before the first key. (A plain `wtype -- x` loses the
# press and delivers only the orphan release; `wtype -k Return` alone therefore
# does nothing.)
#
# Modifiers and named keys are wtype commands too (`-M/-m <mod>`, `-k <keysym>`
# with an xkb keysym name) — note `ctrl+a` is *not* a keysym name and makes
# wtype exit 1 with "Unknown key", so combos are spelled
# `-M ctrl -k a -m ctrl`. Every call below propagates wtype's exit status:
# silently swallowing it is how broken injections once passed as checks.

# shellcheck source=lib.sh
. "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/lib.sh"

# Availability is about what the *harness* can set up, not about the host
# compositor: this config boots its own headless sway, so probing the host's
# WAYLAND_DISPLAY for zwp_virtual_keyboard_v1 only ever sees Mutter and would
# report "no" on exactly the machines this lane exists for. The real proof is
# functional: step7's keyboard checks assert the typed text landed.
inject_available() {
  need wtype
  need sway
}

inject_note() {
  if ! command -v wtype >/dev/null 2>&1; then
    printf 'wtype UNAVAILABLE: not on PATH (enter the devshell)\n'
  elif ! command -v sway >/dev/null 2>&1; then
    printf 'wtype UNAVAILABLE: sway not on PATH — the harness needs it to boot\n'
  else
    printf 'wtype via zwp_virtual_keyboard_v1 on the harness-owned headless sway'
  fi
}

# Lead-in before the first key of an invocation: covers the host's
# Leave/Enter around the virtual keyboard's keymap swap (see header).
# Integer milliseconds — wtype's -s atoi()s it and rejects <= 0.
WTYPE_SWAP_SETTLE_MS="${INJECT_SWAP_SETTLE_MS:-150}"

inject_type() {
  wtype -s "$WTYPE_SWAP_SETTLE_MS" -- "$1" >/dev/null 2>&1 \
    || { warn "wtype could not deliver: $1"; return 1; }
  sleep "${INJECT_SETTLE:-0.2}"
}

inject_key() {
  case "$1" in
    Return)    wtype -s "$WTYPE_SWAP_SETTLE_MS" -k Return       >/dev/null 2>&1 ;;
    Escape)    wtype -s "$WTYPE_SWAP_SETTLE_MS" -k Escape       >/dev/null 2>&1 ;;
    BackSpace) wtype -s "$WTYPE_SWAP_SETTLE_MS" -k BackSpace    >/dev/null 2>&1 ;;
    Page_Down) wtype -s "$WTYPE_SWAP_SETTLE_MS" -k Page_Down    >/dev/null 2>&1 ;;
    End)       wtype -s "$WTYPE_SWAP_SETTLE_MS" -k End         >/dev/null 2>&1 ;;
    ctrl+c)    wtype -s "$WTYPE_SWAP_SETTLE_MS" -M ctrl -k c -m ctrl >/dev/null 2>&1 ;;
    ctrl+v)    wtype -s "$WTYPE_SWAP_SETTLE_MS" -M ctrl -k v -m ctrl >/dev/null 2>&1 ;;
    ctrl+x)    wtype -s "$WTYPE_SWAP_SETTLE_MS" -M ctrl -k x -m ctrl >/dev/null 2>&1 ;;
    ctrl+a)    wtype -s "$WTYPE_SWAP_SETTLE_MS" -M ctrl -k a -m ctrl >/dev/null 2>&1 ;;
    ctrl+z)    wtype -s "$WTYPE_SWAP_SETTLE_MS" -M ctrl -k z -m ctrl >/dev/null 2>&1 ;;
    *) warn "wtype has no mapping for '$1'"; return 1 ;;
  esac || { warn "wtype could not deliver: $1"; return 1; }
  sleep "${INJECT_SETTLE:-0.2}"
}