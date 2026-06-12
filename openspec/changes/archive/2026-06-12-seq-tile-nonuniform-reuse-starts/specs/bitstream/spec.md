# bitstream delta: seq-tile-nonuniform-reuse-starts

Advances `AV2-5.18.7-SEGMENTATION-TILING` (the § 5.18.7.4 non-uniform
reuse branch) and `AV2-5.4.2-SEQUENCE-TILE-CONFIG`.

## ADDED Requirements

### Requirement: non-uniform sequence tile reuse

The sequence-header parse SHALL persist the § 5.4.2 derived
`SeqSbColStarts` / `SeqSbRowStarts` arrays, and the § 5.18.7.4
`reuse_tile_params()` path SHALL consume them so a frame reusing a
non-uniform sequence tile layout parses through `tile_info()` instead of
stopping as unimplemented.

#### Scenario: non-uniform reuse parses

- **WHEN** a frame sets `reuse_tile_info == 1` against an in-band sequence
  header with non-uniform tile spacing
- **THEN** `tile_info()` parses using the recorded start arrays

#### Scenario: uniform path unchanged

- **WHEN** a frame reuses a uniform sequence tile layout
- **THEN** parsing behaves exactly as before
