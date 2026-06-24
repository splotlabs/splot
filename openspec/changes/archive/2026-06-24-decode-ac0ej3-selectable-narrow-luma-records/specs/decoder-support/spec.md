## ADDED Requirements

### Requirement: ac0ej3 selectable narrow luma-record support row

The decoder support model SHALL track
`DECODE-AC0EJ3-SELECTABLE-NARROW-LUMA-RECORDS` as a distinct partial ac0ej3 row.
The row SHALL describe that the minimal runtime consumes the observed luma-only
`BLOCK_4X32` SDP selectable transform-record subcase in the local ac0ej3 stream
while remaining fail-closed before decoded frame samples, `FilterClass`
retention, loop-restoration filtering/output, reference refresh, or successful
ac0ej3 decode.

#### Scenario: Matrix evidence records the narrow luma boundary

- **WHEN** decoder support status is validated
- **THEN** `ac0ej3-selectable-narrow-luma-records` appears with Feature ID
  `DECODE-AC0EJ3-SELECTABLE-NARROW-LUMA-RECORDS`
- **AND** the row cites AV2 §5.20.6.1, §5.20.6.3, §5.20.7.24, and §5.20.7.27
- **AND** it lists focused tests plus the local ac0ej3 runtime probe
- **AND** it does not claim decoded frame samples, loop-restoration filtering,
  output, reference refresh, AVM/dav2d byte equality, or successful ac0ej3
  decode
