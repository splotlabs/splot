## Context

The current minimal runtime reconstruction frontier validates one traced
64x64 8-bit YUV420 closed-loop-key fixture and reconstructs luma through the
`splot-recon` current-frame workspace. The same trace observes chroma `H_PRED`,
but the frontier still writes neutral chroma because `splot-recon` lacks a
cardinal directional primitive.

AV2 v1.0.0 §7.13.2.8 defines two non-interpolating cardinal cases:
pAngle 90 copies `AboveRow` into every row, and pAngle 180 copies `LeftCol` into
every column. The `Mode_To_Angle` table in §9.2 maps `V_PRED` to 90 and
`H_PRED` to 180. Those two cases can be implemented without the general
directional derivative, IDIF, MRL, IBP, or wide-angle machinery.

## Goals / Non-Goals

**Goals:**

- Add `RECON-INTRA-CARDINAL-DIRECTIONAL-PREDICTION` as a supported
  source-backed primitive row.
- Implement scheduler-free H/V prediction in `splot-recon` with checked block
  geometry, stride, edge length, sample type, and bit-depth range validation.
- Add workspace helpers that use in-storage prepared above/left edges when they
  are available.
- Use explicit traced chroma `H_PRED` handling in the existing minimal runtime
  reconstruction frontier and update the deterministic hash/Y4M expectations
  from the old neutral-chroma fallback to the spec-correct top-left H_PRED
  fallback samples.
- Extend the existing recon intra fuzz target instead of adding a redundant new
  target.

**Non-Goals:**

- No pAngle values other than 90 and 180.
- No IDIF filtering, MRL, IBP, wide-angle mapping, angle deltas, directional
  filtering, or data-driven intra prediction.
- No CfL/CCTX/MHCCP, palette, residual, transform, quantization, loop filtering,
  film grain, reference refresh, or broad runtime decode support.
- No AVM/dav2d integration, new dependencies, or dependency graph change.

## Decisions

1. Put the primitive in a new `splot-recon` module rather than `splot-decode`.
   `splot-recon` already owns scalar prediction primitives and current-frame
   workspace writes, and it has no dependency on other `splot-*` crates.
   Alternative considered: implement only the minimal runtime fallback in
   `splot-decode`; rejected because it would leave no reusable primitive or
   source-backed recon test coverage.

2. Model the primitive as an explicit H/V mode enum rather than accepting a raw
   pAngle integer. This prevents future callers from passing unsupported
   directional angles and accidentally treating them as implemented. The docs
   and matrix still cite §7.13.2.8 pAngle 90/180 and §9.2 mode mapping.
   Alternative considered: accept all pAngle values and reject non-cardinal
   inputs; rejected as a broader API surface before the general directional
   implementation exists.

3. Keep edge fallback policy at the caller/workspace boundary. The direct
   primitive requires exact prepared edges. Workspace helpers require real
   in-storage above/left edges. The minimal runtime top-left chroma case
   materializes the AV2 left-edge fallback value explicitly before calling
   H_PRED, replacing the old neutral chroma output without claiming full
   §7.13.2.1 edge preparation.

4. Extend `recon_intra_prediction_bytes` rather than adding another fuzz target.
   The existing target already covers recon intra primitives and workspace paths;
   adding a mode branch keeps CI fuzz-smoke bounded.

## Risks / Trade-offs

- Cardinal-only support could be mistaken for full directional prediction.
  Mitigation: narrow type names, matrix notes, OpenSpec non-goals, and support
  status keep broad intra rows partial.
- The minimal runtime output changes from the old neutral fallback to the
  spec-correct top-left H_PRED fallback. Mitigation: lock the corrected
  hash/Y4M bytes in runtime and CLI tests, and keep the matrix explicit that
  only this traced fallback path is supported.
- Edge availability from §7.13.2.1 is broader than this change. Mitigation:
  workspace helpers only operate when the required edge exists; the minimal
  top-left fallback remains narrow and documented.
