## 1. Implementation

- [x] 1.1 Re-export `DCT_KERNEL4` from `splot-recon` (no dependency-graph change).
- [x] 1.2 Add `ForwardTransformCoefficientRangeExceeded` typed error.
- [x] 1.3 Add `ForwardTransformBlock::dct_dct_4x4` (full 16-coefficient transform)
      plus the `forward_round2` / `forward_dct4_1d` helpers and the
      `FORWARD_ROW_SHIFT` / `FORWARD_COL_SHIFT` constants (sum 11, const-asserted).
- [x] 1.4 Leave `dct_dct_4x4_dc_only` unchanged (closed-loop caller preserved).

## 2. Tests

- [x] 2.1 Flat residual matches the DC-only stub (`DC = v*32`, AC `0`) and
      reconstructs bit-exactly through the recon inverse.
- [x] 2.2 Horizontal-ramp hand-computed orientation pin (energy in coefficient row
      0 only; exact `[48, -35, 0, -3, 0, ...]`).
- [x] 2.3 Non-uniform residuals round-trip within the `<= 5` bound (not equality).
- [x] 2.4 `forward_round2` matches the recon `round2`; shifts sum to 11.
- [x] 2.5 Deterministic 2000-residual sweep over `[-255, 255]`: bound + no panic.
- [x] 2.6 Out-of-domain residual returns the typed range error without panicking.
- [x] 2.7 Non-4x4 shape and wrong residual length rejected.

## 3. Tracking

- [x] 3.1 Add the `ENC-FORWARD-TRANSFORM-DCT-4X4` matrix row.
- [x] 3.2 Regenerate feature status + spec coverage; run `cargo xtask ci`.
