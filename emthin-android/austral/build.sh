#!/usr/bin/env bash
# Rebuild the pinned K9 rule artifact from its Austral source.
#
# Toolchain (aarch64 host, local checkout):
#   AUSTRALVM=${AUSTRALVM:-/home/leo/Projects/australVM}
#   $AUSTRALVM/_build/default/bin/austral.exe  (needs the bridge .so on
#   LD_LIBRARY_PATH)
#
# The compiler emits a real CPS v1 binary from LinkRule.aum; emthin-android
# embeds LinkRule.cps with include_bytes! and refuses any artifact whose
# SHA-256 differs from the pin in src/austral_rule.rs. Rebuilding therefore
# requires updating ARTIFACT_SHA256 in the same change — deliberately, so a
# silent artifact swap cannot pass the gate.
set -euo pipefail

HERE="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
AUSTRALVM="${AUSTRALVM:-/home/leo/Projects/australVM}"
COMPILER="$AUSTRALVM/_build/default/bin/austral.exe"

if [ ! -x "$COMPILER" ]; then
  echo "build.sh: no Austral compiler at $COMPILER (set AUSTRALVM)" >&2
  exit 1
fi

cd "$HERE"
LD_LIBRARY_PATH="$AUSTRALVM:${LD_LIBRARY_PATH:-}" \
  "$COMPILER" compile --target-type=tc --use-cps-jit \
  --emit-cps=LinkRule.cps LinkRule.aum

FINGERPRINT="$(sha256sum LinkRule.cps | awk '{print $1}')"
echo "artifact: $HERE/LinkRule.cps ($(stat -c %s LinkRule.cps) bytes)"
echo "sha256:   $FINGERPRINT"
echo "Update ARTIFACT_SHA256 in ../src/austral_rule.rs if this changed."
