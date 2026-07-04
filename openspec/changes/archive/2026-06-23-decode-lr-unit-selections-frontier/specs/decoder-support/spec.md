## ADDED Requirements

### Requirement: local decoder mission LR Unit Selection Frontier Support Row

The decoder support model SHALL track
`DECODE-LR-UNIT-SELECTIONS-FRONTIER` as a distinct local decoder mission support row.
The row SHALL describe that the minimal runtime's traversal boundary retains
supported frame-level Wiener NS LR-unit selections, including plane, unit row,
unit column, and active/inactive state. The row SHALL NOT claim active
loop-restoration filtering, 10-bit reconstruction/output, reference refresh,
raw/Y4M output, or successful local decoder mission decode.

#### Scenario: Matrix evidence records the narrow selection boundary

- **WHEN** decoder support status is validated
- **THEN** `lr-unit-selections-frontier` remains partial
- **AND** the row lists tests proving inactive, active, and multi-unit selection
  retention
- **AND** the live local decoder mission runtime diagnostic remains fail-closed before output
