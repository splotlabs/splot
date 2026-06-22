## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-SUBPEL-MC` to the implementation matrix, decoder support matrix, and the decoder-conformance-coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add `subpel_predict_block`, `SubpelPredictParams`, `ReferencePlaneView`, `InterpolationFilter`, and the verbatim § 7.13.3.18 `SUBPEL_FILTERS[6][16][8]` table to a new `crates/splot-recon/src/subpel_mc.rs`.
- [x] 2.2 Transcribe the § 7.13.3.18 two-pass convolution (the horizontal `Round2(s, InterRound0)` pass into the intermediate array, the vertical `Round2(s, InterRound1)` pass, the small-block 4-tap substitution per pass, the sub-pel phase selection, and the final § 4.8 `Clip1` single-reference write) over caller-resolved scaling, clipping region, filter, dimensions, and reference samples.
- [x] 2.3 Keep the function total and panic-free (validated reference buffer length and non-zero dimensions, rejected zero / oversized block and negative step, the guarded vertical-pass intermediate index, and overflow-free `i64` sums); add the typed `ReconError` variants; export the items and update the crate and module `//!` docs.

## 3. Tests

- [x] 3.1 Add a verbatim-table invariant test (every row sums to 128, all taps even, distinctive-row spot checks).
- [x] 3.2 Add hand-anchored worked examples (full-pel → bit-exact copy, flat → flat for any phase, a hand-computed `EIGHTTAP_SHARP` half-pel, the border-extension corner, 10-bit `Clip1`, the small-block 4-tap substitution, the error cases).
- [x] 3.3 Add a 2000-case property test comparing `subpel_predict_block` against an independent in-test re-trace of the § 7.13.3.18 pseudocode over varied content, block sizes, filters, and sub-pel phases.
- [x] 3.4 Run focused `splot-recon` tests plus clippy, doc, source-lines, dependency-direction, decoder-support, and feature-status checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, the conformance-coverage group, and OpenSpec artifacts.
- [x] 4.2 Run `openspec validate recon-subpel-mc --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
