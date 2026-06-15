# Change: frame-header-writer-tiling

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.18.7.3-TILE-PARAMS` (advances its `write` stage `todo -> done`)
- `AV2-5.18.7-SEGMENTATION-TILING` (advances its `write` stage `todo -> partial`,
  the tiling portion)

## Why

Third slice (#4c) of the frame-header writer (intra path). It inverts `tile_info()`
(§ 5.18.7.2) — the largest remaining control-region structure — reusing the shared
§ 5.18.7.3 `tile_params()` writer from the sequence tile config.

`tile_info()`'s explicit branch reads `uniform_tile_spacing_flag` and derives a full
`TileParams`, but `TileInfo` previously kept only the tile counts / log2 sizes /
`MiColStarts` / `MiRowStarts` — discarding `uniform_spacing`, which the writer needs and
which is not recoverable from the layout (a non-uniform layout can be uniform-shaped). Per
the maintainer's full-byte-exact decision (the same exception taken for the #4b frame-config
bits), this change surfaces the derived `TileParams` on `TileInfo`.

## What changes

- **Model + parser surfacing** (the approved exception): add `TileInfo.tile_params:
  Option<TileParams>` — `Some(layout.params)` on the explicit branch, `None` on the reuse
  branch. `parse_tile_info` populates it without changing any other field or `consumed_bits`.
- **Writer** (`crates/splot-core/src/write/frame_tiling.rs`): `write_tile_info`, the
  byte-exact inverse of `parse_tile_info`:
  - the `reuse_tile_info` eligibility/inference (a bit only when eligible &&
    `allow_tile_info_change`);
  - the **reuse branch** — no layout bits; the stored counts / starts are validated against a
    `reuse_tile_params()` (§ 5.18.7.4) re-derivation;
  - the **explicit branch** — reuses the now-`pub(crate)` `write_tile_params` (§ 5.18.7.3)
    with the surfaced `TileParams` and the `sbColStarts` / `sbRowStarts` recovered from
    `MiColStarts` / `MiRowStarts` (per the branch-dependent `sbShift2`);
  - the **bridge** zero-bit `tile_params()` path;
  - the gated `context_update_tile_id` / `tile_size_bytes_minus_1` tail.
  - Each field is validated up front (reject-before-write); the existing `WriteError`
    variants suffice (no new variant).
- Expose `write_tile_params` / `compute_tile_grid` / `TileGrid` as `pub(crate)` in
  `seq_tile.rs` for the reuse.

## Validator impact

None. No new diagnostics; the validator is unchanged.

## Non-goals

- No `segmentation_params()` (§ 5.18.7.1) or the filter-param children (lr/ccso/gdf/cdef) —
  later #4 slices.
- No composing `write_frame_header`.

## Impact

- Crate: `crates/splot-core` (the approved `TileInfo` surfacing + the additive `write`
  module + `pub(crate)` visibility on the tile-params writer helpers).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (+ regenerated `docs/FEATURE-STATUS.md`).
