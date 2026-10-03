#!/usr/bin/env bash
# Start/stop just the privileged half of the `ydotool` configuration.
#
# `ydotoold` is the only part that needs root: it holds /dev/uinput, and
# `ydotool(1)` then talks to it over $XDG_RUNTIME_DIR/.ydotool_socket as an
# ordinary user. Prefer this over running the whole test under sudo — one root
# process, and the test itself stays unprivileged.
#
#   sudo ./scripts/e2e/ydotoold-daemon.sh start
#   ./scripts/e2e/run.sh --config ydotool
#   sudo ./scripts/e2e/ydotoold-daemon.sh stop
#
# `modprobe uinput` is attempted and tolerated: on a stock NixOS kernel the
# module is usually already loaded, and in a container it cannot be loaded at
# all — in which case /dev/uinput exists precisely because the host passed it
# through, so the module is not needed here.

set -euo pipefail

XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
SOCKET="$XDG_RUNTIME_DIR/.ydotool_socket"
PIDFILE="$XDG_RUNTIME_DIR/.ydotoold.pid"

usage() { echo "usage: $0 {start|stop|status}" >&2; exit 2; }

case "${1:-}" in
  start)
    if [ "$(id -u)" != 0 ]; then
      echo "ydotoold needs root: re-run as sudo $0 start" >&2
      exit 1
    fi
    if [ -e "$SOCKET" ]; then
      echo "ydotoold already running (socket $SOCKET)" >&2
      exit 0
    fi
    if [ ! -e /dev/uinput ]; then
      echo "no /dev/uinput on this host; this configuration cannot work here" >&2
      exit 1
    fi
    modprobe uinput 2>/dev/null || true
    if [ ! -w /dev/uinput ]; then
      echo "/dev/uinput is $(stat -c '%A %U:%G' /dev/uinput) and not writable even as root;" >&2
      echo "the uinput module is probably not loadable in this environment" >&2
      exit 1
    fi
    echo "starting ydotoold (holding /dev/uinput)"
    setsid ydotoold -b -s "$SOCKET" </dev/null >/dev/null 2>&1 &
    echo $! > "$PIDFILE"
    for _ in $(seq 1 20); do
      [ -S "$SOCKET" ] && { echo "ydotoold up on $SOCKET"; exit 0; }
      sleep 0.25
    done
    echo "ydotoold did not create $SOCKET" >&2
    exit 1
    ;;

  stop)
    if [ -f "$PIDFILE" ]; then
      kill "$(cat "$PIDFILE")" 2>/dev/null || true
      rm -f "$PIDFILE"
    fi
    pkill -x ydotoold 2>/dev/null || true
    rm -f "$SOCKET"
    echo "ydotoold stopped"
    ;;

  status)
    if [ -S "$SOCKET" ]; then
      echo "ydotoold running on $SOCKET"
      exit 0
    fi
    echo "ydotoold not running"
    exit 1
    ;;

  *) usage ;;
esac