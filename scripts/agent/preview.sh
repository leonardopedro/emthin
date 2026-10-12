#!/usr/bin/env bash
# Agent loop, headless tier: inspect a document's figures without a
# display, a compositor, or an app.
#
#   ./scripts/agent/preview.sh <doc.typ>                    # geometry JSON
#   ./scripts/agent/preview.sh <doc.typ> --png <out.png>    # also render the page
#
# This is the *semantic-first* tier of docs/AGENT_LOOP.md: it answers
# "where will every figure land, and is anything structurally wrong?"
# for a few hundred tokens of JSON, before anyone boots a compositor.
# The optional PNG is the visual tier for the same document — the same
# pixels emthin's docui lays out, rasterised headlessly.
#
# It runs mathed_mini from the sibling velysterm checkout (the same
# path dependency the emthin build uses) inside velysterm's devshell,
# because the mathed_mini *binary* needs the `gui` feature that
# emthin's `default-features = false` build turns off.

set -euo pipefail

HERE="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
VELYSTERM="$(cd "$REPO/../velysterm" 2>/dev/null && pwd)" || {
  echo "FAIL sibling checkout ../velysterm not found (needed for mathed_mini)" >&2
  exit 1
}

DOC=""
PNG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --png) PNG="${2:?--png requires a path}"; shift 2 ;;
    -h|--help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) echo "FAIL unknown argument: $1" >&2; exit 2 ;;
    *) [ -z "$DOC" ] || { echo "FAIL exactly one document, got '$DOC' and '$1'" >&2; exit 2; }
       DOC="$1"; shift ;;
  esac
done
[ -n "$DOC" ] || { echo "usage: $0 <doc.typ> [--png <out.png>]" >&2; exit 2; }
[ -f "$DOC" ] || { echo "FAIL no such document: $DOC" >&2; exit 2; }
# mathed_mini reads the path itself; make it absolute so the velysterm
# cwd below cannot shadow a relative one.
DOC="$(cd "$(dirname "$DOC")" && pwd)/$(basename "$DOC")"

# Re-enter velysterm's devshell the same way its own scripts do: the
# binary is built there, and its fonts/rasteriser deps resolve in it.
run_mini() {
  if [ "${VELYSTERM_DEVSHELL:-}" = 1 ]; then
    (cd "$VELYSTERM" && cargo run -q -p mathed_mini --bin mathed_mini -- "$@")
  else
    (cd "$VELYSTERM" && nix develop --command cargo run -q -p mathed_mini --bin mathed_mini -- "$@")
  fi
}

# Semantic tier first: geometry + diagnostics, one compact JSON line.
run_mini --layout-json "$DOC"

# Visual tier only on request — a PNG costs an order of magnitude more
# tokens than the JSON, so the loop default is to not ask for eyes
# until the semantics say the numbers are right.
if [ -n "$PNG" ]; then
  run_mini --doc-image "$DOC" --out "$PNG" >&2
  echo "page raster -> $PNG" >&2
fi
