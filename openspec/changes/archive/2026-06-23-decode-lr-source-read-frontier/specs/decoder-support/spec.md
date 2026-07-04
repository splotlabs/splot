## ADDED Requirements

### Requirement: local decoder mission LR Source-Read Frontier Support Row

The decoder support model SHALL track
`DECODE-LR-SOURCE-READ-FRONTIER` as a distinct local decoder mission support row. The
row SHALL describe that the minimal runtime derives active §7.20.1 source-bound
facts, rejects the local decoder mission two-class luma bank at the §7.20.4
pixel-classified Wiener boundary, and still fails closed before source sample
value reads, loop-restoration filtering, reconstruction output, or successful
local decoder mission decode. The row SHALL also describe the non-classified source-read
derivation that resolves §7.20.2 output/tap/chroma-luma coordinates.

#### Scenario: Matrix evidence records the source-read boundary

- **WHEN** decoder support status is validated
- **THEN** `lr-source-read-frontier` remains partial
- **AND** the row lists focused tests proving source-read frontier behavior,
  Wiener tap/luma-source coverage, source-read limit accounting, classified
  Wiener ordering, and the live local decoder mission runtime diagnostic
- **AND** the previous `unsupported_wienerns_lr_source_bounds` reason is no
  longer the live local decoder mission frontier
