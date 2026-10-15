#!/usr/bin/env bash
# Agent loop, replay tier: run a deterministic document scenario —
# observe, apply an edit script, assert — with no compositor and no
# display.
#
#   ./scripts/agent/replay.sh <scenario.txt>
#
# The scenario is a line-based script with a CLOSED vocabulary (an
# unknown verb fails the run; it is never ignored):
#
#   # comment / blank lines        ignored
#   doc <path>                     set the working document (first one
#                                  starts the scenario; a later one
#                                  resets the baseline)
#   apply <path>                   apply an edit script: *.patch/*.diff
#                                  is applied with patch(1), anything
#                                  else REPLACES the working document
#                                  (an LLM's returned edit script is
#                                  either form)
#   expect <assertion>             machine-checkable claim about the
#                                  current state (see below)
#
# Assertions — operators are == != < <= > >=; unknown verbs fail:
#
#   pages <op> <n>                 page count
#   figures <op> <n>               \app figure count
#   diag <op> <n>                  structural diagnostics count
#   figure <key> w|h|line|page|x|y <op> <n>
#   figure <key> caption == <text> (rest of the line is the text)
#   defs <op> <n>                  definition count
#   def <name> uses <op> <n>       resolved use count
#   def <name> line <op> <n>       1-based source line
#   unresolved <op> <n>            free identifiers
#   models <op> <n>                \model statements
#   model <name> present|absent
#   statements <kind> <op> <n>     e.g. statements model == 1
#
# stdout is the transcript: a label line per step ("==> verb ..."),
# then exactly one single-line JSON observation per state change
# (filter with ^{ to get just the data). Errors go to stderr.
# Paths are resolved relative to the scenario file, so a run is
# reproducible from any cwd.
#
# This is a *boundary* lane on its own (it crosses the filesystem and
# compiles the document, but owns no compositor); the acceptance lane
# remains scripts/e2e/run.sh against a real nested session — see
# scripts/testing-depths.md.

set -euo pipefail

HERE="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
PREVIEW="$HERE/preview.sh"

usage() {
  echo "usage: $0 <scenario.txt>" >&2
  exit 2
}

