## ADDED Requirements

### Requirement: read_quant golomb tail for a single golomb coefficient

The encoder coefficient tokenizer SHALL code one coefficient whose magnitude exceeds the
base-range cap (low-frequency ≥ 8, high-frequency ≥ 6) with the AV2 §5.20.7.28
`read_quant` golomb tail, emitted in the sign+quant pass after the coefficient's sign
token: for the `m = 1` single-coefficient case, the `q_length` unary (capped at 5), the
optional golomb-length unary, and the `coeff_rem` literal as bypass bits matching the
decoder read order. It SHALL cap the per-coefficient golomb extension at
`x = magnitude - maxLevel ≤ 517` (keeping the golomb-prefix length ≤ 8) and reject a
larger magnitude or a block with two-or-more golomb-range coefficients with typed errors.
This is a private, non-emitting stage tracked by `ENC-COEFF-GENERAL-WALK-GOLOMB`; it does
not code two-or-more golomb coefficients, chroma, or produce packets.

#### Scenario: a finite-q golomb coefficient roundtrips

- **WHEN** a coefficient with magnitude 10 (LF) is tokenized
- **THEN** the golomb tail emits the finite-q bypass bits after the sign token
- **AND** the roundtrip recovers the exact magnitude

#### Scenario: a golomb-prefix coefficient roundtrips

- **WHEN** a coefficient with magnitude 50 or 525 (LF) is tokenized
- **THEN** the golomb tail emits the golomb-prefix bypass bits
- **AND** the roundtrip recovers the exact magnitude

#### Scenario: an over-length or second golomb coefficient is rejected

- **WHEN** a coefficient's `x = magnitude - maxLevel` exceeds 517, or a block has two
  golomb-range coefficients
- **THEN** the tokenizer returns a typed error without panicking

#### Scenario: the base-range path is unchanged

- **WHEN** a block with all magnitudes at or below the base-range cap is tokenized
- **THEN** the emitted tokens are byte-identical to the prior brick
