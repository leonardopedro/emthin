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
# Overridable: ANDROID_API (33), ANDROID_PLATFORM (37.0),
# ANDROID_BUILD_TOOLS (37.0.0), ANDROID_NDK_VERSION (30.0.14904198-rc1).
set -euo pipefail
cd "$(dirname "$0")"

API=${ANDROID_API:-33}
PLATFORM=${ANDROID_PLATFORM:-37.0}
BUILD_TOOLS=${ANDROID_BUILD_TOOLS:-37.0.0}
NDK_VERSION=${ANDROID_NDK_VERSION:-30.0.14904198-rc1}
TARGET=x86_64-linux-android

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
JDK=$(nix_path jdk21_headless)
BOX64=$(nix_path box64)
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

for p in "$SDK_STORE" "$JDK" "$BOX64" "$GLIBC" "$LIBCXX" "$LIBGCC" "$ZLIB"; do
    [ -e "$p" ] || { echo "error: resolved path is missing: $p" >&2; exit 1; }
done
echo "    sdk    $SDK_STORE"
echo "    jdk    $JDK"
echo "    box64  $BOX64"
echo "    glibc  $GLIBC (x86_64)"

SDK="$SDK_STORE/libexec/android-sdk"
LLVM="$SDK/ndk/$NDK_VERSION/toolchains/llvm/prebuilt/linux-x86_64/bin"
ANDROID_JAR="$SDK/platforms/android-$PLATFORM/android.jar"
for p in "$SDK/build-tools/$BUILD_TOOLS/aapt2" "$LLVM/clang-21" "$ANDROID_JAR"; do
    [ -e "$p" ] || { echo "error: SDK layout changed, missing $p" >&2; exit 1; }
done

# Mirror what the executed binaries look for: glibc's loader and libc in
# lib/, the C++ and gcc runtimes beside them, and lib64/ + etc/ + share/
# for the pieces that are resolved by path rather than by soname.
echo "==> [2/4] building the x86_64 sysroot farm in $TC/sysroot"
SR=$TC/sysroot
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
box_shim clang "$LLVM/clang-21" "--target=$TARGET$API"
box_shim clang++ "$LLVM/clang++" "--target=$TARGET$API"
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
for pair in "android-sdk:$SDK_STORE" "jdk:$JDK" "box64:$BOX64" \
            "glibc:$GLIBC" "libcxx:$LIBCXX" "libgcc:$LIBGCC" "zlib:$ZLIB"; do
    root_path "${pair%%:*}" "${pair#*:}"
done

cat > "$TC/env.sh" <<EOF
# Generated by setup-box64-toolchain.sh - do not edit.
# Source from emthin-android/ (build-apk.sh does it automatically).
export ANDROID_SDK_ROOT="$SDK"
export ANDROID_NDK_ROOT="$SDK/ndk/$NDK_VERSION"
export ANDROID_BUILD_TOOLS="$BUILD_TOOLS"
export ANDROID_API="$API"
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

echo
echo "OK: toolchain in $TC/"
echo "    use it with:  . .toolchain/env.sh && ./build-apk.sh"
echo "    (build-apk.sh also sources $TC/env.sh on its own)"
