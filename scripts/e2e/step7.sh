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

# run.sh sources this file after lib.sh; sourcing lib again would reset the
# assertion counters that already include the per-document preference checks.
if ! declare -F check >/dev/null 2>&1; then
  # shellcheck source=lib.sh
  . "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")/lib.sh"
fi

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
  if [ -n "${E2E_DUMP_STATE:-}" ]; then
    # Debug affordance: dump the raw list_state frames next to the run so an
    # empty document can be told apart from a parser that found no `state`
    # notification (the two look identical through `doc_from_state`).
    python3 "$PWD/scripts/e2e/ipc.py" "$E2E_IPC" list_state \
      >"$E2E_RUN/last_list_state.json" 2>"$E2E_RUN/last_list_state.err"
    doc_from_state <"$E2E_RUN/last_list_state.json"
  else
    python3 "$PWD/scripts/e2e/ipc.py" "$E2E_IPC" list_state | doc_from_state
  fi
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

# `page page_count` from the live state notification.
page_from_state() {
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
        p = obj["params"]
        print(p["page"], p["page_count"])
        break
'
}

current_page_state() {
  python3 "$PWD/scripts/e2e/ipc.py" "$E2E_IPC" list_state | page_from_state
}

# The compact status line, or an empty string in quiet state.
status_from_state() {
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
        print(obj["params"].get("status") or "")
        break
'
}

current_status() {
  python3 "$PWD/scripts/e2e/ipc.py" "$E2E_IPC" list_state | status_from_state
}

await_status_contains() {
  local needle=$1 status="" deadline=$((SECONDS + ${2:-10}))
  while :; do
    status="$(current_status 2>/dev/null || true)"
    case "$status" in
      *"$needle"*) printf '%s\n' "$status"; return 0 ;;
    esac
    if [ "$SECONDS" -ge "$deadline" ]; then
      printf '%s\n' "$status"
      return 1
    fi
    sleep 0.1
  done
}

await_status_lacks() {
  local needle=$1 status="" deadline=$((SECONDS + ${2:-10}))
  while :; do
    status="$(current_status 2>/dev/null || true)"
    case "$status" in
      *"$needle"*) ;;
      *) printf '%s\n' "$status"; return 0 ;;
    esac
    if [ "$SECONDS" -ge "$deadline" ]; then
      printf '%s\n' "$status"
      return 1
    fi
    sleep 0.1
  done
}

# Poll until the visible page is `$1`, returning the final `page page_count`.
await_page() {
  local want=$1 state="" page count deadline=$((SECONDS + 10))
  while :; do
    state="$(current_page_state 2>/dev/null || true)"
    if [ -n "$state" ]; then
      read -r page count <<<"$state"
      [ "$page" = "$want" ] && { printf '%s\n' "$state"; return 0; }
    fi
    if [ "$SECONDS" -ge "$deadline" ]; then
      printf '%s\n' "$state"
      return 1
    fi
    sleep 0.1
  done
}

