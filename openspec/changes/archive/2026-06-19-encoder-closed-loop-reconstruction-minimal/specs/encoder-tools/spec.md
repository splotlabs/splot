## ADDED Requirements

### Requirement: Encoder closed-loop reconstruction minimal

The encoder SHALL provide a private closed-loop reconstruction stage tracked by
`ENC-CLOSED-LOOP-RECONSTRUCTION-MINIMAL`. For the current minimal subset, the
stage SHALL accept a borrowed 8-bit luma 4x4 top-left source block, predict it
with AV2 §7.13.2.10 no-neighbor DC intra prediction, form and quantize a
residual through the existing private encoder residual, forward-transform, and
fixed-quantization stages, and reconstruct the decoder-visible samples through
`splot-recon` using AV2 §7.14.4/§7.14.2 dequantization, §7.15.4 inverse
transform, and §7.14.3 residual addition. The stage SHALL freeze the
reconstructed block into a `splot-recon` current-frame workspace and compute its
decoded-frame hash. Every decoder-visible step SHALL be performed by
`splot-recon`; the encoder SHALL NOT reimplement decoder-visible prediction,
dequantization, inverse transform, or residual addition. It SHALL NOT emit tile
payloads, coded packets, public CLI success, reference-frame storage, chroma or
inter reconstruction, or any reconstruction outside the declared minimal tier.

#### Scenario: Lossless qindex-zero flat block reconstructs to the source

- **WHEN** a flat 8-bit luma 4x4 top-left source block is reconstructed at
  quantizer index zero
- **THEN** the closed loop SHALL reconstruct decoder-visible samples equal to
  the source samples
- **AND** SHALL expose the reconstructed samples and a decoded-frame hash for
  the reconstructed workspace.

#### Scenario: Reconstruction and hash are deterministic

- **WHEN** the same source block and quantization parameters are reconstructed
  more than once
- **THEN** the reconstructed samples and the decoded-frame hash SHALL be
  byte-identical across runs
- **AND** the decoded-frame hash SHALL match an independently constructed
  `splot-recon` workspace filled with the reconstructed samples.

#### Scenario: Emitted coefficient decisions reconstruct identically

- **WHEN** the quantized block reconstructed by the closed loop is tokenized and
  its token records are roundtripped through the in-tree AV2 §8.2 symbol
  encoder/decoder
- **THEN** the decoded token symbols SHALL recover the exact quantized DC
  coefficient that the closed loop reconstructed from
- **AND** the reconstruction derived from that recovered coefficient SHALL equal
  the closed loop's reconstructed samples.

#### Scenario: Unsupported inputs are rejected

- **WHEN** closed-loop reconstruction receives a non-uniform source block, a
  source view whose visible size is not 4x4, an unsupported bit depth, or any
  input the underlying residual, forward-transform, or quantization stages
  reject
- **THEN** the stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial reconstruction data.

#### Scenario: Closed-loop reconstruction does not produce packets

- **WHEN** closed-loop reconstruction is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, reference storage, inter support, rate control, or CLI success from
  closed-loop reconstruction alone.
