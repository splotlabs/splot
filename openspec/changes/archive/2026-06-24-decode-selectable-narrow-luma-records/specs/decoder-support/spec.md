## ADDED Requirements

### Requirement: local decoder mission selectable narrow luma-record support row

The decoder support model SHALL track
`DECODE-SELECTABLE-NARROW-LUMA-RECORDS` as a distinct partial local decoder mission row.
The row SHALL describe that the minimal runtime consumes the observed luma-only
`BLOCK_4X32` SDP selectable transform-record subcase in the local decoder mission stream
while remaining fail-closed before decoded frame samples, `FilterClass`
retention, loop-restoration filtering/output, reference refresh, or successful
local decoder mission decode.

#### Scenario: Matrix evidence records the narrow luma boundary

- **WHEN** decoder support status is validated
- **THEN** `selectable-narrow-luma-records` appears with Feature ID
  `DECODE-SELECTABLE-NARROW-LUMA-RECORDS`
- **AND** the row cites AV2 §5.20.6.1, §5.20.6.3, §5.20.7.24, and §5.20.7.27
- **AND** it lists focused tests plus the local decoder mission runtime probe
- **AND** it does not claim decoded frame samples, loop-restoration filtering,
  output, reference refresh, AVM/dav2d byte equality, or successful local decoder mission
  decode
