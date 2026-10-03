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

# `sudo` does not inherit the caller's environment, so this cannot rely on being
# inside `nix develop`: find ydotoold from the flake if it is not already on
# PATH. Run as root, so only read the store paths — nothing here needs privilege
# beyond holding /dev/uinput.
if ! command -v ydotoold >/dev/null 2>&1; then
  # Resolve through the flake rather than assuming the caller is inside it:
  # `sudo` does not inherit the caller environment, so "run it from nix develop"
  # is not something this script can rely on. Asking the flake is slower than a
  # PATH lookup but it is the thing that is actually guaranteed to be right.
  _here="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
  _repo="$(cd "$_here/../.." && pwd)"
  # `tail -1` because the flake shellHook prints a banner on stdout, so the
  # capture would otherwise be the banner plus the path.
  _ydotoold="$(nix develop "$_repo" --command bash -c 'command -v ydotoold' 2>/dev/null | tail -1 || true)"
  if [ -n "$_ydotoold" ] && [ -x "$_ydotoold" ]; then
    PATH="$(dirname "$_ydotoold"):$PATH"
    export PATH
    echo "resolved ydotoold from the flake: $_ydotoold" >&2
  else
    echo "could not find ydotoold, on PATH or in the flake." >&2
    echo "Run the whole test under the flake instead, which keeps the PATH:" >&2
    echo "  sudo -E $_repo/scripts/e2e/run.sh --config ydotool" >&2
    exit 1
  fi
fi

# Where the socket goes -- and this must be the *invoking user's* runtime
# directory, not root's.
#
# `sudo` does not preserve XDG_RUNTIME_DIR, so the obvious default
# `/run/user/$(id -u)` resolves to `/run/user/0` under sudo, which does not exist
# on a normal desktop (only `/run/user/1000` does). Observed exactly: the daemon
# started as `ydotoold -b -s /run/user/0/.ydotool_socket`, created no socket, and
# the unprivileged test -- which looks in its own `/run/user/1000` -- correctly
# reported /dev/uinput as unavailable and skipped every keyboard check. The two
# halves could never meet.
#
# Deliberately not a world-writable location such as /tmp: this socket hands out
# the ability to inject keystrokes system-wide, so it stays in a 0700 directory
# only the invoking user can reach.
_target_user="${SUDO_USER:-}"
if [ -n "$_target_user" ] && id -u "$_target_user" >/dev/null 2>&1; then
  _target_uid="$(id -u "$_target_user")"
  RUNTIME_DIR="/run/user/$_target_uid"
  if [ ! -d "$RUNTIME_DIR" ]; then
    # No systemd user session for them (a container, a bare TTY). A private
    # directory they own, still 0700.
    RUNTIME_DIR="/tmp/e2e-ydotool-$_target_uid"
    mkdir -p "$RUNTIME_DIR"
    chown "$_target_uid" "$RUNTIME_DIR"
    chmod 0700 "$RUNTIME_DIR"
  fi
elif [ -n "${XDG_RUNTIME_DIR:-}" ] && [ -d "${XDG_RUNTIME_DIR}" ]; then
  RUNTIME_DIR="$XDG_RUNTIME_DIR"
else
  RUNTIME_DIR="/run/user/$(id -u)"
fi

SOCKET="$RUNTIME_DIR/.ydotool_socket"
PIDFILE="$RUNTIME_DIR/.ydotoold.pid"