[ $# -eq 1 ] || usage
SCENARIO="$1"
[ -f "$SCENARIO" ] || { echo "FAIL no such scenario: $SCENARIO" >&2; exit 1; }
SCENARIO="$(cd "$(dirname "$SCENARIO")" && pwd)/$(basename "$SCENARIO")"
SCENARIO_DIR="$(dirname "$SCENARIO")"

# python3 for the assertion engine — uv-managed installs can live
# outside the default non-login PATH (the same P7 discovery the
# workspace gates use); absent-without-candidates is a hard failure,
# never a silent skip.
if ! command -v python3 >/dev/null 2>&1; then
  for candidate in /usr/local/bin/python3 "$HOME/.local/bin/python3"; do
    if [ -x "$candidate" ]; then
      PATH="$(dirname "$candidate"):$PATH"
      export PATH
      break
    fi
  done
fi
command -v python3 >/dev/null 2>&1 || {
  echo "FAIL python3 not found (checked PATH, /usr/local/bin, ~/.local/bin) — the assertion engine needs it" >&2
  exit 1
}
command -v patch >/dev/null 2>&1 || {
  echo "FAIL patch(1) not found — apply of .patch files needs it" >&2
  exit 1
}

SANDBOX="$(mktemp -d -t emthin-replay-XXXXXX)"
cleanup() {
  if [ "${REPLAY_KEEP:-0}" = 1 ] || [ "${REPLAY_FAILED:-0}" = 1 ]; then
    echo "replay: sandbox kept at $SANDBOX" >&2
  else
    rm -rf "$SANDBOX"
  fi
}
trap cleanup EXIT

WORK="$SANDBOX/doc.typ"
PREV="$SANDBOX/prev.typ"
LAYOUT="$SANDBOX/layout.json"
SEM="$SANDBOX/semantic.json"
HAVE_DOC=0
OK=0
FAILED=0

fail() {
  FAILED=$((FAILED + 1))
  REPLAY_FAILED=1
  echo "$@" >&2
}

# ------------------------------------------------------------------
# Observe: preview.sh owns the mathed_mini plumbing (staleness,
# devshell, input guards); replay only sequences it. Each call is a
# fresh process by design — the loop's rule ("nothing held between
# calls but the session is up") applies here too.
# ------------------------------------------------------------------
observe() {
  "$PREVIEW" "$WORK" >"$LAYOUT"
  "$PREVIEW" "$WORK" --semantic >"$SEM"
  echo "{layout}"
  cat "$LAYOUT"
  echo "{semantic}"
  cat "$SEM"
}

# ------------------------------------------------------------------
# Assertion engine — one line in, "ok" or a FAIL reason out.
# ------------------------------------------------------------------
assert_line() {
  local line="$1" out rc
  set +e
  out="$(python3 - "$LAYOUT" "$SEM" "$line" <<'PYEOF'
import json, re, sys

layout_path, sem_path, line = sys.argv[1], sys.argv[2], sys.argv[3]
try:
    layout = json.load(open(layout_path))
    sem = json.load(open(sem_path))
except Exception as e:
    print(f"cannot read observation: {e}")
    sys.exit(1)

parts = line.split(None, 1)
verb = parts[0] if parts else ""
rest = parts[1] if len(parts) > 1 else ""

def num(x):
    return float(x) if isinstance(x, str) and "." in x else int(x)

OPS = {
    "==": lambda a, b: a == b,
    "!=": lambda a, b: a != b,
    "<": lambda a, b: a < b,
    "<=": lambda a, b: a <= b,
    ">": lambda a, b: a > b,
    ">=": lambda a, b: a >= b,
}

def check(op, actual, want, what):
    if op not in OPS:
        print(f"unknown operator {op!r} (want one of {' '.join(OPS)})")
        return False
    if not OPS[op](actual, want):
        print(f"{what}: got {actual!r}, want {op} {want!r}")
        return False
    return True

def want_num(tok, what):
    try:
        return num(tok)
    except ValueError:
        print(f"{what}: {tok!r} is not a number")
        raise SystemExit(1)

try:
    if verb in ("pages", "figures", "diag", "defs", "unresolved", "models"):
        key = {"pages": "page_count", "figures": "figures", "diag": "diagnostics",
               "defs": "defs", "unresolved": "unresolved", "models": "models"}[verb]
        src = layout if verb in ("pages", "figures", "diag") else sem
        op, tok = rest.split(None, 1)
        ok = check(op, len(src[key]) if verb != "pages" else src[key],
                   want_num(tok, verb), verb)
        sys.exit(0 if ok else 1)

    if verb == "figure":
        m = re.match(r"(\S+)\s+(w|h|line|page|x|y|caption)(?:\s+(==|!=|<|<=|>|>=)\s+(.*))?$", rest)
        if not m:
            print("figure assertion malformed: want 'figure <key> <field> <op> <value>'")
            raise SystemExit(1)
        key, field, op, tok = m.group(1), m.group(2), m.group(3), m.group(4)
        fig = next((f for f in layout["figures"] if f["key"] == key), None)
        if fig is None:
            print(f"figure {key}: no such figure (have {[f['key'] for f in layout['figures']]})")
            raise SystemExit(1)
        if field == "caption":
            if op != "==":
                print("caption only supports ==")
                raise SystemExit(1)
            ok = fig.get("caption", "") == tok
            if not ok:
                print(f"figure {key} caption: got {fig.get('caption')!r}, want {tok!r}")
            sys.exit(0 if ok else 1)
        actual = fig[field] if field in fig else (fig.get("rect") or {}).get(field)
        if actual is None:
            print(f"figure {key}: field {field!r} absent (figure placed nowhere?)")
            raise SystemExit(1)
        sys.exit(0 if check(op, actual, want_num(tok, field), f"figure {key} {field}") else 1)

    if verb == "def":
        m = re.match(r"(\S+)\s+(uses|line)\s+(==|!=|<|<=|>|>=)\s+(\S+)$", rest)
        if not m:
            print("def assertion malformed: want 'def <name> uses|line <op> <n>'")
            raise SystemExit(1)
        name, field, op, tok = m.group(1), m.group(2), m.group(3), m.group(4)
        d = next((d for d in sem["defs"] if d["name"] == name), None)
        if d is None:
            print(f"def {name}: no such definition (have {[x['name'] for x in sem['defs']]})")
            raise SystemExit(1)
        sys.exit(0 if check(op, d[field], want_num(tok, field), f"def {name} {field}") else 1)

    if verb == "model":
        m = re.match(r"(\S+)\s+(present|absent)$", rest)
        if not m:
            print("model assertion malformed: want 'model <name> present|absent'")
            raise SystemExit(1)
        name, want = m.group(1), m.group(2)
        has = name in sem["models"]
        ok = has if want == "present" else not has
        if not ok:
            print(f"model {name}: {'absent' if has else 'present'}, want {want}")
        sys.exit(0 if ok else 1)

    if verb == "statements":
        m = re.match(r"(\S+)\s+(==|!=|<|<=|>|>=)\s+(\S+)$", rest)
        if not m:
            print("statements assertion malformed: want 'statements <kind> <op> <n>'")
            raise SystemExit(1)
        kind, op, tok = m.group(1), m.group(2), m.group(3)
        sys.exit(0 if check(op, sem["statements"].get(kind, 0),
                            want_num(tok, kind), f"statements {kind}") else 1)

    print(f"unknown assertion verb {verb!r} (closed vocabulary — see replay.sh header)")
    sys.exit(1)
except SystemExit:
    raise
except Exception as e:
    print(f"assertion error: {e}")
    sys.exit(1)
PYEOF
)"
  rc=$?
  set -e
  if [ $rc -eq 0 ]; then
    echo "==> expect $line  ok"
    OK=$((OK + 1))
  else
    echo "==> expect $line  FAIL — ${out:-assertion engine exited $rc}"
    fail "expect failed: $line"
  fi
}

