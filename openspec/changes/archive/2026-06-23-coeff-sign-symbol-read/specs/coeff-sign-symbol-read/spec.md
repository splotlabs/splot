## ADDED Requirements

### Requirement: Decode coefficient sign reads
The decoder coefficient-loop boundary SHALL provide a crate-private ordinary
non-FSC sign-read helper tracked by `DECODE-COEFF-SIGN-SYMBOL-READ`. The helper
SHALL accept local `Level[]` state, a checked scan walk, and caller-resolved
sign sources for each entry, SHALL read selected `dc_sign` / `dc_sign_horz_vert`
CDF rows or `sign_bit` literals in scan-walk order, and SHALL return sign
summaries without writing `QuantSign[]`, `Quant[]`, tile context lines, or
reconstruction state.

#### Scenario: Sign sources are read in scan-walk order
- **WHEN** the helper receives matching sign inputs for a checked scan walk
- **THEN** it reads each requested CDF or literal source in the walk order
- **AND** it returns the checked scan entry, local level, source kind, raw symbol
  or literal bit, and boolean sign for each entry
- **AND** entries whose source is disabled return sign false without consuming
  syntax

#### Scenario: Nonzero levels require a sign source
- **WHEN** local `Level[row][col]` is nonzero for an entry
- **THEN** the helper rejects a disabled sign source before consuming any sign
  syntax
- **AND** zero-level entries may still request a sign read for caller-resolved
  parity-hidden behavior

#### Scenario: Invalid reached CDF selectors fail transactionally
- **WHEN** the first reached sign input names an out-of-range `dc_sign` selector
- **THEN** the helper returns a typed selector error before consuming symbol bits
  or mutating CDF rows

#### Scenario: Runtime quantization remains out of scope
- **WHEN** the minimal runtime decode path is exercised after this change
- **THEN** it still does not execute nonzero sign reads
- **AND** it does not write `QuantSign[]` or `Quant[]`, run `read_quant`,
  dequantize, transform, add residuals, reconstruct pixels, or change fixture
  output
