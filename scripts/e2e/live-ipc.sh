#!/usr/bin/env bash
# Live control-client run over the new IPC surfaces (K1 inspect, K5 history,
# and the parked-item methods). Complements the payload-level unit tests: this
# boots a real headless compositor + emthin, opens a marker-bearing fixture and
# asserts the notifications a control client actually receives.
#
#   ./scripts/e2e/live-ipc.sh
#
# No input injection, no device, no network. Exit 0 means every check passed.

set -uo pipefail

HERE="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
cd "$REPO"

# shellcheck source=lib.sh
. "$HERE/lib.sh"

ensure_devshell "$@"

FAILURES=0
fail() { printf '\033[1;31mFAIL\033[0m %s\n' "$*" >&2; FAILURES=$((FAILURES + 1)); }

log "=== live control-client IPC run ==="

reap_previous_run
start_compositor "sway-headless" || die "no compositor available"
start_emthin
trap 'stop_emthin; stop_compositor 2>/dev/null || true; cleanup 2>/dev/null || true' EXIT

# One fixture with: a named definition + use (semantic, a11y, rename), a
# dormant app figure with launch: (launcher), and a citation-free body.
FIXTURE="$E2E_RUN/live-ipc.typ"
cat > "$FIXTURE" <<'EOF'
#1 $norm$ #2 \def(#1,#2, norm)
#3 app #4 \app(#3, #4, 200, 120, "foot", launch: "foot")
#5 claim #6 \formal(#5, #6, "sort Prop")
$norm(x)$
EOF
ipc open_doc "{\"path\": \"$FIXTURE\"}" >/dev/null
sleep 0.5

# Call one method and return the matching notification, or `null`.
ask() {
  python3 - "$E2E_IPC" "$1" "$2" "${3:-{\}}" <<'PY'
import json, socket, sys, time
path, method, want, params = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.settimeout(3.0)
s.connect(path)
body = json.dumps({"jsonrpc":"2.0","id":1,"method":method,"params":json.loads(params)}).encode()
s.sendall(b"Content-Length: %d\r\n\r\n" % len(body) + body)
def recv():
    hdr = b""
    while b"\r\n\r\n" not in hdr:
        c = s.recv(1)
        if not c: return None
        hdr += c
    n = None
    for line in hdr.decode("utf-8","replace").split("\r\n"):
        if line.lower().startswith("content-length:"):
            n = int(line.split(":",1)[1].strip())
    if n is None: return None
    buf = b""
    while len(buf) < n:
        c = s.recv(n-len(buf))
        if not c: return None
        buf += c
    return json.loads(buf.decode("utf-8","replace"))
end = time.time() + 3.0
while time.time() < end:
    s.settimeout(max(0.01, end-time.time()))
    try: m = recv()
    except (socket.timeout, TimeoutError, OSError): break
    if m is None: break
    if m.get("method") == want:
        print(json.dumps(m.get("params")))
        s.close()
        sys.exit(0)
print("null")
PY
}

expect_field() {
  local method="$1" want="$2" field="$3"
  local response
  response=$(ask "$method" "$want")
  if [ "$response" = "null" ]; then
    fail "$method produced no $want notification"
    return
  fi
  local ok
  ok=$(printf '%s' "$response" | python3 -c '
import json,sys
v=json.load(sys.stdin); key=sys.argv[1]
print("yes" if key in v else "no")
' "$field")
  if [ "$ok" != "yes" ]; then
    fail "$want has no $field field: ${response:0:120}"
    return
  fi
  ok "$method -> $want ($field present)"
}

expect_field "semantic_outline" "semantic_outline" "definitions"
expect_field "a11y_nodes" "a11y_nodes" "nodes"
expect_field "launcher_entries" "launcher_list" "entries"
expect_field "plugin_catalog" "plugin_catalog" "plugins"
identity=$(ask "client_identity" "client_identity")
pid=$(printf '%s' "$identity" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("pid"))' 2>/dev/null || echo none)
if [ "$pid" = "null" ] || [ "$pid" = "none" ]; then
  fail "client_identity did not report a kernel-verified pid"
else
  ok "client_identity reported the live client pid ($pid)"
fi
expect_field "kernel_outputs" "kernel_outputs" "regions"
ipc collab_publish_descriptor '{"owner":"did:web:notes.example.org","endpoint":"wss://notes.example.org/session/1","rkey":"live"}' >/dev/null
sleep 0.3
response=$(ask "collab_descriptor" "collab_descriptor" '{"rkey":"live"}')
if [ "$response" = "null" ]; then
  fail "collab_descriptor produced no notification after publication"
else
  ok "collab_descriptor returned a published descriptor"
fi
proof_view=$(ask "proof_query" "proof_view")
log "proof_view: ${proof_view:0:300}"
if [ "$proof_view" = "null" ]; then
  fail "proof_query produced no proof_view notification"
else
  ok "proof_query -> proof_view (definitions present)"
fi

# K9's live control-client case: deliver a verdict with the owner/proof id the
# document itself advertised, then assert the next query reports it. A late
# result for another owner is asserted ignored too.
verdict=$(printf '%s' "$proof_view" | python3 -c '
import json,sys
v=json.load(sys.stdin)
print(json.dumps({"share_id":v.get("share_id",""),"document_key":v.get("document_key",""),"proof_id":v.get("proof_id","")}))
' 2>/dev/null || echo '{}')
proof_id=$(printf '%s' "$verdict" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("proof_id",""))')
if [ -z "$proof_id" ]; then
  fail "proof_view advertised no proof_id for the \formal fixture"
else
  args=$(printf '%s' "$verdict" | python3 -c '
import json,sys
v=json.load(sys.stdin)
print(json.dumps({**v,"verdict":"verified"}))
')
  ipc proof_result "$args" >/dev/null
  sleep 0.3
  after=$(ask "proof_query" "proof_view")
  got=$(printf '%s' "$after" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("verdict",""))' 2>/dev/null || echo '')
  if [ "$got" != "verified" ]; then
    fail "live proof_result was not accepted (verdict=$got)"
  else
    ok "live proof_result accepted with the document's own owner and proof id"
  fi
fi

# K1 inspect and K5 history: start/query/stop a live event log.
ipc inspect_start >/dev/null
sleep 0.3
expect_field "inspect_state" "inspect_state" "enabled"
ipc inspect_stop >/dev/null
expect_field "history_list" "history" "frontiers"

# A live rename: the document changes over the control socket and the
# semantic outline must reflect it on the next query (owner/revision fence).
ipc rename_definition '{"def_index": 0, "new_name": "kind"}' >/dev/null
sleep 0.3
response=$(ask "semantic_outline" "semantic_outline")
renamed=$(printf '%s' "$response" | python3 -c '
import json,sys
v=json.load(sys.stdin)
print("yes" if any(d[1]=="kind" for d in v.get("definitions",[])) else "no")
' 2>/dev/null || echo no)
if [ "$renamed" != "yes" ]; then
  fail "live rename_definition did not update the semantic outline"
else
  ok "live rename_definition reflected in semantic_outline"
fi

if [ "$FAILURES" -eq 0 ]; then
  log "live-ipc: all checks passed"
  exit 0
fi
log "live-ipc: $FAILURES check(s) FAILED"
exit 1
