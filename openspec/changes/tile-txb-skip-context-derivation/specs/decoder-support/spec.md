## ADDED Requirements

### Requirement: Block-symbol all_zero (txb_skip) CDF context formula

The tile CDF selection boundary (`DECODE-TILE-CDF-SELECTION-BOUNDARY`) SHALL
compute the AV2 § 8.3.2 `all_zero` (`txb_skip` / `v_txb_skip`) CDF context for the
luma (`plane == 0`) and V (`plane == 2`) symbols via the spec formula rather than
hardcoded literals. The luma context SHALL be `TXB_SKIP_CONTEXTS - 1` when
`fsc_mode && enable_fsc`, `0` when the transform fills its plane residual block
(`bw == w && bh == h`), and otherwise `(Min(top, 4) + Min(left, 4) + 3) >> 1`,
where `top` / `left` are the OR-reductions of `AboveLevelContext[0]` /
`LeftLevelContext[0]` over the transform block's in-frame 4x4 columns / rows. The
V context SHALL be `(above != 0) + (left != 0)` plus `3` when the chroma residual
block exceeds the transform (`bw * bh > w * h`) and plus `6` when `EobU != 0`,
where `above` / `left` are the OR of the V-plane level and DC contexts over the
in-frame 4x4 columns / rows. The minimal block-symbol trace SHALL select these
CDF rows using the formula with the level-context contribution *derived* as zero
for the first transform block (no prior decoded transform blocks; out-of-frame
neighbours; the U plane decoded all-zero so `EobU == 0`). The transform-block
geometry (`tx_fills_block`, `chroma_block_larger_than_tx`) MAY be caller-supplied
until the § 5.20 transform-block syntax derives it. This requirement SHALL NOT
implement the level-context / DC-context buffers, the coefficient decode, the
`EobU` / `fsc_mode` / `txSz` / residual-geometry derivation, the U-plane
`txb_skip` branch, partition decisions, full § 8.3 CDF selection, `decode_tile()`,
reconstruction, hashes, Y4M output, or reference refresh.

#### Scenario: First-block all_zero contexts match the forced fixture values

- **WHEN** the minimal flat-intra block-symbol trace selects the luma `txb_skip`
  and V `v_txb_skip` CDF rows for the first transform block
- **THEN** the level context is derived as zero (first block, out-of-frame
  neighbours, `EobU == 0`), the luma context is `0` (the transform fills its
  block) and the V context is `3` (the chroma block exceeds the transform)
- **AND** the decoded trace is byte-for-byte unchanged (the no-output-change
  snapshot of symbol count, trailing bit, and padding end stays green), which is
  the value the conformant fixture forces

#### Scenario: The all_zero formula follows § 8.3.2

- **WHEN** the luma context is computed for a non-filling transform
- **THEN** it is `(Min(top, 4) + Min(left, 4) + 3) >> 1`, and `fsc_mode` overrides
  it to `TXB_SKIP_CONTEXTS - 1`
- **AND** the V context adds `3` when the chroma block exceeds the transform and
  `6` when `EobU != 0` to the `(above != 0) + (left != 0)` base

#### Scenario: Block-symbol context derivation remains partial

- **WHEN** decoder support status is rendered
- **THEN** the tile CDF selection boundary still reports partial status
- **AND** the level-context buffers, the transform-block geometry derivation, the
  coefficient decode, and the U-plane `txb_skip` branch remain out of scope
