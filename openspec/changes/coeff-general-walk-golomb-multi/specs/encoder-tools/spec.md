## ADDED Requirements

### Requirement: multiple read_quant golomb coefficients per block

The encoder coefficient tokenizer SHALL code an arbitrary number of §5.20.7.28
`read_quant` golomb coefficients in a 4x4 luma block, threading the running `hrLevelAvg`
predictor in reverse scan so the golomb parameter `m = Clip3(1,6,GetMsb(hrLevelAvg))`
varies per coefficient, emitting the general-`m` golomb tail (finite-q `L(m)` `coeff_rem`
or the golomb-prefix), and updating `hrLevelAvg = (x + hrLevelAvg) >> 1` after each.
`compose_sign_pass`, `validate_general_lf_scope`, and `recover_quant_from_tokens` SHALL
thread `hrLevelAvg` identically. The single-golomb-coefficient path SHALL be byte-identical
to the prior brick (the first golomb coefficient sees `hrLevelAvg=0 → m=1`). This is a
private, non-emitting stage tracked by `ENC-COEFF-GENERAL-WALK-GOLOMB-MULTI`; it does not
code chroma or produce packets.

#### Scenario: two golomb coefficients drive m above one

- **WHEN** a block with two golomb coefficients is tokenized
- **THEN** the second coefficient's golomb tail uses `m` derived from the threaded
  `hrLevelAvg` (above 1)
- **AND** the roundtrip recovers the exact block

#### Scenario: many golomb coefficients roundtrip

- **WHEN** a block with two or three golomb coefficients across positions and magnitudes
  is tokenized
- **THEN** each roundtrips through the §8.2 coder and recovers exactly

#### Scenario: the single-golomb path is unchanged

- **WHEN** a block with exactly one golomb coefficient is tokenized
- **THEN** the emitted tokens are byte-identical to the prior brick
