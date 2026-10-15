#!/usr/bin/env bash
# Agent loop, headless tier: inspect a document's figures without a
# display, a compositor, or an app.
#
#   ./scripts/agent/preview.sh <doc.typ>                    # geometry JSON
#   ./scripts/agent/preview.sh <doc.typ> --semantic         # meaning JSON (tier 1b)
#   ./scripts/agent/preview.sh <doc.typ> --png <out.png>    # also render the page
#   ./scripts/agent/preview.sh --diff <before.typ> <after.typ>   # what changed
#
# Contract: stdout is ONE compact JSON line (geometry, semantics or a
# delta — ~150-600 tokens). Everything else — devshell banners,
# progress, errors — goes to stderr. This is the *semantic-first* tier
# of docs/AGENT_LOOP.md: it answers "where will every figure land, and
# is anything structurally wrong?", "what does this document mean?"
# (definitions, uses, dangling identifiers — no Typst compile, so it
# works on documents whose unresolved idents would not even render),
# and "what did my last edit change?" before anyone boots a
# compositor.
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

usage() {
  echo "usage: $0 <doc.typ> [--semantic] [--png <out.png>]" >&2
  echo "       $0 --diff <before.typ> <after.typ>" >&2
}

DOC=""
PNG=""
SEMANTIC=""
BEFORE=""
AFTER=""
while [ $# -gt 0 ]; do
  case "$1" in
    --png) PNG="${2:?--png requires a path}"; shift 2 ;;
    --semantic) SEMANTIC=1; shift ;;
    --diff)
      shift
      if [ $# -ne 2 ]; then
        echo "FAIL --diff takes exactly two documents (before after)" >&2
        exit 2
      fi
      BEFORE="$1"
      AFTER="$2"
      shift 2
      ;;
    -h|--help) sed -n '2,29p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) echo "FAIL unknown argument: $1" >&2; exit 2 ;;
    *) [ -z "$DOC" ] || { echo "FAIL exactly one document, got '$DOC' and '$1'" >&2; exit 2; }
       DOC="$1"; shift ;;
  esac
done

if [ -n "$BEFORE" ]; then
  [ -z "$DOC" ] || { echo "FAIL --diff cannot be combined with a single document" >&2; usage; exit 2; }
  [ -z "$PNG" ] && [ -z "$SEMANTIC" ] || {
    echo "FAIL --diff is its own mode; --png/--semantic do not apply" >&2
    exit 2
  }
fi

# ---------------------------------------------------------------------------
# Input guard: these tools lay out the document's TEXT source. emthin's
# session snapshot is a binary loro CRDT file — passing one reads as a
# cryptic `stream did not contain valid UTF-8` from deep inside typst.
# Prints the absolutized path on stdout; errors go to stderr with the
# documented exit codes (3 = refused input).
# ---------------------------------------------------------------------------
guard_doc() {
  local f="$1"
  [ -f "$f" ] || { echo "FAIL no such document: $f" >&2; return 1; }
  if [ "$(head -c 4 "$f" 2>/dev/null)" = "loro" ]; then
    echo "FAIL $f is a binary emthin session snapshot (loro CRDT), not a text document." >&2
    echo "     Export the text first: live.sh save out.typ (IPC save_doc writes source text)." >&2
    return 3
  fi
  if command -v iconv >/dev/null 2>&1 && ! iconv -f UTF-8 -t UTF-8 "$f" >/dev/null 2>&1; then
    echo "FAIL $f is not valid UTF-8 text — refusing to lay it out." >&2
    return 3
  fi
  # mathed_mini reads the path itself; make it absolute so the
  # velysterm cwd below cannot shadow a relative one.
  echo "$(cd "$(dirname "$f")" && pwd)/$(basename "$f")"
}

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

if [ -n "$BEFORE" ]; then
  BEFORE="$(guard_doc "$BEFORE")"
  AFTER="$(guard_doc "$AFTER")"
  # Snapshot/diff: one delta line. A compile failure in either side is
  # an error (exit != 0, message on stderr) — "the edit broke the
  # compile" never reads as "the figure disappeared".
  run_mini --diff "$BEFORE" "$AFTER"
  exit 0
fi

[ -n "$DOC" ] || { usage; exit 2; }
DOC="$(guard_doc "$DOC")"

# Semantic tier first: geometry + diagnostics (or, with --semantic,
# the tier-1b meaning dump), one compact JSON line.
if [ -n "$SEMANTIC" ]; then
  run_mini --semantic-json "$DOC"
else
  run_mini --layout-json "$DOC"
fi

# Visual tier only on request — a PNG costs an order of magnitude more
# tokens than the JSON, so the loop default is to not ask for eyes
# until the semantics say the numbers are right.
if [ -n "$PNG" ]; then
  run_mini --doc-image "$DOC" --out "$PNG" >&2
  echo "page raster -> $PNG" >&2
fi
