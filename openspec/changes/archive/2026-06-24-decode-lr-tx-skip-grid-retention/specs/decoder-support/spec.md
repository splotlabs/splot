## ADDED Requirements

### Requirement: local decoder mission LR Tx-Skip Grid Retention Support Row

The decoder support model SHALL track
`DECODE-LR-TX-SKIP-GRID-RETENTION` as a distinct local decoder mission row named
`lr-tx-skip-grid-retention`. The row SHALL record that the decoder has a
value-backed helper for deriving complete boolean `LrTxSkip` storage from parsed
luma transform skip/eob records, while live local decoder mission tile traversal still does not
populate that grid before the current unsupported-feature diagnostic.

#### Scenario: Support matrix lists tx-skip grid retention

- **WHEN** decoder support status is generated
- **THEN** `lr-tx-skip-grid-retention` appears as a partial support row
- **AND** its notes exclude live decoded samples, live tile-populated
  `LrTxSkip`, `FilterClass` grid retention, LR filtering, output, reference
  refresh, AVM/dav2d byte equality, and successful local decoder mission decode
