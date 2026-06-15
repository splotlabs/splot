# Tasks

## Model + parser surfacing (maintainer-approved exception)
- [x] `tiling.rs`: add `TileInfo.tile_params: Option<TileParams>` (`Some` on the explicit
      branch, `None` on reuse). `parse_tile_info` populates it; `consumed_bits` + existing
      fields unchanged; existing parser tests pass.

## Writer
- [x] `write/frame_tiling.rs`: `write_tile_info` inverting `tile_info()` — reuse inference,
      reuse-branch re-derivation, explicit-branch `write_tile_params` reuse (per-branch
      `sbShift2`), bridge zero-bit path, and the gated tail; all validated up front by
      `check_tile_info_encodable` (reject-before-write).
- [x] Expose `write_tile_params` / `compute_tile_grid` / `TileGrid` as `pub(crate)` in
      `seq_tile.rs`. Register the module + re-export `write_tile_info` in `write/mod.rs`.

## Tests and proof
- [x] Round-trip tests across reuse/explicit/bridge × uniform/non-uniform × single/multi-tile
      × avg-CDF gate × not-eligible; one reject test per `WriteError` path (`bit_len()==0`); a
      round-trip property test.

## Matrix and docs
- [x] Advance `write` `todo -> done` on `AV2-5.18.7.3-TILE-PARAMS` and `todo -> partial` on
      `AV2-5.18.7-SEGMENTATION-TILING` (tiling portion), with proof + the model-extension note.
      Regenerate `docs/FEATURE-STATUS.md`.

## Checks
- [x] `cargo xtask ci` and `openspec validate frame-header-writer-tiling --strict`
