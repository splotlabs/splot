# Tasks: reference-frame buffer state model

## 1. Bookkeeping

- [x] 1.1 Confirm/create the § 7.23 matrix row; `openspec_change` set;
  re-read § 7.23 verbatim plus the CLK/CVS reset and SEF semantics it
  references; locate every reference-state-gated § 5.18/§ 6/§ 7 site.

## 2. Implementation

- [x] 2.1 Per-layer per-slot tracker updated per § 7.23 at
  segmenter-authoritative frame boundaries.
- [x] 2.2 Honest poisoning for unparsed refresh masks / ambiguity;
  grounded resets.
- [x] 2.3 Thread into FrameHeaderParseInput.reference_state; any newly
  decidable diagnostic with citation.

## 3. Docs

- [x] 3.1 Matrix proof; named residuals; generated docs; roadmap.

## 4. Verification

- [x] 4.1 § 7.23 semantics tests; poisoning tests; diagnostic tests both
  orders.
- [x] 4.2 `check-feature-status` + `check-diagnostic-registry` pass.
- [x] 4.3 `cargo xtask ci` (bare, exit checked) passes.
