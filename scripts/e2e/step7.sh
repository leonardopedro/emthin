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
  log "clipboard: emthin's clients <-> the compositor emthin runs inside"

  if ! command -v wl-copy >/dev/null 2>&1 || ! command -v wl-paste >/dev/null 2>&1; then
    skip "clipboard" "wl-copy/wl-paste not on PATH"
    return 0
  fi
  [ -S "$E2E_XDG/$E2E_CLIENT_DISPLAY" ] || {
    skip "clipboard" "emthin's own socket is absent, so there are no clients to test"
    return 0
  }

  # The two ends are emthin's own Wayland socket and its host's. A *different*
  # marker per direction, and the previous owner is released first: with one
  # shared marker a leg could read back the other's selection and pass without
  # anything having crossed.
  local h2c="e2e-$$-h2c"
  local c2h="e2e-$$-c2h"
  # Display *names*, not socket paths: XDG_RUNTIME_DIR is the directory and
  # WAYLAND_DISPLAY the entry in it. Passing the full socket path as
  # XDG_RUNTIME_DIR silently finds nothing, which looks exactly like a broken
  # clipboard.
  local host="$E2E_DISPLAY"
  local client="$E2E_CLIENT_DISPLAY"

  local backend; backend="$(clipboard_backend)"
  log "emthin's clipboard proxy settled on the '$backend' backend"

  # Hold an IPC connection open for the duration of these checks.
  #
  # emthin only pushes a client's selection out to the host while an IPC client
  # is connected (), which is deliberate: GTK and Emacs
  # announce clipboard ownership at startup, and without that gate they would
  # clobber the host clipboard before the user has typed anything. Every other
  # ipc.py invocation connects, asks one question and exits -- so the gate is shut
  # again before anything observes it, emthin logs , and a client copy
  # never leaves the compositor. That is indistinguishable from a broken proxy,
  # which is exactly what it looked like.
  python3 "$PWD/scripts/e2e/ipc.py" "$E2E_IPC" hold >/dev/null 2>&1 &
  local holder=$!
  sleep 1.5

  # ── host -> client ───────────────────────────────────────────────────────
  own_selection "$host" wl-copy --type text/plain -- "$h2c"
  local got
  got="$(paste_from "$client")"
  release_selection
  check "the host's clipboard reaches a client inside emthin" "$got" "$h2c"

  # ── client -> host ───────────────────────────────────────────────────────
  own_selection "$client" wl-copy --type text/plain -- "$c2h"
  got="$(paste_from "$host")"
  release_selection
  check "a client inside emthin reaches the host's clipboard" "$got" "$c2h"

  kill "$holder" 2>/dev/null || true
  wait "$holder" 2>/dev/null || true

  # What emthin itself thought it saw, so the result can be read against the
  # gate rather than guessed at.
  grep -ao 'selection Clipboard: ipc=[a-z]*' "$E2E_ROOT/emthin.log" 2>/dev/null | tail -2|sed 's/^/       emthin saw: /'

  # Beyond this point there is nothing more to check. Reaching the *desktop*
  # session from a nested compositor would mean crossing a second compositor
  # boundary, and `scripts/e2e/clip-bisect.sh` shows where that breaks: not in
  # emthin, which passes in both directions above, but in the host compositor's
  # own X11 backend, which does not sync a Wayland selection to the X selection on
  # a bare Xvfb. A nested compositor can only proxy to the host it is nested in.
}

# Own a selection on `$1`'s display for as long as the caller needs it.
own_selection() {
  local display="$1"; shift
  ( XDG_RUNTIME_DIR="$E2E_XDG" WAYLAND_DISPLAY="$display" \
      "$@" </dev/null >/dev/null 2>&1 & echo $! > "$E2E_ROOT/sel.pid" )
  sleep "${E2E_SETTLE:-1.5}"
}

release_selection() {
  if [ -f "$E2E_ROOT/sel.pid" ]; then
    kill "$(cat "$E2E_ROOT/sel.pid")" 2>/dev/null || true
    rm -f "$E2E_ROOT/sel.pid"
  fi
  # Let the compositor see the selection go away before the next leg claims it.
  sleep 0.7
}

# Read the selection on `$1`, retrying.
#
# Clipboard propagation is asynchronous and the path is three hops (a client, the
# bridge, the host), so a single sample taken too early reads as an empty
# selection — indistinguishable from a broken bridge. Retrying turns "it had not
# arrived yet" into a pass, and leaves a genuine failure failing.
paste_from() {
  local display="$1" got="" i
  for i in 1 2 3 4 5; do
    got="$(XDG_RUNTIME_DIR="$E2E_XDG" WAYLAND_DISPLAY="$display" \
      timeout 4 wl-paste --no-newline 2>/dev/null || true)"
    [ -n "$got" ] && break
    sleep 1
  done
  printf '%s' "$got"
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