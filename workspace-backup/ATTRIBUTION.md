# Attribution ledger

What this project borrowed, from where, under what licence, and where it landed.

This is the workspace-level ledger for the seven sibling checkouts. Each repo
carries a short `ATTRIBUTION.md` pointing here with the rows that concern it, so
the full table has one home and cannot drift per-repo.

## Policy

1. **Adapt ideas, do not copy source.** Where a pattern was taken from another
   project, the row below says what was adapted and where it landed. No snippet
   of third-party source is vendored into these trees.
2. **Never re-licence.** Apache-2.0-derived code must not be published under MIT.
   The repo licences below are as declared in each repo's `LICENSE`.
3. **Copyleft stays behind a subprocess.** Cadabra2 (GPL-3.0) and Why3 (LGPL) are
   invoked as external programs. The Rust and OCaml binaries never link them, so
   the surrounding licence is unaffected. This is a deliberate architectural
   constraint, recorded here because it is easy to erode by accident.
4. **Adding a row is part of the change.** If a commit adapts an external
   pattern, add the row in the same commit. `scripts/check-attribution`
   enforces the mechanical half: every attribution claim (`adapted from`,
   `inspired by`, `ported from`, `borrowed from`) and every derivation phrase
   (`based on`, `derived from`) in the seven licensed checkouts and the root
   plan docs must resolve — to a source name in this file or in the citing
   repo's own `ATTRIBUTION.md`, to an internal document the line points at, or
   to a reviewed allowlist entry in the script (each entry carries a reason;
   derivation-only exemptions are class 2 and can never excuse an attribution
   claim). The script's header states what the check cannot prove.

## Repo licences

| repo | licence |
|---|---|
| `unfer` | Apache-2.0 |
| `australVM` | Apache-2.0 |
| `timepiece` | Apache-2.0 |
| `dynamic-arctic` | MIT |
| `velysterm` | MIT OR Apache-2.0 (dual) |
| `emthin` | GPL-3.0 |
| `test` | prose site, see below |

## External sources

