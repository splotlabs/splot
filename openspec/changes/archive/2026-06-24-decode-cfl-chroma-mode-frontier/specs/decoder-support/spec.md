## ADDED Requirements

### Requirement: local decoder mission CfL chroma-mode support row
The decoder support model SHALL track
`DECODE-CFL-CHROMA-MODE-FRONTIER` as a distinct partial local decoder mission row. The
row SHALL describe that the minimal runtime consumes supported AV2 §5.20.5.6
active CfL chroma mode syntax and AV2 §5.20.7.32 CfL alpha syntax while
remaining fail-closed before CfL prediction, chroma reconstruction, loop
restoration, 10-bit output, reference refresh, or successful local decoder mission decode.

#### Scenario: Matrix evidence records the CfL mode boundary
- **WHEN** decoder support status is validated
- **THEN** `cfl-chroma-mode-frontier` appears with Feature ID
  `DECODE-CFL-CHROMA-MODE-FRONTIER`
- **AND** the row cites AV2 §5.20.5.6, §5.20.7.32, §8.3.2, and §9.3
- **AND** it lists focused tests plus the local decoder mission runtime probe
- **AND** it does not claim CfL prediction, decoded chroma samples, loop
  restoration filtering, output, reference refresh, or successful local decoder mission decode
