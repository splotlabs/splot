## ADDED Requirements

### Requirement: ac0ej3 CfL chroma-mode support row
The decoder support model SHALL track
`DECODE-AC0EJ3-CFL-CHROMA-MODE-FRONTIER` as a distinct partial ac0ej3 row. The
row SHALL describe that the minimal runtime consumes supported AV2 §5.20.5.6
active CfL chroma mode syntax and AV2 §5.20.7.32 CfL alpha syntax while
remaining fail-closed before CfL prediction, chroma reconstruction, loop
restoration, 10-bit output, reference refresh, or successful ac0ej3 decode.

#### Scenario: Matrix evidence records the CfL mode boundary
- **WHEN** decoder support status is validated
- **THEN** `ac0ej3-cfl-chroma-mode-frontier` appears with Feature ID
  `DECODE-AC0EJ3-CFL-CHROMA-MODE-FRONTIER`
- **AND** the row cites AV2 §5.20.5.6, §5.20.7.32, §8.3.2, and §9.3
- **AND** it lists focused tests plus the local ac0ej3 runtime probe
- **AND** it does not claim CfL prediction, decoded chroma samples, loop
  restoration filtering, output, reference refresh, or successful ac0ej3 decode
