## ADDED Requirements

### Requirement: State-derived ordinary base and level first pass
The decoder SHALL provide a crate-private ordinary non-FSC coefficient first-pass
helper that derives base/base-range CDF selectors from checked scan entries,
current local `Level[]`, and first-pass TCQ state while producing decoded
`Level[]` values.

#### Scenario: Derived first pass writes levels in scan order
- **GIVEN** a decoded nonzero EOB block start, a checked scan walk,
  caller-resolved ordinary non-FSC block facts, and tile CDF/symbol state
- **WHEN** the first-pass helper runs
- **THEN** it derives the first entry's `coeff_base_eob` selector from the scan
  entry and block facts
- **AND** it derives later `coeff_base` selectors from the current local
  `Level[]` state before each read
- **AND** it derives and reads `coeff_br` only when the decoded level exceeds the
  selected base-level threshold and the low-frequency chroma exception does not
  apply
- **AND** it writes each decoded level to local `Level[]` before deriving the
  next entry's selector
- **AND** it returns the base-read summaries, final local block state, and
  first-pass `sumAbs1`, `numNz`, `isHidden`, and `tcqState` summary

#### Scenario: TCQ and hidden summaries follow the first pass
- **GIVEN** a luma ordinary non-FSC block with caller-resolved TCQ or parity
  hiding enabled
- **WHEN** the first-pass helper reads decoded levels
- **THEN** luma `coeff_base` selectors use `(tcqState >> 1) & 1` before the
  current level updates TCQ state when TCQ is enabled
- **AND** `sumAbs1`, `numNz`, and `isHidden` update only through the § 5.20.7.27
  parity-hiding first-pass rules for `c > 0` when parity hiding is enabled

#### Scenario: Parity-hidden base row remains unsupported until loaded
- **GIVEN** selector derivation reaches the parity-hidden-only
  `TileCoeffBasePhCdf` bank
- **WHEN** the first-pass helper attempts to derive the base selector
- **THEN** it returns a typed unsupported boundary instead of reading a missing
  CDF row

#### Scenario: Runtime decode remains unchanged
- **WHEN** the existing minimal decode runtime runs
- **THEN** it does not call the state-derived first-pass helper yet
- **AND** existing fixture output and unsupported-feature diagnostics remain
  unchanged until a later runtime `coeffs()` integration change wires real block
  facts into the helper