wayland_only_check() {
  echo >&2
  log "transport check: does emthin hold any X11 connection?"

  local pid; pid="$(emthin_pid)"
  if [ -z "$pid" ]; then
    skip "wayland-only transport" "emthin is not running"
    return 0
  fi

  # Check the process's own descriptors, not the log. A log line can announce
  # one backend while the other sits quietly connected -- which is exactly how an
  # X dependency hides: `Clipboard backend: data_control active` was true while
  # arboard was still an X11 client.
  local x11found="" ino name
  for fd in /proc/"$pid"/fd/*; do
    # `case`, not `[ ... = socket:* ]`: inside `[` the right-hand side is a
    # literal, so that test never matches and the loop silently inspects
    # nothing -- which is how this check would have passed forever.
    case "$(readlink "$fd" 2>/dev/null)" in
      socket:*) ;;
      *) continue ;;
    esac
    ino="$(stat -L -c %i "$fd" 2>/dev/null)" || continue
    name="$(awk -v i="$ino" '$7==i {print $8}' /proc/net/unix 2>/dev/null | head -1)"
    case "$name" in
      *X11-unix*|*"@/tmp/.X11"*) x11found="$name" ;;
    esac
  done

  if [ -n "$x11found" ]; then
    check "emthin opens no X11 connection" "$x11found" "(none)"
  else
    check "emthin opens no X11 connection" "(none)" "(none)"
    log "  DISPLAY seen by emthin: $(tr '\0' '\n' < "/proc/$pid/environ" | grep '^DISPLAY=' || echo '(unset)')"
  fi
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

  multipage_checks
}

# Page navigation through the same injected keyboard path. A fresh document is
# used so the check does not depend on the preference fixture's page count.
multipage_checks() {
  echo >&2
  log "multi-page navigation: PgDn/PgUp via the keyboard path"

  local doc="$E2E_RUN/multipage.typ"
  python3 - "$doc" <<'PY'
import sys
text = ("A page-navigation fixture line with enough text to paginate. " * 8 + "\n\n") * 40
text += "findcancelprobe\n"
with open(sys.argv[1], "w", encoding="utf-8") as f:
    f.write(text)
PY
  ipc open_doc "{\"path\":\"$doc\"}" >/dev/null 2>&1 || true

  local state="" page count deadline=$((SECONDS + 10))
  while :; do
    state="$(current_page_state 2>/dev/null || true)"
    if [ -n "$state" ]; then
      read -r page count <<<"$state"
      [ "$count" -ge 3 ] && break
    fi
    if [ "$SECONDS" -ge "$deadline" ]; then
      skip "multi-page navigation" "fixture laid out ${count:-0} page(s); need >= 3"
      return 0
    fi
    sleep 0.1
  done

  # Start from a known page, then move only with keys.
  ipc goto_page '{"page":0}' >/dev/null 2>&1 || true
  await_page 0 >/dev/null || true

  local advanced=0 returned=0
  inject_key Page_Down || warn "Page_Down was not delivered"
  if await_page 1 >/dev/null; then advanced=1; fi
  check "PgDn advances to the next page" "$advanced" "1"

  inject_key Page_Up || warn "Page_Up was not delivered"
  if await_page 0 >/dev/null; then returned=1; fi
  check "PgUp returns to the previous page" "$returned" "1"

  find_cancel_checks
}

# Open Find, jump to a unique probe on a later page, then cancel with Escape.
# The check deliberately asserts both the close and the restored origin, since
# a bar that closes but leaves the reader at the match has not cancelled.
find_cancel_checks() {
  # wtype's synthetic Ctrl modifier is not delivered with the following key:
  # emthin sees `KEY_f` with no ctrl and types a literal `f` into the document.
  # Find is therefore XTEST-only, the same boundary as the IME check, until the
  # wtype path can carry a chord.
  [ "${CONFIG:-}" = xdotool ] || return 0

  echo >&2
  log "find: Escape cancels and restores the pre-search page"

  if ! inject_available; then
    skip "Find cancel" "no injection available"
    return 0
  fi

  local doc="$E2E_RUN/multipage.typ"
  ipc open_doc "{\"path\":\"$doc\"}" >/dev/null 2>&1 || true
  if ! await_page 0 >/dev/null; then
    skip "Find cancel" "fixture did not return to page 0"
    return 0
  fi
  local before
  before="$(current_document)"

  inject_key ctrl+f || warn "Ctrl+F was not delivered"
  local opened=0
  if await_status_contains "Find" 5 >/dev/null; then opened=1; fi
  check "Ctrl+F opens the Find bar" "$opened" "1"

  inject_type "findcancelprobe" || true
  local matched=0
  if await_status_contains "Find 1/1" 10 >/dev/null; then matched=1; fi
  check "Find reports the unique probe as 1/1" "$matched" "1"

  # F3 is a global binding and remains available while the bar is open. It
  # selects the match and reveals its page, which gives Escape something to
  # restore.
  inject_key F3 || warn "F3 was not delivered"
  local moved=0 state="" page count deadline=$((SECONDS + 10))
  while :; do
    state="$(current_page_state 2>/dev/null || true)"
    if [ -n "$state" ]; then
      read -r page count <<<"$state"
      [ "$page" -gt 0 ] && { moved=1; break; }
    fi
    [ "$SECONDS" -ge "$deadline" ] && break
    sleep 0.1
  done
  check "F3 reveals the matching page" "$moved" "1"

  inject_key Escape || warn "Escape was not delivered"
  local closed=0 restored=0 after=""
  if await_status_lacks "Find" 5 >/dev/null; then closed=1; fi
  if await_page 0 >/dev/null; then restored=1; fi
  after="$(current_document)"
  check "Escape cancels Find" "$closed" "1"
  check "cancelling Find restores the pre-search page" "$restored" "1"
  check "cancelling Find leaves the document unchanged" "$after" "$before"
}

# fcitx5 commits into the focused text-input-v3 client. wtype delivers keys after
# wlroots' input-method grab, so this is an XTEST-lane check by construction; the
# other lanes neither claim nor count it.
ime_checks() {
  [ "${CONFIG:-}" = xdotool ] || return 0

  if [ "${IME_READY:-0}" != 1 ]; then
    skip "fcitx5 IME commit" "fcitx5/rime did not start; see $E2E_ROOT/fcitx5.log"
    return 0
  fi
  if ! command -v kgx >/dev/null 2>&1; then
    skip "fcitx5 IME commit" "kgx is not available as the text-input client"
    return 0
  fi

  echo >&2
  log "IME: fcitx5/rime -> emthin text-input-v3 -> kgx"

  local probe="$E2E_RUN/ime-probe.typ"
  printf '#1 IME probe #2 \\app(#1, #2, 600, 300, "kgx")\n' > "$probe"
  ipc open_doc "{\"path\":\"$probe\"}" >/dev/null 2>&1 || true
  sleep 1
  ipc spawn '{"cmd":"kgx"}' >/dev/null 2>&1 || true

  local st="" bound=0
  for _ in $(seq 1 80); do
    st="$(state 2>/dev/null || true)"
    case "$st" in
      *f0*) case "$st" in *"window=None"*) ;; *) bound=1; break ;; esac ;;
    esac
    sleep 0.25
  done
  if [ "$bound" != 1 ]; then
    skip "fcitx5 IME commit" "kgx did not bind to a figure; state: ${st:-empty}"
    return 0
  fi

  ipc focus '{"figure":"f0"}' >/dev/null 2>&1 || true
  sleep 1
  # GTK/VTE requests text-input-v3 lazily, only after the first key. That key is
  # deliberately harmless: it only exists to make the client enable the IME.
  inject_type "x" || true
  local enabled=0
  for _ in $(seq 1 120); do
    if grep -aq 'winit.set_ime_allowed(true)' "$E2E_ROOT/emthin.log" 2>/dev/null; then
      enabled=1
      break
    fi
    sleep 0.25
  done
  if [ "$enabled" != 1 ]; then
    skip "fcitx5 IME commit" "no text-input-v3 client enabled the host IME"
    return 0
  fi
  sleep 1

  # "nihao" is Rime's pinyin for 你好; space selects the first candidate, which
  # fcitx emits as a commit to emthin's host, then to the focused text-input
  # client. The log line is the assertion because kgx's pty may render the text
  # without exposing a stable file to read.
  inject_type "nihao" || true
  local preedit_line=""
  for _ in $(seq 1 40); do
    preedit_line="$(grep -anF 'winit Ime event: Preedit("ni hao"' \
      "$E2E_ROOT/emthin.log" 2>/dev/null | tail -1 | cut -d: -f1 || true)"
    [ -n "$preedit_line" ] && break
    sleep 0.25
  done
  inject_key space || true

  local committed=0 commit_line=""
  for _ in $(seq 1 40); do
    commit_line="$(grep -anF 'winit Ime event: Commit("你好")' \
      "$E2E_ROOT/emthin.log" 2>/dev/null | tail -1 | cut -d: -f1 || true)"
    if [ -n "$commit_line" ]; then
      committed=1
      break
    fi
    sleep 0.25
  done
  check "fcitx5 commits 你好 through emthin's IME host" "$committed" "1"

  # The preedit is the half-typed pinyin, not the final glyph. Assert both that
  # it appeared and that it was logged before the commit, so a client that
  # jumps straight to Commit cannot pass by matching the final line alone.
  local ordered=0
  if [ -n "$preedit_line" ] && [ -n "$commit_line" ] &&
    [ "$preedit_line" -lt "$commit_line" ]; then
    ordered=1
  fi
  check "fcitx5 logs the preedit before the commit" "$ordered" "1"

  ipc close '{"figure":"f0"}' >/dev/null 2>&1 || true
  ipc focus '{"figure":null}' >/dev/null 2>&1 || true
}

wayland_only_check
clipboard_checks
keyboard_checks
ime_checks
finish