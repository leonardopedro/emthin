#!/usr/bin/env bash
# Host check for the WSS half of the gateway lane: a TLS/WebSocket fixture in
# front of the scripted Core, driven by the pad's real transport code
# (WssTransport + WebSocketFrame + WebSocketHandshake) on the host JVM.
#
# The device step is the same fixture behind `adb reverse`; this script proves
# the transport and the fixture agree before an APK is built.
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$(cd .. && pwd)

find_jdk_tool() { # <tool>
    local tool=$1
    if command -v "$tool" >/dev/null 2>&1; then command -v "$tool"; return; fi
    if [[ -n "${JAVA_HOME:-}" && -x "$JAVA_HOME/bin/$tool" ]]; then
        printf '%s\n' "$JAVA_HOME/bin/$tool"; return
    fi
    local pinned=/nix/store/mp9lqgch671kqq9qad0gsmw5r373izcn-openjdk-headless-21.0.12.1+1
    if [[ -x "$pinned/bin/$tool" ]]; then printf '%s\n' "$pinned/bin/$tool"; return; fi
    echo "error: $tool not found; install a JDK or set JAVA_HOME" >&2
    return 3
}

for tool in openssl python3; do
    command -v "$tool" >/dev/null 2>&1 || { echo "error: $tool is required" >&2; exit 3; }
done
JAVAC=$(find_jdk_tool javac)
JAVA=$(find_jdk_tool java)
KEYTOOL=$(find_jdk_tool keytool)

OUT=$(mktemp -d "${TMPDIR:-/tmp}/emthin-wss.XXXXXX")
FIXTURE_PID=
cleanup() {
    [[ -n "$FIXTURE_PID" ]] && kill "$FIXTURE_PID" 2>/dev/null || true
    rm -rf "$OUT"
}
trap cleanup EXIT

free_port() {
    python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}
STREAM_PORT=$(free_port)
ADMIN_PORT=$(free_port)

openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj "/CN=localhost" \
    -addext "subjectAltName=DNS:localhost" \
    -keyout "$OUT/key.pem" -out "$OUT/cert.pem" >/dev/null 2>&1
"$KEYTOOL" -importcert -noprompt -alias fixture -file "$OUT/cert.pem" \
    -keystore "$OUT/ca.p12" -storetype PKCS12 -storepass changeit >/dev/null 2>&1

python3 "$ROOT/fixtures/herdr-gateway-v1/wss_fixture.py" \
    --listen "127.0.0.1:$STREAM_PORT" --admin "127.0.0.1:$ADMIN_PORT" \
    --cert "$OUT/cert.pem" --key "$OUT/key.pem" >"$OUT/fixture.log" 2>&1 &
FIXTURE_PID=$!
for _ in $(seq 1 100); do
    grep -q "wss fixture listening" "$OUT/fixture.log" 2>/dev/null && break
    kill -0 "$FIXTURE_PID" 2>/dev/null || break
    sleep 0.05
done
if ! grep -q "wss fixture listening" "$OUT/fixture.log"; then
    echo "error: the WSS fixture did not start" >&2
    cat "$OUT/fixture.log" >&2
    exit 1
fi

"$JAVAC" -Xlint:all -d "$OUT/classes" \
    android/src/com/emthin/pad/WebSocketFrame.java \
    android/src/com/emthin/pad/WebSocketHandshake.java \
    android/src/com/emthin/pad/GatewayTransport.java \
    android/src/com/emthin/pad/WssTransport.java \
    android/test/com/emthin/pad/WssFixtureProbe.java
"$JAVA" -ea -cp "$OUT/classes" com.emthin.pad.WssFixtureProbe \
    "$OUT/ca.p12" localhost "$STREAM_PORT"
echo "wss lane: transport and fixture agree"
