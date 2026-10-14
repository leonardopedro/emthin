# PLAN_ANALYTIC_DECISION_ENGINE — an analytic System-1 decision layer for the seven folders

*Status: **executed 2026-10-06**. All fifteen items were attempted; the
implementation is `unfer/decide_core` (91 tests) plus `unfer_protocol::decide`
and a pinned calibration report. **The headline result is negative and is enforced
in code: no candidate met the R1–R5 contract, so the engine emits scores and
`calibrated: true` is not licensed.** Owner lanes: eng (core + wiring), docs (site
+ attribution), LLM-Lean4-specialist (Part V work orders only — this file writes no
Lean code). Companion documents:
`PROJECT_REVIEW_AND_IMPROVEMENT_PLAN.md` (the 7-repo review; item IDs X/C/G/N
below refer to it), `timepiece/CONSOLIDATED_PLAN.md` (Lean work orders, §
"Addendum 2026-10-06"), `ATTRIBUTION.md` (licence ledger), and
`unfer/docs/DECIDE_SPEC.md` (the code↔math map this plan's D1 asked for).

## Execution summary

| item | verdict | the one thing to know |
|---|---|---|
| D1 spec | **DONE** | `unfer/docs/DECIDE_SPEC.md`, code↔math map per field, both modes, R1–R5, with the cited equations. |
| D2 `anacp` | **DONE** | Streaming stats, seeded RP+GELU, Cholesky ridge, spread-mean targets, NCM head. One **documented deviation**: Lemma-1's sign rule is *searched* against a measured objective, not transcribed. |
| D3 `ananc` | **DONE, plus a discovered precondition** | ETF/Procrustes + cosine NCM + max-cosine ID score. The constraint is satisfiable **only when features ≥ classes**; that is now a loud precondition, because violating it yields a target that is not an ETF at all (defect A2 resurfacing). |
| D4 properties | **DONE** | 4 properties in `properties.rs`, run in CI. |
| D5 typed protocol | **DONE** | `unfer_protocol::decide`, 4 ops registered, observe-kind only. `confidence: Option<f64>` because `f64::NAN` cannot survive NDJSON. |
| D6 calibration | **PARTIAL — and this is the substantive result** | All three mechanisms implemented and reported; **none met R2 or R5** on the pinned split, so `Calibrator::is_calibrated` refuses. The report is a committed fixture and CI fails if a fresh run drifts from it. |
| D7 backbone seam | **DONE** | `embed:` prefix over the C7 vocabulary; embedding-model identity enforced; L2 normalization at the seam. |
| D8 consumers (D-C1/D-C2) | **DONE** | Board triage + preset routing, advisory by construction. |
| D9 consumers (D-C3/D-C5) | **DONE** | Detail-level routing and nudge ordering, both degrading to a default rather than erroring. |
| D10 editor ranking | **DONE — closed 2026-10-08, with the deferred design decision made** | `decide_core::consumers::ranked()` (set-identity by type) landed with D5; the velysterm half was written 2026-10-08: `mathed_core::rank` (local engine, frozen model-free lexical features, `MATHED_RANK` config toggle **default off**), wired into the Bevy cite popups + references panel and into `mathed_mini`'s references panel, with golden tests asserting the rendered *set* is unchanged and only the order moves. The open question is answered **local**, not service (reasons in the D10 record). One listed surface does not exist: `mathed_core::completion` has no list to reorder — `completion_at` yields exactly one candidate by construction — so that row is recorded as absent rather than invented. |
| D11 benchmark | **DONE** | Forgetting matrix published. **`ananc` forgets catastrophically here** (1.0 vs `anacp`'s 0.5). |
| D12 drift monitor | **DONE** | Drift measured and declared; measured 0.067 against ε = 0.05, so R5 fails honestly. |
| D13 certified mode | **PARTIAL by its own gate** | Interval enclosures implemented and property-tested, but D13 is *conditional on D6*, and D6 did not pass — so the width is a **reported bound**, not a certificate, and the file says so. |
| D14 site page | **DONE** | `test/software/analytic-decision.md` + SUMMARY entry; `check_site.py` green. |
| D15 attribution | **DONE** | Four ledger rows (AnaCP, AnaNC, open-jev, the superseded sketch) plus `laya`-absent; the mechanical checks were re-run and widened. |

**The two findings worth carrying forward**, both recorded in `DECIDE_SPEC.md` §1–2
and in the `CONSOLIDATED_PLAN.md` addendum:

1. **A property test found R3 violated.** A Platt calibrator with a negative slope
   inverts the ID ordering — "I have never seen this class" becomes "I am certain
   of class 7". The fix was not to clamp the slope but to make a negative slope
   *inadmissible*, so monotonicity is a validated invariant with the failing test
   kept in the tree.
2. **The calibration bridge is the research component, and it earned nothing yet.**
   Measured ECE 0.068–0.108 against a bar of 0.05. The corpus is synthetic, so
   this is a mechanism check, not a calibration result — which is itself the most
   useful thing the report says, because it is the difference between "the code
   runs" and "the numbers mean something".

---

# Part I — What the sources actually say

The starting point (`PromptANaCP`, saved at the workspace root) claims "no
single pre-packaged, unified library integrates all of these pieces" and
proposes a merged implementation. That is correct as a claim about
*engineering packaging*, but the merged sketch gets the *science* wrong in
several places (Part II). This part states what each source actually is, in
the notation this plan will use.

## I.1 AnaCP — Analytic Contrastive Projection (NeurIPS 2025)

Source: `AnaCPtex/neurips_2025.tex` (Momeni, Xiao & Liu, "AnaCP: Toward
Upper-Bound Continual Learning via Analytic Contrastive Projection"); code
suite `AnaCP/`. **Setting**: class-incremental learning (CIL) — tasks of
disjoint classes arrive in sequence, no replay of raw data, no task id at test
time — with a frozen pre-trained model (PTM: DINO-v2, MoCo-v3) as feature
extractor. **Vocabulary**: "analytic" = closed-form, no gradient updates.

**The pipeline (paper §4):**

1. **Input layer.** PTM features $x \in \mathbb{R}^d$ (L2-normalized in code).
2. **Random projection (RP) layer** (ELM, Huang et al. 2011):
   $z = \phi(x R)$, $R \in \mathbb{R}^{d \times D}$ fixed random, $\phi$ = GELU,
   $D = 5000$ default. Non-trainable; adds nonlinear expressiveness.
3. **Contrastive projection (CP) layer** — the paper's contribution. Ridge
   regression from random features $Z$ to **target prototypes** $P$:
   $$W = (Z^\top Z + \lambda I)^{-1} Z^\top T, \qquad H = Z^\top T = M\,N\,P$$
   where $M \in \mathbb{R}^{D \times C}$ holds per-class means of the random
   features, $N = \mathrm{diag}(n_1..n_C)$ the counts, $P \in \mathbb{R}^{C
   \times d}$ the target prototypes. The factorization $H = MNP$ is what makes
   the layer *incremental*: $M$ and $N$ accumulate class-wise; $P$ is
   re-derived from accumulated input-layer means.
4. **Target prototypes = positive alignment + negative repulsion.** With
   input-layer class means $C = (\mu_1.. \mu_C)$ and an incrementally
   maintained **shared** covariance $\Sigma$ (paper eq. 13):
   - whiten: $\hat C = \Sigma^{-1/2} C$;
   - SVD: $\hat C = U S V^\top$;
   - perturb singular values: $\tilde S = S + \alpha\,\mathrm{Diag}\{\delta_i\}$,
     with the signs $\delta_i \in \{-1, 0, 1\}$ chosen by **Lemma 1** so the
     summed pairwise cosine similarity of the shifted means is *guaranteed*
     to decrease for small $\alpha > 0$ (paper sets $\alpha = 1$);
   - de-whiten: $P = U \tilde S V^\top \Sigma^{1/2}$.
   This is the "contrastive" effect — classes pulled to (separated)
   prototypes — achieved with zero gradient steps.
5. **Multi-head.** $H = 3$ independent RP+CP heads sharing one $P$; outputs
   averaged. (Ablation: $H=1$ already beats all baselines.)
6. **Classifier.** Either (a) **NCM** to the target prototypes, or (b) an
   **ELM classifier with pseudo-replay**: sample features from
   $\mathcal{N}(\mu_c, \Sigma)$ (shared covariance; class-specific optional),
   push through RP+CP, fit ridge to one-hot. Replay is *synthetic features*,
   not stored data.

**Facts worth carrying into this project** (paper §5): last-accuracy gains of
+1.03–3.17% over all baselines (relative error reduction 4.8–31.4%); with
DINO-v2 it matches or exceeds *joint fine-tuning* (the CIL upper bound) on
most datasets (≤2% gap elsewhere); the core analytic step costs **5 seconds**
on CIFAR-100 vs 3h47m for a prompt-learning baseline; memory ≈ 106.6M stored
parameters for $C{=}200, d{=}768, D{=}5000, H{=}3$ — dominated by $D \times D$
Gram matrices — ≈ 102 MiB in float8 since nothing is trained. Forgetting
analysis is done in the TIL setting via the accuracy matrix $A[t][i]$
(flat columns = no forgetting). **Stated limitations**: needs a strong PTM;
CIL only (TIL/DIL not covered).

## I.2 AnaNC — Analytic Neural Collapse (AAAI 2026)

Source: `AnaNC.md` (Momeni, Xiao & Liu, "Continual Out-of-Distribution
Detection with Analytic Neural Collapse", AAAI-26; note the paper's own
line: *Copyright © 2026, AAAI. All rights reserved.*); code suite `AnaNC/`.
**Setting**: CIL **+ continual OOD detection** — the model must also flag
inputs from classes it has never seen.

**Neural Collapse (NC) properties** (Papyan, Han & Donoho 2020) it realizes
*analytically*:

- **NC1** — within-class variability collapses: every sample of class $c$ is
  mapped to one target mean $\tilde\mu_c$.
- **NC2** — the target means form a **simplex ETF** (equal norms, equal
  angles): centered/normalized means satisfy
  $\langle \tilde\mu_i, \tilde\mu_j \rangle = 1$ if $i{=}j$, $-\frac{1}{C-1}$
  otherwise; gram matrix $S = I - \frac{1}{C-1}(\mathbf{1}\mathbf{1}^\top - I)$.
- **NC4** — classification reduces to **NCM** (here: cosine similarity on the
  output layer, paper §"Achieving Simplex ETF").

**The pipeline:** same frozen PTM → L2-normalize → RP layer ($D = 5000$,
GELU) → analytic output layer
$$W = (Z^\top Z + \lambda I)^{-1} H, \qquad H = Z^\top Y = R\,N\,\tilde M$$
with $R$ = per-class random-feature means (incremental), $N$ = counts,
$\tilde M$ = **target means**, and $\lambda = 10^2$ (tuned over
$[10^{-2}, 10^6]$).

**The ETF target construction (paper eqs. 9–15) — this is the piece the
sketch got wrong.** The target means are *not* accumulated statistics; they
are the solution of a constrained projection recomputed in closed form at
every task:
$$\min_{\tilde M} \|\tilde M - M\|_F^2 \quad \text{s.t.} \quad \tilde M \tilde M^\top = S.$$
Because $S$ is PSD, write $S = U_S \Lambda U_S^\top$ and parameterize
$\tilde M = U_S \Lambda^{1/2} Q^\top$ with $Q^\top Q = I$ (any such $\tilde M$
satisfies the ETF constraint). Then choose $Q$ by the **orthogonal
Procrustes** problem $\min_{Q^\top Q = I} \|U_S \Lambda^{1/2} Q^\top - M\|_F^2$,
whose closed form (paper Lemma 1) is $Q^* = U V^\top$ from the SVD
$M^\top U_S \Lambda^{1/2} = U \Sigma V^\top$, giving
$\tilde M = U_S \Lambda^{1/2} V U^\top$. Procrustes keeps the reorganization
of feature space minimal, which makes the ridge map learnable.

**Inference and OOD.** Prototypes are the *images* of the class means under
the learned map. Classification = argmax cosine similarity. **The OOD score
is the maximum cosine similarity** (the "ID score"); the paper reports
threshold-independent **AUC and FPR95** and explicitly says threshold
selection is *out of scope* ("depends on the application"). There is **no
energy, no softmax probability, and no $P(\text{ID})$ anywhere in AnaNC.**
Results (DINO, in-dataset OOD, 10-task splits): average AUC 82.66 / FPR95
69.64 vs next-best RanPac 80.44 / 72.88; consistent wins with MOCO too. All
feature-extractor baselines (and AnaNC itself) use **first-session
adaptation** (FSA: adapters fine-tuned on task 1, then frozen).

## I.3 open-jev — the typed System-1 interface

Source: `open-jev/` (Apache-2.0; a from-first-principles, random-weight
reconstruction of TypeSafe AI's "Jev" ideas — the README is explicit that it
reproduces no production results). What is worth adapting is the **type
discipline**, not the model:

- Questions are typed dataclasses: `Noul{text}` (is this true? → one
  probability), `Choice{text, options}` (pick exactly one of runtime-supplied
  unique options; ≥2 required), `Score{text, labels}` (ordinal rubric).
- Answers: `NoulAnswer{key, noul}`, `ChoiceAnswer{key, choice,
  probabilities, confidence}`, `ScoreAnswer{key, score, probabilities,
  confidence}` — all `frozen=True`. The file's own comment: *"These
  dataclasses ARE the type safety… a schema violation is not 'unlikely' — it
  is unrepresentable."*
- `confidence` is **epistemic certainty, explicitly not `max(probabilities)`**
  — the two are separate outputs. An evidential head + `RLCDLoss` (soft-target
  NLL, Brier, consistency, confidence, **ECE** terms) make calibration a
  first-class training objective.

**`laya`** — named in `PromptANaCP` as a second decision framework — is
**not present anywhere in the workspace** (grep hits are substrings of
"display a"/"play a"). Nothing can be reviewed or adapted from it; recorded
as *not used (absent)* in Part VI.

## I.4 What no source provides (the gap this plan must own)

1. **Probabilities.** Neither paper outputs $P(Y \mid X)$ or $P(\text{ID}
   \mid X)$. AnaNC outputs a *score* and evaluates it threshold-free.
   The "calibration bridge" in the sketch is genuinely new work with its own
   proof/validation burden (Part III.4). Normalization is not calibration.
2. **Text/embedding-agnostic packaging.** Both papers are CV benchmark suites
   (CIFAR-100, ImageNet-R, CUB, TinyImageNet, Cars) with DINO/MOCO backbones
   and FSA adapters. Nothing is packaged as an inference library; nothing
   handles text embeddings.
3. **A unified engine.** AnaCP handles CIL accuracy; AnaNC handles CIL + OOD
   geometry. They share the RP/ridge skeleton but have *different* target
   constructions (spread means vs ETF-Procrustes) and *different* heads
   (NCM/ELM vs NCM-cosine). A unification must pick per-surface what it
   uses, not average the two.

---

# Part II — Audit of the proposed `UnifiedAnalyticDecisionEngine`

The sketch in `PromptANaCP` (also pasted in the originating chat) is a useful
*product shape* but an unreliable *method implementation*. Everything it gets
right is architectural; everything it gets wrong is mathematical. Defects,
numbered for the work items of Part IV to track:

| # | Defect | What the sources actually require |
|---|---|---|
| A1 | "AnaCP" is implemented as `W_proj = pinv(R_xx) @ S_B` with `S_b = MᵀM`, then SVD-truncated. This is **not** AnaCP. | AnaCP solves ridge **in the RP space** to *target prototypes* $P = U\tilde S V^\top \Sigma^{1/2}$ (whitened + Lemma-1-perturbed means), $H = MNP$, multi-head. Between-class scatter appears nowhere. |
| A2 | "ETF prototypes" are `normalize(running class sums)` — plain NCM means. | AnaNC's ETF is a *constraint*: $\tilde M \tilde M^\top = S$, solved by **Procrustes** (eqs. 11–15), re-derived per task. Normalized means are not an ETF and never form one. |
| A3 | No random-projection/ELM expansion: raw features are projected linearly. | Both papers expand to $D \approx 5000$ with GELU **before** any analytic layer; the RP layer is where the nonlinearity lives. |
| A4 | `class_sums` accumulate in the **projected** space while `W_proj` is **re-derived every update** — sums taken under different projections are mixed into one prototype. Mathematically inconsistent. | Both papers accumulate statistics in *fixed* spaces (input-layer means; RP-space means under a **fixed** random $R$) and only the ridge solve is re-run. The sketch's "Zero Forgetting!" claim is therefore unsupported. |
| A5 | Energy-based OOD: `-τ·logsumexp`, threshold = 95th-percentile energy, `ood_logit = -θ/τ`, `confidence = sigmoid(-gap)` — invented, unvalidated, and internally inconsistent (two different OOD numbers: `confidence` and `p_ood`). | AnaNC's ID score is **max cosine similarity** and threshold selection is explicitly out of scope. Any threshold/energy policy is *ours to validate*, not theirs. |
| A6 | `probabilities = softmax(cos/τ)` is presented as "strictly sums to 1.0" probability. | Sum-to-1 is normalization, not calibration. open-jev treats calibration as a loss term with ECE; analytic methods must calibrate **post-hoc** and measure ECE. Until calibrated, the numbers are scores. |
| A7 | Implementation bugs: `_expand_classes` renames only when the class list grows (stale names otherwise); `for k in range(max_label)` skips classes outside the current batch (prototype drift); `torch.eye(feature_dim, proj_dim)` is not an identity embedding when rectangular; index↔name mapping shifts when names are reordered. | The reference code maintains an explicit `class_map`/`inverse_map`, and per-class stats keyed by mapped id. |
| A8 | No citations, no licence handling; derived from two all-rights-reserved papers. | Reimplement from the *published equations* (facts and methods are not copyrightable; code and text are), cite both papers, add `ATTRIBUTION.md` rows (Part VI). |
| A9 | The framing "Why This Fulfills Your Original Goal" asserts GDA-on-sphere posteriors and physically grounded confidence as accomplished. | No source establishes either. The calibration bridge is **research**, with acceptance tests, and the docs must say so until they pass (M9-style non-claim enforcement). |

**What survives the audit (adopt as-is):**

- **S1** — streaming sufficient statistics (counts, means, Gram/cross
  matrices) with closed-form solves; this matches papers eqs. 3/8 exactly.
- **S2** — a dynamically growing class vocabulary with explicit id mapping.
- **S3** — the typed answer vocabulary (`Noul`/`Choice`/`Score` questions;
  `choice`/`probabilities`/`confidence` answers) as the product seam.
- **S4** — an explicit **open-world $(K{+}1)$ rejection state** in the answer
  type. The papers motivate it (continual OOD); the mechanism for turning it
  into a number is ours to prove out.
- **S5** — a backbone-agnostic embedding interface (frozen features in,
  decisions out) — which maps directly onto this project's C7 model-spec
  seam.

**Re-submission trace — the sketch arrived verbatim again (2026-10-08).
Every element of its “Unified Implementation” maps to this plan, implemented
*corrected* rather than as written; no element is unaccounted for:**

| sketch element | plan item | code | verdict |
|---|---|---|---|
| `update(features, labels, class_names)` — closed-form, zero backprop, streaming | D2/D3 | `decide_core/{stats,rp,targets}.rs`; property “batch ≡ incremental” (`properties.rs`) | **implemented** — as the papers' ridge-to-target-projections, not the sketch's update |
| `W_proj = pinv(R_xx) @ S_B` with `S_B = MᵀM`, SVD-truncated | — | — | **rejected as A1/A3**: not AnaCP; the RP+GELU expansion + whitened/SVD-perturbed spread-mean targets are implemented instead |
| `prototypes = normalize(class_sums)` called an ETF | D3 | `targets.rs` `etf_procrustes`/`etf_gram`; property `‖M̃M̃ᵀ−S‖_F ≤ 1e-5`, the `features ≥ classes` precondition | **replaced (A2)**: normalized running means are not an ETF and never form one |
| `_expand_classes` rename-only-when-growing, `for k in range(max_label)` skipping classes, rectangular `eye` “identity” | S2/D2 | explicit `class_map` (`lib.rs:238`, audited A7), per-class stats keyed by mapped id, stats keyed by embedding version (A4) | **A7/A4 designed out at the type level** |
| `softmax(logits)` presented as “strictly sums to 1.0” probability | D6, R1 | `calibrate.rs` R1 test (sums to 1 ± 1e-9) | **normalization ≠ calibration** — `probabilities` is emitted only with `calibrated: true`, which the failed D6 never licenses |
| `sigmoid(-energy_gap)` confidence, `quantile(0.95)` threshold, separate K+1 `ood_logit` | D6 candidates, R3/R4 | `Calibrator::p_id` monotone by construction (negative Platt slope inadmissible — the property test that found R3 violated), `p_ood = 1 − p_id` | **re-derived and tested (A5 designed out)** — one estimator, two views; measured ECE 0.068–0.108 against the 0.05 bar, so R2/R5 fail honestly |
| `ChoiceAnswer{choice, probabilities, confidence, p_ood}` | D5 | `unfer_protocol::decide` — `calibrated: bool`, `p_ood: Option<f64>`, `engine_version`, `calibrated: true` unconstructable without a passing calibrator | **implemented and stricter** — open-jev vocabulary (cited); `laya` absent, recorded (Part VI.5) |
| ModernBERT `embed()` hook (frozen backbone, mean pooling) | D7 | `backbone.rs` — `embed:hf/…` / `embed:openai-compatible:<url>` via C7, embedding-model identity part of engine state | **implemented** (generic over the C7 seam, not one pinned model); *2026-10-08*: the choice between a text encoder (e.g. ModernBERT) and a multimodal embedding model (e.g. EmbeddingGemma) is expressed host-side by the spec string plus `#truncate` / `#instruction` / dual-stream concat, all folded into the identity — recorded above under D7 |
| “Zero Forgetting!” on class arrival | D11/R5 | forgetting matrix published; `ananc` forgets 1.0 vs `anacp` 0.5; drift measured 0.067 vs ε = 0.05 | **claim replaced by measurement** — declared, never asserted |
| “Why This Fulfils Your Original Goal” (GDA-on-sphere posteriors, physically grounded confidence) | A9/M9 | `DECIDE_SPEC.md` §1–2; `check-non-claims` gate | **not licensed** — D6 failed; every document says so |
| torch/Python in-repo | III.6 | — | **deliberate non-goal**: Rust `decide_core` in unfer, no torch dependency in the seven repos |

---

# Part III — Adaptation to this project

## III.1 Where a System-1 layer belongs — and where it must never be

This project's identity is *certified* computation: bands and certificates in
`unfer/fock_sirk`, golden gates S23/S24, `verify-invariants`, nanoda
re-verification, the M9 non-claim enforcement. A statistical decision layer
has exactly one legitimate role in such a project: **advice, labeled as
advice**. The trust model, binding for every item below:

1. **Observe-kind only (S21).** Every engine interaction — updates,
   predictions, calibrations — is an `observe`-kind effect. No `Mutate`
   grants; no engine output can approve, merge, gate, sign, or certify
   anything.
2. **Never in a trust path.** Non-consumers, forever: `dynamic-arctic`
   signing/consensus; the golden release gate; Lean/nanoda/`verify-invariants`
   checks; certificate reading (S30); the G4 evidence gate; anything in the
   M9 non-claim list. An engine failure can at worst *lose advice*.
3. **Vocabulary discipline.** Until a calibration report (D6) exists and is
   pinned, engine outputs are **scores**; the `probabilities` field is
   emitted only in calibrated mode with `calibrated: true`. Docs and UI may
   not call uncalibrated numbers probabilities. (This is the same discipline
   as the mass-gap boundary sentences: the honest boundary is part of the
   result.)
4. **Every suggestion is overridable and audited.** Consumers render engine
   output as *suggestions with scores*; a human or worker always chooses;
   each suggestion event goes through the S23 audit ring and
   `sanitize_sensitive` (G1 rule).

## III.2 Architecture

Placement principle (the project's improvement principle): this is an
improvement of the existing agent/editor seams, not a new platform.

```
frozen backbone (C7 seam: hf/… or openai-compatible:<url>; never vendored)
        │  embeddings (d ≈ 384–768, L2-normalized at the seam)
        ▼
unfer/decide_core  (new Rust crate in the unfer workspace, Apache-2.0)
  • state: class_map, counts n_c, input means μ_c, shared Σ (d×d),
    per-head RP matrices R_h (fixed, seeded), per-head Gram A_h (D×D),
    per-head RP means r_{h,c}, ridge θ_h, target means M̃ (ETF), prototypes
  • modes: anacp (spread-mean targets + NCM/ELM head) and
           ananc (ETF targets + cosine NCM + OOD score)
  • calibration module (Part III.4), versioned + pinned like other fixtures
        │  NDJSON ops (observe-kind): decide_update / decide_answer /
        │  decide_state / decide_recalibrate
        ▼
unfer_protocol (shared serde types, C4 discipline)
  Question = Noul | Choice | Score
  Answer   = NoulAnswer | ChoiceAnswer | ScoreAnswer
             (+ open-world: p_ood, calibrated: bool, engine_version)
        ▼
consumers (Part III.5): board triage, preset routing, summarizer routing,
editor ranking, nudge priority, N2 benchmark family
```

**Why Rust in `unfer`, not Python.** The engine is small dense linear
algebra over frozen features; the project's Rust stack already carries the
habits this engine needs — property tests (M5/Kani discipline), pinned
fixtures, NDJSON ops, and certified-linear-algebra experience
(`fock_sirk`) for a later certified mode (D9). Python stays where the
backbones live (behind the C7 HTTP seam); no torch dependency enters the
seven repos. The reference `AnaCP/`/`AnaNC/` suites remain untouched,
unvendored evaluation material.

**Sizing note (from the papers).** The $D \times D$ Gram matrices dominate
memory ($D{=}5000 \Rightarrow$ 25M floats per head). Project label sets are
small (tens of classes), so defaults may be $D \in [512, 2048]$, $H = 1..3$,
configurable; the plan requires the D-ablation of AnaCP Table 2 (rows 13–16)
to be re-run on project data (D8) before any default is blessed.

> **Addendum 2026-10-11 — the D-ablation has been run.** Harness:
> `unfer/decide_core/src/ablation.rs` + `--bin decide_ablation`, pinned as
> `tests/fixtures/decide_d_ablation.json` and byte-gated in CI, sweeping
> $D \in \{4..512\}$ through the pinned report's own protocol (the $D=256$
> cells reproduce that report exactly, asserted by test). **The result is
> asymmetric and is recorded as such in `DECIDE_SPEC.md` §4:** for `anacp`,
> $D{=}512$ is measured-best (0.467 → 0.900 across the grid), so the default is
> now measured rather than merely sensible; for `ananc` the same $D$ is the
> **worst** cell (best: $D{=}16$, 0.500 vs 0.333), and its catastrophic
> forgetting is 1.0 at *every* $D$ — so "increase $D$" is ruled out as the D11
> remedy. No $D$ rescues calibration (R5 fails everywhere; R2 only at scattered
> cells, never at the default), so `calibrated: true` stays unlicensed. The
> default is deliberately **not** re-tuned on synthetic data; re-running the
> same harness on real project embeddings is what would license a change. The
> large-$D$ divergence is explained by measurement, not speculation: the sweep
> reads the crate's own whitening instrument and shows `anacp` flooring
> $D-89$ directions once $D \ge 128$ (rank cap $n_\text{train}=90$), while
> `ananc` floors zero because the ETF construction does no whitening.

## III.3 The two method modes, adapted

**Mode `anacp`** (accuracy-first surfaces, e.g. routing/triage):
RP expansion → CP layer with target prototypes from spread means
(whiten → SVD → Lemma-1 perturbation → de-whiten) → NCM head first (paper row
3), optional ELM head with Gaussian pseudo-replay (shared covariance) later.
Lemma 1's sign rule is transcribed from the *statement* of the lemma (facts
are free); its proof is credited.

**Mode `ananc`** (open-world surfaces, e.g. board triage, editor ranking):
RP expansion → ridge to ETF targets $\tilde M$ (Procrustes, recomputed when
the class set changes) → cosine NCM + **ID score = max cosine similarity**
(AnaNC's actual OOD mechanism — not energy). This is the mode the open-world
$(K{+}1)$ state of S4 builds on.

Both modes share: incremental Gram/cross updates (batch ≡ incremental is a
property test, D4), fixed seeded RP matrices (reproducibility), explicit
`class_map`, and per-class statistics accumulated **only in fixed spaces**
(audit defect A4 is designed out at the type level: RP-space statistics are
keyed by RP-seed, input-space statistics by embedding-model version).

## III.4 The calibration bridge — a research component, specified honestly

Goal: turn scores into (a) $P(Y \mid X)$ over the $K$ known classes summing to
1.0, and (b) $P(\text{ID} \mid X) \in [0,1]$ with the $(K{+}1)$-st state as
reject. **No source provides this** (Part I.4). Requirements (each is a test):

- **R1 Normalization.** `probabilities` sums to 1 ± 1e-9 (softmax over
  calibrated logits).
- **R2 Calibration.** ECE ≤ 0.05 on held-out project data, measured per
  surface and per class-count regime; report alongside every pinned model
  (the ECE framing is adapted from open-jev's `RLCDLoss` — cited).
- **R3 Monotonicity.** $P(\text{ID})$ is monotonically non-decreasing in the
  ID score (max cosine). Any bridge that violates this is rejected.
- **R4 Open-world coherence.** $P(\text{ID}) + P(\text{OOD}) = 1$, and
  `p_ood` and `confidence` are two views of one quantity, not two estimators
  (sketch defect A5 is designed out).
- **R5 Class-arrival stability.** Adding a class may move probabilities only
  within a bounded drift ε (measured), or the engine must declare
  `recalibrated: true` and consumers must show it. (Zero-forgetting for the
  *calibrator* is impossible — new classes change the simplex; the honest
  deliverable is *declared* drift, like the certificate bands.)

Candidate mechanisms (evaluate all; pick on metrics, D6): temperature scaling
on cosine logits; isotonic/Platt on the ID score; a $(K{+}1)$-logit model
fit with the IDF-style rejection bias calibrated on held-out OOD; Gaussian
discriminant analysis on the sphere *if and only if* its assumptions are
tested on project data. Until R1–R5 pass on pinned data, consumers render
scores (III.1.3).

## III.5 Consumers — each improves an existing feature

| # | Consumer | Feature improved | Behavior |
|---|---|---|---|
| D-C1 | **Board triage assist** | G1 shared context board | Suggests `kind` (`FACT`/`FAIL`/`CLAIM`) for a drafted entry and flags likely duplicates; worker confirms or edits; suggestion + resolution audited (S23). Mode `ananc`. |
| D-C2 | **Preset routing** | C5 worker role presets | Suggests which preset (`reader`, `prover-runner`, `reviewer`, …) a queued task fits; the dispatcher (or human) decides. |
| D-C3 | **Summarizer channel routing** | C6 summarizer pass | Chooses detail level per target surface (editor overlay vs `test/` draft vs console) from `Score` answers; raw output still in the session log (S46). |
| D-C4 | **Editor ranking** | velysterm references panel, cite popups, `mathed_mini` completion list | Advisory ordering only — the displayed set is unchanged, only order changes; a wrong guess costs a scroll, never a wrong citation. |
| D-C5 | **Nudge priority** | G9 budget nudges | Orders which queued nudges surface first at a checkpoint. |
| D-C6 | **N2 benchmark family** | N2 reproduction-benchmark harness | A new task family *continual decision benchmark* over project-native labels (board kinds, doc routing, editor intents): reports `A_last`, `A_avg`, AUC, FPR95, ECE, and AnaCP's TIL accuracy-matrix anti-forgetting protocol. This is what makes "no catastrophic forgetting" a *measured* claim for this project. |

## III.6 Non-goals

- No new model training anywhere; the backbone is frozen and external (C7).
- No Python in the seven repos; no torch dependency.
- No replacement of any existing gate, checker, or approval path.
- No claim of calibrated probabilities before D6 passes (and no claim of
  "zero forgetting" before D-C6's matrix is published).
- No token/market features (standing exclusion, main plan §9.1).

---

# Part IV — Work items

Sizes: S ≤ 1 day, M = 2–5 days, L = 1–2 weeks. Owner lanes: eng / docs /
specialist (Part V only). Stage order is the dependency order.

## Stage A — spec and core

- **D1 (S, docs+eng)** `unfer/docs/DECIDE_SPEC.md` — code→math map for
  `decide_core`: every field of the engine state mapped to a paper symbol
  (this plan Part I notation), both modes, the two target constructions, and
  the R1–R5 calibration contract. *Accept*: a reader can reconstruct the
  engine from the spec without the papers; the papers are cited, never
  paraphrased into uncredited prose.
- **D2 (M, eng)** `unfer/decide_core` crate, mode `anacp`: streaming stats
  (counts, input means, shared Σ), RP layer (seeded, GELU), CP ridge
  (Cholesky solve of $(A + \lambda I)\theta = B$ with $B = \sum_c n_c\,
  r_c \tilde\mu_c^\top$), spread-mean targets (whiten/SVD/perturb/de-whiten
  with the Lemma-1 sign rule), NCM head. *Accept*: batch ≡ incremental on
  random streams (D4 property); deterministic under fixed seed; no
  `unsafe`, no `unwrap` on the op path (unfer S30 discipline).
- **D3 (M, eng)** mode `ananc`: ETF targets via eigen-decomposition of $S$
  and Procrustes (SVD of $M^\top U_S \Lambda^{1/2}$), cosine NCM, ID score =
  max cosine, $(K{+}1)$ answer state. *Accept*: $\|\tilde M\tilde M^\top -
  S\|_F \le 10^{-5}$ after every class-set change (property test); ID score
  monotonicity (R3) holds by construction on the raw score.
- **D4 (S, eng)** property-test suite (proptest + Kani on the pure kernels):
  batch≡incremental ridge; order-invariance of `(class_map, stats)` under
  shuffled arrivals; ETF constraint; simplex normalization; monotonicity.
  *Accept*: runs in CI under `cargo test`; fuzz-style generators over class
  counts 2..64 (the D.1 discipline from the main plan applies if any
  libFuzzer target is added: workspace-excluded + a CI property-test twin).

## Stage B — calibration and evidence

- **D5 (M, eng)** typed protocol in `unfer_protocol`: `Question`/`Answer`
  serde types in the open-jev vocabulary (adapted shapes, cited — Part VI),
  NDJSON ops `decide_update`/`decide_answer`/`decide_state`, all
  observe-kind; events into the S23 ring. *Accept*: round-trip tests;
  schema versioned like C4's message type; `Mutate`-kind is unrepresentable.
- **D6 (L, eng)** calibration bridge (Part III.4): implement the candidate
  mechanisms, fit on project-native labels, produce a **pinned calibration
  report** (ECE, AUC, FPR95, R1–R5 verdicts) as an NDJSON fixture in the
  unfer fixture style. *Accept*: R1–R5 pass on the pinned split or the
  report honestly records which failed; `calibrated: true` is emitted only
  for a passing (engine_version, calibrator_version) pair.
- **D7 (M, eng)** backbone seam through C7 model specs (`hf/…`,
  `openai-compatible:<url>`): one `embed(texts) -> vectors` contract,
  embedding-model version recorded in engine state (defect A4's second
  half). *Accept*: a smoke test against a local endpoint (C7's own
  acceptance); engine refuses `decide_update` across embedding versions
  unless an explicit migration op is used.
  **Recorded 2026-10-08 — backbone *choice* added on top of D7.** The seam was
  already model-agnostic, so “choose ModernBERT or EmbeddingGemma(-2)” is a
  **host-side** choice expressed as two spec strings and two widths, not an
  engine mode: the engine is vector-in and never sees text, so a backbone that
  maps every modality into one shared space needs no engine change to be
  multimodal. What was genuinely missing is that three host-side options each
  silently change the input space and therefore must be part of *identity*:
  `#truncate=N` (Matryoshka / MRL prefix truncation, now applied **and
  re-normalized** at the seam — a truncated vector is no longer unit length, so
  normalize-after-truncate, never before), `#instruction=TEXT` (prefix applied
  by the host before embedding; prefixed and bare vectors are not comparable),
  and dual-stream concat (`backbone::concat_streams`, pairwise, width `2d`,
  which is what `embedding_dim` must be set to). `backbone::version_string`
  folds all three into `embedding_version`, so A4 refuses a cross-option update
  exactly as it refuses a cross-model one, and `feed` refuses a `truncate`
  claim that disagrees with `embedding_dim`. Code: `decide_core/src/backbone.rs`.
  **No latency, quality, parameter-count or benchmark figure for any model is
  claimed anywhere** — the model ids in the examples are illustrative, the seam
  stays generic over C7 rather than pinned to one model, and which model runs is
  the host's decision.

## Stage C — consumers

- **D8 (M, eng)** D-C1 board triage + D-C2 preset routing behind the
  existing `unfer_agent` ops. *Accept*: suggestions are advisory in every
  code path (integration test: a wrong suggestion cannot block a merge);
  every suggestion audited; measured accuracy on a labeled sample recorded
  in the D-C6 family.
- **D9 (S, eng)** D-C3 summarizer routing + D-C5 nudge priority. *Accept*:
  C6's acceptance criterion ("overlay shows only summarized text; full trace
  retrievable") unchanged; routing failures degrade to default ordering.
- **D10 (M, eng)** D-C4 velysterm ranking (references panel, cite popups,
  `mathed_mini` completions). *Accept*: rendered item *sets* provably
  unchanged (golden UI test), only order differs; ranking toggle in config.
  **Recorded 2026-10-08 — partial, and this was mis-recorded as DONE in the
  execution summary above.** What exists: `decide_core::consumers::ranked()`
  (`unfer/decide_core/src/consumers.rs`, D-C4 section) — ordering only, the
  returned set identical by construction — with its tests, committed with
  D5 in `28468aa`; and the engine hosted in velysterm's `unfer_agent` (D5's
  observe-kind wire). What does not exist, verified by sweep: no
  `ranked`/decide reference in any velysterm crate (the references panel
  `crates/mathed/src/cite_refs.rs` and `mathed_core`'s completion list keep
  their original order), no ranking toggle in config, no golden UI test.  Closing it needs the design decision this file deferred to the D6 gate — *local
  engine state in the editor vs the unfer service* (the service path
  implies a ranking op the 4-op protocol does not carry; the local path
  implies an embedding source the editor does not have) — and D6 came back
  negative, so the decision is still owner-shaped. Until then this row stays
  PARTIAL; consumers remain library-side advice.
  **Recorded 2026-10-08 — the decision is made and the velysterm half exists.**
  *The decision:* D-C4 runs **locally** in velysterm against a small engine
  state, not against the unfer service, for three reasons. (1) The embedding
  source is needed either way: `decide_update` *receives* vectors, it does not
  produce them, so the service path would not remove the editor's need for
  features — it would add a round trip to an ordering redrawn on every caret
  move and buy no correctness. (2) The service path is not expressible on D5's
  committed four-op wire: `decide_answer`'s `Choice` scores are
  max-over-classes, which `ranked` itself documents as near-constant, so
  service-side ranking would need a fifth, rank-shaped op — a protocol change
  for a UI convenience. (3) Neither path sits in a trust path, so "single audit
  path" confers no safety the local path lacks.
  *What exists now:* `velysterm/crates/mathed_core/src/rank.rs` — a frozen,
  deterministic, **model-free** lexical featureizer (hashed word tokens and
  character trigrams, L2-normalized at the seam, named
  `local:mathed-rank/lexical-v1` through the ordinary `embed:` identity so its
  statistics can never be read as a real embedding model's) feeding a local
  `DecideCore`; `rank_rows` returns a permutation **or the identity**, and
  re-validates the permutation before any row is moved. Wired into
  `mathed/src/cite_refs.rs` (cite popups + references panel) and
  `mathed_mini/src/references_panel.rs`. The toggle is the config variable
  `MATHED_RANK`, **off unless opted in**, so a deployment that says nothing
  keeps byte-for-byte the order it had. Golden tests in both frontends assert
  D10's criterion directly — same rendered set, different order — plus that
  default-off equals the pre-D10 build.
  *What is honestly absent:* `mathed_core::completion` has **no list**;
  `completion_at` yields exactly one candidate (U2 is a unique-prefix
  expansion), so the "`mathed_mini` completion list" row of D-C4 has nothing
  to reorder. That is recorded rather than a list invented to satisfy the row.
  **No accuracy, latency or quality figure for this ranking is claimed** — the
  featureizer is lexical, not semantic, and the only thing asserted about the
  result is its shape: a permutation of the input.

## Stage D — benchmark and hardening

- **D11 (M, eng+docs)** D-C6 N2 task family `continual decision benchmark`:
  project-native labeled sets (board kinds, doc routing, editor intents),
  the papers' metrics, and the TIL accuracy-matrix anti-forgetting protocol
  (AnaCP §4.3). *Accept*: the harness reports `A_last`/`A_avg`/AUC/FPR95/ECE
  and a forgetting matrix; runs at 1 and 4 workers under N2's coverage rules
  (its `pass_rate()`/`Report::comparable` guards stay as they are).
- **D12 (S, eng)** drift monitor: embedding-version and class-arrival drift
  are measured on every `decide_state`, and a drift beyond ε forces
  `recalibrated: true` (R5). *Accept*: unit tests with synthetic drift.
- **D13 (S, eng, conditional)** certified mode: interval-enclosed solves for
  the ridge systems in the `fock_sirk`/`SirkFinitePrecision` style, so
  decision scores can carry bands like every other number this project
  emits. Only after D6. *Accept*: enclosure contains the f64 answer; width
  reported with the answer.

## Stage E — docs and attribution

- **D14 (S, docs)** `test/software/analytic-decision.md` (GitBook page +
  SUMMARY entry): pedagogical walkthrough — what closed-form continual
  learning is, what the ETF geometry is, *why probabilities are the hard
  part*, and the trust boundary (III.1) stated like the certification page
  states its own. *Accept*: `check_site.py` green (via `nix-shell -p
  python3`); boundary sentences verbatim.
- **D15 (S, docs)** `ATTRIBUTION.md` rows (Part VI) + a "considered and not
  used" row for `laya` (absent) and for the sketch's energy mechanism
  (superseded by AnaNC's max-cosine). *Accept*: the ledger's grep checks
  still resolve.

---

# Part V — Lean-shaped work orders

Per the standing rule, no Lean code is written here. The following are filed
as work-order rows in `timepiece/CONSOLIDATED_PLAN.md` (2026-10-05 entry) for
the LLM-Lean4-specialist; each is optional for the engineering track — the
engine ships as *measured*, uncertified advice without them (D13 is the
engineering-side analogue):

- **L1 — ETF/Procrustes correctness.** For PSD $S = U_S \Lambda U_S^\top$
  and $Q^\top Q = I$: $\tilde M = U_S \Lambda^{1/2} Q^\top \Rightarrow
  \tilde M \tilde M^\top = S$; and the SVD solution $Q^* = UV^\top$ (from
  $M^\top U_S \Lambda^{1/2} = U\Sigma V^\top$) minimizes
  $\|U_S \Lambda^{1/2} Q^\top - M\|_F$ over semi-orthogonal $Q$.
- **L2 — Incremental ridge equivalence.** Accumulating $G_t = G_{t-1} +
  X_t^\top X_t$, $H_t = H_{t-1} + X_t^\top Y_t$ (zero-padded columns for new
  classes) yields the same $W = (G+\lambda I)^{-1}H$ as the batch solve, for
  $\lambda > 0$.
- **L3 — Calibration sanity.** Softmax normalization (sums to 1);
  monotonicity of $\sigma(-(E-\theta)/s)$ in $E$ and of the argmax-preserving
  temperature scaling.
- **L4 — (Conditional, mirrors D13)** certified enclosures for the ridge
  solves, in the `ChapterSirkFinitePrecision` enclosure style.

---

# Part VI — Licensing and copyright compliance

Binding for every item; mirrors `ATTRIBUTION.md` policy ("adapt ideas, do not
copy source; never re-licence; rows added in the same commit").

1. **AnaCP** (Momeni, Xiao & Liu, NeurIPS 2025; `AnaCPtex/`, `AnaCP/`).
   **No `LICENSE` file exists in either tree** (checked 2026-10-05) —
   default copyright applies to the code; the paper text is likewise
   all-rights-reserved. **Policy: no code, no text.** Reimplement from the
   published *equations* (methods and facts are not protected expression),
   paraphrase in our own words, cite by author–year–venue everywhere the
   spread-mean targets, Lemma-1 perturbation, or pseudo-replay appear.
   Row: *"used (ideas, equations) — reimplemented, not copied"*.
2. **AnaNC** (same authors, AAAI-2026; `AnaNC.md`, `AnaNC/`). The paper
   carries an explicit *Copyright © 2026, AAAI. All rights reserved* notice;
   the code has **no `LICENSE`**. Same policy as AnaCP: Procrustes-onto-ETF
   construction re-derived from eqs. 9–15 with citation; no code, no text.
3. **open-jev** (`open-jev/`, Apache-2.0). Code reuse is *legally* permitted
   with the Apache notice + change record; project policy is still adapt-not-
   copy. The **type shapes** (`Noul`/`Choice`/`Score` and their answers) are
   reimplemented as serde types with an attribution row; the ECE-as-loss
   framing is cited. **Do not carry TypeSafe AI's "Jev" name, badges, or
   branding** (third-party trademark/endorsement risk; open-jev's own README
   disclaims production equivalence).
4. **`PromptANaCP` sketch.** In-house AI-assisted draft; superseded by this
   plan. Its energy-calibration mechanism is recorded as *considered and not
   used* (defects A5/A6).
5. **`laya`.** Not present in the workspace; nothing reviewed, nothing used.
   Row in "considered and not used": *absent*.
6. **Repositories.** All landing repos are permissive (unfer/australVM/
   timepiece Apache-2.0; velysterm MIT/Apache dual), so no relicensing risk;
   nothing lands in `emthin` (GPL-3.0) or `dynamic-arctic` (MIT) at all.

---

# Part VII — Risks, open questions, sequencing

**Risks.**

- **R-1 Calibration rot.** Calibrators expire when classes or the embedding
  model change (R5). Mitigation: versioned calibrators, forced
  `recalibrated: true`, D12 drift monitor.
- **R-2 Advice treated as authority.** The consumer surfaces must render
  scores as suggestions. Mitigation: III.1 rules enforced at the type level
  (observe-kind only) and integration tests that wrong advice cannot block.
- **R-3 Memory/latency of $D \times D$ Grams.** Mitigation: default $D$
  chosen by the re-run ablation (D2 note), float32 storage with f64 solves,
  config cap.
- **R-4 Backbone dependence.** The papers show performance tracks PTM
  strength (AnaCP's stated limitation). Mitigation: embedding version is
  part of engine identity (D7); benchmarks report per-backbone.
- **R-5 Overclaiming.** Mitigation: the M9-style vocabulary rule (III.1.3)
  and the pinned calibration report (D6) are the only routes to the words
  "probability" and "confidence".

**Open questions (decide at the D6 gate).**

- One engine with two modes, or two engines sharing `decide_core` stats?
  (Plan default: one crate, two modes.)
- Is $(K{+}1)$-softmax-with-calibrated-bias enough, or does the open-world
  state need an explicit density model? (Decide on R2/R3 metrics.)
- Does D-C4's ranking run locally in velysterm against a small engine
  state, or against the unfer service? (Latency vs single-audit-path.)
  **Decided 2026-10-08: locally** — the embedding source is required either
  way, the service path needs a fifth op D5 does not carry, and neither path
  is in a trust path. Full reasoning and what was built: the D10 record above.

**Sequencing.** A (D1→D4) → B (D5→D7) → C (D8→D10) → D (D11→D13) → E
(D14→D15). Gates: Stage B → C gate = pinned calibration report exists
(passing or honestly failing); Stage C → D gate = "advice cannot block"
integration tests green; Stage D gate = benchmark family runs under N2
coverage rules; D13 gated on D6; Part V rows are independent and optional.

---

# Part VIII — Relation to the existing plans

This plan is an *improvement* program first (the main plan's principle):

- improves **C4** (the shared message schema becomes the Question/Answer
  schema seam), **C5** (D-C2), **C6** (D-C3 closes C6's UI half), **C7**
  (the embedding seam is a C7 model-spec consumer), **G1/G9** (D-C1, D-C5),
  **N2** (D-C6 adds its first ML task family);
- respects **X-series** hygiene (NDJSON evidence, pinned fixtures, status
  docs) and **M9** non-claim enforcement (III.1.3);
- defers to **Part V** of this file and the `CONSOLIDATED_PLAN.md` work
  orders for anything theorem-shaped.

Nothing here touches Lean, certificates, gates, or signing; the engine is
the first component this project ships whose output is *deliberately not
trusted*, and every document it appears in says so.

*[Literature reviewed 2026-10-05 from `AnaCPtex/neurips_2025.tex`,
`AnaCP/`, `AnaNC.md`, `AnaNC/`, `open-jev/`, and the `PromptANaCP` sketch.
Equations are cited to the papers' numbering; all prose is ours.]*
