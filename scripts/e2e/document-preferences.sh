#!/usr/bin/env bash
# Per-document view, reading, and citation preferences over IPC.
#
# The session is re-used instead of started here: `prepare_document_preferences`
# runs before `start_emthin`, and `document_preferences_checks` runs after the
# session is up. That lets the fixture put the two documents' preferences in
# `session.json` before the first load, while the switches themselves go through
# the real `open_doc` IPC method.
#
# No input injection is needed for the preference part: opening each document,
# then reading its page, reading size, zoom and citation format back through
# `list_state`, exercises the restore path; the saved JSON is checked after
# each switch so a default overwriting a saved preference cannot hide behind an
# in-memory value.
#
# Sourced by run.sh, so it uses the shared `check`/`log` helpers and does not
# call `finish`.

PREF_DOC_A="$E2E_RUN/preferences-a.doc"
PREF_DOC_B="$E2E_RUN/preferences-b.doc"
PREF_SESSION_JSON="$E2E_RUN/session.json"

# Write the two documents and their distinct preferences. Called before emthin
# starts, so the first `open_doc` already sees both entries.
prepare_document_preferences() {
  mkdir -p "$E2E_RUN"
  # The snapshot from the previous run restores that run's document at its page
  # and autosaves it before the fixture is opened, overwriting session.json. The
  # fixture is the state under test, so start from no snapshot.
  rm -f "$E2E_DOC" "$E2E_DOC.lock"
  python3 - "$PREF_DOC_A" "$PREF_DOC_B" "$PREF_SESSION_JSON" <<'PY'
import json
import sys

doc_a, doc_b, session = sys.argv[1:]
text = ("Per-document preferences must survive switching files. " * 8 + "\n\n") * 20
for path in (doc_a, doc_b):
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)

def key(path):
    import os
    return os.path.realpath(path)

payload = {
    "version": 1,
    "documents": {
        key(doc_a): {
            "view_zoom": 2.0,
            "view_pan_x": 0.0,
            "view_pan_y": 24.0,
            "citation_style": "vancouver",
            "citation_locale": "de-DE",
            "current_page": 1,
            "text_size": 20.0,
        },
        key(doc_b): {
            "view_zoom": 0.75,
            "view_pan_x": 0.0,
            "view_pan_y": 0.0,
            "citation_style": "mla",
            "citation_locale": "fr-FR",
            "current_page": 2,
            "text_size": 14.0,
        },
    },
}
with open(session, "w", encoding="utf-8") as f:
    json.dump(payload, f)
PY
}

# page reading_size page_count, from the live state notification.
preference_state() {
  python3 "$PWD/scripts/e2e/ipc.py" "$E2E_IPC" list_state | python3 -c '
import json, sys
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
        page, text_value, count = p["page"], p["text_size"], p["page_count"]
        zoom, pan_x, pan_y = p["view_zoom"], p["view_pan_x"], p["view_pan_y"]
        style, locale = p["citation_style"], p["citation_locale"]
        print(
            f"{page} {text_value:g} {count} {zoom:g} {pan_x:g} {pan_y:g} "
            f"{style} {locale}"
        )
'
}

# Poll the live state until the selected document has settled. A fixed sleep
# here is a guess about how long the compositor needs; this waits for the
# values the checks actually assert, and fails at the deadline with the last
# state for the diagnostic.
await_live_preferences() {
  local want_page=$1 want_text=$2 want_zoom=$3 want_style=$4 want_locale=$5
  local deadline=$((SECONDS + 10))
  local state page text_value count zoom pan_x pan_y style locale
  while :; do
    state="$(preference_state 2>/dev/null || true)"
    if [ -n "$state" ]; then
      read -r page text_value count zoom pan_x pan_y style locale <<<"$state"
      if [ "$page" = "$want_page" ] &&
        [ "$text_value" = "$want_text" ] &&
        [ "$zoom" = "$want_zoom" ] &&
        [ "$style" = "$want_style" ] &&
        [ "$locale" = "$want_locale" ]; then
        printf '%s\n' "$state"
        return 0
      fi
    fi
    if [ "$SECONDS" -ge "$deadline" ]; then
      printf '%s\n' "$state"
      return 1
    fi
    sleep 0.1
  done
}

# The session file's write time. The load path saves the outgoing document
# before replacing it, so a switch changes this without waiting for autosave.
session_stamp() {
  stat -c '%y' "$PREF_SESSION_JSON" 2>/dev/null || true
}

# Wait for the session file to be written after a switch. The deadline is a
# failure bound, not the expected duration; the write normally lands with the
# `open_doc` reply.
await_session_write() {
  local before=$1
  local deadline=$((SECONDS + 15))
  while [ "$(session_stamp)" = "$before" ]; do
    if [ "$SECONDS" -ge "$deadline" ]; then
      warn "session.json was not written within the deadline"
      return 1
    fi
    sleep 0.05
  done
}

