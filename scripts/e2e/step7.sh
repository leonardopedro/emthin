#!/usr/bin/env bash
# §6 step 7: IME, and clipboard host<->client and doc-copy.
#
# The keyboard half is written against the injection seam in lib.sh, so it runs
# unchanged under any of the configurations in inject-*.sh. The clipboard half
# needs no keys at all and is always run: `wl-copy` on the host session and
# `wl-paste` in the nested one exercises host->client through emthin's proxy,
# and the reverse exercises client->host.
#
# Run via ./run.sh --config <name>.

# shellcheck source=lib.sh
. "$(dirname "$(readlink -f "$0")")/lib.sh"

HOST_WAYLAND="${E2E_HOST_WAYLAND:-wayland-0}"
SKIP_CLIPBOARD=0

# ── part 1: the clipboard, no injection required ────────────────────────────
#
# `wl-copy` must be held running while something reads, because it owns the
# selection; that is what makes this a two-process dance rather than a command.

# Which clipboard backend did emthin's proxy settle on?
#
# It matters, because the fallback chain can end somewhere that cannot reach the
# clipboard the test is using. The chain is DataControl -> WlDataDevice -> X11,
# and in a nested session the first two are aimed at `$WAYLAND_DISPLAY`, which is
# emthin itself:
#
#   data_control   "Host supports neither ext_data_control_v1 nor
#                   zwlr_data_control_v1" — emthin offers neither to its own
#                   clients, so it asks itself and gets no.
#   wl_data_device the roundtrip pipe breaks, same reason.
#   x11            initialised, against the host's Xwayland via $DISPLAY.
#
# So a `wl-copy` on the host sets a *Wayland* selection while the bridge is
# listening on *X11*, and whether the two meet depends on the host's
# Wayland<->X11 selection sync. That is a property of this nesting arrangement,
# not of emthin, but it does mean this check cannot pass here — so it is
# reported with the reason rather than as a bare failure.
clipboard_backend() {
  local log="${E2E_ROOT:-.e2e}/emthin.log"
  [ -f "$log" ] || { printf 'unknown (no log)'; return; }
  local line
  line="$(grep -a 'X11 clipboard sync initialized' "$log" | tail -1 || true)"
  if [ -n "$line" ]; then printf 'x11'; return; fi
  line="$(grep -a 'Clipboard backend:' "$log" | tail -1 || true)"
  case "$line" in
    *x11*)          printf 'x11' ;;
    *data_control*) printf 'data_control' ;;
    *wl_data_device*) printf 'wl_data_device' ;;
    *)              printf 'none' ;;
  esac
}

# The current document text, straight out of `list_state`.
#
# Read over IPC rather than off the screen, which is the whole point: it does not
# depend on rendering, so it still tells the truth about a regression that leaves
# the page stale.
current_document() {
  python3 "$PWD/scripts/e2e/ipc.py" "$E2E_IPC" list_state | doc_from_state
}

doc_from_state() {
  python3 -c '
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
}

