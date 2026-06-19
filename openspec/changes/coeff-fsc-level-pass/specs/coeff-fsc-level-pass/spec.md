## ADDED Requirements

### Requirement: FSC coefficient level pass

The decoder coefficient-loop layer SHALL expose a crate-private FSC/IDTX level
pass tracked by `DECODE-COEFF-FSC-LEVEL-PASS`. The pass SHALL accept a nonzero
coefficient block start, a checked `FscCoeffScanWalk`, and caller-resolved q and
transform-size context facts; SHALL read `coeff_base_bob` for the first
`bob..segEob` entry, `coeff_base_idtx` for later entries, and conditional
`coeff_br_idtx` when the decoded level exceeds `NUM_BASE_LEVELS`; SHALL write
the resulting levels into local `Level[]` state in forward scan order; and SHALL
NOT read IDTX signs, run `read_quant`, write `QuantSign[]` or `Quant[]`, commit
tile context lines, invoke reconstruction, or claim runtime `useFsc` support.

#### Scenario: Forward FSC levels are read and written

- **WHEN** decoded EOB and `segEob` define a checked `bob..segEob` walk
- **AND** the local coefficient block geometry matches the caller-resolved
  adjusted transform size
- **THEN** the pass reads one `coeff_base_bob` symbol for the first entry,
  `coeff_base_idtx` symbols for later entries, optional `coeff_br_idtx` symbols
  after `NUM_BASE_LEVELS`, and writes each resulting `Level[row][col]`

#### Scenario: Static invalid facts fail before reads

- **WHEN** the checked walk cardinality, adjusted block geometry, or entry
  position facts do not match the local coefficient block
- **THEN** the helper returns a typed coefficient-loop error
- **AND** tile CDF rows, symbol decoder state, and local coefficient state remain
  unmodified

#### Scenario: Runtime FSC decode remains unchanged

- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the FSC level pass yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the FSC/IDTX path
