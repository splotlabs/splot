## ADDED Requirements

### Requirement: Adjusted transform-size base-context handoff
The ordinary coefficient branch `txSz`-dimensions wrapper SHALL derive
`Adjusted_Tx_Size[txSz]` from generated AV2 section 9.2 conversion tables and
SHALL use `Tx_Width_Log2[Adjusted_Tx_Size[txSz]]`,
`Tx_Width[Adjusted_Tx_Size[txSz]]`, and
`Tx_Height[Adjusted_Tx_Size[txSz]]` for the section 8.3.2 ordinary base-context
pass. The wrapper SHALL continue to use raw `Tx_Width[txSz]` and
`Tx_Height[txSz]` for section 5.20.7.27 block geometry, and raw
`Tx_Width_Log2[txSz]` and `Tx_Height_Log2[txSz]` for nonzero EOB-size context
selection.

#### Scenario: Adjusted dimensions feed base contexts
- **WHEN** the wrapper handles a nonzero branch with a transform size whose
  adjusted transform size differs from the raw transform size
- **THEN** the base-context pass receives the adjusted width, height, and
  width-log2 values derived from generated section 9.2 tables
- **AND** the nonzero EOB context and block geometry still receive raw transform
  dimensions

#### Scenario: Invalid adjusted table values fail atomically
- **WHEN** `Adjusted_Tx_Size[txSz]` is missing or maps outside the generated
  transform-size conversion tables
- **THEN** the wrapper fails with a typed ordinary branch error before mutating
  tile coefficient context state, CDF rows, or symbol-decoder state

#### Scenario: Runtime coefficient wiring stays deferred
- **WHEN** adjusted dimensions become available to the loaded ordinary branch
- **THEN** no runtime `coeffs()` call site, `txSzCtx` derivation,
  `compute_tx_type`, scan derivation, dequantization, reconstruction, output, or
  reference refresh behavior changes in this feature
