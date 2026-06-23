# coeff-ordinary-branch-lossless-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-ordinary-branch-lossless-handoff`.

## Requirements
### Requirement: Derive lossless ordinary branch DCT_DCT before txSet

The decoder SHALL provide a crate-private ordinary coefficient branch handoff
that handles the AV2 §5.20.7.29 nonzero `Lossless` branch which selects
`DCT_DCT` before delegating to any AV2 §5.20.8.3 `txSet` or `Mode_To_Txfm`
logic. The handoff SHALL preserve the all-zero branch behavior and SHALL keep
full `compute_tx_type` and runtime `coeffs()` unsupported.

#### Scenario: Lossless nonzero branch selects DCT_DCT

- **WHEN** the handoff receives a nonzero lossless branch whose caller-resolved
  facts select the staged `DCT_DCT` lossless outcome
- **THEN** it passes `PlaneTxType = DCT_DCT` to the existing transform-size
  dimensions handoff
- **AND** the resulting ordinary branch behavior matches an explicit
  transform-size dimensions input with `DCT_DCT`

#### Scenario: Lossless short-circuits lower non-lossless validation

- **WHEN** the handoff receives a nonzero lossless branch with lower
  `txSet`/`Mode_To_Txfm` facts that would be invalid on a non-lossless path
- **THEN** it resolves the staged lossless `DCT_DCT` branch before lower
  non-lossless validation
- **AND** coefficient context state, tile CDF rows, and symbol-decoder
  progression match the explicit `DCT_DCT` path

#### Scenario: Non-lossless branch delegates to txSet handoff

- **WHEN** the handoff receives a nonzero non-lossless branch
- **THEN** it delegates to the existing AV2 §5.20.8.3 `txSet` ordinary-branch
  handoff
- **AND** the resulting ordinary branch behavior matches direct `txSet` input

#### Scenario: Invalid transform size fails atomically

- **WHEN** the lossless branch receives an invalid transform-size/table domain
- **THEN** it returns a typed ordinary-branch error before mutating coefficient
  context state, tile CDF rows, or symbol-decoder state

#### Scenario: Unsupported lossless inter branch fails atomically

- **WHEN** the lossless handoff receives a nonzero inter branch
- **THEN** it returns a typed ordinary-branch error before mutating coefficient
  context state, tile CDF rows, or symbol-decoder state
- **AND** IDTX and `TxTypes` lossless inter handling remain unsupported

#### Scenario: Runtime scope remains unchanged

- **WHEN** the minimal runtime and existing staged ordinary-branch paths run
- **THEN** they remain no-output-change
- **AND** FSC/IDTX lossless cases, inter/luma transform-state lookup,
  directional wide-angle mapping, frame-state parsing, runtime `coeffs()`,
  dequantization, reconstruction, output, and reference refresh remain
  unsupported
