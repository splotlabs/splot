## ADDED Requirements

### Requirement: FSC coefficient sign pass

The decoder coefficient-loop layer SHALL expose a crate-private FSC/IDTX sign
pass tracked by `DECODE-COEFF-FSC-SIGN-PASS`. The pass SHALL accept a completed
`NonZeroCoeffFscLevelPass`, caller-provided scan table, and caller-resolved q
and transform-size context facts; SHALL iterate checked scan entries
`0..segEob`; SHALL derive and read `idtx_sign` only for entries whose local
`Level[row][col]` is nonzero; SHALL write local `QuantSign[row][col]` to `-1`
for decoded sign `1` and `1` for decoded sign `0` before later sign contexts;
and SHALL NOT run `read_quant`, write nonzero `Quant[]`, commit tile context
lines, invoke reconstruction, or claim runtime `useFsc` support.

#### Scenario: Forward FSC signs are read and written

- **WHEN** a completed FSC level pass has populated local `Level[]`
- **AND** the caller-provided scan table covers `0..segEob`
- **THEN** the sign pass derives `IdtxSign` selectors from current
  `QuantSign[]` and `Level[]`, reads `idtx_sign` only for nonzero levels, and
  writes the resulting local `QuantSign[]` entries in forward scan order

#### Scenario: Evolving sign contexts observe prior signs

- **WHEN** a later nonzero FSC coefficient has a left, above, or above-left
  neighbour whose sign was decoded earlier in the same pass
- **THEN** the later `IdtxSign` selector is derived from the already written
  neighbour `QuantSign[]` value

#### Scenario: Static invalid facts fail before sign reads

- **WHEN** the adjusted block geometry, scan length, or scan-entry position
  facts do not match the local coefficient block
- **THEN** the helper returns a typed coefficient-loop error
- **AND** tile CDF rows, symbol decoder state, and local coefficient sign state
  remain unmodified

#### Scenario: Runtime FSC decode remains unchanged

- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the FSC sign pass yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the FSC/IDTX path