| source | licence | what was adapted | where it landed |
|---|---|---|---|
| [smithay](https://github.com/Smithay/smithay) (fork `ningxilai/smithay`, branch `emskin-patches`) | MIT | Wayland compositor substrate. Not a fork we maintain from scratch: the x11 backend, `text_input` handling and `seat_data` selection patches are carried on the branch, and `AGENTS.md` records that reading a clean upstream tree will not match what emthin links. | `emthin/` — the entire compositor; see `emthin/AGENTS.md` "smithay is a fork" |
| [velyst](https://github.com/voxell-tech/velyst) | see upstream | Document layout and rasterisation, taken as a git dependency at tag `v0.0.1`. | `velysterm/crates/mathed_*`, and the geometry source for `emthin`'s page raster |
| [Cadabra2](https://cadabra2.org) | GPL-3.0 | Field-theory CAS reached as an external process (`cadabra2-cli`, `CADABRA_LI` override). No GPL code is linked; the extracted expressions are the user's own programs. | `unfer/prob_kernel/src/symbolic.rs`, `unfer/docs/*.cdb` |
| [Why3](https://why3.org) | LGPL-2.1 (verified from the clone at `why3/`) | Proof and extraction, as an external process (`why3 prove` / `why3 extract`). | `unfer/prob_kernel/src/whyml.rs`, `australVM/lib/why3_plugin/` |
| [alt-ergo](https://alt-ergo.com) | LGPL | SMT provers for the WhyML gate. | `unfer/flake.nix` dev shell |
| [nanoda_lib](https://github.com/ammkrn/nanoda_lib) | see upstream | Independent re-verification of `lean4export` proof payloads, so a proof is checked by something other than the system that produced it. | `unfer/prob_kernel/src/verify.rs` |
| [`lean4export`](https://github.com/leanprover/lean4export) (official `leanprover/lean4export`) | Apache-2.0 | Export format for Lean proofs. The legacy `ammkrn` exporter emits the old textual format 2.0.0 that nanoda rejects, so the official tool is required. | `unfer/prob_kernel/tests/fixtures/*.ndjson`, pinned fixtures |
| [mathlib](https://github.com/leanprover-community/mathlib4) | MIT | Standard library, as a Lake dependency at tag `v4.28.0`. | `timepiece/`, pinned by `lean-toolchain` |
| [typos](https://github.com/crate-ci/typos) | Apache-2.0 | The index/graph discipline: a checked-in index that CI verifies against a fresh rebuild, rather than linting prose ad hoc. | `timepiece/scripts/doc_index.py` + its CI gate, `test/scripts/check_site.py` |
| [driftwm](../driftwm) | GPL-3.0 | Canvas mechanics for the document desktop (sibling repo, same author). | `emthin/crates/emthin/src/handlers/apps.rs`, `docs/REWRITE_PLAN.md` |
| **EWM** — the original emthin (identified by the owner; the GitHub fork parent of `leonardopedro/emthin` is `emskin/emskin`, github.com/emskin/emskin) | GPL-3.0 (verified from the clone at `emskin/`) | The pre-fork upstream this compositor derives from — "an outdated version of emthin, for all practical purposes" (owner). Cited where a design is carried over from it. | `emthin/crates/emthin/src/protocols/workspace.rs` (diff-based workspace refresh); tree at `emskin/` |
| [deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) | MIT — verified from the clone at `deepseek-harness/` (2026-10-07); "a Cordis-based agent harness" matches `unfer/PLAN_HARNESS.md`'s description | Agent-harness layout and invariant-gate discipline. | `unfer/PLAN_HARNESS.md`, `unfer/scripts/verify-invariants` |
| [Arctic](https://eprint.iacr.org/2024/466) (Komlo & Goldberg, PKC 2025) | MIT (this repo) | The threshold-Schnorr scheme is *implemented from the paper*, and the paper is credited in `dynamic-arctic/README_UPSTREAM.md`. | `dynamic-arctic/src/arctic_core.rs`, `shine_core.rs` |
| [Theseus](https://theseus.io) | MIT | Capability-oriented architecture as the model for linear types + JIT policy. Named as inspiration in `australVM/README.md`. | `australVM/` — capability tokens, Cedar policy in the JIT |
| [`celld`](../celld) (sibling repo, same author) | Apache-2.0 | Strict CLI contract discipline: `--json` prints one structured object per line (NDJSON), unknown/malformed arguments exit with a usage error rather than a panic, `--help` is part of the tested surface. | `australVM/safestos/cranelift/src/bin/modhost.rs` (+ `tests/modhost_cli.rs`), `velysterm/docs/BENCH.md`, `dynamic-arctic/Cargo.toml` (`lab` profile), `test/scripts/check_site.py` |
| [REA](https://github.com/morluto/rea) (`rea-agents` v6.4.0, checkout at `../rea/`) | MIT | Reverse-engineering toolchain; **patterns only, no source vendored**. Adapted: the tagged error contract (`{code, category, message, retryable, remediation}` + schema-validated closed categories); generated tool/method catalogs kept in sync with source contracts; AGENTS.md-as-authority-boundary spec (observations vs inferences, evidence provenance, fix-the-producing-boundary rule); E2E scenario contracts; CI layering (fast gate vs scheduled real-environment workflows); credential-redaction discipline for logs. | `PLAN_CROSS_PROJECT_REVIEW_2026-10-12.md` §4; `emthin/docs/error-contract.md`; `emthin/scripts/check-ipc-docsync`; `scripts/check-all-gates`; `unfer/.github/workflows/heavy-tests.yml` |
| **Agensh** — Zhan, Song, Dong, Huang, Lian, Xia, Wei, *"Agensh: Scaling Organizational Intelligence to 1,024 Agents"*, Microsoft Research technical report | paper text all-rights-reserved; companion repo licence **unverified** (treated as all-rights-reserved) | **Mechanisms only, described in this project's own words.** No code, no prompt text, no config, nothing vendored — the five-step cooperation loop, the typed shared-context board, claim/verify/merge and the scaling *evaluation framing* are all reimplementations over our own stack. The paper **must** be cited by author-year wherever those mechanisms are documented (`PROJECT_REVIEW_AND_IMPROVEMENT_PLAN.md` §9.2), so citations by name are expected and correct; what is forbidden is copying, not naming. | `unfer/unfer_protocol/src/{board,coop,evidence,nudge}.rs`, `velysterm/crates/kernel_client/src/{coop_loop,org_runner,bench}.rs`, cited in `test/software/org-harness.md`, `velysterm/docs/BENCH.md` |
| **ClawSwarm** (The-Swarm-Corporation) | Apache-2.0 | **Operational posture only, reimplemented.** The unified-gateway idea (one ingest surface, poll-with-cursor, health), narrow-role workers over a grant vocabulary, bounded memory with retrieval, documented config precedence, and the deployment runbook shape. No source copied; the name is never used as a feature name, and no badges or branding are used (trademark/endorsement). | `unfer/unfer_protocol/src/{event_log,model_spec,ingest}.rs`, `unfer/unfer_edge/src/config.rs`, `unfer/deploy/`, cited in `test/software/org-harness.md` |
| **AnaCP** — Momeni, Xiao & Liu, *"AnaCP: Toward Upper-Bound Continual Learning via Analytic Contrastive Projection"*, NeurIPS 2025 (`AnaCPtex/`, `AnaCP/`) | **no `LICENSE` file exists in either tree** → default copyright; paper text likewise all-rights-reserved | **Reimplemented from the published equations; no code, no text.** Methods and facts are not protected expression. Taken: the closed-form ridge to target prototypes, the whitened + SVD-perturbed "spread means" construction (§4 step 4), the RP/ELM expansion with GELU, synthetic pseudo-replay as a *future* option. **One documented deviation:** the Lemma-1 sign rule is **searched** over `δ ∈ {−1,0,+1}` against a measured objective rather than transcribed in closed form — see `unfer/docs/DECIDE_SPEC.md` §1. | `unfer/decide_core/src/targets.rs` (`spread_means`), `rp.rs`, `heads.rs` |
| **AnaNC** — Momeni, Xiao & Liu, *"Continual Out-of-Distribution Detection with Analytic Neural Collapse"*, AAAI 2026 (`AnaNC.md`, `AnaNC/`) | paper carries an explicit *"Copyright © 2026, AAAI. All rights reserved"* notice; the code has **no `LICENSE`** | **Reimplemented from the published equations; no code, no text.** Taken: the simplex-ETF target constraint solved by orthogonal Procrustes (eqs. 9–15), the maximum-cosine ID score, the `k ≥ C` precondition that makes the constraint satisfiable at all. **Explicitly not taken:** the paper's threshold selection is out of scope there and remains out of scope here. | `unfer/decide_core/src/targets.rs` (`etf_procrustes`, `etf_gram`), `heads.rs` (`NcmHead::id_score`) |
| **open-jev** (`open-jev/`) | Apache-2.0 | **Type shapes and the ECE framing, reimplemented as serde types.** The `Noul`/`Choice`/`Score` question vocabulary, its answer types, the rule that `confidence` is epistemic certainty and *not* `max(probabilities)`, and treating calibration as a first-class objective. Code reuse is legally permitted with the Apache notice; project policy is still adapt-not-copy. **No branding carried:** TypeSafe AI's "Jev" name, badges and framing are not used, and open-jev's own README disclaims production equivalence. | `unfer/unfer_protocol/src/decide.rs`, `unfer/decide_core/src/calibrate.rs` |
| [niri](https://github.com/YaLTeR/niri) | GPL-3.0-or-later | The xwayland-satellite socket setup + spawn/watch helpers, ported from `src/utils/xwayland/`; per-function attribution lives in the submodules. GPL code in a GPL-3.0 repo — compatible and deliberate. | `emthin/crates/emthin/src/xwayland_satellite/` |
| [foot](https://codeberg.org/dnkl/foot) | MIT | Search semantics ported from foot's `search.c`: case-insensitivity iff the query has no uppercase (foot's `hasc32upper` rule), empty-query behaviour. | `velysterm/crates/mathed/src/search_sys.rs`, `velysterm/docs/mathed/TASKS.md` |
| [Bevy](https://bevyengine.org) | MIT OR Apache-2.0 | Editor/event design: composed-character input events, and the ECS-shaped editor structure inherited through `velyst`; `mathed` still depends on bevy 0.19. | `velysterm/crates/mathed_mini/src/app.rs` (event design), `velysterm/crates/mathed` (direct dep) |
| **ax** | Apache-2.0 — verified from the clone at `ax/` (`github.com/google/ax`, LICENSE read 2026-10-08; the owner cloned it locally, superseding the 2026-10-07 *retired* verdict, and it matches the record in `test/AGENTS.md`) | The `/healthz` endpoint pattern (one probe per existing check) and the doc/AGENTS pattern discipline. | `timepiece/scripts/health.sh`, `test/AGENTS.md` |
| [cloudflare-os](https://github.com/cloudflare/cloudflare-os) | Apache-2.0 — verified from the clone at `cloudflare-os/` (2026-10-07); also recorded in `unfer/docs/PLAN_cloudflare_os_adaptation.md` | Gatekeeper approval queue (a side-effecting action returns *pending* and a human resolves it) and "nothing is ambient" resource introduction (introduce/use/forfeit at one chokepoint); blueprint immutability. Design reimplemented, no source copied. | `unfer/unfer_ffi/src/lib.rs` (`uk_gate_*`, `uk_resource_*`), `unfer/unfer_data/src/blueprint.rs`, `unfer/unfer_edge/src/admin.rs` |
| **SafestOS** | Apache-2.0 with LLVM-exception (stated in its README; the in-tree copy carries **no LICENSE file** — verify) | The C runtime: lock-free scheduler, cell loader, memory management (`australVM/README.md` §Architecture item 3). Same author's project; the tree itself lives in this workspace. | `australVM/` runtime, recorded at `australVM/README.md`; tree at `australVM/safestos/` |
| [Azuro](https://azuro.org) | see upstream public documentation (no code taken) | vAMM + NegRisk market framing for trigger-probability markets. **Plan-stage only** — no contract code written. | `unfer/docs/MATHBOND.md`, `unfer/docs/PLAN_REFI_EXCHANGE.md` |
| **qm** | MIT — verified from the clone at `qm/` (`github.com/yc-software/qm`, LICENSE read 2026-10-08; the owner cloned it locally, superseding the 2026-10-07 *retired* verdict) | Harness patterns: durable-by-default store, idempotency keys + leader lease, job-queue claim slots, harness-router, security-posture classifier (PLAN_HARNESS H2/H4/H7/H8/H9). | `unfer/PLAN_HARNESS.md` |
| [`pattern`](../pattern) + [`pattern-main`](../pattern-main) — **two checkouts of the same fork** (`leonardopedro/pattern`, a fork of `orual/pattern`); owner layout established 2026-10-08: `pattern/` holds branch `rewrite-v3`, `pattern-main/` holds branch `main` | `pattern/` = **MPL-2.0**, verified from its checked-out working tree (`rewrite-v3` tip 208bf142 of 2026-07-01, above the `MPL-2.0 relicense` commit 2fe18484; read 2026-10-08). `pattern-main/` = **AGPL-3.0**, verified from a fresh single-branch clone of `main` (tip 50241994 of 2026-01-09; read 2026-10-08). The plan's licence split — `../pattern` MPL-2.0, `../pattern-main` AGPL-3.0 — matches this two-folder layout (`unfer/docs/IMPLEMENTATION_PLAN.md` §P11.19) | Linear session memory + shared-context design, adapted into the sibling crate. **Copyleft constraint: the adapted code stays inside `../pattern` and is never vendored into `unfer`.** Second adaptation (2026-10-14): `pattern_memory::loro_sync::SyncedDoc`'s persistence doctrine — stamp-checked saves (mtime+len+hash echo suppression) with `RejectAndNotify` as the default and explicit `force` as the only overwrite — reimplemented from the design idea in `velysterm`'s save path (no code copied; MPL-2.0 design idea, MIT/Apache implementation). | `../pattern/crates/pattern_unfer` — present in the `pattern/` checkout (branch `rewrite-v3`, with `pattern_memory`); absent from `pattern-main/` on `main`. Doctrine landed at `velysterm/crates/mathed_core/src/format.rs` (`FileStamp`, `save_snapshot_guarded`) + `velysterm/crates/mathed/src/main.rs` |
| **zentinel** (`zentinelproxy/zentinel`, forked as `leonardopedro/zentinel`) | Apache-2.0 — verified from the clone at `zentinel/` (2026-10-07) | Edge-module design cues only: ai-gateway-style op routing, data-masking JSON redaction, allowlist gating. No code vendored; zentinel itself is deliberately *not* integrated. | `unfer/unfer_edge/` |
| **claurst** (`Kuberwastaken/claurst`, forked as `leonardopedro/claurst`) | GPL-3.0 — verified from the clone at `claurst/` (2026-10-07) | Inspiration only for the Nix sandbox layering; never vendored, never linked. | `unfer/unfer_nixvm/` (design influence) |
| **cloud-hypervisor-build** | Apache-2.0/BSD (as recorded in `unfer/docs/IMPLEMENTATION_PLAN.md` §P11.23; **retired 2026-10-07, recreated 2026-10-08** — the owner had the original only on another PC as a Debian-package build tree (SpectrumOS + crossvm patches, vhost included), so `australVM/cloud_hypervisor_vm/setup.sh` regenerated it on this NixOS machine; licences now read firsthand from the regenerated tree: cloud-hypervisor `LICENSES/` Apache-2.0 + BSD-3-Clause (docs CC-BY-4.0), crosvm LICENSE BSD-3-Clause, rust-vmm/vhost LICENSE Apache-2.0 + BSD-3-Clause, SpectrumOS patch tarball Apache-2.0 AND LicenseRef-BSD-3-Clause-Google; NixOS-adapted: `#!/usr/bin/env bash` shebangs, minijail `/bin/echo` hardcode dropped in `setup.sh`, nix-shell build deps documented — **no .deb** produced (owner decision; the release binaries + flake images are the artifacts): `cloud-hypervisor v50.0-2-g2ffa2c05a` and `crosvm 0.1.0` built and smoke-checked, spectrum0 patches incl. `vhost.patch` verified applied in the clones) | NixOS-on-a-VM execution sandbox, composed rather than edited — except one launch-script path bug fixed in place. | `unfer/unfer_nixvm/` |
| **DeepSeek Engram paper** — Cheng et al., *"Conditional Memory via Scalable Lookup: A New Axis of Sparsity for Large Language Models"*, arXiv:2601.07372 | companion repo `Engram/` is Apache-2.0 for its code; the paper text is arXiv-default (authors retain copyright) | **The paper's full text is reproduced as a design note.** Recorded plainly because that is a verbatim third-party reproduction: policy 1 ("no snippet of third-party source is vendored") is the owner's line to draw, and a ledger is the wrong place to pretend it away. | `australVM/engram.md`, `australVM/docs/ENGRAM.md`; reference trees `Engram/`, `engram-reference/` at the workspace root |
| [loop](https://github.com/soketlabs/loop) | Apache-2.0 — read from `loop/LICENSE` ("Copyright 2026 Soket AI"); the checkout is at `loop/` in this workspace | Exec-output capture discipline for the kernel worker, as two ideas rather than two copies: (a) sanitize control characters *before* a captured byte stream becomes a `Diagnostic` string, because `from_utf8_lossy` repairs invalid bytes but still lets an ESC sequence, a BEL or a NUL through to whatever renders it; (b) keep the **tail** of a failure diagnostic so the decisive line survives a byte cap, while output that is read top-down keeps its head. Implementation, names and tests are ours; no source vendored. | `velysterm/crates/kernel_client/src/worker.rs` (`sanitize`, `truncate_tail`, `capture`, `capture_tail`) |

Note the deliberate asymmetry: Agensh is cited in **both** the borrowed table and
the not-used table below, because its mechanisms were adapted *and* its code is
off-limits. ClawSwarm appears only above, since nothing was taken from it that
required a licence carve-out.

## Considered and *not* used

Recorded because a ledger that only lists what was taken cannot show that
something was deliberately left alone.

| source | licence | status |
|---|---|---|
| **Agensh** companion code (`microsoft/Agensh`) | **unverified** → all-rights-reserved | **No code, no prompt strings, no config, no vendored files.** Nothing was read out of the repository into these trees. Only the paper's *described* mechanisms were adapted, and they are cited by name per the row above — a citation is not a copy. Re-verify the licence before this ever changes. |
| **`laya`** | unknown — **not present in the workspace** | **Absent, so nothing was reviewed and nothing was used.** Grep hits for the name are substrings of unrelated words ("display a", "play a"). Recorded as *absent* rather than *unused*, because the distinction matters: there is no evidence either way about its content. |
| **`PromptANaCP`** (workspace root, in-house AI-assisted draft) | ours | **Superseded** by `PLAN_ANALYTIC_DECISION_ENGINE.md` Part II, which audits it. **Its energy-based OOD mechanism (`−τ·logsumexp`, 95th-percentile threshold) is considered and NOT used**: AnaNC's maximum-cosine ID score replaced it, and the threshold policy it implied is a decision this project has not validated. Its `softmax(cos/τ)` "probability" is likewise not used — that is normalization, not calibration. See `unfer/docs/DECIDE_SPEC.md` §2. |
| The `UnifiedAnalyticDecisionEngine` sketch's *architecture* (two modes, open-world `(K+1)` state, typed answers) | ours | **Adopted.** The product shape was useful even where the method implementation was wrong; Part II of the plan lists which specific mechanisms were rejected and why. |
| ClawSwarm token-launch / Solana fee tooling | Apache-2.0 | Not adapted, and not on the roadmap. Speculative finance, unrelated to this project's mission. |
| ClawSwarm "10,000 agents" framing; OpenClaw-comparison marketing | Apache-2.0 | Not adapted. The scaling claim in `test/software/org-harness.md` is stated as *unmeasured* until the N2 benchmark earns it. |
| Gitea / Mattermost | MIT / mixed | Not adopted. The board and direct-message operations of §7 G1/G3 are implemented in-project instead. |
| Swarms framework, Claude Agent SDK | various | Not imported at all. |
| Voice / canvas / companion-app product surfaces | — | Out of scope by decision (`emthin`'s document-as-desktop is this project's UI thesis). |

## `test/` content licence

The `test/` site is prose by the author (ORCID
`0000-0003-1564-9606`), currently unlicensed. It quotes no third-party source.
Any future third-party quotation needs a row above and a licence decision here
first — the site's own licence is an open question, not a default.

## Keeping this honest

- **The mechanical half of policy #4 is machine-checked.**
  `scripts/check-attribution` (workspace root, mirrored at
  `unfer/scripts/check-attribution` and run by unfer's CI hygiene job) scans
  the seven licensed checkouts plus the root plan docs and fails when an
  adaptation claim resolves to nothing — first green run 2026-10-07, with the
  fail→fix path verified (a planted claim fails, removing it passes). Its
  header states what it cannot prove: a crowded line resolves on the first
  recorded source found, `book.tex`/`ODE.tex` are excluded as literature, and
  no grep distinguishes "cited" from "copied from". What is left — the prose
  itself, and the licence judgements in the rows — stays reviewed by hand.
- **This file previously carried a false claim and was corrected on 2026-10-05.**
  It asserted "a grep for `ClawSwarm` across all seven repos returns zero files"
  and the same for `Agensh`. Both were false: §9.2 *requires* the Agensh paper to
  be cited by author-year wherever the G-items are documented, so
  `test/software/org-harness.md` naming both projects is the compliant behaviour,
  not a violation. A ledger that claims a checkable zero and is wrong is worse
  than one that states the real rule, so the check is now the real rule. It was
  widened on the same day to cover the three decision-engine sources.

  Check 1 is the load-bearing one — nothing is vendored or depended on:

  ```sh
  grep -rniE 'clawswarm|agensh|anacp|ananc|open-jev' \
        --include='Cargo.toml' --include='Cargo.lock' \
        --include='flake.nix' --include='dune-project' . \
    | grep -v target \
    | grep -v '^\./ClawSwarm/' | grep -v '^\./agensh/' \
    | grep -v '^\./AnaCP/'   | grep -v '^\./AnaNC/' \
    | grep -v '^\./open-jev/'                                  # expect: NO OUTPUT
  ```

  Check 2 enumerates every remaining mention, all of which must be documentation
  or doc comments — never identifiers, paths, dependency names or copied strings:

  ```sh
  grep -rliE 'clawswarm|agensh|anacp|ananc|open-jev' \
        --include='*.md' --include='*.rs' . \
    | grep -v target \
    | grep -v '^\./ClawSwarm/' | grep -v '^\./agensh/' \
    | grep -v '^\./AnaCP/'   | grep -v '^\./AnaNC/' \
    | grep -v '^\./open-jev/'
  ```

  As of 2026-10-05, check 1 returns nothing and check 2 returns exactly 20
  files, every one a citation:

  ```
  AnaNC.md                                    (the source paper, workspace root)
  ATTRIBUTION.md
  PLAN_ANALYTIC_DECISION_ENGINE.md
  PROJECT_REVIEW_AND_IMPROVEMENT_PLAN.md
  test/software/org-harness.md
  timepiece/CONSOLIDATED_PLAN.md
  unfer/docs/DECIDE_SPEC.md
  unfer/decide_core/src/{lib,stats,calibrate,consumers,report,backbone,properties}.rs
  unfer/decide_core/src/bin/decide_report.rs
  unfer/unfer_protocol/src/{decide,board}.rs
  velysterm/docs/BENCH.md
  velysterm/crates/kernel_client/src/bench.rs
  ```

  Note `decide_core/src/{targets,rp}.rs` are **not** in that list: they carry the
  implementations of the cited constructions but name no source in their text.
  The citations live in the spec and in this ledger instead, which is the
  correct place for them — an implementation file naming a paper it does not
  quote would be a claim about provenance that a reader cannot check.
- A note on what check 2 cannot prove: it cannot distinguish "cited" from
  "copied from". That rests on the authors' provenance record, which is why the
  §9 rule is written as an obligation ("quote nothing, paraphrase mechanisms")
  rather than as something a grep could enforce. Check 1 is the part that *is*
  mechanically enforced, and it is the part that matters most: it proves no
  third-party artefact entered the build graph.