## Why

The coefficient-decode subsystem needs the §9.3 coefficient CDF banks wired into
the `splot-decode` block CDF subset. After `eob_extra`, the `eob_pt` family is the
next self-contained set: its §8.3.2 selection uses the closed-form
`eobCtx = (plane > 0) ? 2 : is_inter` plus a transform-size class, with no
`Level[]`, scan order, or eob-position state. Wiring it is additive plumbing
toward the coefficient-decode loop, verifiable now and unchanged in output.

## What Changes

- Advance Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` (stays partial) by
  adding the seven `eob_pt` transform-size class CDF banks.
- In `crates/splot-decode/src/tile_payload/cdf/block_rows.rs`: add the seven
  `EobPt{16,32,64,128,256,512,1024}CdfRows` type aliases
  (`[coeff_cdf_q_ctx][eobCtx][N]`, class-specific width N), the `EobPtSize` enum,
  the seven fields on `BlockCdfRows` loaded from the generated
  `DEFAULT_EOB_PT_*_CDF` tables, a `BlockCdfSelector::EobPt { size,
  coeff_cdf_q_ctx, eob_ctx }` variant with `row` / `row_mut` arms (bounds-checked
  by `checked_coeff_cdf_q_context` and a new `checked_eob_plane_ctx`), generic
  per-bank avg/scale helpers used in `avg_from_tile` /
  `scale_counts_for_frame_end_update`.
- In `crates/splot-decode/src/tile_payload/cdf.rs`: add `TileCdfArray::EobPt`,
  `TileCdfSelector::EobPt`, and the `row` / `row_mut` delegation.
- Tests: each of the seven banks loads its generated default and selects by
  size + `coeff_cdf_q_ctx` + `eobCtx`; an out-of-range `coeff_cdf_q_ctx` or
  `eob_ctx` returns a typed `SelectorOutOfRange`; a tile copy does not alias the
  frame.

This follows the same store-all-`COEFF_CDF_Q_CONTEXTS` design as the already-merged
`txb_skip` / `v_txb_skip` / `eob_extra` banks (selection resolved at read time via
`coeff_cdf_q_ctx`). Collapsing the coefficient banks to the single
`base_q_idx`-initialized row per AV2 §6.19.1 `init_coeff_cdfs()` is a separate,
cross-cutting follow-up tracked to land with the `coeffs()` decode-loop wiring.

Non-goals:

- No `coeffs()` decode-loop wiring (the family is loaded but not read), so the
  minimal-fixture decode output is unchanged.
- No other coefficient CDF banks (dc_sign, coeff_base, coeff_br), no
  `Level[]` / scan / eob-position state, no `eobMultisize` size derivation from
  `txSz` (read-time, deferred), and no `init_coeff_cdfs` single-row model.
- No reconstruction, hashes, Y4M, reference refresh, public API, AVM/dav2d
  invocation, or scheduler change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the `eob_pt` coefficient CDF family in the tile CDF
  selection subset, while broader §8.3 coefficient CDF selection and the
  coefficient decode loop remain partial.

## Impact

- `crates/splot-decode/src/tile_payload/cdf/block_rows.rs`
- `crates/splot-decode/src/tile_payload/cdf.rs`
- `crates/splot-decode/src/tile_payload/cdf/tests.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
