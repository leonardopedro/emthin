#!/usr/bin/env bash
# Step 7, configuration `ydotool`: keyboard-level injection via /dev/uinput.
#
# This is the only configuration that works against a *headless* compositor,
# because it injects below Wayland entirely — the kernel hands the events to
# whatever has the input device, and Mutter's seat picks them up like any other.
# That also makes it the configuration that survives the compositor's own
# key-binding machinery, which is why it is the one to reach for when you
# actually need a key to land.
#
# It needs write access to /dev/uinput, which is root:root 0600 on a stock
# NixOS box, so it needs sudo. There are two ways to run it, and they differ in
# what gets the privilege:
#
#   1. Whole-script sudo — simplest:
#        sudo -E ./scripts/e2e/run.sh --config ydotool
#      `ydotoold` runs as root and holds /dev/uinput; the rest runs as you.
#      Note that `sudo -E` is needed for the flake's environment, or run
#      `nix develop` first and `sudo -E` from inside it.
#
#   2. Just the daemon — preferred, because then only one root process exists
#      and the test itself is unprivileged:
#        sudo modprobe uinput            # if the module is not loaded
#        sudo ./scripts/e2e/ydotoold-daemon.sh start
#        ./scripts/e2e/run.sh --config ydotool
#        sudo ./scripts/e2e/ydotoold-daemon.sh stop
#      `ydotool` itself needs no privilege once `ydotoold` owns the device; it
#      talks to it over $XDG_RUNTIME_DIR/.ydotool_socket.
#
# Requirements:
#   * /dev/uinput present.         `ls -l /dev/uinput`
#   * the `uinput` module loaded.   `lsmod | grep uinput`
#   * the invoking user may write it, or you run as root.
#
# What it cannot do here: nothing, as far as injection goes. Unlike wtype it
# needs no compositor support, and unlike xdotool it needs no X window. It only
# needs the device permission.

# shellcheck source=lib.sh
. "$(dirname "$(readlink -f "$0")")/lib.sh"

# ── the injection seam ──────────────────────────────────────────────────────

inject_available() {
  need ydotool
  need ydotoold
  # Either we can open the device ourselves, or a daemon already holds it.
  if [ -S "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/.ydotool_socket" ]; then
    return 0
  fi
  [ -w /dev/uinput ] || return 1
  command -v ydotoold >/dev/null 2>&1 || return 1
}

inject_note() {
  if [ -w /dev/uinput ]; then
    printf 'ydotool via /dev/uinput (writable by this user)'
  elif [ -S "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/.ydotool_socket" ]; then
    printf 'ydotool via a running ydotoold (device held elsewhere)'
  else
    printf 'ydotool UNAVAILABLE: /dev/uinput is %s\n' \
      "$(stat -c '%A %U:%G' /dev/uinput 2>/dev/null || echo 'absent')"
    printf '            run it with sudo, or start the daemon:\n'
    printf '              sudo modprobe uinput\n'
    printf '              sudo %s/scripts/e2e/ydotoold-daemon.sh start\n' "$PWD"
  fi
}

# ydotool keycodes are evdev codes, so the mapping has to be spelled out. Only
# the keys step 7 needs.
ydotool_keycode() {
  case "$1" in
    Return)      echo 28 ;;
    Escape)      echo 1 ;;
    BackSpace)   echo 14 ;;
    Delete)      echo 111 ;;
    Tab)         echo 15 ;;
    Left)        echo 105 ;;
    Right)       echo 106 ;;
    Up)          echo 103 ;;
    Down)        echo 108 ;;
    Home)        echo 102 ;;
    End)         echo 107 ;;
    Page_Down)   echo 109 ;;
    Page_Up)     echo 104 ;;
    space)       echo 57 ;;
    ctrl)        echo 29 ;;
    shift)       echo 42 ;;
    Return_L)    echo 28 ;;
    # Letters and digits are the ASCII rows of the default keymap, which is
    # what `setxkbmap`/libinput assume: 1..26 = a..z, 30..38 = 1..9, 39 = 0.
    [a-z])       echo $(( $(printf '%d' "'${1}") - 96 )) ;;
    [1-9])       echo $(( $(printf '%d' "'$1") - 19 )) ;;
    0)           echo 39 ;;
    *)           return 1 ;;
  esac
}

# Named modifiers need the key *down* before the other key and *up* after.
inject_key() {
  local name="$1"
  local -a codes=()
  case "$name" in
    ctrl+c)   codes=(29 46) ;;   # ctrl down, c, (both up)
    ctrl+v)   codes=(29 47) ;;
    ctrl+x)   codes=(29 45) ;;
    ctrl+a)   codes=(29 30) ;;
    ctrl+z)   codes=(29 44) ;;
    Page_Down) codes=(109) ;;
    Return)    codes=(28) ;;
    *)         ydotool_keycode "$name" >/dev/null || {
      warn "unknown key name: $name"; return 1; }
               codes=( "$(ydotool_keycode "$name")" ) ;;
  esac

  # One key at a time so the key-up always follows its key-down. A modifier
  # stays held across the pair and is released after the second key goes up.
  if [ "${#codes[@]}" = 2 ]; then
    local mod="${codes[0]}" key="${codes[1]}"
    ydotool key "${mod}:1" "${key}:1" "${key}:0" "${mod}:0" >/dev/null
  else
    local c="${codes[0]}"
    ydotool key "${c}:1" "${c}:0" >/dev/null
  fi
  sleep "${INJECT_SETTLE:-0.12}"
}

inject_type() {
  local text="$1" i ch
  for (( i = 0; i < ${#text}; i++ )); do
    ch="${text:i:1}"
    case "$ch" in
      ' ') inject_key space ;;
      *)   ydotool_keycode "$ch" >/dev/null || {
              warn "no evdev code for '$ch'; skipping"; continue; }
           ydotool key "$(ydotool_keycode "$ch"):1" "$(ydotool_keycode "$ch"):0" >/dev/null
           sleep "${INJECT_TYPE_DELAY:-0.02}" ;;
    esac
  done
}

export -f inject_available inject_note inject_type inject_key 2>/dev/null || true