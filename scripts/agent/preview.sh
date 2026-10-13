#!/usr/bin/env bash
# Agent loop, headless tier: inspect a document's figures without a
# display, a compositor, or an app.
#
#   ./scripts/agent/preview.sh <doc.typ>                    # geometry JSON
#   ./scripts/agent/preview.sh <doc.typ> --png <out.png>    # also render the page
#
# Contract: stdout is ONE compact JSON line (geometry + diagnostics,
# ~150-600 tokens). Everything else — devshell banners, progress,
# errors — goes to stderr. This is the *semantic-first* tier of
# docs/AGENT_LOOP.md: it answers "where will every figure land, and
# is anything structurally wrong?" before anyone boots a compositor.
#
# Fast path: runs the prebuilt velysterm/target/debug/mathed_mini
# directly (~0.1 s) when it is up to date with the Rust sources.
# Otherwise (first use, or sources newer than the binary) it rebuilds
# inside velysterm's devshell via `cargo run` (~6 s warm, minutes on
# a cold target dir), then the fast path applies again.
#
# The binary comes from the sibling velysterm checkout — the same path
# dependency the emthin build uses — with the `gui` feature that
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
    -h|--help) sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) echo "FAIL unknown argument: $1" >&2; exit 2 ;;
    *) [ -z "$DOC" ] || { echo "FAIL exactly one document, got '$DOC' and '$1'" >&2; exit 2; }
       DOC="$1"; shift ;;
  esac
done
[ -n "$DOC" ] || { echo "usage: $0 <doc.typ> [--png <out.png>]" >&2; exit 2; }
[ -f "$DOC" ] || { echo "FAIL no such document: $DOC" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Input guard: this tool lays out the document's TEXT source. emthin's
# session snapshot is a binary loro CRDT file — passing one reads as a
# cryptic `stream did not contain valid UTF-8` from deep inside typst.
# ---------------------------------------------------------------------------
if [ "$(head -c 4 "$DOC" 2>/dev/null)" = "loro" ]; then
  echo "FAIL $DOC is a binary emthin session snapshot (loro CRDT), not a text document." >&2
  echo "     Export the text first: live.sh save out.typ (IPC save_doc writes source text)." >&2
  exit 3
fi
if command -v iconv >/dev/null 2>&1 && ! iconv -f UTF-8 -t UTF-8 "$DOC" >/dev/null 2>&1; then
  echo "FAIL $DOC is not valid UTF-8 text — refusing to lay it out." >&2
  exit 3
fi

# mathed_mini reads the path itself; make it absolute so the velysterm
# cwd below cannot shadow a relative one.
DOC="$(cd "$(dirname "$DOC")" && pwd)/$(basename "$DOC")"

MINI="$VELYSTERM/target/debug/mathed_mini"

# Stale when the binary is missing or any source it depends on is newer.
mini_stale() {
  [ -x "$MINI" ] || return 0
  local probe
  probe=$(find "$VELYSTERM/crates/mathed_core/src" "$VELYSTERM/crates/mathed_mini/src" \
              -name '*.rs' -newer "$MINI" -print -quit 2>/dev/null || true)
  [ -n "$probe" ] && return 0
  for f in "$VELYSTERM/crates/mathed_core/Cargo.toml" "$VELYSTERM/crates/mathed_mini/Cargo.toml"; do
    [ -e "$f" ] && [ "$f" -nt "$MINI" ] && return 0
  done
  return 1
}

run_mini() {
  if mini_stale; then
    # Rebuild inside the devshell (fonts/rasteriser deps resolve there);
    # cargo run compiles into target/debug, arming the fast path.
    if [ "${VELYSTERM_DEVSHELL:-}" = 1 ]; then
      (cd "$VELYSTERM" && cargo run -q -p mathed_mini --bin mathed_mini -- "$@")
    else
      (cd "$VELYSTERM" && nix develop --command cargo run -q -p mathed_mini --bin mathed_mini -- "$@")
    fi
  else
    "$MINI" "$@"
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
