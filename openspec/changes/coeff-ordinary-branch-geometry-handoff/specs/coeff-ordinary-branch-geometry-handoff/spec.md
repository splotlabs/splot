## ADDED Requirements

### Requirement: Handoff ordinary branch block geometry to state context
The decoder coefficient-loop boundary SHALL provide a crate-private ordinary
branch handoff, tracked by `DECODE-COEFF-ORDINARY-BRANCH-GEOMETRY-HANDOFF`, that
accepts nonzero branch inputs with state-context CDF facts only, derives
state-context `x4`, `y4`, `w4`, and `h4` from
`NonZeroCoeffBlockStartInput.block`, and delegates to the existing
`plane_type` ordinary branch. The handoff SHALL preserve existing all-zero
branch behavior and SHALL NOT derive raw `startX`/`startY`/`txSz`, implement
`compute_tx_type`, derive scan order, consume additional symbols beyond the
delegated branch, mutate extra CDF rows, dequantize, reconstruct, or expose a
public API.

#### Scenario: Nonzero branch derives state-context geometry before delegation
- **WHEN** the caller supplies a nonzero ordinary branch input with block-start
  geometry
- **THEN** the handoff derives the matching state-context geometry and returns
  the same branch result as the existing explicit-geometry ordinary branch input

#### Scenario: Mismatched explicit geometry is no longer accepted at the wrapper
- **WHEN** the caller uses the geometry-derived wrapper
- **THEN** the wrapper has no separate nonzero state-context `x4`, `y4`, `w4`,
  or `h4` fields that can contradict the block-start geometry

#### Scenario: All-zero branch is unchanged
- **WHEN** the caller supplies an all-zero ordinary branch input
- **THEN** the handoff delegates to the existing all-zero coefficient state path
  without requiring or deriving nonzero state-context geometry
