## ADDED Requirements

### Requirement: Encoder forward transform foundation

The encoder SHALL provide a private forward-transform stage tracked by
`ENC-FORWARD-TRANSFORM-FOUNDATION`. For the current minimal subset, the stage
SHALL accept a 4x4 uniform signed residual block and produce a row-major 4x4
DCT_DCT coefficient block with only the DC coefficient populated. The stage SHALL
validate input shape and supported residual content before returning
coefficients, SHALL use checked arithmetic, and SHALL NOT emit syntax or create
coded packets.

#### Scenario: Uniform 4x4 residual maps to a DC-only coefficient block

- **WHEN** a 4x4 signed residual block contains the same value in every sample
- **THEN** the forward-transform stage SHALL return a 16-coefficient row-major
  block
- **AND** coefficient 0 SHALL contain the checked DC coefficient for the no-op
  quant/dequant 4x4 DCT_DCT path
- **AND** all AC coefficients SHALL be zero.

#### Scenario: No-op quant/dequant inverse reconstructs the residual block

- **WHEN** the produced coefficient block is passed unchanged through the
  `splot-recon` 4x4 DCT_DCT inverse transform path
- **THEN** the reconstructed residual block SHALL match the input uniform
  residual samples exactly
- **AND** the proof SHALL remain private test evidence rather than a public
  encoder output claim.

#### Scenario: Unsupported transform inputs are rejected

- **WHEN** the residual input is not exactly 16 samples, is non-uniform, or the
  DC coefficient calculation would overflow
- **THEN** the forward-transform stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial coefficient data.

#### Scenario: Forward transform foundation does not produce packets

- **WHEN** forward transform calculation is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later quantization, tokenization, tile-body, and writer integration
  changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output from forward transform calculation alone.
