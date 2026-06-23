# coeff-ordinary-branch-scan-order Specification

## Purpose

Capture the completed OpenSpec requirements synchronized for `coeff-ordinary-branch-scan-order`.

## Requirements
### Requirement: Transform scan-order handoff
The ordinary coefficient branch `txSz`-dimensions wrapper SHALL derive the AV2
section 5.20.7.30 scan order from `txSz` and `txClass` using the section
5.20.7.27 sequence `txClass = get_tx_class(PlaneTxType)` followed by
`scan = get_scan(txSz, txClass)`, and SHALL feed that scan order to the existing
ordinary pass.

#### Scenario: Two-dimensional scan is derived

- **WHEN** the wrapper handles a nonzero branch whose `PlaneTxType` maps to
  `TX_CLASS_2D`
- **THEN** the ordinary pass receives the section 5.20.7.30 anti-diagonal scan
  order derived from `Tx_Width[txSz]` and `Tx_Height[txSz]`
- **AND** raw dimensions still drive block geometry and EOB-size context
- **AND** adjusted dimensions still drive base-context geometry

#### Scenario: Directional scan is derived

- **WHEN** the wrapper handles nonzero branches whose `PlaneTxType` maps to
  horizontal or vertical transform classes
- **THEN** the ordinary pass receives the section 5.20.7.30 column-major or
  row-major scan order for that transform class
- **AND** callers of the `txSz` wrapper no longer provide a scan slice

#### Scenario: Invalid scan shapes fail atomically

- **WHEN** generated transform-size dimensions for scan derivation map outside
  the supported section 5.20.7.30 scan extent
- **THEN** the wrapper fails with a typed ordinary branch error before mutating
  tile coefficient context state, CDF rows, or symbol-decoder state

#### Scenario: Runtime coefficient wiring stays deferred

- **WHEN** scan order becomes available to the loaded ordinary branch
- **THEN** no runtime `coeffs()` call site, `compute_tx_type`, dequantization,
  reconstruction, output, or reference refresh behavior changes in this feature
