## ADDED Requirements

### Requirement: local decoder mission intra IST zero support row

The decoder support matrix SHALL include
`DECODE-INTRA-IST-ZERO-FRONTIER` as a distinct partial local decoder mission row named
`intra-ist-zero-frontier`. The row SHALL record that the decoder consumes
the covered `sec_tx_type == 0` intra IST syntax and remains fail-closed for
active secondary transforms and successful local decoder mission decode output.

#### Scenario: Support matrix records intra IST zero frontier

- **WHEN** decoder support status is generated
- **THEN** `intra-ist-zero-frontier` appears with Feature ID
  `DECODE-INTRA-IST-ZERO-FRONTIER`
- **AND** it lists focused CDF/residual tests plus the local decoder mission runtime
  probe
- **AND** it does not claim successful local decoder mission decode, raw/Y4M output, reference
  refresh, or AVM/dav2d byte equality
