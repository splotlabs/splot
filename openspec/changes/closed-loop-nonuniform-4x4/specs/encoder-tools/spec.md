## ADDED Requirements

### Requirement: non-uniform closed-loop reconstruction of a 4x4 luma block

The encoder closed loop SHALL reconstruct a non-uniform 8-bit luma 4x4 DCT_DCT intra
block end to end: AV2 §7.13.2.10 no-neighbor DC prediction, an encoder-policy
residual, the real 16-coefficient forward DCT, per-coefficient quantization, then
`splot-recon`'s AV2 §7.14 dequantization, §7.15.4 inverse transform, and §7.14.3
residual addition — so the reconstructed decoder-visible samples and their
decoded-frame hash are produced entirely through the decoder-visible `splot-recon`
process. This is a private, non-emitting stage tracked by
`ENC-CLOSED-LOOP-NONUNIFORM-4X4`; the flat entry point still rejects a non-uniform
residual via the flat-only forward transform, and the loop does not tokenize
coefficients, emit syntax, or produce packets.

#### Scenario: a non-uniform source reconstructs through the real forward DCT

- **WHEN** a non-uniform 4x4 luma source is reconstructed through
  `reconstruct_luma_4x4`
- **THEN** the quantized block carries non-zero AC levels (the real forward DCT
  engaged, not a DC-only degenerate case)
- **AND** the reconstructed samples and hash are produced entirely through
  `splot-recon`

#### Scenario: reconstruction is near-lossless at qindex 0

- **WHEN** the non-uniform block is reconstructed at qindex 0
- **THEN** every reconstructed sample is within a bounded distance of the source
- **AND** it is not asserted to be bit-exact (quant rounding plus the DCT residue)

#### Scenario: a uniform source matches the flat entry point

- **WHEN** a uniform source is reconstructed through `reconstruct_luma_4x4`
- **THEN** the reconstructed samples, quantized levels, and hash equal those of the
  flat `reconstruct_luma_4x4_dc_only` entry point

#### Scenario: the flat entry point rejects a non-uniform source

- **WHEN** a non-uniform source is reconstructed through
  `reconstruct_luma_4x4_dc_only`
- **THEN** it fails with `ForwardTransformNonUniformResidual`
