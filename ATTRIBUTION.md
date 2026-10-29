# Attribution

Borrowed patterns and components that concern *this* repo. The authoritative,
cross-repo table — including what was deliberately **not** adopted — is
[`../ATTRIBUTION.md`](../ATTRIBUTION.md).

| source | licence | what was adapted | where it landed |
|---|---|---|---|
| smithay (fork `ningxilai/smithay`, branch `emskin-patches`) | MIT | Wayland compositor substrate; the x11/text_input/seat_data patches are carried on the branch | the compositor itself — `AGENTS.md` "smithay is a fork" |
| driftwm (sibling repo, same author) | GPL-3.0 | canvas mechanics for the document desktop | `handlers/apps.rs`, `docs/REWRITE_PLAN.md` |
| velyst (git dep, tag `v0.0.1`) | see upstream | page rasterisation for the document layer | `doc_render.rs` |
| **EWM** — the original emthin (`emskin/emskin`, fork parent of `leonardopedro/emthin`; identified by the owner) | GPL-3.0 (verified from the clone at `../emskin/`) | the pre-fork upstream this compositor derives from | `crates/emthin/src/protocols/workspace.rs` (diff-based workspace refresh) |
| niri (github.com/YaLTeR/niri) | GPL-3.0-or-later | xwayland-satellite socket setup + spawn/watch helpers ported; per-function attribution in the submodules | `crates/emthin/src/xwayland_satellite/` |
| **REA** — `rea-agents` (morluto/rea; tree not currently checked out) | MIT — as cited at the use sites (`rea/docs/reconstruction-obligation-ledgers.md`, MIT); row restored from the reviewed workspace-ledger table (preserved at `workspace-backup/ATTRIBUTION.md`) 2026-10-28 | patterns only, no source vendored: the tagged error contract (`{code, message}` with category/retryability/remediation decidable without parsing prose — a client branches on `code`, never on `message`), the E2E scenario contract shape (precondition, steps, expected observation, and explicitly verified / structurally broken / not exercised), and obligation-ledger closure (a claim closes only when a verifier of comparable authority runs and its result is recorded) | `crates/emthin/src/ipc/messages.rs`, `docs/error-contract.md`, `scripts/e2e/SCENARIOS.md` |

## Notes

This repo is GPL-3.0. It receives Apache-2.0 and MIT code without
incompatibility, and none of the rows above re-licence anything: smithay stays
MIT, driftwm stays GPL.

The substrate deserves one warning that is easy to lose: `smithay` is a **fork
with patches**, and `AGENTS.md` records that reading a clean upstream checkout
will not match what this binary links against. Three patched files each hide a
bug described under "Key Gotchas"
