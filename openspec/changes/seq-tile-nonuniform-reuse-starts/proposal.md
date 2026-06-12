# Proposal: record sequence tile start arrays for non-uniform reuse

## Feature IDs

- `AV2-5.18.7-SEGMENTATION-TILING` (the § 5.18.7.4 non-uniform reuse branch)
- `AV2-5.4.2-SEQUENCE-TILE-CONFIG` (the persisted start arrays)

## Why

`parse_tile_info` returns `Unimplemented` when `reuse_tile_info` is taken with
non-uniform sequence spacing: the § 5.4.2 parse computes
`SeqSbColStarts` / `SeqSbRowStarts` (mirror `05-syntax-structures.md`:654-656)
but discards them, so the § 5.18.7.4 reuse path (mirror :6449/:6475-6477)
cannot rebuild the layout. The reuse implementation already accepts the
arrays (passing unit test in `tile.rs`); only the persistence is missing.
This is the `TODO(spec: AV2-5.18.7-SEGMENTATION-TILING)` at `tiling.rs:37`
and one residual keeping that row partial.

## What Changes

1. Persist the computed start arrays on the stored sequence tile state
   (`TileParams` or its container) at § 5.4.2 parse time.
2. Wire them into the § 5.18.7.4 `reuse_tile_params()` input; delete the
   `Unimplemented` stop on the non-uniform reuse branch and the
   `tiling.rs:37` TODO.
3. Frames whose active sequence header predates the recorded arrays cannot
   arise (the arrays are recorded at parse time for every in-band header);
   external/unavailable headers keep the existing unavailable routing.
4. Matrix rows advance with proof; inspect output unchanged unless the
   newly-parsing branch surfaces fields it already surfaces on the uniform
   path.

## Non-goals

- Any other § 5.18.7 residual (MFH deblocking, etc.).
- Tile-layout validation semantics beyond what § 5.18.7.4 prescribes.

## Acceptance criteria

- [ ] A non-uniform sequence layout with `reuse_tile_info == 1` parses
  through `tile_info()`; positive/negative/EOF tests; uniform-path
  regression intact.
- [ ] The `tiling.rs:37` TODO is gone; `check-feature-status` passes.
- [ ] Matrix proof recorded; `cargo xtask ci` green.
