## Why

The general intra decode path reconstructs DC_PRED blocks (single-block and
split-partition multi-block) bit-exactly. Real AV2 intra frames use non-DC
prediction modes heavily. The next step is the first non-DC luma prediction:
the § 7.13.2.13 smooth modes, which need the § 7.13.2.1 prediction-edge
construction in addition to the residual path that already exists.

## What Changes

- Add Feature ID `DECODE-GENERAL-INTRA-NONDC-LUMA-SMOOTH`.
- Reconstruct the § 7.13.2.13 `SMOOTH_V_PRED` and `SMOOTH_H_PRED` luma modes for
  the top-left (no-neighbour) block: build the smooth prediction over the
  § 7.13.2.1 no-neighbour fallback edges (8-bit: `AboveRow` `127`, `LeftCol`
  `129`, the smooth sentinels sharing those fallbacks) via the shared
  `splot-recon` `predict_intra_smooth_rect_into`, then add the § 5.20.7.27 AC
  residual.
- Refactor the residual reconstruction to take an arbitrary per-sample
  prediction buffer (`reconstruct_general_intra_block_with_prediction`); the DC
  path becomes the flat-prediction special case.
- Map the reconstructed § 9.2 luma mode to the supported predictor
  (`IntraYMode::supported_nondc`) and gate the block decode: DC chroma only, the
  supported non-DC luma modes only at the top-left (no-neighbour) block,
  everything else rejected with a structured `decode/unsupported-feature`
  diagnostic before any reconstruction.
- Add the project-owned `syn-vsmooth-intra-64x64-q120.ivf` (SMOOTH_V) and
  `syn-hsmooth-intra-64x64-q120.ivf` (SMOOTH_H) fixtures and prove they decode
  bit-exactly to the avmdec/dav2d oracle.
- Update decoder tracking, generated status docs, and OpenSpec tasks.

## Capabilities

### New Capabilities
- `decode-general-intra-nondc-smooth`: Crate-private single-block non-DC
  (§ 7.13.2.13 smooth vertical/horizontal) luma intra prediction over the
  § 7.13.2.1 no-neighbour fallback edges plus AC residual.

### Modified Capabilities
- `decoder-support`: Track the new partial decoder-support row for the general
  intra single-block non-DC luma smooth decode.

## Impact

- Affects `crates/splot-decode/src/tile_payload/cdf/block_context.rs`,
  `crates/splot-decode/src/tile_payload/general_intra_block.rs`,
  `crates/splot-decode/src/tile_payload/general_intra_residual.rs`,
  `crates/splot-decode/src/tile_payload.rs`,
  `crates/splot-decode/src/runtime_minimal.rs`, and
  `crates/splot-decode/src/runtime_minimal_recon.rs`.
- Updates `docs/IMPLEMENTATION-MATRIX.toml`,
  `docs/DECODER-SUPPORT-MATRIX.toml`, `docs/LOCAL-REFERENCE-EVIDENCE.toml`,
  `tests/conformance/manifest.toml`, and generated status docs.
- No public API, dependency graph, encoder, or validator changes. The remaining
  non-DC modes (SMOOTH, PAETH), directional modes, multi-block non-DC prediction
  (reading reconstructed neighbours), non-DC chroma, non-64x64 frames, inter
  prediction, in-loop filters, and live in-CI AVM/dav2d remain out of scope.
