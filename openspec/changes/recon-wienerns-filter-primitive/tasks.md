## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-WIENERNS-FILTER-PRIMITIVE` to the implementation matrix,
  decoder support matrix, generated decoder support status, and decoder
  conformance coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add a new `splot-recon` Wiener NS luma filter module with the §7.20.3
  `Wiener_Ns_Config_Y` table, public constants, parameter type, and
  `wiener_ns_filter_luma_block`.
- [x] 2.2 Keep source samples, subclasses, and coefficient classes
  caller-resolved, and validate output shape, subclass shape/range, coefficient
  class presence, sample type, and source sample range before mutating output.
- [x] 2.3 Export the primitive and update crate docs.

## 3. Tests

- [x] 3.1 Add focused tests for zero-coefficient copy, hand-computed tap
  accumulation, subclass selection, and 8-bit/10-bit `Clip1` clamping.
- [x] 3.2 Add fail-atomic tests for invalid output shape, missing classes,
  subclass length/range errors, unsupported sample storage, and source samples
  outside the active bit depth.

## 4. Validation And PR Discipline

- [x] 4.1 Run `openspec validate recon-wienerns-filter-primitive --strict`.
- [x] 4.2 Run focused `splot-recon` tests plus feature-status, decoder-support,
  conformance-coverage, and relevant repo checks.
- [ ] 4.3 Create a ready PR only; request Claude and Codex reviews, wait for both
  latest-head responses, and address actionable feedback before merge.
