## ADDED Requirements

### Requirement: non-EOB data-dependent coeff_br base-range tier

The encoder coefficient tokenizer SHALL emit the AV2 §5.20.7.27 `coeff_br`
base-range token for the non-EOB coefficient of a general low-frequency 4x4 DCT_DCT
luma block when its magnitude exceeds the base tier, allowing the non-EOB
coefficient magnitude 1..=7 (so both coefficients of an eob<=2 block span 1..=7).
The `coeff_br` context SHALL be derived from the running `Level[]` by mirroring the
decoder `CoeffBrContext::ctx` — the first-`num` `SIG_REF_DIFF_OFFSET` neighbour sum
(each clamped to `MAX_BASE_BR_RANGE - 1`), `mag = Min((sum + 1) >> 1, 6)`, the 2D LF
luma mapping — using the already-imported splot-core offset table (no hand-coded
table). The recovery helper SHALL reconstruct each base-pass coefficient's level
including its interleaved `coeff_br`. This is a private, non-emitting stage tracked
by `ENC-COEFF-GENERAL-WALK-DC-BR`; it does not code golomb magnitudes, eob > 2,
high-frequency or chroma coefficients, or produce packets.

#### Scenario: a non-EOB DC above the base tier emits a data-dependent coeff_br

- **WHEN** the non-EOB DC has magnitude 5, 6, or 7 with an EOB AC neighbour
- **THEN** a `coeff_br` token follows its `coeff_base` with the context derived from
  the AC neighbour magnitude (1-2 -> ctx 1, 3-4 -> ctx 2, 5-7 -> ctx 3) and symbol
  `magnitude - 5`
- **AND** the roundtrip recovers the exact signed magnitude

#### Scenario: both coefficients carry coeff_br

- **WHEN** both the EOB and non-EOB coefficients have magnitude > 4
- **THEN** each emits its `coeff_br` (the EOB at its constant context, the non-EOB at
  its data-dependent context) and the roundtrip recovers both exactly

#### Scenario: out-of-scope magnitude is rejected

- **WHEN** any coefficient magnitude exceeds 7
- **THEN** the tokenizer returns a typed unsupported-magnitude error without panicking
