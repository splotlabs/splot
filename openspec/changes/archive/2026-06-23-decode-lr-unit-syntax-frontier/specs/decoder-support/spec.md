## MODIFIED Requirements

### Requirement: Tile Partition Traversal Support Row
The decoder support model SHALL track `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`
as a distinct crate-private row named `tile-partition-traversal-boundary`. The
row SHALL mark only the partition traversal frontier to the first
`decode_block()` boundary plus the narrow frame-level Wiener NS LR unit syntax
frontier tracked by `DECODE-LR-UNIT-SYNTAX-FRONTIER` as supported, and
SHALL keep broader `tile-payload-decode`, `symbol-decoder`, CDF lifecycle,
runtime decode output, block syntax, `MiSizes` mutation, loop-restoration
filtering, and reconstruction rows honest when they remain partial.

#### Scenario: Traversal row is supported without broad decode overclaim
- **WHEN** the decoder support matrix is regenerated after this change
- **THEN** `tile-partition-traversal-boundary` appears with Feature ID
  `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`
- **AND** it cites AV2 §5.20.3.1, §5.20.3.2, §5.20.9.1, §5.20.10.4,
  §5.20.10.5, §8.3.2, and §9.2 as applicable evidence sections
- **AND** it identifies §5.20.10.6 per-unit coefficient parsing,
  loop-restoration filtering, and reconstruction as outside this boundary
- **AND** it does not cite §5.20.4.1 as parsed or tested evidence while block
  syntax remains outside this boundary
- **AND** `tile-payload-decode` remains partial for full `decode_tile()`, block
  syntax, `MiSizes` mutation, reconstruction, output, CDF lifecycle, and
  reference refresh work

#### Scenario: Matrix evidence names focused tests
- **WHEN** `cargo xtask check-decoder-support` validates the matrix
- **THEN** the traversal row names focused crate-private tests for prefix
  child-call ordering, frontier records, transactional CDF handling, checked
  arithmetic/resource failures, unsupported SDP/BRU/inter gates, and supported
  frame-level Wiener NS LR unit symbol consumption

## ADDED Requirements

### Requirement: local decoder mission LR Unit Syntax Frontier Support Row
The decoder support model SHALL track `DECODE-LR-UNIT-SYNTAX-FRONTIER`
as a distinct local decoder mission support row. The row SHALL describe that the local mission
stream can parse the frame-level Wiener NS bank and consume the covered
§5.20.10.4/§5.20.10.5 LR unit `use_wiener_ns` symbols, then fail closed with a
structured `decode/unsupported-feature` diagnostic before loop-restoration
filtering, 10-bit reconstruction/output, reference retention, hash, raw, or Y4M
output.

#### Scenario: Diagnostic resolves to support row
- **WHEN** the minimal runtime reaches the local decoder mission LR unit syntax frontier
- **THEN** it emits `decode/unsupported-feature` with reason
  `unsupported_wienerns_lr_unit_syntax`
- **AND** the diagnostic has matrix row `lr-unit-syntax-frontier`,
  Feature ID `DECODE-LR-UNIT-SYNTAX-FRONTIER`, and AV2 spec section
  `5.20.10.4`

#### Scenario: Support row does not claim decode success
- **WHEN** decoder support status is regenerated
- **THEN** `lr-unit-syntax-frontier` remains partial
- **AND** the row states that it does not implement loop-restoration
  reconstruction/filtering, 10-bit output, reference refresh, or successful
  local decoder mission decode
