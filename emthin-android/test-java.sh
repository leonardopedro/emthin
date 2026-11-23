#!/usr/bin/env bash
# Host-only tests for the plain-Java owners: A2 ownership scheduling, the
# A3 recovery envelope and A4 provider/save helpers. No Android SDK is required.
set -euo pipefail
cd "$(dirname "$0")"

find_jdk_tool() { # <tool>
    local tool=$1
    if command -v "$tool" >/dev/null 2>&1; then
        command -v "$tool"
        return
    fi
    if [[ -n "${JAVA_HOME:-}" && -x "$JAVA_HOME/bin/$tool" ]]; then
        printf '%s\n' "$JAVA_HOME/bin/$tool"
        return
    fi
    # Optional local Nix JDK used by build-apk.sh on this host.
    local pinned=/nix/store/mp9lqgch671kqq9qad0gsmw5r373izcn-openjdk-headless-21.0.12.1+1
    if [[ -x "$pinned/bin/$tool" ]]; then
        printf '%s\n' "$pinned/bin/$tool"
        return
    fi
    echo "error: $tool not found; install a JDK or set JAVA_HOME" >&2
    return 3
}

JAVAC=$(find_jdk_tool javac)
JAVA=$(find_jdk_tool java)
KEYTOOL=$(find_jdk_tool keytool)
OUT=$(mktemp -d "${TMPDIR:-/tmp}/emthin-java-tests.XXXXXX")
trap 'rm -rf "$OUT"' EXIT

# Two throwaway certificates for the WSS test: one naming localhost (what the
# client asks for) and one naming a host it never asks for, so the
# endpoint-identification check has something trusted-but-wrong to reject.
make_cert() { # <file> <dns-name>
    "$KEYTOOL" -genkeypair -alias t -keyalg RSA -keysize 2048 -validity 2 \
        -dname "CN=$2" -ext "SAN=dns:$2" -storetype PKCS12 \
        -keystore "$1" -storepass changeit -keypass changeit >/dev/null 2>&1
}
make_cert "$OUT/localhost.p12" localhost
make_cert "$OUT/other.p12" other.example

"$JAVAC" -Xlint:all -d "$OUT" \
    android/src/com/emthin/pad/PadSession.java \
    android/src/com/emthin/pad/RenderController.java \
    android/src/com/emthin/pad/RecoveryCodec.java \
    android/src/com/emthin/pad/ProviderIo.java \
    android/src/com/emthin/pad/FindController.java \
    android/src/com/emthin/pad/NavigationController.java \
    android/src/com/emthin/pad/CitationController.java \
    android/src/com/emthin/pad/InspectorView.java \
    android/src/com/emthin/pad/StatusModel.java \
    android/src/com/emthin/pad/ActionCatalog.java \
    android/src/com/emthin/pad/SessionCatalog.java \
    android/src/com/emthin/pad/GatewayJson.java \
    android/src/com/emthin/pad/PairingLink.java \
    android/src/com/emthin/pad/PairingProtocol.java \
    android/src/com/emthin/pad/GatewayEndpoints.java \
    android/src/com/emthin/pad/HttpsGatewayHttp.java \
    android/src/com/emthin/pad/GatewayTransport.java \
    android/src/com/emthin/pad/WebSocketFrame.java \
    android/src/com/emthin/pad/WebSocketHandshake.java \
    android/src/com/emthin/pad/WssTransport.java \
    android/test/com/emthin/pad/GatewayPairingTest.java \
    android/test/com/emthin/pad/WebSocketTest.java \
    android/test/com/emthin/pad/OwnershipTest.java \
    android/test/com/emthin/pad/RecoveryCodecTest.java \
    android/test/com/emthin/pad/ProviderIoTest.java \
    android/test/com/emthin/pad/FindControllerTest.java \
    android/test/com/emthin/pad/NavigationControllerTest.java \
    android/test/com/emthin/pad/CitationControllerTest.java \
    android/test/com/emthin/pad/InspectorViewTest.java \
    android/test/com/emthin/pad/StatusModelTest.java \
    android/test/com/emthin/pad/ActionCatalogTest.java \
    android/test/com/emthin/pad/SessionCatalogTest.java
"$JAVA" -ea -cp "$OUT" com.emthin.pad.OwnershipTest
"$JAVA" -ea -cp "$OUT" com.emthin.pad.RecoveryCodecTest
"$JAVA" -ea -cp "$OUT" com.emthin.pad.ProviderIoTest
"$JAVA" -ea -cp "$OUT" com.emthin.pad.FindControllerTest
"$JAVA" -ea -cp "$OUT" com.emthin.pad.NavigationControllerTest
"$JAVA" -ea -cp "$OUT" com.emthin.pad.CitationControllerTest
"$JAVA" -ea -cp "$OUT" com.emthin.pad.InspectorViewTest
"$JAVA" -ea -cp "$OUT" com.emthin.pad.StatusModelTest
"$JAVA" -ea -cp "$OUT" com.emthin.pad.ActionCatalogTest
"$JAVA" -ea -cp "$OUT" com.emthin.pad.SessionCatalogTest
"$JAVA" -ea -cp "$OUT" com.emthin.pad.GatewayPairingTest
"$JAVA" -ea -cp "$OUT" com.emthin.pad.WebSocketTest \
    "$OUT/localhost.p12" "$OUT/other.p12"
