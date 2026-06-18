## ADDED Requirements

### Requirement: Encoder quantization v0

The encoder SHALL provide a private fixed-quantizer stage tracked by
`ENC-QUANTIZATION-V0`. For the current minimal subset, the stage SHALL accept a
4x4 DCT_DCT DC-only transform coefficient block and a validated fixed quantizer
index, produce row-major quantized coefficients, and produce decoder-visible
dequantized coefficients through `splot-recon`. The stage SHALL validate
quantizer inputs, coefficient ranges, and arithmetic before returning data, and
SHALL NOT emit syntax or create coded packets.

#### Scenario: Fixed qindex quantizes the DC-only coefficient block

- **WHEN** a supported 4x4 DCT_DCT DC-only coefficient block and fixed qindex
  are supplied
- **THEN** the quantization stage SHALL return a 16-coefficient row-major
  quantized block
- **AND** the DC coefficient SHALL use the resolved DC quantizer
- **AND** AC coefficients SHALL use the resolved AC quantizer and remain zero
  for the current DC-only input subset.

#### Scenario: Dequant and inverse reconstruct through splot-recon

- **WHEN** the produced quantized block is dequantized by `splot-recon` and
  passed through the existing 4x4 DCT_DCT inverse transform path
- **THEN** the reconstructed residual samples SHALL match the expected current
  v0 subset evidence for fixed qindex zero
- **AND** the proof SHALL remain private test evidence rather than a public
  encoder output claim.

#### Scenario: Unsupported quantization inputs are rejected

- **WHEN** the quantizer index is outside the active bit-depth range, the
  dequant denominator is zero, a coefficient is outside the supported
  dequant-visible range, or quantization arithmetic would overflow
- **THEN** the quantization stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial quantized coefficient data.

#### Scenario: Quantization v0 does not produce packets

- **WHEN** quantization calculation is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tokenization, tile-body, and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, rate control, or CLI success from quantization alone.
