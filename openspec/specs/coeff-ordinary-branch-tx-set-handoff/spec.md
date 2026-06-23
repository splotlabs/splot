# coeff-ordinary-branch-tx-set-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-ordinary-branch-tx-set-handoff`.

## Requirements
### Requirement: Derive ordinary branch txSet from get_tx_set

The decoder SHALL provide a crate-private ordinary coefficient branch handoff
that derives AV2 §5.20.8.3 `txSet` from `txSz`, plane, `is_inter`,
caller-resolved `reduced_tx_set`, caller-resolved `enable_chroma_dctonly`, and
generated §9.2 transform-size conversion tables before delegating to the
existing `Mode_To_Txfm` ordinary branch subset. The handoff SHALL preserve the
all-zero branch behavior and SHALL keep broad `compute_tx_type` and runtime
`coeffs()` unsupported.

#### Scenario: Intra chroma branch derives default intra set

- **WHEN** the handoff receives a non-lossless intra chroma nonzero branch with
  a valid transform size whose §5.20.8.3 branch selects the default intra set
- **THEN** it passes that derived `txSet` to the existing `Mode_To_Txfm` handoff
- **AND** the resulting ordinary branch behavior matches an explicit
  `Mode_To_Txfm` input with the same `txSet`

#### Scenario: Reduced chroma transform set is derived

- **WHEN** the handoff receives intra chroma nonzero input with
  caller-resolved `enable_chroma_dctonly` set
- **THEN** it derives the reduced chroma transform set required by §5.20.8.3
  before the lower `Mode_To_Txfm` wrapper applies its DCT-only short-circuit

#### Scenario: Large intra transform derives DCT-only set

- **WHEN** the handoff receives an intra nonzero branch whose transform-size
  square-up value selects the §5.20.8.3 large-transform DCT-only branch
- **THEN** it passes `TX_SET_DCTONLY` to the existing `Mode_To_Txfm` handoff
- **AND** mapped non-DCT transforms fall back to `DCT_DCT` through the lower
  membership check

#### Scenario: Invalid domains fail atomically

- **WHEN** the handoff receives an invalid `reduced_tx_set` value or an invalid
  transform-size/table domain before delegation
- **THEN** it returns a typed ordinary-branch error before mutating coefficient
  context state, tile CDF rows, or symbol-decoder state

#### Scenario: Runtime scope remains unchanged

- **WHEN** the minimal runtime and existing staged ordinary-branch paths run
- **THEN** they remain no-output-change
- **AND** frame-state derivation, full `compute_tx_type`, luma/inter/lossless
  branches, directional wide-angle mapping, runtime `coeffs()`, dequantization,
  reconstruction, output, and reference refresh remain unsupported
