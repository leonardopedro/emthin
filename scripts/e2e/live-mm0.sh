#!/usr/bin/env bash
# K9 live verifier run: verify a real .mm0 proof with mm0web, then feed the
# backend's verdict into emthin over the control socket and assert that a
# document citing that external proof id reports the result.
#
#   ./scripts/e2e/live-mm0.sh
#
# Needs a local mm0web binary (default /home/leo/Projects/unfer/target/debug/
# mm0web, overridable by MM0WEB_BIN) and the Peano fixture (MM0_DIR).
# No network beyond loopback. Exit 0 means every check passed.

set -uo pipefail

HERE="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
cd "$REPO"

# shellcheck source=lib.sh
. "$HERE/lib.sh"

ensure_devshell "$@"

FAILURES=0
fail() { printf '\033[1;31mFAIL\033[0m %s\n' "$*" >&2; FAILURES=$((FAILURES + 1)); }

MM0WEB_BIN="${MM0WEB_BIN:-/home/leo/Projects/unfer/target/debug/mm0web}"
MM0_DIR="${MM0_DIR:-/tmp/mm0/examples}"
PORT="${MM0WEB_PORT:-18081}"
STATE="$(mktemp -d /tmp/opencode/mm0web-live-XXXXXX)"
WEB_PID=""

cleanup_web() {
  if [ -n "$WEB_PID" ] && kill -0 "$WEB_PID" 2>/dev/null; then
    kill -TERM "$WEB_PID" 2>/dev/null || true
    wait "$WEB_PID" 2>/dev/null || true
  fi
  rm -rf "$STATE"
}
trap 'cleanup_web; stop_emthin 2>/dev/null || true; stop_compositor 2>/dev/null || true; cleanup 2>/dev/null || true' EXIT

log "=== live MM0 verifier -> emthin proof_result ==="

if [ ! -x "$MM0WEB_BIN" ]; then
  fail "no mm0web binary at $MM0WEB_BIN (set MM0WEB_BIN)"
  exit 1
fi
for f in "$MM0_DIR/peano.mm0" "$MM0_DIR/peano.mm1"; do
  if [ ! -f "$f" ]; then
    fail "missing MM0 fixture $f (set MM0_DIR)"
    exit 1
  fi
done

# 1. Start a loopback mm0web on a scratch state dir.
"$MM0WEB_BIN" --bind "127.0.0.1:$PORT" --state "$STATE" \
  >"$E2E_ROOT/mm0web.log" 2>&1 &
WEB_PID=$!
for _ in $(seq 1 50); do
  if curl -s --max-time 1 "http://127.0.0.1:$PORT/status" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done
if ! kill -0 "$WEB_PID" 2>/dev/null; then
  fail "mm0web did not start (see $E2E_ROOT/mm0web.log)"
  exit 1
fi
ok "mm0web serving on 127.0.0.1:$PORT"

# 2. Verify the real Peano proof. JSON mode returns the checker verdict.
RESPONSE="$(curl -s --max-time 180 -H 'Accept: application/json' \
  -X POST "http://127.0.0.1:$PORT/api/v1/verify" \
  --data-urlencode 'project=peano-live' \
  --data-urlencode 'title=Peano live' \
  --data-urlencode "mm0@$MM0_DIR/peano.mm0" \
  --data-urlencode "mm1@$MM0_DIR/peano.mm1")"
ACCEPTED="$(printf '%s' "$RESPONSE" | python3 -c 'import json,sys; print("yes" if json.load(sys.stdin).get("accepted") else "no")' 2>/dev/null || echo no)"
if [ "$ACCEPTED" != "yes" ]; then
  fail "mm0web did not accept the Peano proof: ${RESPONSE:0:200}"
  exit 1
fi
ok "mm0web accepted the real Peano proof (mm0-rs + mm0-c)"

# The backend mints a content-addressed id; read it from its audit trail.
PROOF_ID="$(python3 - "$STATE/events.jsonl" <<'PY'
import json, sys
for line in open(sys.argv[1]):
    event = json.loads(line)
    if event.get("action") == "verified":
        print(event["proof"]); break
PY
)"
if [ -z "$PROOF_ID" ]; then
  fail "mm0web audit trail has no verified proof id"
  exit 1
fi
ok "live backend minted proof id $PROOF_ID"

# 3. Boot emthin with a document that cites that external id via `proof:`.
FIXTURE="$E2E_RUN/live-mm0.typ"
cat > "$FIXTURE" <<EOF
#g claim #h \\formal(#g, #h, "sort Prop", proof: "$PROOF_ID")
The claim step is cited to the live checker record $PROOF_ID$.
EOF
reap_previous_run
start_compositor "sway-headless" || die "no compositor available"
start_emthin
ipc open_doc "{\"path\": \"$FIXTURE\"}" >/dev/null
sleep 0.5

ask() {
  python3 - "$E2E_IPC" "$1" "$2" "${3:-{\}}" <<'PY'
import json, socket, sys, time
path, method, want, params = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.settimeout(3.0); s.connect(path)
body = json.dumps({"jsonrpc":"2.0","id":1,"method":method,"params":json.loads(params)}).encode()
s.sendall(b"Content-Length: %d\r\n\r\n" % len(body) + body)
def recv():
    hdr=b""
    while b"\r\n\r\n" not in hdr:
        c=s.recv(1)
        if not c: return None
        hdr+=c
    n=None
    for line in hdr.decode("utf-8","replace").split("\r\n"):
        if line.lower().startswith("content-length:"): n=int(line.split(":",1)[1].strip())
    if n is None: return None
    buf=b""
    while len(buf)<n:
        c=s.recv(n-len(buf))
        if not c: return None
        buf+=c
    return json.loads(buf.decode("utf-8","replace"))
end=time.time()+3.0
while time.time()<end:
    s.settimeout(max(0.01,end-time.time()))
    try: m=recv()
    except (socket.timeout,TimeoutError,OSError): break
    if m is None: break
    if m.get("method")==want:
        print(json.dumps(m.get("params"))); s.close(); sys.exit(0)
print("null")
PY
}

view=$(ask "proof_query" "proof_view")
if [ "$view" = "null" ]; then
  fail "proof_query produced no proof_view"
  exit 1
fi
advertised=$(printf '%s' "$view" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("proof_id",""))')
if [ "$advertised" != "$PROOF_ID" ]; then
  fail "document did not cite the live proof id (advertised $advertised)"
  exit 1
fi
ok "document cites the live backend proof id via \`proof:\`"

args=$(printf '%s' "$view" | python3 -c '
import json,sys
v=json.load(sys.stdin)
print(json.dumps({"share_id":v["share_id"],"document_key":v["document_key"],"proof_id":v["proof_id"],"verdict":"accepted"}))
')
ipc proof_result "$args" >/dev/null
sleep 0.3
after=$(ask "proof_query" "proof_view")
verdict=$(printf '%s' "$after" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("verdict",""))' 2>/dev/null || echo '')
if [ "$verdict" != "accepted" ]; then
  fail "live backend verdict was not reflected in proof_query (got $verdict)"
  exit 1
fi
ok "live mm0web verdict reflected in emthin proof_query"

if [ "$FAILURES" -eq 0 ]; then
  log "live-mm0: all checks passed (backend $PROOF_ID)"
  exit 0
fi
log "live-mm0: $FAILURES check(s) FAILED"
exit 1