clipboard_checks() {
  echo >&2
  log "clipboard host<->client through emthin's proxy (no keys needed)"

  if ! command -v wl-copy >/dev/null 2>&1 || ! command -v wl-paste >/dev/null 2>&1; then
    warn "wl-copy/wl-paste not on PATH; skipping the clipboard checks"
    return 0
  fi
  [ -S "$E2E_XDG/$E2E_DISPLAY" ] || { warn "no nested socket; skipping"; return 0; }

  # The two ends must be *different compositors*, or the test proves nothing:
  # wl-copy and wl-paste both talking to the nested session would just be one
  # client talking to itself, with emthin in the middle as the compositor rather
  # than as the thing bridging two clipboards. The host end is therefore the
  # real session's socket, and only the proxy is under test.
  local host_runtime="${E2E_HOST_RUNTIME:-/run/user/$(id -u)}"
  local host_socket="$host_runtime/$HOST_WAYLAND"
  if [ ! -S "$host_socket" ]; then
    warn "no host Wayland socket at $host_socket."
    warn "Set E2E_HOST_WAYLAND / E2E_HOST_RUNTIME to point at the real session."
    warn "Without one there is no host clipboard to bridge to, so host<->client"
    warn "cannot be checked at all — only what a client sees inside the session."
  fi

  local backend; backend="$(clipboard_backend)"
  log "emthin's clipboard proxy settled on the '$backend' backend"
  if [ "$backend" = x11 ]; then
    warn "the bridge is on X11 (via \$DISPLAY -> Xwayland) while this test drives"
    warn "the host's *Wayland* selection. Whether they meet depends on the host's"
    warn "Wayland<->X11 selection sync, which is not happening here — so the two"
    warn "checks below are expected to fail, and are reported as SKIPPED."
    SKIP_CLIPBOARD=1
  fi

  # A *different* marker per direction. Sharing one made a leak between the two
  # legs indistinguishable from success: the client->host leg could read back the
  # host selection the host->client leg had just left there, and pass without
  # anything having crossed the bridge.
  local marker_h2c="e2e-$$-h2c"
  local marker_c2h="e2e-$$-c2h"

  # ── host -> client ───────────────────────────────────────────────────────
  if [ -S "$host_socket" ]; then
    ( XDG_RUNTIME_DIR="$host_runtime" WAYLAND_DISPLAY="$HOST_WAYLAND" \
        wl-copy --type text/plain -- "$marker_h2c" </dev/null >/dev/null 2>&1 &
      echo $! > "$E2E_ROOT/wlcopy.pid" )
    sleep 1.5
    local got
    got="$(XDG_RUNTIME_DIR="$E2E_XDG" WAYLAND_DISPLAY="$E2E_DISPLAY" \
            timeout 4 wl-paste --no-newline 2>/dev/null || true)"
    if [ "${SKIP_CLIPBOARD:-0}" = 1 ]; then
      skip "host->client clipboard" "the bridge backend cannot carry a Wayland selection"
    else
      check "host clipboard reaches a client inside the nested session" "$got" "$marker_h2c"
    fi
    kill "$(cat "$E2E_ROOT/wlcopy.pid" 2>/dev/null)" 2>/dev/null || true
    rm -f "$E2E_ROOT/wlcopy.pid"
  else
    warn "skipping host->client"
  fi

  # ── client -> host ───────────────────────────────────────────────────────
  # This is the direction that matters for the document: text copied out of the
  # document has to be pasteable in an app, which is what the bridge's
  # HostSelectionChanged echo and its `SelectionOrigin::Host` bookkeeping exist
  # for. A client inside the session owns the selection; the host must read it.
  # Wait for the previous owner to go before taking the selection ourselves, so
  # the host is not still holding the other direction's marker.
  kill "$(cat "$E2E_ROOT/wlcopy.pid" 2>/dev/null)" 2>/dev/null || true
  rm -f "$E2E_ROOT/wlcopy.pid"
  sleep 0.7
  ( XDG_RUNTIME_DIR="$E2E_XDG" WAYLAND_DISPLAY="$E2E_DISPLAY" \
      wl-copy --type text/plain -- "$marker_c2h" </dev/null >/dev/null 2>&1 &
    echo $! > "$E2E_ROOT/wlcopy.pid" )
  sleep 1.5
  if [ -S "$host_socket" ]; then
    local back
    back="$(XDG_RUNTIME_DIR="$host_runtime" WAYLAND_DISPLAY="$HOST_WAYLAND" \
            timeout 4 wl-paste --no-newline 2>/dev/null || true)"
    if [ "${SKIP_CLIPBOARD:-0}" = 1 ]; then
      skip "client->host clipboard" "the bridge backend cannot carry a Wayland selection"
    else
      check "a client inside the nested session reaches the host clipboard" \
        "$back" "$marker_c2h"
    fi
  else
    # No host session: the best that can be asserted is that the selection is
    # readable *within* the session, which at least proves wl-copy took.
    local inside
    inside="$(XDG_RUNTIME_DIR="$E2E_XDG" WAYLAND_DISPLAY="$E2E_DISPLAY" \
              timeout 4 wl-paste --no-newline 2>/dev/null || true)"
    check_contains "a client can own a selection in the nested session" \
      "$inside" "$marker_c2h"
    warn "client->host NOT verified: no host session to bridge to"
  fi
  kill "$(cat "$E2E_ROOT/wlcopy.pid" 2>/dev/null)" 2>/dev/null || true
  rm -f "$E2E_ROOT/wlcopy.pid"
}

# ── part 2: the keyboard ────────────────────────────────────────────────────
#
# The first check is deliberately `typing reaches the document`, because that is
# the regression 5ce7751 fixed and the one most worth keeping honest: before it,
# a keystroke changed the model and was autosaved while the page kept showing the
# previous raster, so nothing appeared until something unrelated rebuilt the
# layout. IPC is the only way to see it — the document text comes back through
# `list_state`, which does not depend on rendering.

keyboard_checks() {
  echo >&2
  log "keyboard path via: $(inject_note)"

  if ! inject_available; then
    warn "no injection available in this configuration; skipping the keyboard checks"
    echo >&2
    warn "§6 step 7's keyboard half is therefore still unverified."
    return 0
  fi

  local marker="e2e$$"
  if ! inject_type "$marker"; then
    warn "the backend reported a failed delivery; the checks below will not mean much"
  fi
  # Give the compositor a moment to route the keys through the seat.
  sleep "${E2E_SETTLE:-1.5}"
  local doc
  doc="$(current_document)"
  check_contains "typed text reaches the document" "$doc" "$marker"

  # doc-copy: select the marker, Ctrl+C, move, Ctrl+V. Exercises the arboard
  # path that 1a79b38 fixed — before it, `host_clipboard` read a handle that
  # only copy created, so the first paste of a session did nothing.
  inject_key ctrl+a || warn "ctrl+a was not delivered"
  inject_key ctrl+c || warn "ctrl+c was not delivered"
  inject_key End    || warn "End was not delivered"
  inject_key Return || warn "Return was not delivered"
  inject_type "$marker" || warn "the second burst was not delivered"
  inject_key ctrl+v || warn "ctrl+v was not delivered"
  sleep 0.5
  local doc2
  doc2="$(current_document)"
  local count
  count="$(printf '%s' "$doc2" | grep -o "$marker" | wc -l | tr -d ' ')"
  CHECKS_RAN=$(( CHECKS_RAN + 1 ))
  if [ "${count:-0}" -ge 2 ]; then
    ok "doc-copy pastes (the marker appears $count times)"
  else
    printf '\033[1;31mFAIL\033[0m doc-copy: the marker appears %s time(s), want >= 2\n' \
      "${count:-0}" >&2
    printf '      document: %s\n' "$doc2" >&2
    FAILED=$(( FAILED + 1 ))
  fi

  # IME commit into the document caret: with fcitx5 running under the nested
  # session, a commit arrives as a text-input-v3 commit and must land in the
  # document. Reported rather than asserted, because whether an IME is actually
  # engaged depends on the host's fcitx configuration and is not something this
  # script can force.
  if command -v fcitx5 >/dev/null 2>&1; then
    log "fcitx5 present; an IME commit needs the host's fcitx to be engaged"
  else
    warn "fcitx5 not on PATH; the IME commit cannot be exercised at all"
  fi
}

clipboard_checks
keyboard_checks
finish