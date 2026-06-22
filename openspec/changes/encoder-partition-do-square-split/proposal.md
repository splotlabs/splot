## Why

The §5.20.3.2 `do_square_split` flag is the second partition symbol the decoder reads
for a non-`PARTITION_NONE` square block (it reads `do_split`, then `do_square_split`).
The encoder models only `do_split == false` (`PARTITION_NONE`). To descend the partition
tree — needed both by the `general_walk` 4×4-tx decode cross-check (64→32→16→8→4) and by
§10 real-frame partitioning — the encoder must be able to code a `PARTITION_SPLIT`
decision: `do_split == true` then `do_square_split == true`.

## What Changes

- Add `ENC-PARTITION-DO-SQUARE-SPLIT` as a private `splot-encode` encoder-tool feature.
- Add `PartitionSyntax::DoSquareSplit` + `PartitionCdfRowSelector::DoSquareSplit{plane_start,ctx}`.
- Add `emit_root_do_split_split()` (do_split==true, reusing `TileDoSplitCdf[0][12]`) and
  `emit_root_do_square_split_square()` (do_square_split==true, `TileDoSquareSplitCdf[0][0]`).
- Derive the root `do_square_split` context (0) from the decoder `do_square_split_selector`.
- Route a `do_square_split_root` CDF row through `BlockSymbolTraceCdfRows`.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: code the `PARTITION_SPLIT` decision (`do_split` + `do_square_split`) at
  the root 64×64 superblock.

## Impact

- Affected code: `crates/splot-encode/src/partition_emission.rs`,
  `crates/splot-encode/src/block_symbol_trace/{mod.rs,cdf_rows.rs}` (+ tests).
- Scope (explicitly NOT claimed): sub-64×64 per-quadrant neighbour contexts, the full DFS
  partition recursion, a 4×4-tx block, a decodable split frame. §8.2 self-consistency only
  (the [do_split=1, do_square_split=1] trace round-trips to [1,1]); the context's
  cross-crate correctness is decode-proven only once the recursion descends to a decodable
  split frame (a later brick).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status /
  spec coverage.
