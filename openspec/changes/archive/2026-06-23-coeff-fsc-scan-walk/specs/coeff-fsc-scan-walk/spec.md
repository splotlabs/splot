## ADDED Requirements

### Requirement: FSC coefficient scan walk

The decoder coefficient-loop layer SHALL expose a crate-private FSC/IDTX scan
walk boundary tracked by `DECODE-COEFF-FSC-SCAN-WALK`. The boundary SHALL accept
a nonzero coefficient block start, caller-resolved `segEob`, and caller-resolved
`scan[c]`, SHALL derive `bob = segEob - eob`, SHALL return checked entries in
forward `bob..segEob` order, and SHALL NOT consume symbols, mutate CDF rows,
write coefficient state, or claim runtime `useFsc` support.

#### Scenario: Forward FSC scan entries are returned

- **WHEN** decoded EOB is positive and fits caller-resolved `segEob`
- **AND** `segEob` fits the supplied scan table
- **AND** every visited scan position fits the initialized coefficient block
- **THEN** the walk returns `bob`, `segEob`, and forward `CoeffScanEntry`
  records for scan indices `bob` through `segEob - 1`

#### Scenario: Invalid FSC scan facts are rejected

- **WHEN** decoded EOB exceeds `segEob`, `segEob` exceeds the scan table length,
  or a visited scan position exceeds the local coefficient block
- **THEN** the helper returns a typed coefficient-loop context error
- **AND** the nonzero start block, tile CDF rows, symbol decoder state, and tile
  coefficient context state remain unmodified

#### Scenario: Runtime FSC decode remains unchanged

- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the FSC scan walk yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the FSC/IDTX path
