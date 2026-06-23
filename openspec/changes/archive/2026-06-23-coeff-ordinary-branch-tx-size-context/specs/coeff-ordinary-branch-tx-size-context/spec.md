## ADDED Requirements

### Requirement: Transform-size context handoff
The ordinary coefficient branch `txSz`-dimensions wrapper SHALL derive
`txSzCtx` from generated AV2 section 9.2 conversion tables using the AV2 section
5.20.7.27 formula
`(Tx_Size_Sqr[txSz] + Tx_Size_Sqr_Up[txSz] + 1) >> 1`, and SHALL feed that value
to the ordinary base-context pass.

#### Scenario: Rectangular transform sizes derive context

- **WHEN** the wrapper handles a nonzero branch with a rectangular transform
  size
- **THEN** ordinary base-row selection receives the `txSzCtx` derived from
  generated `Tx_Size_Sqr` and `Tx_Size_Sqr_Up` values
- **AND** raw dimensions still drive block geometry and EOB-size context
- **AND** adjusted dimensions still drive base-context geometry

#### Scenario: Invalid square table values fail atomically

- **WHEN** `Tx_Size_Sqr[txSz]` or `Tx_Size_Sqr_Up[txSz]` is missing or maps
  outside the generated transform-size conversion tables
- **THEN** the wrapper fails with a typed ordinary branch error before mutating
  tile coefficient context state, CDF rows, or symbol-decoder state

#### Scenario: Runtime coefficient wiring stays deferred

- **WHEN** `txSzCtx` becomes available to the loaded ordinary branch
- **THEN** no runtime `coeffs()` call site, `compute_tx_type`, scan derivation,
  dequantization, reconstruction, output, or reference refresh behavior changes
  in this feature
