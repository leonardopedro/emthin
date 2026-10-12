#!/usr/bin/env bash
# Agent loop, live tier: a real nested session (sway on Xvfb, emthin
# inside it) that an LLM can boot, drive over IPC, screenshot, and
# tear down — repeatedly.
#
#   ./scripts/agent/live.sh boot [-- <emthin args...>]   start the session
#   ./scripts/agent/live.sh state                        figure list + rects (semantic)
#   ./scripts/agent/live.sh shot <out.png>               screenshot the output (visual)
#   ./scripts/agent/live.sh size <fig> <w> <h>           rewrite a figure's \app args
#   ./scripts/agent/live.sh spawn <cmd> [args...]        launch an app into a figure
#   ./scripts/agent/live.sh close <fig>                  unbind the app (figure goes dormant)
#   ./scripts/agent/live.sh open <path>                  replace the document
#   ./scripts/agent/live.sh save <path>                  snapshot the document
#   ./scripts/agent/live.sh page <n>                     switch page
#   ./scripts/agent/live.sh type <text>                  type into the focused surface (xdotool XTEST)
#   ./scripts/agent/live.sh key <sym>                    send one key (Return, ctrl+c, Page_Down, …)
#   ./scripts/agent/live.sh doc                          dump the document text (semantic, over IPC)
#   ./scripts/agent/live.sh log [n]                      last n lines of emthin's trace (default 50)
#   ./scripts/agent/live.sh down                         graceful teardown (SIGTERM, session saved)
#
# The session is sway (WLR_BACKENDS=x11 on Xvfb) rather than headless
# Mutter because that combination needs no logind session and maps a
# real window — the same reasoning as the e2e's xdotool configuration
# (see scripts/e2e/README.md). grim screenshots it over
# wlr-screencopy: emthin is a *client* of sway, so the compositor
# under test needs no screencopy protocol of its own.
#
# Every subcommand re-derives its environment from the e2e lib, so
# each one is a fresh process: an agent never has to hold state
# between calls beyond "the session is up".

set -uo pipefail

HERE="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
cd "$REPO"

# Reuse the e2e session machinery verbatim: workspace layout, socket
# paths, devshell re-entry, clean teardown.
# shellcheck source=../e2e/lib.sh
. "$REPO/scripts/e2e/lib.sh"

CMD="${1:-}"
[ $# -gt 0 ] && shift

ensure_devshell "$CMD" "$@"

case "$CMD" in
boot)
  reap_previous_run
  # sway maps a real X window under Xvfb, which is what both xdotool
  # and grim want; headless Mutter is reachable but screenshot-blind.
  start_compositor sway
  start_emthin "$@"
  ok "session up: IPC=$E2E_IPC output=$E2E_DISPLAY (wayland socket in $E2E_XDG)"
  log "next: ./scripts/agent/live.sh state | shot <out.png> | down"
  ;;

state)
  # The semantic tier against the *live* session: real bound apps,
  # real composited rects — not the headless estimate.
  need python3
  state
  ;;

shot)
  need grim
  OUT="${1:?usage: live.sh shot <out.png>}"
  [ -S "$E2E_IPC" ] || die "no session (IPC socket $E2E_IPC missing) — run: live.sh boot"
  # grim is a client of sway, so it captures what sway shows, which is
  # emthin's window with the app surfaces composited over their
  # figures. XDG_RUNTIME_DIR must be the harness's, and WAYLAND_DISPLAY
  # the socket sway actually created (same discovery as lib.sh).
  export XDG_RUNTIME_DIR="$E2E_XDG"
  local_sock=""
  for sock in "$E2E_XDG"/wayland-*; do
    case "$sock" in *.lock) continue ;; esac
    [ -S "$sock" ] && { local_sock="$(basename "$sock")"; break; }
  done
  [ -n "$local_sock" ] || die "no sway socket in $E2E_XDG (is the session up?)"
  WAYLAND_DISPLAY="$local_sock" grim "$OUT"
  ok "screenshot -> $OUT"
  ;;

