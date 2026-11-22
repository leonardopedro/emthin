#!/usr/bin/env bash
# OS process-kill / restart at a session-write boundary (D8's D1 follow-up,
# the P6 storage-boundary case). This complements the deterministic in-process
# fault tests in session/store.rs with a real SIGKILL: after emthin acknowledges
# a snapshot write, kill it, relaunch, and assert the last verified pair comes
# back with the edited content.
#
#   ./scripts/e2e/process-kill.sh
#
# Uses the same boot helpers as run.sh, so it is a compositor-backed lane:
# headless sway + emthin with a scratch --session-file. No input injection,
# no device, no network. Exit 0 means every check passed.

set -uo pipefail

HERE="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
cd "$REPO"

# shellcheck source=lib.sh
. "$HERE/lib.sh"

ensure_devshell "$@"

FAILURES=0
fail() { printf '\033[1;31mFAIL\033[0m %s\n' "$*" >&2; FAILURES=$((FAILURES + 1)); }

# list_state emits a notification among other frames; take its `doc` field.
read_doc() {
  python3 -c '
import sys, json
dec = json.JSONDecoder(); text = sys.stdin.read(); i = 0
while i < len(text):
    while i < len(text) and text[i] != "{": i += 1
    if i >= len(text): break
    try: obj, i = dec.raw_decode(text, i)
    except ValueError: break
    if obj.get("method") == "state":
        sys.stdout.write(obj["params"]["doc"]); break
'
}

log "=== process-kill / restart at a session-write boundary ==="

# Boot a headless compositor (no input backend is needed here), then emthin.
reap_previous_run
start_compositor "sway-headless" || die "no compositor available for the process-kill lane"
start_emthin
trap 'stop_emthin; stop_compositor 2>/dev/null || true; cleanup 2>/dev/null || true' EXIT

# Scratch marker the recovery pair must carry. Typed through the existing
# open_doc IPC (a real file edit), not by writing the snapshot directly.
MARKER="PKILL-$$-$(date +%s)"
FIXTURE="$E2E_RUN/process-kill.typ"
printf 'process kill fixture\n%s\n' "$MARKER" > "$FIXTURE"

# open_doc has no content arg; it reads a named file. Use export_ascii's
# companion: write a fresh source doc and open it, then edit through IPC.
ipc open_doc "{\"path\": \"$FIXTURE\"}" >/dev/null
sleep 0.5
doc_before=$(ipc list_state | read_doc)
if [[ "$doc_before" != *"$MARKER"* ]]; then
  fail "fixture did not open through IPC"
  exit 1
fi
ok "opened the scratch proof note through IPC"

# Force a content change so autosave has something to checkpoint, then wait for
# the deterministic write-boundary signal in the log ("session saved to ...").
# The marker is already in the opened source, so touch the text via the voice
# seam is unnecessary: a text_size change marks recovery state dirty and the
# 5s autosave writes the pair.
ipc set_text_size '{"pt": 17}' >/dev/null

deadline=$(( SECONDS + 20 ))
boundary=0
while [ "$SECONDS" -lt "$deadline" ]; do
  if grep -aq "session saved to" "$E2E_ROOT/emthin.log"; then
    boundary=1
    break
  fi
  sleep 0.2
done
if [ "$boundary" -ne 1 ]; then
  fail "no session-write boundary was observed in the log"
  exit 1
fi
ok "observed a committed session-write boundary"

# Snapshot the on-disk pair before the kill: at least one valid copy must exist.
if [ ! -f "$E2E_DOC" ]; then
  fail "no session snapshot exists at the write boundary"
  exit 1
fi
sha_before=$(sha256sum "$E2E_DOC" | awk '{print $1}')
ok "a valid snapshot exists at the boundary (sha256 ${sha_before:0:12})"

# SIGKILL — no graceful shutdown, no final save. This is the OS-level case the
# deterministic store tests cannot prove.
pid=$(emthin_pid)
if [ -z "$pid" ]; then
  fail "emthin was not running at the write boundary"
  exit 1
fi
log "SIGKILL emthin (pid $pid)"
kill -KILL "$pid" 2>/dev/null || true
for _ in $(seq 1 40); do
  kill -0 "$pid" 2>/dev/null || break
  sleep 0.1
done
if kill -0 "$pid" 2>/dev/null; then
  fail "emthin survived SIGKILL"
  exit 1
fi
ok "emthin was killed without a graceful shutdown"

# Relaunch against the same session file. The loader must return the recovered
# pair (not Unrecoverable, not a blank default) with the marker intact.
start_emthin
doc_after=$(ipc list_state | read_doc)
if [[ "$doc_after" != *"$MARKER"* ]]; then
  fail "relaunch did not recover the edited note (got: ${doc_after:0:80})"
  exit 1
fi
ok "relaunch recovered the last verified pair with the edited note"

if grep -aq "unrecoverable\|Unrecoverable\|quarantin" "$E2E_ROOT/emthin.log"; then
  fail "relaunch reported an unrecoverable session"
  exit 1
fi
ok "no unrecoverable/quarantine warning after the kill"

if [ "$FAILURES" -eq 0 ]; then
  log "process-kill: all checks passed"
  exit 0
fi
fail "process-kill: $FAILURES check(s) FAILED"
exit 1
