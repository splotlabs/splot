## ADDED Requirements

### Requirement: FSC branch segment extent handoff

The decoder SHALL provide a crate-private loaded-but-unwired FSC/IDTX coefficient
branch handoff that derives `segEob` from the caller-resolved scan window length
before delegating to the lower-level FSC branch.

#### Scenario: Derived segment extent matches explicit branch

- **WHEN** the handoff receives a nonzero FSC branch input with a caller-resolved
  scan window
- **THEN** it derives `segEob` from that scan window length and returns the same
  pass, CDF state, symbol state, and tile context state as the explicit
  `apply_coeff_fsc_branch` path using that `segEob`

#### Scenario: All-zero routing remains fail-atomic

- **WHEN** the handoff receives all-zero routing
- **THEN** it rejects the input before reading EOB syntax, mutating CDF rows,
  mutating tile context state, or consuming symbols

#### Scenario: Non-luma routing remains fail-atomic

- **WHEN** the handoff receives a nonzero FSC input whose context plane is not
  luma
- **THEN** it rejects the input before reading EOB syntax, mutating CDF rows,
  mutating tile context state, or consuming symbols

#### Scenario: Short scan fails before FSC symbol reads

- **WHEN** the derived `segEob` from the scan length is smaller than the decoded
  nonzero EOB value
- **THEN** the handoff reports a checked scan/segment error after the EOB branch
  and before any FSC level, sign, or quant symbol reads

### Requirement: FSC segment handoff scope remains partial

The decoder SHALL NOT claim runtime `coeffs()` support or output changes from
the FSC segment handoff alone.

#### Scenario: Runtime integration remains deferred

- **WHEN** the handoff is implemented
- **THEN** runtime `useFsc` derivation, scan derivation, transform facts,
  dequantization, inverse transform, residual add, reconstruction, and reference
  refresh remain outside the supported scope
