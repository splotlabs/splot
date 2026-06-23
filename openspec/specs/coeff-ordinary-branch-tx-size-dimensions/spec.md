# coeff-ordinary-branch-tx-size-dimensions Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-ordinary-branch-tx-size-dimensions`.

## Requirements
### Requirement: Handoff ordinary branch tx size dimensions
The decoder coefficient-loop boundary SHALL provide a crate-private ordinary
branch handoff, tracked by `DECODE-COEFF-ORDINARY-BRANCH-TX-SIZE-DIMENSIONS`,
that accepts `plane`, `startX`, `startY`, and `txSz`, derives
`Tx_Width[txSz]`, `Tx_Height[txSz]`, `Tx_Width_Log2[txSz]`, and
`Tx_Height_Log2[txSz]` from generated AV2 section 9.2 conversion tables, and
delegates to the existing ordinary branch `coeffs()` geometry handoff. The
handoff SHALL preserve existing all-zero branch behavior and SHALL NOT derive
`Tx_Size_Sqr[txSz]`, `Tx_Size_Sqr_Up[txSz]`, `txSzCtx`,
`Adjusted_Tx_Size[txSz]`, `compute_tx_type`, scan order, dequantization,
reconstruction, or a public API.

#### Scenario: Nonzero branch derives dimensions before delegation
- **WHEN** the caller supplies a nonzero ordinary branch input with `txSz`
- **THEN** the handoff derives matching width, height, width log2, and height
  log2 facts from the generated conversion tables
- **AND** it returns the same branch result as the existing ordinary branch input
  with explicit `coeffs()` geometry and transform-size dimension facts

#### Scenario: Explicit dimensions are no longer accepted at the wrapper
- **WHEN** the caller uses the tx-size-dimension-derived wrapper
- **THEN** the wrapper has no separate `Tx_Width[txSz]`, `Tx_Height[txSz]`,
  `Tx_Width_Log2[txSz]`, or `Tx_Height_Log2[txSz]` fields that can contradict
  `txSz`

#### Scenario: Invalid tx size is fail atomic
- **WHEN** the caller supplies a `txSz` index outside the generated table bounds
- **THEN** the handoff rejects the input before mutating coefficient context
  state, tile CDF rows, or symbol-decoder state

#### Scenario: All-zero branch is unchanged
- **WHEN** the caller supplies an all-zero ordinary branch input with `txSz`
- **THEN** the handoff delegates to the existing all-zero coefficient state path
  with the same derived block geometry
