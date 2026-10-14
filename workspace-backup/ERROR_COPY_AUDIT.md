# Error-copy audit — the federation's user-visible failure strings
**Date:** 2026-10-13 · **Pattern:** harm-ranked ledger adapted from REA's
`docs/user-visible-errors.csv` (MIT, ideas only — no source copied; see
`ATTRIBUTION.md`'s REA row). **Policy #4:** every string below was read at its
producing boundary; fixes happen there, never at a consumer.

Scope: the three surfaces whose text reaches a human (or a document):
emthin's IPC `error` messages, `unfer/voice/voice.sh` FAIL reasons (emthin
*splices these into the user's document* — the highest-blast-radius surface),
and `modhost`'s stderr/`--json` errors. Re-audit rule: a new user-visible
string gets its row here in the same change (root `scripts/README.md`,
"Admission criteria").

Harm rank: 1 = could leak a credential or corrupt user data · 2 = leaks
internals, no recovery path · 3 = recovery missing but truthful · 4 = fine,
recorded for the census.

## Ledger

| rank | location | trigger | current copy | risk | action |
|---|---|---|---|---|---|
| 4 | `emthin/src/ipc/dispatch.rs` NotBound ×2 | `close`/`focus` on an unbound figure | `no app bound to figure {key}` | figure keys are document-derived, no internals | keep; N2 now ships `remediation` alongside |
| 4 | `dispatch.rs` AppGone | focus after app exit | `figure {key}'s app is gone` | same | keep + remediation |
| 4 | `dispatch.rs` UnknownFigure ×2 | bad figure key | `unknown figure {key}` | caller's own input echoed | keep + remediation |
| 4 | `dispatch.rs` VoiceFailed | harness failure | `voice: {reason}` | **chain**: `reason` may be a voice.sh FAIL line, which may carry `head -c 300` of an LLM error response — third-party text reaching the document | accepted by design (it is the transcript's content); URL userinfo/query already stripped at the source (`LLM_URL_DISP`, S2/R12). Truncation bounds the blast radius. No change. |
| 3→4 | `dispatch.rs` IoFailed ×2 | open/save failure | `could not {read,write} {path}: {io_err}` | truthful, but recovery ("check path exists and is writable") was doc-only | **fixed by N2**: `remediation` now rides the wire |
| 4 | `voice.sh` `die env` | tool missing | `missing tool: {t} (run inside: nix develop .#voice)` | exact recovery | keep |
| 4 | `voice.sh` `die env` | model file missing | `piper voice not found: {path} (nix develop .#voice sets it)` | caller-set env echoed; recovery exact | keep |
| 4 | `voice.sh` `die usage` ×3 | bad args | names the offending value + the valid set | exact | keep |
| 4 | `voice.sh` `die history` ×2 | missing/malformed history | names the file + the expected shape | exact | keep |
| 2 | `voice.sh` `die llm` | no LLM reply | `no reply from {LLM_URL_DISP} (model up? /usr/local/bin/gemma4-toggle): {head -c 300 of response}` | the 300-byte response excerpt is untrusted server text; on this surface it lands **in the user's document** via emthin's splice | keep the excerpt (it is the diagnostic and the document is the user's own), URL redacted since S2. Recorded as the federation's one deliberate untrusted-text→document path. |
| 4 | `voice.sh` `die mic` | empty transcription | `heard nothing (microphone up? VOICE_SECS={n}s)` | recovery hinted | keep |
| 4 | `modhost` `usage_error` | bad flags | `{flag} requires a value` / `unknown option '{x}'` + usage line, exit 2 | exact, never a silent default | keep (celld strict-CLI contract) |
| 4 | `modhost` `cmd_authorize` | manifest unreadable/invalid | `cannot read manifest {p}: {e}` / `invalid manifest: {e}` + `--json` `kind` field | local CLI, caller-supplied path; TOML error quotes the caller's own file | keep |
| 4 | `modhost` `cmd_host` | load/attestation failure | `kind: load_failed` / `attestation_refused` events + stderr | structured `kind` = already taggable | keep |

## Verdict

**Zero rank-1 findings.** The round-1 redaction discipline (S2/R12: URL
userinfo+query stripped at the producing boundary; `Option<String>` keys,
never defaulted-empty) holds on re-audit. The one structural gap was rank 3:
emthin's truthful-but-recovery-less messages kept their recovery guidance in a
doc the client had to fetch. **N2 fixed that at the boundary** by shipping
`category`/`retryable`/`remediation` on the wire, byte-pinned to
`docs/error-contract.md` by `the_documented_error_semantics_match_the_wire_
fields` — so this ledger's largest class closed itself.

The `voice.sh` llm row stays a deliberate, documented rank-2: the response
excerpt is the diagnostic, the splice target is the user's own document, and
alternatives (drop the excerpt) would make failures undebuggable. Bounded
(300 bytes) + redacted URL is the accepted trade.
