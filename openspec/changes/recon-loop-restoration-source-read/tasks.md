## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-LOOP-RESTORATION-SOURCE-READ` to the implementation
  matrix, decoder support matrix, generated decoder support status, and decoder
  conformance coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add `LoopRestorationSourceSampleValue<T>` and
  `loop_restoration_source_sample_value`.
- [x] 2.2 Reuse `loop_restoration_source_sample` for section 7.20.2 coordinate
  clipping and source selection, then read from the selected immutable
  `FrameRef`.
- [x] 2.3 Add typed errors for mismatched source-frame metadata and
  out-of-visible-plane selected samples.
- [x] 2.4 Export the helper and update crate docs.

## 3. Tests

- [x] 3.1 Add focused tests for in-stripe `CdefFrame` reads and out-of-stripe
  `CurrFrame` reads after the two-line clamp.
- [x] 3.2 Add focused tests for chroma reads, visible-rect origins, mismatched
  frame metadata, out-of-visible-plane bounds, and missing chroma planes.

## 4. Validation And PR Discipline

- [x] 4.1 Run `openspec validate recon-loop-restoration-source-read --strict`.
- [x] 4.2 Run focused `splot-recon` tests plus feature-status, decoder-support,
  conformance-coverage, and relevant repo checks.
- [ ] 4.3 Create a ready PR only; request Claude and Codex reviews, wait for
  both latest-head responses, and address actionable feedback before merge.
