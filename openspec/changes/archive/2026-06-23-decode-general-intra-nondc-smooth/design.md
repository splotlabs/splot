## Context

The general intra decode reconstructs DC_PRED blocks bit-exactly. The DC path
builds a flat prediction (`vec![dc_sample; n]`) and adds the § 5.20.7.27
residual. Non-DC prediction differs only in the prediction step: a per-sample
predicted block computed from the § 7.13.2.1 prediction edges.

A single block has no above/left neighbours, so its § 7.13.2.1 edges are pure
fallbacks. This makes the top-left block the smallest correct non-DC target: it
exercises the predictor and the residual integration without needing the
in-storage neighbour reads (and the neighbour mode-context array) that
multi-block non-DC prediction requires.

## Decisions

- **Reuse the `splot-recon` predictor.** `predict_intra_smooth_rect_into`
  already implements § 7.13.2.13. The decoder only constructs the § 7.13.2.1
  edges and calls it; it does not reimplement smooth prediction.
- **Construct fallback edges explicitly.** The workspace edge helpers read
  in-storage neighbours only (no § 7.13.2.1 fallback synthesis). For the
  no-neighbour block the edges are the constants `AboveRow = (1<<(BD-1))-1`,
  `LeftCol = (1<<(BD-1))+1` (127 / 129 at 8-bit); the smooth sentinels
  `above[w]` / `left[h]` share those fallbacks.
- **Refactor, do not duplicate, the residual path.**
  `reconstruct_general_intra_block_with_prediction` takes the predicted block;
  the existing flat-DC `reconstruct_general_intra_block` becomes a thin wrapper.
- **Gate to the verified subset.** Only `SMOOTH_V_PRED` / `SMOOTH_H_PRED` (the
  modes with single-block oracle fixtures) are accepted, only at the top-left
  block, only with DC chroma. `SMOOTH`, `PAETH`, directional modes, multi-block
  non-DC, and non-DC chroma are rejected before any reconstruction so a wrong
  prediction can never silently produce a wrong-but-plausible frame.

## Risks / Trade-offs

- The fallback-edge construction is asserted by the end-to-end oracle test
  (prediction + residual == avmdec == dav2d); an incorrect edge constant would
  fail bit-exactness, caught by the § 8.2.4 `exit_symbol()` guard plus the pinned
  hash. SMOOTH and PAETH are deferred because single blocks do not exercise them.
