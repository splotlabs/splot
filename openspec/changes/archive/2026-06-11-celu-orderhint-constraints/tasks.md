# Tasks: CELU and OrderHint constraints

## 1. Bookkeeping

- [x] 1.1 Matrix `openspec_change` on the three rows; register in
  `openspec/changes/README.md`; re-read § 7.3.6 (07 mirror 517–617),
  § 7.3.7 DOH (650–657), § 7.4.6 (1316–1320), § 6.4.1 verbatim.

## 2. § 7.3.6 in-unit ordering

- [x] 2.1 `celu/` namespace + the LCR→OPS→Atlas→SeqHdr→frame-units order
  with ascending mlayer, PADDING-free, building on FrameUnitSegmenter.

## 3. § 7.3.6 constraint family

- [x] 3.1 Output-unit presence, non-output-implies-output,
  same-OrderHint-across-output-units (Unknown drops), CLK/OLK
  first-unit + lowest-layer rules, no CLK+OLK mix, all-leading-or-none,
  CI-first-unit (CELU scope).

## 4. § 7.3.7/§ 7.4.6 DOH checks

- [x] 4.1 Flag-gated same-OrderHintBits-in-TU and
  same-OrderHint-across-CELU-output-units checks.

## 5. § 6.4.1 documented-blocked split

- [x] 5.1 The output-timing residuals' notes name the blocker; the
  OrderHint-agreement parts reference the landed checks.

## 6. Docs, registry, artifacts

- [x] 6.1 Register ids; matrix advances with proof; regenerate generated
  docs; roadmap Phase 5 updated.

## 7. Verification

- [x] 7.1 Tests per acceptance criteria.
- [x] 7.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 7.3 `cargo xtask ci` (bare, exit checked) passes.