usage() {
  echo "usage: $0 {start|stop|status|socket|where|whereami}" >&2
  exit 2
}

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
    echo "  socket: $SOCKET"
    # Flags, which I had wrong and which made the daemon exit before doing
    # anything:
    #
    #   -p  --socket-path   the socket. There is no `-s`.
    #   -P  --socket-perm   socket mode.
    #   -o  --socket-own    socket owner.
    #   (no -b: ydotoold has no such flag. It runs in the foreground, which is
    #    what the `setsid ... &` below is for.)
    #
    # Verified without root by watching it reject the old invocation:
    #   $ ydotoold -b -s /tmp/probe.sock
    #   ydotoold: invalid option -- 'b'
    #   ydotoold: invalid option -- 's'
    #   $ ydotoold -p /tmp/probe.sock
    #   failed to open uinput device: Permission denied     <- flags accepted
    #
    # -o matters as much as -p: the daemon runs as root, and it creates the
    # socket 0600 root-owned by default, so the unprivileged test would find the
    # socket present and then be refused by it. Hand it to the invoking user.
    _own=""
    if [ -n "$_target_user" ] && id -u "$_target_user" >/dev/null 2>&1; then
      _own="-o $(id -u "$_target_user"):$(id -g "$_target_user")"
    fi
    # shellcheck disable=SC2086 # _own is deliberately two words
    setsid ydotoold -p "$SOCKET" -P 0600 $_own </dev/null >/dev/null 2>&1 &
    # The pidfile is only a convenience for `stop`. If it cannot be written the
    # daemon is still fine, and under `set -e` an unguarded write aborted the
    # script *after* the daemon had started -- leaving it running with nothing
    # left to report it, which is how the original /run/user/0 run went quiet.
    echo $! > "$PIDFILE" 2>/dev/null || true
    for _ in $(seq 1 20); do
      if [ -S "$SOCKET" ]; then
        echo "ydotoold up on $SOCKET"
        exit 0
      fi
      sleep 0.25
    done
    echo "ydotoold did not create $SOCKET" >&2
    echo "it may have exited; run it in the foreground to see why:" >&2
    echo "  sudo ydotoold -p $SOCKET -P 0600" >&2
    exit 1
    ;;

  stop)
    if [ -f "$PIDFILE" ]; then
      kill "$(cat "$PIDFILE")" 2>/dev/null || true
      rm -f "$PIDFILE"
    fi
    # Also match on the socket path, so a daemon started before the pidfile was
    # fixed can still be stopped.
    pkill -f "ydotoold .*-s $SOCKET" 2>/dev/null || true
    pkill -x ydotoold 2>/dev/null || true
    rm -f "$SOCKET"
    echo "ydotoold stopped"
    ;;

  socket)
    # Which path the unprivileged side looks for. Printed so that a mismatch
    # between the two halves is obvious instead of silent.
    echo "$SOCKET"
    ;;

  where)
    # Run as the invoking user to confirm both halves agree. This is the check
    # that would have caught the /run/user/0 mismatch immediately.
    echo "socket path:    $SOCKET"
    echo "directory:      $RUNTIME_DIR ($(stat -c '%A %U:%G' "$RUNTIME_DIR" 2>/dev/null || echo missing))"
    if [ -S "$SOCKET" ]; then
      echo "socket:         present"
    else
      echo "socket:         ABSENT"
      echo "check as the invoking user with: ls -l $SOCKET"
    fi
    ;;

  whereami)
    # Deliberately does not need root: this is the side the test runs on.
    _me_uid="$(id -u)"
    _dir="/run/user/$_me_uid"
    echo "uid:            $_me_uid"
    echo "XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-unset}"
    echo "looking for:    $_dir/.ydotool_socket"
    if [ -S "$_dir/.ydotool_socket" ]; then
      echo "socket:         present"
    else
      echo "socket:         absent"
    fi
    if [ -w /dev/uinput ]; then
      echo "/dev/uinput:    writable by this user"
    else
      echo "/dev/uinput:    NOT writable by this user (needs the daemon)"
    fi
    ;;

  status)
    if [ -S "$SOCKET" ]; then
      echo "ydotoold running on $SOCKET ($(stat -c '%A %U:%G' "$SOCKET"))"
      # A root-owned 0600 socket is present but unusable by the test, which is
      # the failure mode -o exists to prevent.
      if [ "$(id -u)" != 0 ] && [ ! -r "$SOCKET" ]; then
        echo "WARNING: not readable by uid $(id -u); the daemon was started"
        echo "without --socket-own, so the test cannot connect to it."
        echo "stop it and start it again as: sudo $0 start"
      fi
      exit 0
    fi
    echo "ydotoold not running"
    exit 1
    ;;

  *) usage ;;
esac