## ADDED Requirements

### Requirement: Dequantization process

The repository SHALL provide a scheduler-free `splot-recon` primitive for the AV2
§ 7.14.4 dequantization process, tracked by `RECON-DEQUANT-PROCESS`. The
`dequant_coefficient` function SHALL compute, for a coded coefficient `qc`, a
resolved per-coefficient quantizer `q2`, and a dequant denominator
`dq_denom = 1 << shift`, the § 7.14.4 result
`Clip3(-(1 << (7 + BitDepth)), (1 << (7 + BitDepth)) - 1, sign * (Round2(Abs(qc) * q2 & 0xFFFFFF, QUANT_TABLE_BITS) / dq_denom))`
where `sign = (qc < 0) ? -1 : 1` and `QUANT_TABLE_BITS = 3`. The
`dequantize_block` function SHALL apply it over a `tx_width * tx_height`
row-major transform block (each dimension 4, 8, 16, or 32), selecting the DC
quantizer for the `(0, 0)` coefficient and the AC quantizer for every other
coefficient (the non-quantization-matrix path). The primitive SHALL be total and
panic-free for every input (widened intermediates, `i32::MIN`-safe absolute
value, a zero `dq_denom` treated as 1, and the `Clip3` bound), and SHALL validate
the transform shape and that the `quant` and `out` buffers are each exactly
`tx_width * tx_height` long, returning typed `ReconError` values otherwise. The
primitive SHALL read no frame, segment, or tile state and SHALL NOT implement the
§ 7.14.4 quantization-matrix weighting (the `Quantizer_Matrix` / `UserQm`
lookups), the `shift` / `useFsc` / `allow_tcq` derivation, the coefficient
entropy decode that produces `Quant`, the § 7.15.4 inverse transform, residual
addition, tile syntax traversal, runtime decode output, or reference-refresh
semantics.

#### Scenario: Dequantization succeeds with self-contained tests

- **WHEN** `cargo test -p splot-recon dequant_process --locked` runs
- **THEN** the test suite covers the `Round2` rounding with the 24-bit mask, the
  `dq_denom` divide, both bit-depth `Clip3` bounds, the `i32::MIN` /
  maximum-quantizer totality extreme, and the DC-versus-AC block selection
- **AND** the implementation uses no AVM, dav2d, ffmpeg, runtime decode, or
  external decoder invocation

#### Scenario: Invalid dequantization input is typed

- **WHEN** callers pass a `tx_width` / `tx_height` that is not 4/8/16/32, or
  `quant` / `out` buffers that are not `tx_width * tx_height` long
- **THEN** `splot-recon` returns a structured `ReconError`
- **AND** library code does not panic, overflow, unwrap, or emit `decode/*`
  diagnostics

#### Scenario: Full reconstruction remains incomplete

- **WHEN** decoder support status is generated
- **THEN** the matrix records the dequantization process as supported
- **AND** broader reconstruction remains partial until the quantization-matrix
  weighting, the coefficient entropy decode, the § 7.15.4 inverse transform
  invocation, and prediction/workspace integration are implemented and proven
