#!/usr/bin/env bash
# The whole `ydotool` route in one command.
#
#   ./scripts/e2e/step7-ydotool.sh
#
# Two `sudo` calls bracket an unprivileged test run, which is the arrangement
# that keeps privilege as small as it can be: only `ydotoold` runs as root, and
# only because it has to hold /dev/uinput. The test itself — and everything it
# observes — runs as you.
#
# Splitting this into three commands by hand is how the earlier socket-path bug
# got as far as it did: the daemon was started in one shell and the test run in
# another. Here they cannot drift, and `whereami` in the middle checks that both
# halves agree on the socket path before any key is sent.

set -uo pipefail

HERE="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
DAEMON="$HERE/ydotoold-daemon.sh"
RUN="$HERE/run.sh"

log() { printf '\033[1;34m==>\033[0m %s\n' "$*" >&2; }

cleanup() {
  log "stopping the daemon"
  sudo -n "$DAEMON" stop >/dev/null 2>&1 || sudo "$DAEMON" stop >/dev/null 2>&1 || true
}

if ! command -v sudo >/dev/null 2>&1; then
  echo "this configuration needs sudo, for /dev/uinput. Try --config xdotool instead." >&2
  exit 1
fi

trap cleanup EXIT

log "starting ydotoold (sudo; prompts for your password)"
if ! sudo "$DAEMON" start; then
  echo >&2
  echo "The daemon did not start, so there is no way to inject keys here." >&2
  echo "The two usual reasons, both reported by the script above:" >&2
  echo "  * /dev/uinput is not writable even as root -> the uinput module is not" >&2
  echo "    loadable in this environment (a container without --device /dev/uinput)." >&2
  echo "  * ydotoold exits immediately -> run it in the foreground; the script" >&2
  echo "    prints the exact command." >&2
  exit 1
fi

# The check that would have caught the /run/user/0 mismatch: both sides must be
# looking at the same socket. Runs unprivileged, because that is the side that
# matters.
log "checking both halves agree on the socket"
if ! "$DAEMON" whereami; then
  echo >&2
  echo "The daemon is up but the socket is not where the test looks for it." >&2
  exit 1
fi

log "running §6 step 7 with --config ydotool"
"$RUN" --config ydotool
rc=$?

case "$rc" in
  0) log "step 7 passed" ;;
  2) log "step 7 verified nothing — every check was skipped" ;;
  *) log "step 7 failed (exit $rc)" ;;
esac
exit "$rc"