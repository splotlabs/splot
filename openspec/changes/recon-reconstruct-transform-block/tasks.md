## 1. Planning And Feature Tracking

- [x] 1.1 Validate the OpenSpec change before implementation.
- [x] 1.2 Add `RECON-RECONSTRUCT-TRANSFORM-BLOCK` to the implementation matrix, decoder support matrix, and the decoder-conformance-coverage group.

## 2. Reconstruction Implementation

- [x] 2.1 Add `crates/splot-recon/src/reconstruct_block.rs` with `reconstruct_transform_block_residual`, composing `dequantize_block` → `inverse_transform_2d_outer` → `reconstruct_add_residual` over caller-owned scratch buffers.
- [x] 2.2 Keep it a total, panic-free, allocation-free `pub` composition that propagates the underlying typed `ReconError` before mutating `out` (no new error variant); register and export the module.
- [x] 2.3 Update the crate `//!` implemented list and feature-tracking enumeration.

## 3. Tests

- [x] 3.1 Add all-zero-preserves-prediction and uniform signed nonzero-DC residual tests at TX_4X4.
- [x] 3.2 Add the same two at TX_64X64 (adjusted-to-original sample duplication) and a fail-atomic inconsistent-buffer rejection.
- [x] 3.3 Run focused `splot-recon` tests plus clippy, doc, dependency-direction, and decoder-support checks.

## 4. Documentation, Review, And PR Discipline

- [x] 4.1 Update roadmap, decoder support matrix/status, implementation matrix, feature status, spec coverage, the conformance-coverage group, and OpenSpec artifacts.
- [ ] 4.2 Run `openspec validate recon-reconstruct-transform-block --strict` and required local gates before commit/PR.
- [ ] 4.3 Create a ready PR only; do not create a draft PR.
- [ ] 4.4 After the final commit, request review and wait for completed latest-head review before merge.
