# coeff-scan-walk Specification

## Purpose

Define the completed OpenSpec requirements for `coeff-scan-walk`.

## Requirements
### Requirement: Decode coefficient scan walk
The decoder coefficient loop SHALL provide a crate-private helper, tracked by
`DECODE-COEFF-SCAN-WALK`, that walks the ordinary non-FSC AV2 § 5.20.7.27
nonzero coefficient scan window from `c = eob - 1` down to `0` over caller
supplied `scan[c]` positions. The helper SHALL NOT derive `get_scan`, SHALL NOT
import `splot-recon`, SHALL NOT consume symbols or CDF rows, and SHALL NOT write
nonzero coefficient values.

#### Scenario: Reverse scan entries are exposed
- **WHEN** a nonzero coefficient block start has `eob = 4` and the caller supplies
  at least four scan positions within the initialized block extent
- **THEN** the helper returns checked entries for scan indexes `3`, `2`, `1`, and
  `0`, each with raster position and row/column facts
- **AND** no symbol decoder state, CDF row, tile coefficient context line, or
  coefficient value is mutated

#### Scenario: EOB beyond scan length is rejected before traversal
- **WHEN** a nonzero coefficient block start has an EOB larger than the supplied
  scan slice length
- **THEN** the helper returns a typed coefficient-loop error
- **AND** it does not consume symbols, mutate CDF rows, or write coefficient state

#### Scenario: Scan position outside the block is rejected
- **WHEN** a visited scan position is outside the initialized
  `TransformCoeffBlockState` extent
- **THEN** the helper returns a typed coefficient-loop error that names the
  offending scan index, position, and coefficient count
- **AND** it does not consume symbols, mutate CDF rows, or write coefficient state
