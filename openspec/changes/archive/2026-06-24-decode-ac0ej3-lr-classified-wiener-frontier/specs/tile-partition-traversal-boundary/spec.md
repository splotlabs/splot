## ADDED Requirements

### Requirement: Active LR Source Blocks Retain Tile Bounds

The tile partition traversal frontier SHALL retain the tile MI bounds needed by
AV2 §7.20.4 for each active Wiener NS LR source block.

#### Scenario: Runtime can derive classified Wiener clipping bounds

- **WHEN** the runtime receives active Wiener NS LR source blocks from the tile
  traversal frontier
- **THEN** each block includes `MiRowStart`, `MiRowEnd`, `MiColStart`, and
  `MiColEnd` facts sufficient to derive §7.20.4 `BlockEndX` and `get_tx_skip`
  clipping without guessing from frame-wide source bounds.
