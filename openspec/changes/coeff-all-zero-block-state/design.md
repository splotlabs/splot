## Context

`TileCoeffContextState` owns the above/left coefficient context lines, and the
minimal block-symbol trace already reads those lines to derive luma and V
`all_zero` CDF contexts. However, the trace still treats an all-zero coefficient
block as a symbol assertion only. AV2 §5.20.7.27 also initializes per-block
coefficient arrays and writes `culLevel` / `dcCategory` back into the above/left
context lines at the end of `coeffs()`.

## Goals / Non-Goals

**Goals:**

- Represent zeroed `Quant[]` together with the existing `Level[]` and
  `QuantSign[]` transform-block state.
- Add a small stateful all-zero coefficient-block helper that is directly
  grounded in §5.20.7.27.
- Wire the minimal luma and V all-zero symbol reads to apply zero context writes
  through the existing tile context state.
- Preserve the current minimal fixture trace and output.

**Non-Goals:**

- No nonzero EOB path, scan walk, coefficient level/sign entropy reads,
  `read_quant`, dequantization, transforms, residual add, or reconstruction.
- No U-plane expansion, `TxTypes` writes, CCTX handling, or broad transform-block
  syntax derivation.

## Decisions

- Keep `coeff_state.rs` as the owner of coefficient storage and add `Quant[]`
  there. `Quant[]` is part of the §5.20.7.27 coefficient state, and keeping it
  beside `Level[]` / `QuantSign[]` gives the later nonzero loop one state object
  to fill.
- Keep the all-zero branch in `coeff_loop.rs`. This module already composes
  coefficient context state with the minimal symbol trace, so it is the narrowest
  home for the first stateful `coeffs()` branch.
- Accept caller-resolved 4x4 transform dimensions. Full `txSz`, adjusted
  transform size, transform type, and scan order remain separate work; this
  helper only computes `Min(32, 4 * w4)` / `Min(32, 4 * h4)` for zero-state
  allocation and writes the caller-provided context-line ranges.
- Return explicit all-zero summary values. `eob`, `culLevel`, and `dcCategory`
  are the values later loop stages need, and tests can assert the helper remains
  a no-output-change zero path.

## Risks / Trade-offs

- Chroma context-line units remain a follow-up convention risk. The helper uses
  the same caller-provided V-plane coordinates as the state-backed context read;
  nonzero chroma propagation still needs the chroma-relative versus luma-relative
  addressing decision before it becomes behaviorally meaningful.
- This adds another no-output-change foundation brick. The mitigation is tight
  scope, direct wiring into the minimal trace, and explicit matrix notes that
  nonzero coefficient decode and reconstruction remain partial.
