# coeff-ordinary-branch-luma-txtypes-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-ordinary-branch-luma-txtypes-handoff`.

## Requirements
### Requirement: Derive luma TxTypes transform type

The decoder SHALL provide a crate-private ordinary coefficient branch handoff,
tracked by `DECODE-COEFF-ORDINARY-BRANCH-LUMA-TXTYPES-HANDOFF`, that handles
the AV2 section 5.20.7.29 non-lossless luma subset by using a caller-resolved
`TxTypes[blockY][blockX]` value as `PlaneTxType` and delegating to the existing
transform-size/scan ordinary branch.

#### Scenario: Luma TxTypes maps to PlaneTxType

- **WHEN** a nonzero ordinary branch uses luma plane input with a caller-resolved
  `TxTypes` value
- **THEN** the handoff SHALL derive the same `PlaneTxType` as that value
- **AND** the resulting ordinary branch behavior SHALL match an explicit
  `PlaneTxType` input

#### Scenario: Luma TxTypes ignores chroma-only fallback

- **WHEN** a nonzero luma ordinary branch has `enable_chroma_dctonly` set and a
  non-DCT caller-resolved `TxTypes` value
- **THEN** the handoff SHALL use the luma `TxTypes` value rather than the
  chroma-only `DCT_DCT` fallback

#### Scenario: Luma TxTypes remains fail-atomic on invalid domain

- **WHEN** the caller-resolved luma `TxTypes` value is outside the AV2
  `TX_TYPES` domain
- **THEN** the handoff SHALL return a typed ordinary branch error before
  mutating tile coefficient context state, tile CDF rows, or symbol-decoder
  position

### Requirement: Preserve chroma Mode_To_Txfm subset behavior

The luma `TxTypes` extension SHALL preserve the existing all-zero, chroma
non-directional, chroma directional, chroma-DCT-only, chroma inter/lossless
rejection, and transform-set fallback behavior of the staged transform-type
handoff.

#### Scenario: Existing chroma behavior is unchanged

- **WHEN** the handoff receives chroma input
- **THEN** it SHALL continue to derive `PlaneTxType` from the existing chroma
  `Mode_To_Txfm` and directional `wide_angle_mapping` paths

#### Scenario: Chroma unsupported subsets remain rejected

- **WHEN** the handoff receives chroma inter or lossless input outside the
  staged subset
- **THEN** it SHALL keep returning typed unsupported-subset errors before state,
  CDF, or symbol mutation
