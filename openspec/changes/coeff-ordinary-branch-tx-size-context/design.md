## Context

The staged ordinary coefficient branch has been removing caller-resolved facts
from the AV2 section 5.20.7.27 `coeffs()` boundary. It now derives
`Tx_Width`, `Tx_Height`, raw log2 dimensions, and adjusted base-context
dimensions from generated section 9.2 tables. The next remaining table-derived
fact is `txSzCtx`, which section 5.20.7.27 computes before the all-zero branch.

## Goals / Non-Goals

**Goals:**

- Resolve `Tx_Size_Sqr[txSz]` and `Tx_Size_Sqr_Up[txSz]` through generated
  `splot_core::tables::conversion` tables.
- Derive `txSzCtx = (Tx_Size_Sqr[txSz] + Tx_Size_Sqr_Up[txSz] + 1) >> 1` in the
  ordinary branch `txSz` wrapper.
- Feed that derived value into ordinary base row selection.
- Keep raw transform dimensions for block geometry and EOB-size context.
- Keep adjusted transform dimensions for section 8.3.2 ordinary base-context
  geometry.
- Keep failures before any coefficient context state, CDF row, or symbol-decoder
  mutation.

**Non-Goals:**

- Do not implement section 5.20.7.29 `compute_tx_type`.
- Do not derive scan order or wire runtime `coeffs()`.
- Do not dequantize, inverse transform, residual-add, reconstruct, output, or
  refresh references.

## Decisions

- Remove `tx_size_ctx` from
  `CoeffOrdinaryBranchTxSizeDimensionsBaseConfig`; the lower explicit base config
  APIs keep their `tx_size_ctx` fields for staged tests and direct handoffs.
- Derive `txSzCtx` only for the nonzero wrapper path. The all-zero branch does
  not consume ordinary base rows, so it should not perform unnecessary
  `Tx_Size_Sqr` lookups.
- Reuse the existing table-bound/value validation helper for generated tables.
  Invalid `txSz` or invalid square table values therefore fail through existing
  typed `CoeffOrdinaryBranchError` variants before downstream mutation.

## Risks / Trade-offs

- [Risk] The wrapper now combines raw dimensions, adjusted dimensions, and
  `txSzCtx`.
  -> Mitigation: keep helper names explicit, cite section 5.20.7.27 in the doc
  comment, and test a rectangular `TX_64X32` case where raw, adjusted, and
  `txSzCtx` values differ.
- [Risk] This can be mistaken for full `txSz` integration.
  -> Mitigation: tracking and roadmap notes continue to list `compute_tx_type`,
  scan derivation, and runtime `coeffs()` as deferred.
