#!/usr/bin/env bash
# Build emthin-pad.apk without Gradle:
#   1. cargo cross-compiles libemthin_pad.so for x86_64-linux-android (waydroid ABI)
#   2. javac + d8 produce classes.dex
#   3. aapt2 links the manifest, jar drops in dex + .so
#   4. zipalign + apksigner produce a debug-signed APK
set -euo pipefail
cd "$(dirname "$0")"

SDK=/nix/store/zsjpyy4w6ya7f6hsaq5h145d1kc1zspp-androidsdk/libexec/android-sdk
BT="$SDK/build-tools/37.0.0"
NDK="$SDK/ndk/30.0.14904198-rc1"
JDK=/nix/store/1arq9a57sqrn2z5ac362y2233dhcqgkc-openjdk-headless-21.0.12.1+1
ANDROID_JAR="$SDK/platforms/android-37.0/android.jar"

API=33
TARGET=x86_64-linux-android
# Android's APK ABI directory name (≠ the Rust target triple).
ABI=x86_64
LLVM="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin"
CLANG="$LLVM/x86_64-linux-android$API-clang"

export JAVA_HOME="$JDK"
export PATH="$JDK/bin:$BT:$PATH"

# cargo linker (cargo's own env vars are underscore-only)
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$CLANG"

# cc-rs wants `CC_x86_64-linux-android` — dashes aren't valid bash
# identifiers, so hand them to cargo via `env` (both dash and
# underscore spellings, cc-rs checks each).
echo "==> [1/6] cargo build --release --target $TARGET"
env \
    "CC_$TARGET=$CLANG" \
    "CC_${TARGET//-/_}=$CLANG" \
    "AR_$TARGET=$LLVM/llvm-ar" \
    "AR_${TARGET//-/_}=$LLVM/llvm-ar" \
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
