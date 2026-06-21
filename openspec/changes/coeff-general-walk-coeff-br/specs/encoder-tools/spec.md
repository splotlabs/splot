## ADDED Requirements

### Requirement: EOB-coefficient coeff_br base-range tier

The encoder coefficient tokenizer SHALL emit the AV2 §5.20.7.27 `coeff_br`
base-range token for the end-of-block coefficient of a general low-frequency 4x4
DCT_DCT luma block when its magnitude exceeds the base tier, allowing the EOB
coefficient magnitude 1..=7. The `coeff_br` SHALL be emitted interleaved, right
after the EOB coefficient's `coeff_base_eob` and before the running `Level[]` is
updated, with the symbol `magnitude - (LF_NUM_BASE_LEVELS + 1)` and the constant
empty-`Level[]` context (0 at the DC raster position, 7 at a non-DC low-frequency
position, per the decoder `CoeffBrContext` with an all-zero `Level[]`). The non-EOB
coefficient SHALL stay base-tier (1..=4). The recovery helper SHALL reconstruct the
EOB level as `coeff_base_eob + 1 + coeff_br`. This is a private, non-emitting stage
tracked by `ENC-COEFF-GENERAL-WALK-COEFF-BR`; it does not read a neighbour offset
table, code the non-EOB `coeff_br`, golomb magnitudes, eob > 2, high-frequency or
chroma coefficients, or produce packets.

#### Scenario: an EOB coefficient above the base tier emits coeff_br

- **WHEN** the EOB coefficient has magnitude 5, 6, or 7
- **THEN** a `coeff_br` token follows its `coeff_base_eob` with the constant EOB
  context and symbol `magnitude - 5`
- **AND** the roundtrip recovers the exact signed magnitude

#### Scenario: an eob=1 DC above the base tier matches the existing tokens

- **WHEN** an eob=1 DC has magnitude 5, 6, or 7
- **THEN** the tokens are consistent with the existing single-DC tokenizer
  (`coeff_base_eob` + `coeff_br` at context 0)

#### Scenario: out-of-scope magnitude is rejected

- **WHEN** the EOB coefficient magnitude exceeds 7, or a non-EOB coefficient
  magnitude exceeds 4
- **THEN** the tokenizer returns a typed unsupported-magnitude error without
  panicking
