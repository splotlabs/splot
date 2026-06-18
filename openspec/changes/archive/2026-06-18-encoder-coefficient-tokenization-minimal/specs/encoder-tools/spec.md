## ADDED Requirements

### Requirement: Encoder coefficient tokenization minimal

The encoder SHALL provide a private coefficient-tokenization stage tracked by
`ENC-COEFFICIENT-TOKENIZATION-MINIMAL`. For the current minimal subset, the
stage SHALL accept a top-left neutral-spatial-context 4x4 DCT_DCT DC-only
quantized block, derive coefficient scan metadata, EOB, begin position,
sign/magnitude facts, coefficient CDF q-context from qindex, and ordered
entropy token records for AV2 §5.20.7.27 and §5.20.7.28. The stage SHALL prove
those token values can be written through the in-tree AV2 §8.2 symbol encoder
with scoped CDF rows and decoded back to the same values. It SHALL NOT emit tile
payloads, coded packets, public CLI success, neighbor-derived spatial contexts,
or broad coefficient syntax beyond the declared minimal tier.

#### Scenario: All-zero block emits skip token only

- **WHEN** a supported 4x4 DCT_DCT quantized block contains only zero
  coefficients
- **THEN** the tokenization stage SHALL report EOB zero
- **AND** SHALL emit only the ordered `all_zero` entropy-token record for the
  current scoped CDF row.

#### Scenario: DC-only block emits ordered base-symbol tokens

- **WHEN** a supported 4x4 DCT_DCT quantized block contains a nonzero DC
  coefficient whose magnitude is covered by the current base-symbol tier and
  all AC coefficients are zero
- **THEN** the tokenization stage SHALL report the DC scan position, EOB, begin
  position, and sign/magnitude facts
- **AND** SHALL emit ordered entropy-token records for `all_zero`, `eob_pt_16`,
  low-frequency `coeff_base_eob`, and DC sign as required by the coefficient
  sign.

#### Scenario: Token records roundtrip through section 8.2 symbols

- **WHEN** the produced token records are written through the in-tree AV2
  section 8.2 symbol encoder using their scoped CDF rows
- **THEN** the bytes SHALL decode through the in-tree symbol decoder to the same
  ordered token values
- **AND** the proof SHALL remain private test evidence rather than packet
  output.

#### Scenario: Unsupported coefficient inputs are rejected

- **WHEN** tokenization receives an unsupported shape, transform subset,
  non-top-left spatial context, non-DC coefficient, or coefficient magnitude
  that would require syntax outside the declared minimal tier
- **THEN** the stage SHALL return a typed encoder error
- **AND** SHALL NOT return partial token data.

#### Scenario: Tokenization does not produce packets

- **WHEN** coefficient tokenization is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
  until later tile-body and writer integration changes land
- **AND** no documentation or matrix row SHALL claim Baseline Encoder Profile v1
  output, rate control, or CLI success from tokenization alone.
