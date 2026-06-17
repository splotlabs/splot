## Context

The residual path now has the dequant quantizer functions and all § 7.15.2 1D
inverse transforms. The § 7.14.3 reconstruct process ends by adding the residual
to the prediction and clamping with `Clip1`. That final addition is independent
of how the residual is produced, so it is a clean self-contained brick.

## Goals / Non-Goals

Goals:

- Implement the § 7.14.3 residual-addition step exactly.
- Keep it total, panic-free, and free of frame/segment/tile state.

Non-Goals:

- The § 7.15.4 2D inverse transform that fills `Residual`, the § 7.14.4
  dequantization process, the § 7.15.3 secondary transform, the DPCM adjustment,
  prediction-sample production, or workspace integration.

## Decisions

- **Standalone primitive over caller buffers.** `reconstruct_add_residual` takes
  the prediction samples and the residual as slices and writes the clamped sum,
  mirroring the existing intra-prediction primitives. The spec reads the
  prediction from `CurrFrame` in place; a future current-frame workspace helper
  can call this primitive over a plane rectangle. Keeping it standalone makes it
  independent of the transform and trivially testable.
- **`Clip1` via the bit depth.** § 4.8 `Clip1(x) = Clip3(0, (1 << BitDepth) - 1, x)`
  clamps to `0..=max_sample`. The sample type is validated to represent the bit
  depth (reusing the crate's `validate_sample_type`), so the clamped result
  always fits the storage type.
- **Totality.** The sum is computed in `i64`, so even `i32::MIN` / `i32::MAX`
  residuals cannot overflow; `Clip1` then bounds the result. Mismatched
  prediction/residual/output lengths return a typed `ReconError`.

## Risks / Trade-offs

- A standalone primitive rather than an in-place workspace method means the
  caller threads the buffers, but it keeps this brick decoupled from the
  workspace and the transform; the workspace integration is a later step.

## Migration Plan

Additive; new module and one new `ReconError` variant. No existing API changes,
and the runtime is unaffected.

## Open Questions

None.
