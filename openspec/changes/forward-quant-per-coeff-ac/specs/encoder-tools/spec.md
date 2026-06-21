## ADDED Requirements

### Requirement: per-coefficient forward quantizer over a real 4x4 block

The encoder forward quantizer SHALL quantize all 16 coefficients of a real 4x4
DCT_DCT transform block per-coefficient by the round-to-nearest policy (index 0 with
the DC quantizer, the rest with the AC quantizer), then dequantize the levels
through the `splot-recon` AV2 § 7.14 dequantization so the stored dequantized array
is exactly what the decoder reconstructs from the emitted levels. It SHALL be total
and panic-free, rejecting coefficients outside the dequant-visible range, arithmetic
overflow, and dequant products beyond the AV2 24-bit limit with typed errors. This
is a private, non-emitting encoder-policy stage tracked by
`ENC-FWD-QUANT-PER-COEFF-AC`; it does not change quantization policy (round-to-
nearest v0, no deadzone/RDO), select rate-control values, tokenize coefficients,
emit syntax, or produce packets.

#### Scenario: real non-uniform block quantizes per-coefficient

- **WHEN** a non-uniform residual is forward-transformed and quantized
- **THEN** every coefficient level equals the round-to-nearest level of its
  coefficient at its selected quantizer
- **AND** the block carries non-zero AC levels (not a DC-only degenerate case)

#### Scenario: dequantized array equals the decoder dequant of the emitted levels

- **WHEN** the quantized block is produced
- **THEN** its stored dequantized array equals an independent `splot-recon`
  `dequantize_block` of the emitted levels
- **AND** every emitted level dequantizes within the AV2 24-bit product limit

#### Scenario: decoder reconstruction is close to the source residual at low qindex

- **WHEN** the emitted coefficients are dequantized and inverse-transformed at
  qindex 0
- **THEN** the reconstruction stays within a bounded distance of the source residual
- **AND** it is not asserted to be bit-exact (quant rounding plus the DCT residue)
