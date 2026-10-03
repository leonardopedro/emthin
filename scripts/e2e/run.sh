#!/usr/bin/env bash
# Entry point for the §6 step-7 E2E.
#
#   ./scripts/e2e/run.sh                       # pick a working configuration
#   ./scripts/e2e/run.sh --config ydotool      # force one
#   ./scripts/e2e/run.sh --list                # what is available here
#   sudo -E ./scripts/e2e/run.sh --config ydotool
#
# Configurations, in the order they are tried:
#
#   ydotool   /dev/uinput, kernel level. Needs root or a running ydotoold.
#             The only one that works against a headless compositor, and the
#             only one that needs no compositor support. See
#             inject-ydotool.sh and ydotoold-daemon.sh.
#   xdotool   XTEST. No privileges, but needs a nested compositor with an X
#             window, which Mutter will only do when the logind session is
#             free — not from inside another desktop.
#   wtype     zwp_virtual_keyboard_v1. No privileges, works headless, but Mutter
#             does not implement the protocol. Use under sway/weston/wayfire.
#   none      No injection. The clipboard half of step 7 still runs; the
#             keyboard half is reported as unverified.
#
# The clipboard checks do not depend on the configuration at all and run in
# every case.

set -uo pipefail

HERE="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
cd "$REPO"

# shellcheck source=lib.sh
. "$HERE/lib.sh"

# Re-enter the flake if needed. Must come before anything that touches a tool.
ensure_devshell "$@"

CONFIG=""
LIST=0
while [ $# -gt 0 ]; do
  case "$1" in
    --config) CONFIG="${2:-}"; shift 2 ;;
    --config=*) CONFIG="${1#*=}"; shift ;;
    --list)   LIST=1; shift ;;
    -h|--help) sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

# The compositor follows the configuration: xdotool needs one with an X window,
# everything else is happy with headless Mutter.
COMPOSITOR_MODE=headless
[ "$CONFIG" = xdotool ] && COMPOSITOR_MODE=sway

export E2E_ROOT="${E2E_ROOT:-$REPO/.e2e}"
mkdir -p "$E2E_ROOT"

# ── --list: report, change nothing ──────────────────────────────────────────
probe_config() {
  local name="$1"
  # shellcheck disable=SC1090
  . "$HERE/inject-$name.sh"
  if inject_available >/dev/null 2>&1; then
    printf '  \033[1;32musable\033[0m  %-9s %s\n' "$name" "$(inject_note)"
  else
    printf '  \033[1;31mno\033[0m      %-9s %s\n' "$name" "$(inject_note)"
  fi
}

if [ "$LIST" = 1 ]; then
  log "input-injection configurations on this machine"
  printf '\n'
  for c in ydotool xdotool wtype none; do
    if [ "$c" = none ]; then
      printf '  \033[1;33mpartial\033[0m  %-9s %s\n' none \
        'clipboard checks only; no keyboard'
    else
      probe_config "$c"
    fi
  done
  printf '\n'
  log "compositor: gnome-shell --headless --wayland --no-x11"
  printf '  --config xdotool uses sway with WLR_BACKENDS=x11 on Xvfb instead,\n'
  printf '  which maps a real X window and needs no logind session.\n'
  exit 0
fi

# ── pick a configuration ────────────────────────────────────────────────────
load_inject() {
  # shellcheck disable=SC1090
  . "$HERE/inject-$1.sh"
}

if [ -z "$CONFIG" ]; then
  log "no --config given; trying each in turn"
  for c in ydotool xdotool wtype; do
    load_inject "$c"
    if inject_available >/dev/null 2>&1; then
      CONFIG="$c"
      ok "using $c: $(inject_note)"
      break
    fi
    warn "$c: not usable here"
  done
  [ -n "$CONFIG" ] || { CONFIG=none; warn "no injection backend available"; }
else
  load_inject "$CONFIG"
fi

# ── bring the session up ────────────────────────────────────────────────────
echo >&2
log "configuration: $CONFIG (compositor: $COMPOSITOR_MODE)"
log "workspace: $E2E_ROOT"

start_compositor "$COMPOSITOR_MODE"
if [ "$COMPOSITOR_MODE" = headless ]; then
  # Headless Mutter exits when its last client goes away, which happens between
  # configurations.
  supervise_compositor
fi
start_emthin

cleanup() {
  echo >&2
  stop_emthin
}
trap cleanup EXIT

# Give the seat a moment so the first keystroke is not swallowed by focus.
sleep 1

# shellcheck disable=SC1090
. "$HERE/step7.sh"