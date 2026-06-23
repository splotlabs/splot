## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-LOOP-RESTORATION-SOURCE-SAMPLE` to the implementation
  matrix, decoder support matrix, generated decoder support status, and decoder
  conformance coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add a new `splot-recon` loop-restoration module with public bounds,
  resolved sample, selected source, and `loop_restoration_source_sample`.
- [x] 2.2 Keep luma bounds, stripe bounds, frame reads, and sequence subsampling
  caller-resolved, and validate bounds/subsampling before coordinate resolution.
- [x] 2.3 Export the selector and update crate docs.

## 3. Tests

- [x] 3.1 Add focused tests for luma inside-stripe `CdefFrame` selection,
  above-stripe and below-stripe `CurrFrame` selection, and two-line clamping.
- [x] 3.2 Add focused tests for chroma subsampled bounds, luma ignoring sequence
  subsampling, invalid subsampling, invalid luma ranges, and stripe bounds.

## 4. Validation And PR Discipline

- [x] 4.1 Run `openspec validate recon-loop-restoration-source-sample --strict`.
- [x] 4.2 Run focused `splot-recon` tests plus feature-status, decoder-support,
  conformance-coverage, and relevant repo checks.
- [ ] 4.3 Create a ready PR only; request Claude and Codex reviews, wait for both
  latest-head responses, and address actionable feedback before merge.
