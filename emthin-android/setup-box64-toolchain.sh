#!/usr/bin/env bash
# Provision the Android toolchain this aarch64 box cannot exec directly.
#
# The SDK's build-tools (aapt2, zipalign) and the NDK's LLVM are *x86_64*
# ELF, so on an aarch64 host they fail with ENOEXEC unless something
# translates them. This script resolves the pieces from nixpkgs, puts a
# persistent symlink farm under .toolchain/ (git-ignored), writes box64
# shims for the four binaries the APK chain needs, and an env.sh that
# build-apk.sh sources.
#
#   ./setup-box64-toolchain.sh          # (re)provision; idempotent
#   . .toolchain/env.sh && ./build-apk.sh
#
# Overridable: ANDROID_ABI (x86_64), ANDROID_API (33),
# ANDROID_PLATFORM (37.0), ANDROID_BUILD_TOOLS (37.0.0),
# ANDROID_NDK_VERSION (30.0.14904198-rc1). Set ANDROID_ABI=arm64-v8a
# when the device is an arm64 Waydroid image; the shims carry that target.
set -euo pipefail
cd "$(dirname "$0")"

API=${ANDROID_API:-33}
PLATFORM=${ANDROID_PLATFORM:-37.0}
BUILD_TOOLS=${ANDROID_BUILD_TOOLS:-37.0.0}
NDK_VERSION=${ANDROID_NDK_VERSION:-30.0.14904198-rc1}
ABI=${ANDROID_ABI:-x86_64}
case "$ABI" in
    arm64-v8a)   TARGET=aarch64-linux-android ;;
    armeabi-v7a) TARGET=armv7a-linux-androideabi ;;
    x86)         TARGET=i686-linux-android ;;
    x86_64)      TARGET=x86_64-linux-android ;;
    *) echo "error: unsupported ANDROID_ABI '$ABI'" >&2; exit 2 ;;
esac

TC=.toolchain

command -v nix >/dev/null || {
    echo "error: nix is required (this script resolves the toolchain from nixpkgs)" >&2
    exit 1
}

# composeAndroidPackages refuses to evaluate without this; the SDK is
# unfree and the license is accepted by the flake's consumer here.
export NIXPKGS_ACCEPT_ANDROID_SDK_LICENSE=${NIXPKGS_ACCEPT_ANDROID_SDK_LICENSE:-1}

