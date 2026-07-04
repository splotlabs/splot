# Tasks

## Reconstruction Genericization

- [x] 1.1 Genericize the `reconstruct_general_intra_block` / `_with_prediction` /
      `_rect` residual helpers over `T: ReconSample` and thread a runtime
      `bit_depth: BitDepth` (replacing the hard-coded `BitDepth::Eight`).
- [x] 1.2 Genericize `new_general_intra_workspace` and every
      `reconstruct_general_intra_*_into` / edge builder in
      `runtime_minimal_recon.rs` over `T: ReconSample` + `bit_depth`, deriving the
      § 7.13.2.1 no-neighbour fallbacks from `bit_depth`.
- [x] 1.3 Dispatch the general-intra decode on the sequence `bit_depth_idc`
      (Eight → `u8`, Ten → `u16`) in `general_intra.rs`.

## Runtime Gate And Output

- [x] 2.1 Relax the runtime sample-storage gate to admit 10-bit, keeping the
      frozen 8-bit tier and every richer 10-bit shape rejected before output.
- [x] 2.2 Gate the 10-bit admission to the DC_PRED-luma + DC-chroma square-leaf
      subset (single or multi 64x64 superblock, flat or AC residual); reject
      10-bit non-DC intra with `unsupported_10bit_non_dc_intra`, 10-bit CFL with
      `unsupported_cfl_intra`, 10-bit non-64x64 partition leaves (rectangular, or
      a split 32x32 / 16x16 square sub-block) with
      `unsupported_10bit_non_64x64_leaf`, the frozen `base_q_idx == 255`
      minimal-tier path with `unsupported_10bit_frozen_minimal_tier`, and 10-bit
      inter / reference retention with `unsupported_10bit_reference_retention`.
- [x] 2.3 Carry the displayed frame as 8-bit or 10-bit
      (`MinimalRuntimeDecodedFrame`) and dispatch the hash / raw / Y4M adapters on
      the storage arm; keep the inter / reference path 8-bit only.

## Tests And Tracking

- [x] 3.1 Add the `syn-flat-intra-64x64-10bit-q80.ivf`,
      `syn-cos-intra-64x64-10bit-q180.ivf`, and
      `syn-2sb-intra-128x64-10bit-q80.ivf` conformance fixtures and decode tests
      pinning the flat Y == 400 / U == 480 / V == 520 planes, the AC-residual frame
      hash, and the multi-superblock frame hash plus its per-superblock luma
      anchors (left 400, right 460).
- [x] 3.2 Confirm the 8-bit corpus stays byte-identical, the 10-bit CFL fixture
      still rejects, and `local-decoder-mission.ivf` still fails closed.
- [x] 3.2a Pin each of the four 10-bit fail-closed reject guards with a
      committed, validator-clean negative fixture and a negative decode test:
      `syn-smooth-intra-64x64-10bit-q80.ivf` → `unsupported_10bit_non_dc_intra`,
      `syn-split-intra-64x64-10bit-q110.ivf` → `unsupported_10bit_non_64x64_leaf`,
      `syn-flat-intra-64x64-10bit-q255.ivf` →
      `unsupported_10bit_frozen_minimal_tier`, and
      `syn-2frame-inter-64x64-10bit.ivf` →
      `unsupported_10bit_reference_retention`.
- [x] 3.3 Add matrix, decoder-support, LOCAL-REFERENCE-EVIDENCE, and conformance
      manifest entries for `DECODE-GENERAL-INTRA-10BIT` (the `.ivf` vectors are
      tracked in `tests/conformance/manifest.toml`, not
      `tests/fixtures/MANIFEST.toml`).
- [x] 3.4 Regenerate generated docs and run the required checks
      (`cargo xtask ci`, `conformance`, `check-fixtures`).
