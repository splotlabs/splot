# coeff-ordinary-branch-tx-class-handoff Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-ordinary-branch-tx-class-handoff`.

## Requirements
### Requirement: Handoff ordinary branch PlaneTxType to txClass
The decoder coefficient-loop boundary SHALL provide a crate-private ordinary
branch handoff, tracked by `DECODE-COEFF-ORDINARY-BRANCH-TX-CLASS-HANDOFF`, that
accepts caller-resolved `PlaneTxType` for the nonzero ordinary coefficient path,
derives AV2 section 8.3.2 `txClass` using the decode-local
`PlaneTxType -> CoeffTransformClass` helper, and delegates to the existing
state-backed ordinary branch. The handoff SHALL preserve the existing all-zero
branch behavior and SHALL NOT implement `compute_tx_type`, derive scan order,
import `splot-recon`, consume additional symbols beyond the delegated branch,
mutate extra CDF rows, dequantize, reconstruct, or expose a public API.

#### Scenario: Nonzero branch derives txClass before delegation
- **WHEN** the caller supplies a nonzero ordinary branch input with a
  caller-resolved `PlaneTxType`
- **THEN** the handoff derives the matching transform class and returns the same
  branch result as the existing explicit `txClass` ordinary branch input

#### Scenario: All-zero branch is unchanged
- **WHEN** the caller supplies an all-zero ordinary branch input
- **THEN** the handoff delegates to the existing all-zero coefficient state path
  without requiring or deriving `PlaneTxType`

#### Scenario: Fallback values remain total
- **WHEN** the caller supplies a nonzero ordinary branch input with a 2D,
  identity, or out-of-range `PlaneTxType`
- **THEN** the handoff uses the AV2 section 8.3.2 fallback two-dimensional class
  and otherwise matches the explicit `txClass` path
