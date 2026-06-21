## ADDED Requirements

### Requirement: eob 11 first high-frequency coefficient

The encoder coefficient tokenizer SHALL tokenize a general 4x4 DCT_DCT luma block with
eob 11, whose end-of-block coefficient sits at the first high-frequency position (scan
index 10, `row+col = 4`). It SHALL select the LF vs HF `coeff_base_eob` / `coeff_br`
token and CDF table per coefficient by `is_lf = (row+col < 4)`, code the HF EOB
coefficient with the 3-symbol HF `coeff_base_eob` table (level saturating at 3) and its
optional HF `coeff_br` (plain-`mag` context, no `+7`), and reject a high-frequency
magnitude above 5 (the HF no-golomb cap) and a nonzero at scan index ≥ 11 with typed
errors. The HF selectors SHALL route through both §8.2 proof routers. This is a private,
non-emitting stage tracked by `ENC-COEFF-GENERAL-WALK-HF-EOB11`; it does not code
non-EOB high-frequency coefficients, golomb magnitudes, chroma, or produce packets.

#### Scenario: an eob=11 block codes the HF EOB coefficient

- **WHEN** an eob=11 block is tokenized
- **THEN** the EOB coefficient emits the HF `coeff_base_eob` selector and the low-frequency
  coefficients keep the LF selector
- **AND** the roundtrip recovers the exact block

#### Scenario: the high-frequency magnitude cap is enforced

- **WHEN** the HF EOB coefficient has magnitude 6
- **THEN** the tokenizer returns a typed unsupported-magnitude error (cap 5), while the
  same magnitude at a low-frequency position is accepted (cap 7)

#### Scenario: the low-frequency path is unchanged

- **WHEN** an eob ≤ 10 (entirely low-frequency) block is tokenized
- **THEN** the emitted tokens are byte-identical to the prior brick

#### Scenario: a beyond-eob-11 coefficient is rejected

- **WHEN** a nonzero coefficient sits at scan index ≥ 11
- **THEN** the tokenizer returns a typed unsupported-eob error without panicking