# Resolve every path from nixpkgs instead of pinning store hashes: the
# pins in build-apk.sh go stale on a `nix flake update`, which is exactly
# the failure this script exists to remove.
nix_path() {
    nix eval --impure --raw --expr "
      let
        f = builtins.getFlake \"nixpkgs\";
        native = import f.outPath { system = \"aarch64-linux\"; config.allowUnfree = true; };
        v = native.${1};
        out = if v ? outPath then v.outPath else throw \"no such attribute: ${1}\";
      in out
    "
}

echo "==> [1/4] resolving toolchain from nixpkgs"
# An SDK that is already on disk wins: re-resolving it from the current
# nixpkgs weekly pins a *different* composition path, which then has to be
# built from scratch just to regenerate four shims. Set ANDROID_SDK_ROOT (the
# .toolchain/env.sh one is used automatically below) to reuse it.
if [[ -z "${ANDROID_SDK_ROOT:-}" && -f .toolchain/env.sh ]]; then
    # shellcheck disable=SC1091
    . .toolchain/env.sh
fi
SDK_STORE=""
if [[ -n "${ANDROID_SDK_ROOT:-}" && -d "$ANDROID_SDK_ROOT/ndk/$NDK_VERSION" ]]; then
    SDK="$ANDROID_SDK_ROOT"
    echo "    reusing SDK $SDK"
else
    SDK_STORE=$(
        nix eval --impure --raw --expr "
          let
            f = builtins.getFlake \"nixpkgs\";
            pkgs = import f.outPath { system = \"aarch64-linux\"; config.allowUnfree = true; };
          in (pkgs.androidenv.composeAndroidPackages {
            platformVersions = [ \"$PLATFORM\" ];
            buildToolsVersions = [ \"$BUILD_TOOLS\" ];
            includeNDK = true;
            ndkVersion = \"$NDK_VERSION\";
          }).androidsdk.outPath
        "
    )
    SDK="$SDK_STORE/libexec/android-sdk"
fi
# Same rule as the SDK: a component that already works stays. A weekly nixpkgs
# bump renames the JDK/box64 derivation, and rebuilding those (or the SDK) just
# to regenerate shims is not what this script is for.
if [[ -n "${JAVA_HOME:-}" && -x "$JAVA_HOME/bin/javac" ]]; then
    JDK=$JAVA_HOME
else
    JDK=$(nix_path jdk21_headless)
fi
if [[ -n "${EMTHIN_BOX64:-}" && -x "$EMTHIN_BOX64/bin/box64" ]]; then
    BOX64=$EMTHIN_BOX64
else
    BOX64=$(nix_path box64)
fi
# The NDK's own ld.lld (x86_64, under box64) aborts with `free(): invalid next
# size` on this box while linking the pad, so clang is pointed at a *native*
# lld instead. It is the same LLVM release as the NDK's (21), so the Android
# target and sysroot arguments behave identically.
if [[ -n "${EMTHIN_NATIVE_LLD:-}" && -x "$EMTHIN_NATIVE_LLD" ]]; then
    NATIVE_LLD=$(dirname "$(dirname "$EMTHIN_NATIVE_LLD")")
else
    NATIVE_LLD=$(nix_path lld)
fi
GLIBC=$(nix eval --impure --raw --expr '
  let f = builtins.getFlake "nixpkgs";
  in f.legacyPackages.x86_64-linux.glibc.outPath')
LIBCXX=$(nix eval --impure --raw --expr '
  let f = builtins.getFlake "nixpkgs";
  in f.legacyPackages.x86_64-linux.libcxx.outPath')
LIBGCC=$(nix eval --impure --raw --expr '
  let f = builtins.getFlake "nixpkgs";
  in f.legacyPackages.x86_64-linux.libgcc.outPath')
ZLIB=$(nix eval --impure --raw --expr '
  let f = builtins.getFlake "nixpkgs";
  in f.legacyPackages.x86_64-linux.zlib.outPath')

for p in $SDK_STORE "$SDK" "$JDK" "$BOX64" "$NATIVE_LLD/bin/ld.lld"; do
    [ -e "$p" ] || { echo "error: resolved path is missing: $p" >&2; exit 1; }
done
echo "    sdk    $SDK"
echo "    jdk    $JDK"
echo "    box64  $BOX64"
echo "    glibc  $GLIBC (x86_64)"
echo "    lld    $NATIVE_LLD/bin/ld.lld (native)"

LLVM="$SDK/ndk/$NDK_VERSION/toolchains/llvm/prebuilt/linux-x86_64/bin"
ANDROID_JAR="$SDK/platforms/android-$PLATFORM/android.jar"
for p in "$SDK/build-tools/$BUILD_TOOLS/aapt2" "$LLVM/clang-21" "$ANDROID_JAR"; do
    [ -e "$p" ] || { echo "error: SDK layout changed, missing $p" >&2; exit 1; }
done

# Mirror what the executed binaries look for: glibc's loader and libc in
# lib/, the C++ and gcc runtimes beside them, and lib64/ + etc/ + share/
# for the pieces that are resolved by path rather than by soname.
#
# Only rebuilt when every resolved x86_64 package is actually on disk. A
# nixpkgs weekly bump renames them into unbuilt store paths, and the farm from
# the previous run still provides exactly the same loader, so re-provisioning
# the shims must not turn into building glibc to satisfy a symlink.
SR=$TC/sysroot
FARM_OK=1
for src in "$GLIBC/lib" "$LIBCXX/lib" "$LIBGCC/lib" "$ZLIB/lib"; do
    [ -d "$src" ] || FARM_OK=0
done
if [[ "$FARM_OK" == 1 ]]; then
echo "==> [2/4] building the x86_64 sysroot farm in $SR"
rm -rf "$SR"
mkdir -p "$SR/lib" "$SR/share"

# First writer wins (glibc before the runtimes), so a name is never
# silently rebound to a second provider.
link_entries() { # <dest-dir> <src-dir>
    local dest=$1 src=$2 f name
    [ -d "$src" ] || return 0
    for f in "$src"/*; do
        [ -e "$f" ] || continue
        name=$(basename "$f")
        [ -e "$dest/$name" ] && continue
        ln -s "$f" "$dest/$name"
    done
}

for d in etc lib64 libexec; do
    [ -e "$GLIBC/$d" ] && ln -s "$GLIBC/$d" "$SR/$d"
done
for src in "$GLIBC/lib" "$LIBCXX/lib" "$LIBGCC/lib" "$ZLIB/lib"; do
    link_entries "$SR/lib" "$src"
done
for src in "$GLIBC/share" "$LIBCXX/share" "$ZLIB/share"; do
    link_entries "$SR/share" "$src"
done
echo "    $(ls "$SR/lib" | wc -l) entries in lib/, $(ls "$SR/share" | wc -l) in share/"
else
    echo "==> [2/4] keeping the existing sysroot farm in $SR"
    echo "    (resolved glibc/libcxx/libgcc/zlib are not built; the farm already has them)"
    [ -e "$SR/lib/ld-linux-x86-64.so.2" ] || {
        echo "error: no usable x86_64 sysroot at $SR and the resolved packages are missing" >&2
        echo "       build them first, e.g. nix build nixpkgs#legacyPackages.x86_64-linux.glibc" >&2
        exit 1
    }
fi

# Absolute paths: the shims below are executed from arbitrary cwds.
SR=$(cd "$SR" && pwd)
TC_BIN=$(cd "$TC" && pwd)/bin
mkdir -p "$TC_BIN"

echo "==> [3/4] writing box64 shims in $TC_BIN"
# Every shim: point the loader at the farm, prefer the emulated libs over
# the host's own x86_64 copies (there are none here, but box64 probes),
# then exec the real binary under box64.
box_shim() { # <name> <x86_64-binary> [fixed arguments...]
    local name=$1 bin=$2 a
    shift 2
    {
        printf '#!/bin/sh\n'
        printf '# Generated by setup-box64-toolchain.sh - runs x86_64 %s under box64.\n' "$name"
        printf 'LD_LIBRARY_PATH="%s/lib:%s/lib64${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"\n' "$SR" "$SR"
        printf 'export LD_LIBRARY_PATH\n'
        printf 'export BOX64_PREFER_EMULATED_LIBS=1\n'
        printf 'exec "%s/bin/box64" "%s"' "$BOX64" "$bin"
        for a in "$@"; do printf ' "%s"' "$a"; done
        printf ' "$@"\n'
    } > "$TC_BIN/$name"
    chmod +x "$TC_BIN/$name"
}

BT="$SDK/build-tools/$BUILD_TOOLS"
box_shim aapt2 "$BT/aapt2"
box_shim zipalign "$BT/zipalign"
# The NDK's own x86_64-linux-android33-clang is a shell script that would
# resolve `clang` on PATH; give the driver the target directly instead.
# `--ld-path` swaps only the linker: the x86_64 driver still compiles, but the
# link step runs the native lld above instead of the emulated one that crashes.
box_shim clang "$LLVM/clang-21" "--target=$TARGET$API" "--ld-path=$NATIVE_LLD/bin/ld.lld"
box_shim clang++ "$LLVM/clang++" "--target=$TARGET$API" "--ld-path=$NATIVE_LLD/bin/ld.lld"
box_shim llvm-ar "$LLVM/llvm-ar"

# GC-root the resolved paths so a `nix-collect-garbage` between setup and
# build cannot pull the toolchain out from under it.
mkdir -p "$TC/roots"
root_path() { # <name> <store-path>
    rm -f "$TC/roots/$1"
    if nix build --impure --out-link "$TC/roots/$1" "$2" >/dev/null 2>&1; then
        return 0
    fi
    echo "    warning: could not GC-root $1 (still usable until the next GC)" >&2
}
for pair in ${SDK_STORE:+"android-sdk:$SDK_STORE"} "jdk:$JDK" "box64:$BOX64" \
            "glibc:$GLIBC" "libcxx:$LIBCXX" "libgcc:$LIBGCC" "zlib:$ZLIB" \
            "lld:$NATIVE_LLD"; do
    root_path "${pair%%:*}" "${pair#*:}"
done

cat > "$TC/env.sh" <<EOF
# Generated by setup-box64-toolchain.sh - do not edit.
# Source from emthin-android/ (build-apk.sh does it automatically).
export ANDROID_SDK_ROOT="$SDK"
export ANDROID_NDK_ROOT="$SDK/ndk/$NDK_VERSION"
export ANDROID_BUILD_TOOLS="$BUILD_TOOLS"
export ANDROID_API="$API"
# The shims, not the APK, are what this records: build-apk.sh still asks
# the connected device for its ABI and uses this only as the no-device
# fallback / mismatch check.
export EMTHIN_TOOLCHAIN_ABI="$ABI"
# Native aarch64 lld the clang shims pass to --ld-path; the emulated NDK
# ld.lld aborts on this host. No backticks here: this heredoc expands.
export EMTHIN_NATIVE_LLD="$NATIVE_LLD/bin/ld.lld"
export EMTHIN_BOX64="$BOX64"
export JAVA_HOME="$JDK"
# Box64 shims for the x86_64 aapt2/zipalign/LLVM; must precede the
# build-tools directory on PATH or the raw ELFs win and fail with ENOEXEC.
export TOOLCHAIN_BIN="$TC_BIN"
EOF

echo "==> [4/4] self-check"
"$TC_BIN/aapt2" version >/dev/null || {
    echo "error: aapt2 shim does not run (box64/sysroot broken)" >&2
    exit 1
}
"$TC_BIN/clang" --version >/dev/null 2>&1 || {
    echo "error: clang shim does not run (box64/sysroot broken)" >&2
    exit 1
}
echo "    aapt2 / clang execute"
# A link, not just a compile: this is the step that used to abort inside the
# emulated ld.lld, so a broken `--ld-path` fails here instead of mid-build.
LINK_PROBE=$(mktemp -d)
printf 'int emthin_probe(void){return 1;}\n' > "$LINK_PROBE/p.c"
if "$TC_BIN/clang" -shared -o "$LINK_PROBE/p.so" "$LINK_PROBE/p.c" 2>/dev/null; then
    echo "    clang links (native lld)"
else
    echo "error: clang shim compiles but cannot link; check $NATIVE_LLD" >&2
    rm -rf "$LINK_PROBE"
    exit 1
fi
rm -rf "$LINK_PROBE"

echo
echo "OK: toolchain in $TC/ (ABI $ABI, target $TARGET)"
echo "    use it with:  . .toolchain/env.sh && ./build-apk.sh"
echo "    (build-apk.sh also sources $TC/env.sh on its own)"
