## ADDED Requirements

### Requirement: ac0ej3 LR Source-Read Frontier Support Row

The decoder support model SHALL track
`DECODE-AC0EJ3-LR-SOURCE-READ-FRONTIER` as a distinct ac0ej3 support row. The
row SHALL describe that the minimal runtime derives active §7.20.1 source-bound
facts, reaches the §7.20.2 source-read boundary, and still fails closed before
loop-restoration filtering, reconstruction output, or successful ac0ej3 decode.

#### Scenario: Matrix evidence records the source-read boundary

- **WHEN** decoder support status is validated
- **THEN** `ac0ej3-lr-source-read-frontier` remains partial
- **AND** the row lists focused tests proving source-read frontier behavior and
  the live ac0ej3 runtime diagnostic
- **AND** the previous `unsupported_wienerns_lr_source_bounds` reason is no
  longer the live ac0ej3 frontier
