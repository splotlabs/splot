## ADDED Requirements

### Requirement: 4x4 DCT_DCT forward transform

The encoder forward transform SHALL map any signed 4x4 residual block to all 16
row-major DCT_DCT coefficients as the numerical inverse of the AV2 § 7.15.4 4x4
inverse transform: the transposed § 9 `DCT_KERNEL4` applied as a row pass then a
column pass, with per-pass down-shifts (`FORWARD_ROW_SHIFT = 0`,
`FORWARD_COL_SHIFT = 11`) that sum to the 11-bit budget pairing the forward 2D DCT
gain with the inverse's `row_shift + col_shift`. The transform SHALL be total and
panic-free: the passes accumulate in `i64` and the final coefficient is a checked
`i32` narrowing that returns a typed error rather than wrapping or panicking. This
is a private, non-emitting encoder-policy arithmetic stage, tracked by
`ENC-FORWARD-TRANSFORM-DCT-4X4`; it does not select transforms, quantize, emit
syntax, or produce packets. Reconstruction through the `splot-recon` inverse is
bit-exact only for a uniform (DC-only) residual; general AC content reconstructs
within a small bound, not bit-exactly, because the AV2 integer DCT4 odd basis rows
are not orthonormal.

#### Scenario: uniform residual reproduces the DC-only stub and reconstructs exactly

- **WHEN** a uniform residual whose every sample equals `v` is forward-transformed
- **THEN** `coefficients[0]` is `v * 32` and every AC coefficient is `0`, identical
  to the flat `dct_dct_4x4_dc_only` stub
- **AND** the `splot-recon` 4x4 DCT_DCT inverse of those coefficients reconstructs
  the uniform residual bit-exactly

#### Scenario: non-uniform residual reconstructs within the bound

- **WHEN** a genuinely non-uniform 4x4 residual is forward-transformed and then run
  back through the `splot-recon` inverse transform
- **THEN** every reconstructed sample is within an observed bound (`<= 5` over the
  tested 8-bit residual domain) of the original residual sample
- **AND** the result is not asserted to be bit-exact (the non-orthogonality residue)

#### Scenario: out-of-domain residual yields a typed error without panicking

- **WHEN** a residual far outside the valid 8-bit domain produces a coefficient that
  does not fit `i32`
- **THEN** the transform returns `ForwardTransformCoefficientRangeExceeded` with the
  offending coefficient index and `i64` value
- **AND** it never panics or wraps
