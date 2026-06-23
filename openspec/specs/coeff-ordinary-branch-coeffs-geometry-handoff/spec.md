# coeff-ordinary-branch-coeffs-geometry-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-ordinary-branch-coeffs-geometry-handoff`.

## Requirements
### Requirement: Handoff ordinary branch coeffs geometry to block geometry
The decoder coefficient-loop boundary SHALL provide a crate-private ordinary
branch handoff, tracked by
`DECODE-COEFF-ORDINARY-BRANCH-COEFFS-GEOMETRY-HANDOFF`, that accepts branch
inputs with `plane`, `startX`, `startY`, `Tx_Width[txSz]`, and
`Tx_Height[txSz]`-style caller facts, derives `AllZeroCoeffBlockInput` with
AV2 § 5.20.7.27 `x4 = startX >> 2`, `y4 = startY >> 2`,
`w4 = Tx_Width[txSz] >> 2`, and `h4 = Tx_Height[txSz] >> 2`, and delegates to
the existing ordinary branch geometry handoff. The handoff SHALL preserve
existing all-zero branch behavior and SHALL NOT derive `Tx_Width[txSz]` or
`Tx_Height[txSz]` from `txSz`, implement `compute_tx_type`, derive scan order,
consume additional symbols beyond the delegated branch, mutate extra CDF rows,
dequantize, reconstruct, or expose a public API.

#### Scenario: Nonzero branch derives block geometry before delegation
- **WHEN** the caller supplies a nonzero ordinary branch input with coeffs
  geometry facts
- **THEN** the handoff derives matching block geometry and returns the same
  branch result as the existing explicit block-geometry ordinary branch input

#### Scenario: Explicit block geometry is no longer accepted at the wrapper
- **WHEN** the caller uses the coeffs-geometry-derived wrapper
- **THEN** the wrapper has no separate `AllZeroCoeffBlockInput` fields that can
  contradict `startX`, `startY`, `Tx_Width[txSz]`, or `Tx_Height[txSz]`

#### Scenario: All-zero branch is unchanged
- **WHEN** the caller supplies an all-zero ordinary branch input with coeffs
  geometry facts
- **THEN** the handoff delegates to the existing all-zero coefficient state path
  with the same derived block geometry
