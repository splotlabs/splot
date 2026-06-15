## ADDED Requirements

### Requirement: Tile Partition Traversal Support Row
The decoder support model SHALL track `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`
as a distinct crate-private row named `tile-partition-traversal-boundary`. The
row SHALL mark only the partition traversal frontier to the first
`decode_block()` boundary as supported, and SHALL keep broader
`tile-payload-decode`, `symbol-decoder`, CDF lifecycle, runtime decode output,
block syntax, `MiSizes` mutation, and reconstruction rows honest when they
remain partial.

#### Scenario: Traversal row is supported without broad decode overclaim
- **WHEN** the decoder support matrix is regenerated after this change
- **THEN** `tile-partition-traversal-boundary` appears with Feature ID
  `DECODE-TILE-PARTITION-TRAVERSAL-BOUNDARY`
- **AND** it cites AV2 §5.20.3.1, §5.20.3.2, §8.3.2, and §9.2 as applicable
  evidence sections
- **AND** it does not cite §5.20.10.4/§5.20.10.5 as parsed or tested evidence
  while loop-restoration syntax remains outside this boundary
- **AND** it does not cite §5.20.4.1 as parsed or tested evidence while block
  syntax remains outside this boundary
- **AND** `tile-payload-decode` remains partial for full `decode_tile()`, block
  syntax, `MiSizes` mutation, reconstruction, output, CDF lifecycle, and
  reference refresh work

#### Scenario: Matrix evidence names focused tests
- **WHEN** `cargo xtask check-decoder-support` validates the matrix
- **THEN** the traversal row names focused crate-private tests for prefix
  child-call ordering, frontier records, transactional CDF handling, checked
  arithmetic/resource failures, and unsupported SDP/BRU/inter gates
