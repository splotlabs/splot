# coeff-ordinary-branch-chroma-inter-txtypes-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-ordinary-branch-chroma-inter-txtypes-handoff`.

## Requirements
### Requirement: Derive chroma-inter TxTypes transform type

The decoder SHALL provide a crate-private ordinary coefficient branch handoff,
tracked by `DECODE-COEFF-ORDINARY-BRANCH-CHROMA-INTER-TXTYPES-HANDOFF`, that
handles the AV2 section 5.20.7.29 non-lossless chroma-inter subset by using a
caller-resolved `TxTypes[y4][x4]` value and the AV2
`Tx_Type_In_Set_Inter[txSet][txType]` membership table before delegating to the
existing transform-size/scan ordinary branch.

#### Scenario: Chroma-inter TxTypes maps when present in txSet

- **WHEN** a nonzero ordinary branch uses chroma plane input with `is_inter`,
  `enable_chroma_dctonly == false`, and a caller-resolved `TxTypes` value that
  is present in the caller-resolved inter transform set
- **THEN** the handoff SHALL derive the same `PlaneTxType` as that value
- **AND** the resulting ordinary branch behavior SHALL match an explicit
  `PlaneTxType` input

#### Scenario: Chroma-inter TxTypes falls back to DCT_DCT when absent from txSet

- **WHEN** a nonzero chroma-inter ordinary branch has a caller-resolved `TxTypes`
  value that is outside `Tx_Type_In_Set_Inter[txSet]`
- **THEN** the handoff SHALL derive `PlaneTxType = DCT_DCT`
- **AND** the resulting ordinary branch behavior SHALL match an explicit
  `DCT_DCT` input

#### Scenario: Chroma-inter TxTypes remains fail-atomic on invalid domain

- **WHEN** the caller-resolved chroma-inter `TxTypes` value is outside the AV2
  `TX_TYPES` domain
- **THEN** the handoff SHALL return a typed ordinary branch error before
  mutating tile coefficient context state, tile CDF rows, or symbol-decoder
  position

#### Scenario: Chroma-inter txSet remains fail-atomic on invalid domain

- **WHEN** the caller-resolved `txSet` value is outside the AV2
  `Tx_Type_In_Set_Inter` domain for chroma-inter input
- **THEN** the handoff SHALL return a typed ordinary branch error before
  mutating tile coefficient context state, tile CDF rows, or symbol-decoder
  position

### Requirement: Preserve existing transform-type subset behavior

The chroma-inter `TxTypes` extension SHALL preserve the existing all-zero, luma
`TxTypes`, chroma-DCT-only, chroma intra non-directional, chroma intra
directional, lossless rejection, and transform-set fallback behavior of the
staged transform-type handoff.

#### Scenario: Existing luma and chroma-intra behavior is unchanged

- **WHEN** the handoff receives luma input, chroma intra input, or chroma input
  with `enable_chroma_dctonly`
- **THEN** it SHALL continue to derive `PlaneTxType` from the existing luma
  `TxTypes`, chroma `Mode_To_Txfm`, directional `wide_angle_mapping`, and
  chroma-DCT-only paths

#### Scenario: Lossless unsupported subset remains rejected

- **WHEN** the handoff receives lossless input outside the staged subset
- **THEN** it SHALL keep returning a typed unsupported-subset error before
  state, CDF, or symbol mutation
