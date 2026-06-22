## ADDED Requirements

### Requirement: root do_square_split (PARTITION_SPLIT) partition symbol

The encoder SHALL be able to code the §5.20.3.2 `PARTITION_SPLIT` decision at the root
64×64 superblock as the two-symbol sequence `do_split == true` then
`do_square_split == true`, each against its decoder-mirrored §8.3.2 CDF row
(`TileDoSplitCdf[0][12]`, then `TileDoSquareSplitCdf[0][0]` — the root `do_square_split`
context is 0). This is a private, non-emitting stage tracked by
`ENC-PARTITION-DO-SQUARE-SPLIT`; it does not descend the partition tree, code a 4×4-tx
block, or produce a packet.

#### Scenario: root PARTITION_SPLIT round-trips

- **WHEN** the `[do_split=1, do_square_split=1]` trace is coded
- **THEN** it round-trips through one §8.2 coder, decoding back to `[1, 1]`

#### Scenario: do_split reuses its row with symbol 1

- **WHEN** `emit_root_do_split_split()` is emitted
- **THEN** it selects the same `TileDoSplitCdf[0][12]` row as `PARTITION_NONE`, differing
  only in the coded symbol (1 vs 0)
