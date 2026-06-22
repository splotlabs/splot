## ADDED Requirements

### Requirement: 16x16 DCT_DCT forward transform and quantizer

The encoder SHALL map any signed 16×16 residual to 256 row-major DCT_DCT coefficients as the
numerical inverse of the §7.15.4 16×16 inverse transform (transposed §9 `DCT_KERNEL16`, a row
pass then a column pass with forward down-shifts summing to 13 = round-trip gain 32 − inverse
total 19), accumulating in i64 with a checked i32 narrowing (never a panic). It SHALL
quantize the coefficients per-coefficient and dequantize through splot-recon. This is a
private, non-emitting stage tracked by `ENC-FORWARD-TRANSFORM-DCT-16X16`; it does not select
transforms, code other sizes/types, or emit a packet.

#### Scenario: flat input is the lossless anchor

- **WHEN** a uniform 16×16 residual is forward-transformed and quantized at qindex 0
- **THEN** it reconstructs bit-exactly through the splot-recon inverse

#### Scenario: random blocks reconstruct within bound

- **WHEN** random 16×16 residual blocks are forward-transformed, quantized at qindex 80, and
  reconstructed through dequant + the §7.15.4 inverse
- **THEN** the reconstruction is within the documented `|err|` bound of the input residual
