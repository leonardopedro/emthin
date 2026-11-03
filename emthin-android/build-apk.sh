#!/usr/bin/env bash
# Build emthin-pad.apk without Gradle:
#   1. cargo cross-compiles libemthin_pad.so for x86_64-linux-android (waydroid ABI)
#   2. javac + d8 produce classes.dex
#   3. aapt2 links the manifest, jar drops in dex + .so
#   4. zipalign + apksigner produce a debug-signed APK
set -euo pipefail
cd "$(dirname "$0")"

# Toolchain resolution: environment → .toolchain/env.sh (written by
# setup-box64-toolchain.sh) → the pinned Nix store paths below. The pins
# are only a fallback for a host that can exec the x86_64 SDK binaries
# directly; on an aarch64 box they go stale on `nix flake update`, so run
# ./setup-box64-toolchain.sh instead of editing them.
if [[ -z "${ANDROID_SDK_ROOT:-}" && -f .toolchain/env.sh ]]; then
    # shellcheck disable=SC1091
    . .toolchain/env.sh
fi

SDK=${ANDROID_SDK_ROOT:-/nix/store/q5isg1d6fixgyfrdgx5xax6wvy9gya3s-androidsdk/libexec/android-sdk}
BT="$SDK/build-tools/${ANDROID_BUILD_TOOLS:-37.0.0}"
NDK="$SDK/ndk/30.0.14904198-rc1"
JDK=${JAVA_HOME:-/nix/store/mp9lqgch671kqq9qad0gsmw5r373izcn-openjdk-headless-21.0.12.1+1}
# Glob rather than pin the platform directory: the API level a flake
# bump lands on should not need a script edit.
ANDROID_JAR=$(ls -d "$SDK"/platforms/android-*/android.jar 2>/dev/null | sort -V | tail -1 || true)

API=${ANDROID_API:-33}
TARGET=x86_64-linux-android
# Android's APK ABI directory name (≠ the Rust target triple).
ABI=x86_64
LLVM="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"

if [[ -n "${TOOLCHAIN_BIN:-}" ]]; then
    # The shims carry the --target flag and run the NDK's x86_64 LLVM
    # under box64; the raw NDK binaries cannot exec on an aarch64 host.
    CLANG="$TOOLCHAIN_BIN/clang"
    AR="$TOOLCHAIN_BIN/llvm-ar"
else
    CLANG="$LLVM/x86_64-linux-android$API-clang"
    AR="$LLVM/llvm-ar"
fi

export JAVA_HOME="$JDK"
if [[ -n "${TOOLCHAIN_BIN:-}" ]]; then
    export PATH="$TOOLCHAIN_BIN:$JDK/bin:$BT:$PATH"
else
    export PATH="$JDK/bin:$BT:$PATH"
fi

[ -n "$ANDROID_JAR" ] || {
    echo "error: no platforms/android-*/android.jar under $SDK" >&2
    exit 1
}
# A raw x86_64 SDK binary on this aarch64 host does not run: the shell
# reports ENOEXEC as status 126 instead. Catch that here, with a pointer at
# the setup script, rather than a bare "cannot execute binary file" from
# whichever step happens to run aapt2 first. Any other status is the
# tool's own answer on purpose (e.g. `zipalign -h` exits 2).
probe_tool() { # <name> [probe args...]
    local t=$1 rc
    shift
    command -v "$t" >/dev/null || {
        echo "error: $t not found — run ./setup-box64-toolchain.sh, or set ANDROID_SDK_ROOT and JAVA_HOME" >&2
        exit 1
    }
    "$t" "$@" >/dev/null 2>&1 || rc=$?
    if [[ "${rc:-0}" == 126 || "${rc:-0}" == 127 ]]; then
        echo "error: $t at $(command -v "$t") cannot execute on this host (status $rc) —" >&2
        echo "       it is an x86_64 binary; run ./setup-box64-toolchain.sh for box64 shims" >&2
        exit 1
    fi
}
probe_tool aapt2 version
probe_tool zipalign -h
probe_tool java -version
for t in javac apksigner keytool jar; do
    command -v "$t" >/dev/null || {
        echo "error: $t not found — run ./setup-box64-toolchain.sh, or set JAVA_HOME" >&2
        exit 1
    }
done

# cargo linker (cargo's own env vars are underscore-only)
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$CLANG"

# cc-rs wants `CC_x86_64-linux-android` — dashes aren't valid bash
# identifiers, so hand them to cargo via `env` (both dash and
# underscore spellings, cc-rs checks each).
echo "==> [1/6] cargo build --release --target $TARGET"
env \
    "CC_$TARGET=$CLANG" \
    "CC_${TARGET//-/_}=$CLANG" \
    "AR_$TARGET=$AR" \
    "AR_${TARGET//-/_}=$AR" \
    cargo build --release --target "$TARGET"

BUILD=build
rm -rf "$BUILD"
mkdir -p "$BUILD/classes" "$BUILD/dex" "$BUILD/stage/lib/$ABI"

echo "==> [2/6] javac"
find android/src -name '*.java' > "$BUILD/sources.txt"
javac --release 8 -classpath "$ANDROID_JAR" -d "$BUILD/classes" @"$BUILD/sources.txt"

echo "==> [3/6] d8"
d8 --min-api "$API" --output "$BUILD/dex" \
    $(find "$BUILD/classes" -name '*.class')

echo "==> [4/6] aapt2 link"
aapt2 link \
    -I "$ANDROID_JAR" \
    --manifest android/AndroidManifest.xml \
    --min-sdk-version "$API" \
    --target-sdk-version "$API" \
    -o "$BUILD/emthin-pad-unsigned.apk"

echo "==> [5/6] add dex + native lib"
cp "target/$TARGET/release/libemthin_pad.so" "$BUILD/stage/lib/$ABI/"
jar --update --file "$BUILD/emthin-pad-unsigned.apk" \
    -C "$BUILD/dex" classes.dex
jar --update --file "$BUILD/emthin-pad-unsigned.apk" \
    -C "$BUILD/stage" "lib/$ABI/libemthin_pad.so"
zipalign -f 4 "$BUILD/emthin-pad-unsigned.apk" "$BUILD/emthin-pad-aligned.apk"

echo "==> [6/6] sign (debug keystore)"
if [[ ! -f debug.keystore ]]; then
    keytool -genkeypair -keystore debug.keystore \
        -storepass android -keypass android \
        -alias androiddebugkey -keyalg RSA -keysize 2048 \
        -validity 10000 \
        -dname "CN=Android Debug,O=Android,C=US"
fi
apksigner sign \
    --ks debug.keystore \
    --ks-key-alias androiddebugkey \
    --ks-pass pass:android \
    --key-pass pass:android \
    --out "$BUILD/emthin-pad.apk" \
    "$BUILD/emthin-pad-aligned.apk"
apksigner verify "$BUILD/emthin-pad.apk"

echo
echo "OK: $BUILD/emthin-pad.apk"
ls -la "$BUILD/emthin-pad.apk"
