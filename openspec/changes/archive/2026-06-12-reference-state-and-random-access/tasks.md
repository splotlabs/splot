# Tasks: reference-slot and random-access conformance

## 1. Bookkeeping

- [x] 1.1 Confirm/create matrix rows; `openspec_change` set; read the
  mirror sections verbatim (§ 7.3.9, § 7.4.2/.4/.5, § 7.3.8.9,
  § 6.17.6.2, § 6.8.9, the § 6.17.2 reference clauses); inventory what
  PRs #54/#59/#62/#63 already landed to avoid duplicates.
  - Created `AV2-7.4-RANDOM-ACCESS` umbrella; advanced
    `AV2-7.3.9-LONG-TERM-REFERENCE-AVAILABILITY` todo→partial; updated
    `AV2-5.18.6-QUANTIZATION`, `AV2-5.13-QUANTIZATION-MATRIX`,
    `AV2-5.8.8-LCR-EMBEDDED-LAYER-INFO`,
    `AV2-6.17.2-FRAME-HEADER-INFO-SEMANTICS`,
    `AV2-7.23-REFERENCE-FRAME-UPDATE`.

## 2. Implementation

- [x] 2.1 § 7.3.9.1 long-term availability + the RAP-CELU rule.
  - IMPLEMENTED: the per-slot RefLongTermId model + the § 6.17.2 RAS
    `long_term_id_in_use` header-observable form
    (`frame-header/ras-ref-long-term-id-not-in-use`).
  - RESIDUAL (named): the § 7.3.9.1 general availability + the RAP-CELU
    CLK-then-OLK first-frame-units ordering need a long-term RAP-replay
    key not yet modeled.
- [x] 2.2 § 7.4.2/.4/.5 header-observable rules.
  - IMPLEMENTED: § 7.4.5 RAS reference restriction (via the § 6.17.2
    rule above).
  - RESIDUAL (named): § 7.4.2 preconditions + § 7.4.4 OLK
    ref_long_term_id iff-conditions need sequential-decode
    buffer-membership state; the § 7.4.4/.5 OrderHint < (1<<OrderHintBits)
    bound is NOT header-decidable (OrderHint = get_disp_order_hint() is
    the unwrapped reference-state-DOH value, output-order-dependent — a
    non-goal).
- [x] 2.3 § 7.3.8.9 QM availability + QmProtected resets; § 6.17.6.2 QM
  layer dependencies.
  - IMPLEMENTED: `frame-header/qm-level-unavailable` + the QmProtected
    reset_qm() discipline (temporal-delimiter clear, QM-OBU protect,
    CLK/OLK + RAS/restricted-SWITCH reset).
  - RESIDUAL (named): the § 6.17.6.2 QM layer-dependency constraints
    (kept the existing TODO); the SWITCH/RAS MLayerPresenceMap reset arm.
- [x] 2.4 § 6.8.9 expected-dims bounds; remaining § 6.17.2 clauses.
  - IMPLEMENTED: `lcr/max-expected-dims-exceed-sequence-max` (the
    pure-arithmetic § 6.8.9 sequence-max clause).
  - RESIDUAL (named): the per-frame FrameWidth <= lcr_max_expected_width
    clause needs (xId,j) frame mapping; the § 6.17.2 06:4340 saved-memory
    clause is covered by PR #62's slot model.

## 3. Docs

- [x] 3.1 Registry entries; matrix proof per row; named residuals;
  generated docs; roadmap.
  - 3 registry entries; matrix proof updated on 6 rows; FEATURE-STATUS.md
    + SPEC-COVERAGE.md regenerated.

## 4. Verification

- [x] 4.1 Per-rule violation/boundary/Unknown/suppression/both-order
  tests.
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes.
