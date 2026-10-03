#!/usr/bin/env bash
# Shared setup for the §6 step-7 E2E configurations.
#
# Sourced, not executed. Everything here is about getting an emthin running
# inside a nested compositor and reaching it over the IPC control socket, which
# is orthogonal to *how* the keystrokes get in — that is what differs between
# configurations, and it is why the injection is factored out.
#
# shellcheck shell=bash

set -euo pipefail

: "${E2E_ROOT:=$PWD/.e2e}"
: "${E2E_DISPLAY:=e2e}"
: "${E2E_XVFB_DISPLAY:=:99}"
: "${E2E_VIEWPORT:=1280x800}"
E2E_XDG="$E2E_ROOT/xdg"
E2E_RUN="$E2E_ROOT/run"
E2E_IPC="$E2E_RUN/emthin.ipc"
E2E_DOC="$E2E_RUN/emthin.doc"
E2E_EMTHIN="$PWD/target/debug/emthin"

log()  { printf '\033[1;34m==>\033[0m %s\n' "$*" >&2; }
warn() { printf '\033[1;33m warn\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31mFAIL\033[0m %s\n' "$*" >&2; exit 1; }
ok()   { printf '\033[1;32m ok \033[0m %s\n' "$*" >&2; }

# Re-enter the flake unless we are already inside it.
#
# Not a convenience: `nix develop` starts an *interactive* shell, so a pasted
# multi-line block that begins with it hands the remaining lines to the nested
# shell as its input. The commands run, but not as a sequence you can see or
# stop, and a `sudo` line in the middle of one is a bad time for that. Making
# the entry point self-sufficient means the documented invocation is just
#
#     ./scripts/e2e/run.sh --config ydotool
#
# with no wrapper at all. The flake's shellHook exports EMTHEIN_DEVSHELL=1, so
# this is a no-op when the caller did enter the shell deliberately.
ensure_devshell() {
  [ "${EMTHEIN_DEVSHELL:-}" = 1 ] && return 0
  [ "${E2E_NO_REEXEC:-0}" = 1 ] && return 0
  command -v nix >/dev/null 2>&1 || {
    warn "not inside the dev shell and nix is not available; some tools will be missing"
    return 0
  }
  log "not inside the dev shell; re-entering via nix develop"
  export E2E_NO_REEXEC=1
  # `bash -c 'exec "$@"' _ "$0" "$@"` keeps the argument list intact, including
  # anything with spaces in it.
  exec nix develop --command bash -c 'exec "$@"' _ "$0" "$@"
}

# Run a command inside the flake, so every path below resolves the same way
# `cargo` does.
in_shell() {
  if [ "${EMTHEIN_DEVSHELL:-}" = 1 ]; then
    bash -c "$*"
  else
    nix develop --command bash -c "$*"
  fi
}

need() { command -v "$1" >/dev/null 2>&1 || die "$1 not on PATH (run under: nix develop)"; }

# ── compositor ──────────────────────────────────────────────────────────────
#
# `start_compositor <mode>` where mode is:
#   headless  Mutter with a virtual output and no host window. Reachable
#             anywhere, but has no X window, so XTEST cannot target it.
#   nested    A real window on $E2E_XVFB_DISPLAY, which is what `xdotool`
#             needs. Mutter only gets this far when the logind session is not
#             already owned — i.e. not from inside another desktop session.
start_compositor() {
  local mode="${1:-headless}"
  mkdir -p "$E2E_XDG" "$E2E_RUN"
  chmod 700 "$E2E_XDG"
  rm -f "$E2E_XDG/$E2E_DISPLAY" "$E2E_XDG/$E2E_DISPLAY.lock" 2>/dev/null || true

  local -a args=(--wayland "--wayland-display=$E2E_DISPLAY" --no-x11)
  [ "$mode" = headless ] && args+=(--headless "--virtual-monitor=$E2E_VIEWPORT")

  if [ "$mode" = nested ]; then
    export DISPLAY="$E2E_XVFB_DISPLAY"
    start_xvfb
  fi
  export XDG_RUNTIME_DIR="$E2E_XDG"

  log "starting compositor: gnome-shell ${args[*]} (DISPLAY=${DISPLAY:-none})"
  ( setsid dbus-run-session -- gnome-shell "${args[@]}" \
      </dev/null >>"$E2E_ROOT/gnome-shell.log" 2>&1 & )

  for _ in $(seq 1 60); do
    [ -S "$E2E_XDG/$E2E_DISPLAY" ] && return 0
    sleep 0.5
  done
  die "compositor did not create $E2E_XDG/$E2E_DISPLAY; see $E2E_ROOT/gnome-shell.log"
}

