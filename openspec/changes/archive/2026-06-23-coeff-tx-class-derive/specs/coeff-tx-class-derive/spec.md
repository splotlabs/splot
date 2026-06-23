## ADDED Requirements

### Requirement: Derive coefficient transform class from PlaneTxType
The decoder coefficient-loop boundary SHALL provide a crate-private helper,
tracked by `DECODE-COEFF-TX-CLASS-DERIVE`, that maps caller-resolved
`PlaneTxType` values to the AV2 § 8.3.2 `get_tx_class` result used by ordinary
coefficient syntax. The helper SHALL be total over all integer inputs, SHALL map
`V_DCT`, `V_ADST`, and `V_FLIPADST` to vertical class, SHALL map `H_DCT`,
`H_ADST`, and `H_FLIPADST` to horizontal class, and SHALL map every other value
to two-dimensional class. The helper SHALL NOT implement `compute_tx_type`,
derive scan order, import `splot-recon`, consume symbols or CDF rows, mutate
coefficient state, dequantize, reconstruct, or expose a public API.

#### Scenario: Directional transform classes are derived
- **WHEN** the caller supplies a `PlaneTxType` value for one of the AV2 vertical
  or horizontal transform-only types
- **THEN** the helper returns the corresponding vertical or horizontal ordinary
  coefficient transform class

#### Scenario: Non-directional and out-of-range values are total
- **WHEN** the caller supplies a 2D, identity, or out-of-range `PlaneTxType`
  value
- **THEN** the helper returns the two-dimensional transform class according to
  the AV2 § 8.3.2 fallback branch

### Requirement: Feed derived transform class into maxLevel derivation
The decoder coefficient-loop boundary SHALL provide a crate-private handoff
that accepts caller-resolved `PlaneTxType`, derives `txClass`, and delegates to
the existing ordinary non-FSC `maxLevel` derivation. The handoff SHALL preserve
the existing checked scan-walk ordering and output shape, and SHALL NOT derive
`PlaneTxType`, scan order, hidden parity, TCQ, lossless state, or runtime block
syntax facts.

#### Scenario: PlaneTxType handoff matches direct txClass path
- **WHEN** the caller supplies a checked scan walk plus `PlaneTxType` values that
  map to each transform class
- **THEN** the handoff returns the same `maxLevel` records as the existing
  direct `txClass` configuration path

#### Scenario: Handoff does not touch mutable decode state
- **WHEN** the handoff derives `txClass` and max-level records
- **THEN** it does not consume symbols, mutate CDF rows, update tile coefficient
  context lines, or write local `Level[]` / `Quant[]` state
