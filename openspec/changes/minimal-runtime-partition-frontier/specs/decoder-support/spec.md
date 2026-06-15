## ADDED Requirements

### Requirement: Minimal Runtime Partition Frontier Integration
The decoder support model SHALL record that the
`DECODE-MINIMAL-TIER-RUNTIME-SUCCESS` minimal hash/Y4M runtime consumes the
`DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY` root partition frontier before the
remaining traced flat-tile symbols. The support evidence SHALL keep
`tile-payload-decode`, broad `symbol-decoder`, CDF lifecycle, `decode_block()`,
`MiSizes` mutation, reconstruction, and reference-refresh work partial while
they remain outside the supported tier.

#### Scenario: Runtime bridge is recorded without broad decode overclaim
- **WHEN** decoder support/status checks run after this change
- **THEN** the minimal runtime support row names the partition-frontier bridge
  as evidence
- **AND** `tile-partition-traversal-boundary` remains the supported row for the
  first `decode_block()` frontier
- **AND** `tile-payload-decode` remains partial for full `decode_tile()`,
  `decode_block()` syntax, `MiSizes` mutation, reconstruction, output expansion,
  CDF lifecycle, and reference refresh work

#### Scenario: Public outputs stay in the same minimal tier
- **WHEN** the committed minimal fixture is decoded through hash and Y4M runtime
  entry points
- **THEN** output bytes remain unchanged
- **AND** the only public success tier remains
  `minimal-intra-8bit420-hash-v1`
