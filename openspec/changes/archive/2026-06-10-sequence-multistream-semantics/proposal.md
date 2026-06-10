# Proposal: Sequence-header multistream semantics

## Why

The validator's §6.4 sequence-header semantics row (`AV2-6.4-SEQUENCE-HEADER-SEMANTICS`)
is the first open gap in the roadmap's dependency order, and its matrix note claiming the
CVS-scoped checks are "blocked on CLK frame-header activation" is stale: CLK-driven exact
§7.3.6 CVS boundaries (`CvsTracker` / `start_cvs_for_xlayer`) and per-frame §5.18.2
sequence activation with frame-confirmed tracking already landed in
`crates/splot-validate/src/context.rs`, and every required parsed field already exists in
`splot-core` (`seq_max_mlayer_count`, `monotonic_output_order_flag`,
`mlayer_dependency_map`, the per-OBU layer-id triple, and the full §5.6 MSDO model).
Three of the four diagnostics below are already named in the
`docs/VALIDATOR-ROADMAP.md` planned-diagnostics backlog; this change burns down that
recorded debt while it is cheap.

## What Changes

Four new error-severity diagnostics in `splot-validate`, each grounded in a quoted
normative sentence from the committed spec mirror:

- `sequence-state/distinct-mlayer-count-exceeds-seq-max` (§ 6.4.1,
  `docs/spec/av2/1.0.0/06-syntax-structures-semantics.md` lines 442–452): "the number of
  distinct values of obu_mlayer_id present in the coded video sequence associated with
  this sequence header is less than or equal to SeqMaxMlayerCnt"; the NOTE clarifies the
  counting applies to **all** OBUs, even non-layer-specific ones.
- `frame-header/switch-or-ras-mlayer-dependency-not-self-contained` (§ 6.4.1, mirror
  lines 615–617): for `OBU_SWITCH` / `OBU_RAS_FRAME`, "for any embedded layer ID m not
  equal to obu_mlayer_id, MLayerDependencyMap[obu_mlayer_id][m] shall be equal to 0".
- `hls/multiple-active-sequence-headers` (§ 7.3.6,
  `docs/spec/av2/1.0.0/07-decoding-process.md` lines 613–616): "Within each extended
  layer, only one sequence header shall remain active for the duration of a coded video
  sequence". The roadmap backlog filed this under §7.3.8 with a "warning until CLK
  parsing exists" hedge; the normative wording lives in §7.3.6 and CLK-driven CVS
  boundaries have landed, so this ships as `error` and the roadmap row is corrected.
  **Reviewer sign-off requested on the severity escalation.**
- `sequence-state/monotonic-output-order-mismatch` (§ 6.4.1, mirror lines 323–325): "in
  a coded multistream video sequence, all extended layers shall be associated with the
  same value of monotonic_output_order_flag". Scoped by a new minimal §7.3.2
  coded-multistream-video-sequence (CMVS) begin/end tracker
  (`docs/spec/av2/1.0.0/07-decoding-process.md` lines 323–344) as a sound
  under-approximation: the check only fires when the tracker is definitively inside a
  CMVS.

Supporting infrastructure (state only, no new parsing):

- a stateful §5.6 MSDO observer in `ValidatorContext` (today the validator has exactly
  one `Msdo` touch);
- the minimal §7.3.2 CMVS begin/end tracker (no `cmvs/*` diagnostics in this change —
  the tracker exists to scope the monotonic check and to seed the future §7.3.2/§7.3.3+
  ordering milestone);
- per-CVS distinct-`obu_mlayer_id` tracking keyed by extended layer.

Process corrections in the same change:

- fix the stale `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` matrix note;
- move the three landed backlog rows out of the `docs/VALIDATOR-ROADMAP.md` backlog
  table and correct its §7.3.8 → §7.3.6 citation;
- register the four rule ids in `docs/VALIDATOR-DIAGNOSTICS.md` (CI
  `check-diagnostic-registry` enforces exact set equality);
- regenerate `docs/FEATURE-STATUS.md` / `docs/SPEC-COVERAGE.md` and re-record the audit
  ledger.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `validator`: four ADDED requirements — distinct-mlayer-count enforcement,
  SWITCH/RAS dependency-map self-containment, single-active-sequence-header per
  extended layer per CVS, and cross-xlayer `monotonic_output_order_flag` agreement
  inside a CMVS.

## Impact

- `crates/splot-validate/src/context.rs` (new state: CMVS tracker, MSDO observer,
  per-CVS mlayer-id sets; new checks in the frame-header core path and the sequence
  activation path) and `crates/splot-validate/src/validator.rs` (tests).
- No `splot-core` parsing changes; no public API changes beyond new diagnostics.
- Matrix rows: `AV2-6.4-SEQUENCE-HEADER-SEMANTICS`, `AV2-7.3.8-HLS-AVAILABILITY`,
  `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT`, `AV2-7.3.2-CMVS-BOUNDARIES` (types/validate
  `todo` → `partial` for the minimal tracker).
- Docs: `VALIDATOR-DIAGNOSTICS.md`, `VALIDATOR-ROADMAP.md`, generated
  `FEATURE-STATUS.md` / `SPEC-COVERAGE.md`, audit ledger.

## Non-goals

- The §6.4.1 operating-point consistency and same-output-time constraints (blocked on
  reference-frame/OrderHint state and decoder-model output timing).
- §7.3.6 OrderHint output-order monotonicity when `monotonic_output_order_flag == 0`
  (blocked on non-intra frame-header parsing of the §5.18.2 output-control block).
- `cmvs/*` boundary diagnostics, §7.3.3–§7.3.5 frame-unit segmentation, and §7.3.9
  long-term-reference availability (the named runner-up milestone; the CMVS tracker
  built here seeds it).
- §6.4.13 buffer-delay sum constancy: mechanically implementable, but the normative
  sentence says "video sequence" (not "coded video sequence") and omits a conformance
  formula, leaving the scope ambiguous. **Maintainer question** — per `AGENTS.md` §10
  this needs a human interpretation decision before any check lands.
- §6.4.11 "no value written into UserQm is equal to 0": structurally unreachable — the
  §5.4.11 parse process retains the previous non-zero value when `quant2 == 0`
  (coef-repeat path), so no diagnostic is needed.
- Encoder, bitstream writer, AVM differential harness, tile-group payload, entropy
  coding, decoder (roadmap fence).

## Feature IDs

- `AV2-6.4-SEQUENCE-HEADER-SEMANTICS` (primary)
- `AV2-7.3.8-HLS-AVAILABILITY` (`hls/multiple-active-sequence-headers`)
- `AV2-7.3.6-CODED-EXTENDED-LAYER-UNIT` (single-active rule citation home)
- `AV2-7.3.2-CMVS-BOUNDARIES` (minimal tracker, `validate` stays honest at `partial`)

## Acceptance criteria

- Each diagnostic ships with the spec citation, positive/negative/boundary tests
  (including CVS/CMVS boundary transitions), and registry entries; matrix stages advance
  only with proof.
- `external HLS` suppression (`ExternalHlsMode::Provided`) and frame-confirmed
  activation gating (`frame_confirmed_xlayers`) prevent false positives for
  `hls/multiple-active-sequence-headers`.
- `cargo xtask ci`, `check-feature-status`, and `check-diagnostic-registry` pass.