# ------------------------------------------------------------------
# Scenario runner
# ------------------------------------------------------------------
STEP=0
while IFS= read -r raw || [ -n "$raw" ]; do
  STEP=$((STEP + 1))
  # strip leading/trailing whitespace, skip blanks and comments
  line="${raw#"${raw%%[![:space:]]*}"}"
  line="${line%"${line##*[![:space:]]}"}"
  [ -z "$line" ] && continue
  case "$line" in \#*) continue ;; esac

  verb="${line%% *}"
  arg=""
  [ "$line" != "$verb" ] && arg="${line#* }"

  case "$verb" in
    doc)
      [ -n "$arg" ] || { fail "scenario line $STEP: doc needs a path"; continue; }
      src="$arg"
      case "$src" in /*) ;; *) src="$SCENARIO_DIR/$src" ;; esac
      [ -f "$src" ] || { fail "scenario line $STEP: no such document: $src"; break; }
      echo "==> doc ${src#"$SCENARIO_DIR"/}"
      cp "$src" "$WORK"
      if [ "$HAVE_DOC" = 1 ]; then
        cp "$WORK" "$PREV" # a new doc resets the baseline
      else
        HAVE_DOC=1
        cp "$src" "$PREV"
      fi
      observe || {
        fail "scenario line $STEP: the document does not compile — the edit broke the compile (see stderr)"
        break
      }
      ;;
    apply)
      [ "$HAVE_DOC" = 1 ] || { fail "scenario line $STEP: apply before doc"; break; }
      [ -n "$arg" ] || { fail "scenario line $STEP: apply needs a path"; break; }
      src="$arg"
      case "$src" in /*) ;; *) src="$SCENARIO_DIR/$src" ;; esac
      [ -f "$src" ] || { fail "scenario line $STEP: no such edit script: $src"; break; }
      echo "==> apply ${src#"$SCENARIO_DIR"/}"
      cp "$WORK" "$PREV"
      case "$src" in
        *.patch|*.diff)
          if ! patch --no-backup-if-mismatch -s "$WORK" <"$src"; then
            fail "scenario line $STEP: patch did not apply cleanly"
            break
          fi
          ;;
        *) cp "$src" "$WORK" ;;
      esac
      # The delta is the "what changed" observation.
      if delta="$("$PREVIEW" --diff "$PREV" "$WORK" 2>"$SANDBOX/diff.err")"; then
        echo "{delta}"
        echo "$delta"
      else
        cat "$SANDBOX/diff.err" >&2
        fail "scenario line $STEP: the edit broke the compile — diff refused (see stderr)"
        break
      fi
      observe || {
        fail "scenario line $STEP: the document does not compile after apply"
        break
      }
      ;;
    expect)
      [ "$HAVE_DOC" = 1 ] || { fail "scenario line $STEP: expect before doc"; continue; }
      [ -n "$arg" ] || { fail "scenario line $STEP: expect needs an assertion"; continue; }
      assert_line "$arg"
      ;;
    *)
      fail "scenario line $STEP: unknown verb '$verb' (closed vocabulary: doc, apply, expect)"
      break
      ;;
  esac
done <"$SCENARIO"

echo "replay: $OK ok, $FAILED failed"
if [ "$FAILED" -gt 0 ]; then
  REPLAY_FAILED=1
  exit 1
fi