start_xvfb() {
  if [ -e "/tmp/.X11-unix/${E2E_XVFB_DISPLAY#:}" ]; then
    log "Xvfb already on $E2E_XVFB_DISPLAY"
    return 0
  fi
  need Xvfb
  log "starting Xvfb on $E2E_XVFB_DISPLAY"
  ( setsid Xvfb "$E2E_XVFB_DISPLAY" -screen 0 1600x1000x24 \
      </dev/null >>"$E2E_ROOT/xvfb.log" 2>&1 & )
  for _ in $(seq 1 40); do
    [ -e "/tmp/.X11-unix/${E2E_XVFB_DISPLAY#:}" ] && return 0
    sleep 0.25
  done
  die "Xvfb did not come up on $E2E_XVFB_DISPLAY"
}

# Headless Mutter exits when its last client disconnects; nothing else keeps it
# alive between configurations.
supervise_compositor() {
  log "supervising the compositor (it exits with its last client)"
  ( setsid bash -c '
      while :; do
        [ -S "$1/$2" ] || {
          rm -f "$1/$2.lock" 2>/dev/null
          DISPLAY=${3:-} XDG_RUNTIME_DIR="$1" dbus-run-session -- \
            gnome-shell --headless --wayland --wayland-display="$2" \
            --virtual-monitor='"$E2E_VIEWPORT"' --no-x11 \
            >>'"$E2E_ROOT"'/gnome-shell.log 2>&1 &
          for _ in $(seq 1 60); do [ -S "$1/$2" ] && break; sleep 0.5; done
          echo "compositor restarted $(date +%T)" >> '"$E2E_ROOT"'/supervisor.log
        }
        sleep 1
      done' _ "$E2E_XDG" "$E2E_DISPLAY" </dev/null >/dev/null 2>&1 & )
}

# ── emthin ──────────────────────────────────────────────────────────────────
#
# `start_emthin [extra args...]`. `--doc` is not passed by default: the point of
# step 6's check is that a session file alone restores everything.
start_emthin() {
  [ -x "$E2E_EMTHIN" ] || die "$E2E_EMTHIN not built (cargo build -p emthin)"
  mkdir -p "$E2E_RUN"
  export XDG_RUNTIME_DIR="$E2E_XDG"
  export WAYLAND_DISPLAY="$E2E_DISPLAY"
  export RUST_LOG="${RUST_LOG:-debug}"
  # winit dlopens libwayland; the flake's shellHook puts it on LD_LIBRARY_PATH,
  # but a shell that sourced env.sh instead will not have it.
  if [ -z "${LD_LIBRARY_PATH:-}" ]; then
    export LD_LIBRARY_PATH="$(dirname "$(command -v wtype 2>/dev/null || command -v xdotool 2>/dev/null || echo /)")/../lib"
  fi

  log "starting emthin ($E2E_EMTHIN)"
  ( setsid "$E2E_EMTHIN" --session-file "$E2E_DOC" --ipc-path "$E2E_IPC" "$@" \
      </dev/null >>"$E2E_ROOT/emthin.log" 2>&1 & )
  for _ in $(seq 1 60); do
    [ -S "$E2E_IPC" ] && { sleep 1; return 0; }
    sleep 0.5
  done
  die "emthin did not create $E2E_IPC; see $E2E_ROOT/emthin.log"
}

emthin_pid() { pgrep -x emthin | head -1; }

stop_emthin() {
  local pid; pid="$(emthin_pid)" || return 0
  [ -n "$pid" ] || return 0
  # SIGTERM, because since a2b393c that is the *graceful* path: it stops the
  # event loop and the session gets saved. SIGKILL here would be testing
  # nothing but the kernel.
  log "stopping emthin (pid $pid) with SIGTERM"
  kill -TERM "$pid" 2>/dev/null || true
  # Generous, and it checks the log rather than just the process: the graceful
  # path is only proven by "shut down cleanly" appearing. A 5s wait produced a
  # spurious SIGKILL that read like the signal handling was broken when it was
  # only slow to be noticed under load — observed directly, with the process
  # exiting normally once given longer.
  local grace="${E2E_STOP_GRACE:-30}"
  local deadline=$(( SECONDS + grace ))
  while [ "$SECONDS" -lt "$deadline" ]; do
    if ! kill -0 "$pid" 2>/dev/null; then
      if grep -aq "shut down cleanly" "$E2E_ROOT/emthin.log" 2>/dev/null; then
        ok "emthin exited cleanly (graceful shutdown confirmed in the log)"
      else
        warn "emthin exited, but never logged a clean shutdown"
      fi
      return 0
    fi
    sleep 0.5
  done
  warn "emthin did not exit within ${grace}s of SIGTERM; sending SIGKILL"
  warn "that is a real failure of the signal path, not a slow exit — look for"
  warn "\"shutdown signal received\" in $E2E_ROOT/emthin.log"
  kill -KILL "$pid" 2>/dev/null || true
}

# ── IPC ─────────────────────────────────────────────────────────────────────
ipc() { python3 "$PWD/scripts/e2e/ipc.py" "$E2E_IPC" "$@"; }
state() { python3 "$PWD/scripts/e2e/ipc.py" "$E2E_IPC" list_state | python3 "$PWD/scripts/e2e/state.py"; }

# ── injection seam ──────────────────────────────────────────────────────────
#
# Each configuration implements these four and nothing else, so the checks in
# step-7.sh are written once.
#
#   inject_available()  -> 0 if keys can be delivered at all
#   inject_note()       ->  one line for the user, saying why not if it cannot
#   inject_type TEXT    ->  deliver TEXT as committed key events
#   inject_key NAME     ->  deliver one named key (e.g. ctrl+c, Return, Page_Down)
#
# The concrete implementations live in inject-*.sh.

# ── assertions ──────────────────────────────────────────────────────────────
FAILED=0
check() {
  local what="$1" got="$2" want="$3"
  if [ "$got" = "$want" ]; then
    ok "$what"
  else
    printf '\033[1;31mFAIL\033[0m %s\n      got:  %s\n      want: %s\n' \
      "$what" "$got" "$want" >&2
    FAILED=1
  fi
}

check_contains() {
  local what="$1" hay="$2" needle="$3"
  case "$hay" in
    *"$needle"*) ok "$what" ;;
    *) printf '\033[1;31mFAIL\033[0m %s\n      %s\n      does not contain: %s\n' \
         "$what" "$hay" "$needle" >&2
       FAILED=1 ;;
  esac
}

check_not_contains() {
  local what="$1" hay="$2" needle="$3"
  case "$hay" in
    *"$needle"*) printf '\033[1;31mFAIL\033[0m %s\n      %s\n      unexpectedly contains: %s\n' \
         "$what" "$hay" "$needle" >&2
       FAILED=1 ;;
    *) ok "$what" ;;
  esac
}

finish() {
  echo >&2
  if [ "$FAILED" = 0 ]; then
    ok "all checks passed"
    exit 0
  fi
  printf '\033[1;31m%d check(s) failed\033[0m\n' "$FAILED" >&2
  exit 1
}