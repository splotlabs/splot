## ADDED Requirements

### Requirement: Block-symbol y_mode_index CDF context derivation

The tile CDF selection boundary (`DECODE-TILE-CDF-SELECTION-BOUNDARY`) SHALL
derive the AV2 § 8.3.2 `y_mode_index` CDF context instead of selecting it with a
hardcoded literal. The derivation SHALL compute
`ctx = (get_joint_mode(0) >= NON_DIRECTIONAL_MODES_COUNT) + (get_joint_mode(1)
>= NON_DIRECTIONAL_MODES_COUNT)`, where `get_joint_mode(dir)` returns the
directional joint mode of the left (`dir == 0`) or above (`dir == 1`) neighbour,
or `DC_PRED` when that neighbour is out of frame (§ 5 `get_joint_mode`). For the
minimal single-block tile-origin case (`MiRow == 0`, `MiCol == 0`) both
neighbours are out of frame, so the derived context is 0. The derivation SHALL be
total and panic-free, and the minimal flat-intra block-symbol trace SHALL select
`TileYModeIndexCdf[ctx]` using the derived context. This requirement SHALL NOT
implement the in-frame `IntraJointModes` neighbour lookup, the `uv_mode`,
`txb_skip`, or `v_txb_skip` contexts, `YMode` reconstruction, partition
decisions, full § 8.3 CDF selection, `decode_tile()`, reconstruction, hashes,
Y4M output, or reference refresh.

#### Scenario: y_mode_index context is derived at the tile origin

- **WHEN** the minimal flat-intra block-symbol trace selects the `y_mode_index`
  CDF row for the single block at the tile origin
- **THEN** both `get_joint_mode` neighbours are out of frame and resolve to
  `DC_PRED`, so the derived § 8.3.2 context is 0
- **AND** the decoded trace is byte-for-byte unchanged (the no-output-change
  snapshot of symbol count, trailing bit, and padding end stays green)

#### Scenario: Directional neighbours raise the context

- **WHEN** the `y_mode_index` context is derived for joint modes at or above
  `NON_DIRECTIONAL_MODES_COUNT`
- **THEN** each directional neighbour contributes 1, giving a context of 1 for one
  directional neighbour and 2 for two
- **AND** a joint mode below `NON_DIRECTIONAL_MODES_COUNT` (including `DC_PRED`)
  contributes 0

#### Scenario: Block-symbol context derivation remains partial

- **WHEN** decoder support status is rendered
- **THEN** the tile CDF selection boundary still reports partial status
- **AND** the in-frame neighbour lookup and the remaining block-symbol contexts
  (`uv_mode`, `txb_skip`, `v_txb_skip`) remain out of scope