# The saved preference fields, one whitespace-delimited line per document, so a
# failed restore is visible as a changed value rather than a silent overwrite.
saved_preferences() {
  python3 - "$PREF_SESSION_JSON" "$PREF_DOC_A" "$PREF_DOC_B" <<'PY'
import json
import os
import sys

path, doc_a, doc_b = sys.argv[1:]
with open(path, encoding="utf-8") as f:
    documents = json.load(f)["documents"]

def fields(path):
    p = documents.get(os.path.realpath(path), {})
    print(
        f"{p.get('view_zoom', 1.0):g} {p.get('citation_style')} "
        f"{p.get('citation_locale')} {p.get('current_page', 0)} "
        f"{p.get('text_size', 17.0):g}",
        end=" ",
    )

fields(doc_a)
fields(doc_b)
print()
PY
}

document_preferences_checks() {
  echo >&2
  log "per-document preferences: two documents over open_doc"

  python3 "$PWD/scripts/e2e/ipc.py" "$E2E_IPC" open_doc \
    "{\"path\":\"$PREF_DOC_A\"}" >/dev/null 2>&1
  local state a_page a_text a_count a_zoom a_pan_x a_pan_y a_style a_locale
  if ! state="$(await_live_preferences 1 20 2 vancouver de-DE)"; then
    warn "document A's live state did not settle before the deadline"
  fi
  read -r a_page a_text a_count a_zoom a_pan_x a_pan_y a_style a_locale <<<"$state"
  check "document A restores its page" "$a_page" "1"
  check "document A restores its reading size" "$a_text" "20"
  check "document A restores its view zoom" "$a_zoom" "2"
  check "document A restores its citation style" "$a_style" "vancouver"
  check "document A restores its citation locale" "$a_locale" "de-DE"
  if [ "${a_count:-0}" -lt 2 ]; then
    warn "the fixture laid out only $a_count page(s); page restore is not distinct"
  fi

  python3 "$PWD/scripts/e2e/ipc.py" "$E2E_IPC" open_doc \
    "{\"path\":\"$PREF_DOC_B\"}" >/dev/null 2>&1
  local b_page b_text b_count b_zoom b_pan_x b_pan_y b_style b_locale
  if ! state="$(await_live_preferences 2 14 0.75 mla fr-FR)"; then
    warn "document B's live state did not settle before the deadline"
  fi
  read -r b_page b_text b_count b_zoom b_pan_x b_pan_y b_style b_locale <<<"$state"
  check "document B restores its page" "$b_page" "2"
  check "document B restores its reading size" "$b_text" "14"
  check "document B restores its view zoom" "$b_zoom" "0.75"
  check "document B restores its citation style" "$b_style" "mla"
  check "document B restores its citation locale" "$b_locale" "fr-FR"
  if [ "${b_count:-0}" -lt 2 ]; then
    warn "the fixture laid out only $b_count page(s); page restore is not distinct"
  fi

  # Returning to A saves B first, then restores A. Wait for that write rather
  # than a fixed autosave delay, so the assertion is about the persisted
  # values, not about how long this machine took to reach them.
  local before_save
  before_save="$(session_stamp)"
  python3 "$PWD/scripts/e2e/ipc.py" "$E2E_IPC" open_doc \
    "{\"path\":\"$PREF_DOC_A\"}" >/dev/null 2>&1
  await_session_write "$before_save" || true

  local a_zoom a_style a_locale a_saved_page a_saved_text
  local b_zoom b_style b_locale b_saved_page b_saved_text
  read -r a_zoom a_style a_locale a_saved_page a_saved_text \
    b_zoom b_style b_locale b_saved_page b_saved_text \
    <<<"$(saved_preferences)"
  check "document A keeps its view zoom" "$a_zoom" "2"
  check "document A keeps its citation style" "$a_style" "vancouver"
  check "document A keeps its citation locale" "$a_locale" "de-DE"
  check "document A keeps its saved page" "$a_saved_page" "1"
  check "document A keeps its saved reading size" "$a_saved_text" "20"

  check "document B keeps its view zoom" "$b_zoom" "0.75"
  check "document B keeps its citation style" "$b_style" "mla"
  check "document B keeps its citation locale" "$b_locale" "fr-FR"
  check "document B keeps its saved page" "$b_saved_page" "2"
  check "document B keeps its saved reading size" "$b_saved_text" "14"

  # A is still open at 2x, so the page overflows the viewport and a two-finger
  # drag has somewhere to pan. The injector asserts the live pan changed.
  local touch_out touch_status
  touch_out="$(python3 "$PWD/scripts/e2e/touch.py" "$E2E_IPC" 0 -80 2>&1)"
  touch_status=$?
  check "two-finger touch pans the zoomed page" "$touch_status" "0"
  [ -n "$touch_out" ] && log "  $touch_out"
}
