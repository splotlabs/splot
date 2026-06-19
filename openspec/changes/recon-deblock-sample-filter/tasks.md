## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-DEBLOCK-SAMPLE-FILTER` to the implementation matrix, decoder support matrix, and the decoder-conformance-coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add `deblock_sample_filter` and `DeblockSampleFilter` in a new `deblock_filter.rs`, computing the § 7.17.7.1 deltaM2 ramp and the per-side `Clip1` updates over a caller-supplied sample line.
- [x] 2.2 Take the per-side widths and the three pre-indexed `Q_Thresh_Mults` / `W_Mult` weights as caller-resolved scalars; keep it total and panic-free with two new typed `ReconError` variants validated before mutation.
- [x] 2.3 Export the items and update the crate `//!` docs.

## 3. Tests

- [x] 3.1 Add a `Round2` rounding test and a hand-computed symmetric width-2 case.
- [x] 3.2 Add an asymmetric/lossless/clamped reference match, a both-lossless no-op, a `Clip1` bit-depth clamp, fail-atomic rejection, and an i32-extreme totality sweep.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, zero-copy, dependency-direction, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, the conformance-coverage group, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-deblock-sample-filter --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
