# Change: frame-header-writer-loop-filters

## Feature IDs

- `ENC-BITSTREAM-WRITER` (advances the writer surface; umbrella stays `partial`)
- `AV2-5.18.5-FILTERING` (advances the `deblocking_filter_params()` portion of its `write`
  stage; the row stays `partial` — the § 5.18.5.1 `read_interpolation_filter()` is an
  inter-path element outside the intra writer scope)
- `AV2-5.18.7-SEGMENTATION-TILING` (advances the `gdf_params()` / `cdef_params()` portion of
  its `write` stage; the row stays `partial` until the `lr_params()` / `ccso_params()` children
  land in #4g)

## Why

Sixth slice (#4f) of the frame-header writer (intra path). It inverts the three frame
loop-filter parsers: `deblocking_filter_params()` (§ 5.18.5.2), `gdf_params()` (§ 5.18.7.9),
and `cdef_params()` (§ 5.18.7.10).

Like #4d / #4e this slice is **additive — no model change**. Every read-but-not-stored point
is a reversible derivation or a redundant encoding (not a layout-affecting discard), handled by
re-derivation / **canonicalization** — the same approach the sequence writer takes for
leb128-minimal:

- `DfDeltaQ[i]` stores the derived offset; the raw `df_delta_q[i] = DfDeltaQ[i] + (1 <<
  (dfParBits - 1))` is reconstructed and range-checked.
- `gdfBlkSize` and the `gdf_per_block` coded/inferred gate are re-derived from the same
  `GdfGeometry` the parser used.
- `cdef_y_pri_zero` / `cdef_uv_pri_zero` are canonical: a zero strength is emitted as the
  zero-flag (not the `f(4) == 0` form).
- The `cdef_*_sec_strength` `3 -> 4` remap is reversed (`4 -> 3`).

## What changes

- **Writers** (`crates/splot-core/src/write/frame_filters.rs`):
  `write_deblocking_filter_params`, `write_gdf_params`, `write_cdef_params` — each validating
  the whole model up front (`check_*_encodable`, reject-before-write; `bit_len() == 0` on every
  reject).
- **One internal extraction** (`crates/splot-core/src/headers/frame/filtering.rs`): the
  `gdf_per_block` coded/inferred gate (the full `gdfBlkSize` derivation) is pulled into a
  `pub(crate) fn gdf_per_block_is_coded(filter, geometry)` that both `parse_gdf_params` and the
  writer call, so the writer re-derives the gate without drift. Behavior-preserving; no syntax
  change.
- **No model field and no new `WriteError` variant** (reuses
  `WriteError::NonCanonicalFrameHeader`; an over-wide `dfParBits` reuses
  `WriteError::BitWidthTooLarge`, mirroring the parser's `BitWidthTooLarge` guard).

## Validator impact

None. No new diagnostics; the validator is unchanged.

## Non-goals

- No `lr_params()` / `ccso_params()` restoration / CCSO writers — the #4g slice.
- No § 5.18.5.1 `read_interpolation_filter()` writer (inter-path, outside the intra scope).
- No composing `write_frame_header`.

## Impact

- Crate: `crates/splot-core` (additive `write` module + a behavior-preserving `pub(crate)`
  gate extraction in the filtering parser).
- Docs: `docs/IMPLEMENTATION-MATRIX.toml` (+ regenerated `docs/FEATURE-STATUS.md`).
