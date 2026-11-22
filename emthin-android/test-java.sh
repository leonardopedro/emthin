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
OUT=$(mktemp -d "${TMPDIR:-/tmp}/emthin-java-tests.XXXXXX")
trap 'rm -rf "$OUT"' EXIT

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