size)
  need python3
  FIG="${1:?usage: live.sh size <figure> <w> <h>}"
  W="${2:?width}"; H="${3:?height}"
  ipc set_figure_size "{\"figure\":\"$FIG\",\"w\":$W,\"h\":$H}"
  ;;

spawn)
  need python3
  CMD0="${1:?usage: live.sh spawn <cmd> [args...]}"
  shift
  args_json="$(printf '%s\n' "$@" | python3 -c 'import json,sys; print(json.dumps([l for l in sys.stdin.read().split("\n") if l]))')"
  ipc spawn "{\"cmd\":\"$CMD0\",\"args\":$args_json}"
  ;;

close)
  need python3
  ipc close "{\"figure\":\"${1:?usage: live.sh close <figure>}\"}"
  ;;

open)
  need python3
  ipc open_doc "{\"path\":\"${1:?usage: live.sh open <path>}\"}"
  ;;

save)
  need python3
  ipc save_doc "{\"path\":\"${1:?usage: live.sh save <path>}\"}"
  ;;

page)
  need python3
  ipc goto_page "{\"page\":${1:?usage: live.sh page <n>}}"
  ;;

type)
  # XTEST through sway's X window — the injection seam the e2e calls
  # `--config xdotool`. Keys land on the focused surface, so focus the
  # document first if a figure has it (Escape).
  need xdotool
  [ -S "$E2E_IPC" ] || die "no session (IPC socket $E2E_IPC missing) — run: live.sh boot"
  # shellcheck source=../e2e/inject-xdotool.sh
  . "$REPO/scripts/e2e/inject-xdotool.sh"
  export DISPLAY="${E2E_XVFB_DISPLAY}"
  inject_type "${1:?usage: live.sh type <text>}"
  ;;

key)
  need xdotool
  [ -S "$E2E_IPC" ] || die "no session (IPC socket $E2E_IPC missing) — run: live.sh boot"
  # shellcheck source=../e2e/inject-xdotool.sh
  . "$REPO/scripts/e2e/inject-xdotool.sh"
  export DISPLAY="${E2E_XVFB_DISPLAY}"
  inject_key "${1:?usage: live.sh key <sym (Return, ctrl+c, Page_Down, ...)>}"
  ;;

doc)
  # The document text itself, over IPC — what the user actually typed
  # survives as, independent of rendering. The state event's `doc`
  # field is the whole document; reading it is how a typing feature is
  # verified without trusting the pixels.
  need python3
  [ -S "$E2E_IPC" ] || die "no session (IPC socket $E2E_IPC missing) — run: live.sh boot"
  ipc list_state | python3 -c '
import sys, json
dec = json.JSONDecoder()
text = sys.stdin.read()
i = 0
while i < len(text):
    while i < len(text) and text[i] != "{":
        i += 1
    if i >= len(text):
        break
    try:
        obj, i = dec.raw_decode(text, i)
    except ValueError:
        break
    if obj.get("method") == "state":
        sys.stdout.write(obj["params"]["doc"])
'
  ;;

log)
  # The compositor's own trace — the first thing to read when a live
  # operation misbehaves (a figure that will not bind, an app that
  # vanishes, a crash). Debugging emthin+mathed *themselves* starts
  # here, not at the screenshot.
  L="${E2E_ROOT}/emthin.log"
  [ -f "$L" ] || die "no log at $L (session never booted?)"
  tail -n "${1:-50}" "$L"
  ;;

down)
  stop_emthin
  for pid in $(pgrep -x sway 2>/dev/null); do
    kill -TERM "$pid" 2>/dev/null || true
  done
  ok "session down"
  ;;

*)
  sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'
  [ -z "$CMD" ] && exit 2 || { echo "FAIL unknown subcommand: $CMD" >&2; exit 2; }
  ;;
esac
