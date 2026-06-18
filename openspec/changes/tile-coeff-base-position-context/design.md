## Context

The `splot-decode` block CDF subset now carries the coefficient CDF banks
(`eob_extra`, `eob_pt`, `dc_sign`) and the block-mode §8.3.2 contexts
(`y_mode_index`, `uv_mode`, the `all_zero` formula). The §5.20.7.27 `coeffs()`
loop additionally needs the §8.3.2 coefficient-symbol context derivations. Those
fall into two groups: the position-only ones (no `Level[]` magnitude buffer) and
the `Level[]`/sign-buffer-dependent ones. This change adds the first group.

## Decisions

- **Position-only first.** `coeff_base_eob` (08-parsing-process.md lines
  1372-1385) keys only on the scan position `c` and the adjusted block geometry;
  `coeff_base_bob` (lines 1397-1406) keys only on the begin position `bob` and the
  segment end-of-block `seg_eob`. Neither reads `Level[]`, `QuantSign[]`, or the
  DC-context buffers, so both are pure functions implementable and testable now,
  ahead of the buffer infrastructure.

- **Caller-resolved geometry.** `coeff_base_eob` uses
  `numCoeffs = Tx_Height[adjTxSz] << Tx_Width_Log2[adjTxSz]`. The function takes
  `bwl` and `height` directly (the caller resolves them from the adjusted
  transform size), mirroring the spec expression and keeping the context module
  free of the §9.2 conversion tables, consistent with the existing
  caller-resolves convention.

- **Total / panic-free.** The `numCoeffs` shift is computed with `checked_shl`
  (saturating to `usize::MAX` on an out-of-range width) inside a `const fn`, so an
  ill-formed caller cannot trigger a shift-overflow panic. `coeff_base_bob` is
  pure integer comparisons. No new error type is needed.

- **Derivation-only, no-output-change.** Like the coefficient banks, these
  contexts are not yet read by any decode path (the `coeffs()` loop does not
  exist), so the change is additive and the minimal-fixture decode output is
  unchanged. They live in a new `cdf/coeff_context.rs`, the coefficient
  counterpart of the block-mode `cdf/block_context.rs`.

## Risks / Trade-offs

- **Threshold/boundary fidelity** is the main risk (the `<=` boundaries at
  `numCoeffs/8`, `numCoeffs/4`, `seg_eob>>3`, `seg_eob>>2`). Mitigated by tests
  that pin each boundary value (`128`/`256` for TX_32X32, `2`/`4` for TX_4X4,
  `8`/`16` for `seg_eob == 64`) and the off-by-one neighbours.
- **Shift totality** is covered by a `bwl == u32::MAX` test asserting no panic.
