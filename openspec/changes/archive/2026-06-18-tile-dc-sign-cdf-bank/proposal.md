## Why

Continuing to wire the §9 coefficient CDF banks into the `splot-decode` block CDF
subset (after `eob_extra` and the `eob_pt` family). `dc_sign` is the next bank:
its §8.3.2 `ctx` is derived from the Above/Left DC-context buffers (deferred with
the coeffs() loop), but the bank itself — the generated default copy plus the
selector over its four index axes — is additive plumbing that is verifiable now
and unchanged in output.

## What Changes

- Advance Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` (stays partial) by
  adding the `dc_sign` coefficient CDF bank.
- In `crates/splot-decode/src/tile_payload/cdf/block_rows.rs`: add the
  `DcSignCdfRows` type alias
  (`[coeff_cdf_q_ctx][plane_type][isHidden group][ctx][3]`), the `dc_sign` field
  loaded from the generated `DEFAULT_DC_SIGN_CDF`, a `BlockCdfSelector::DcSign
  { coeff_cdf_q_ctx, plane_type, group, ctx }` variant with `row` / `row_mut`
  arms (all four axes bounds-checked: `coeff_cdf_q_ctx`, `plane_type`, the new
  `checked_dc_sign_group`, and `ctx`), and the flattened-iterator avg/scale folds
  in `avg_from_tile` / `scale_counts_for_frame_end_update`. Parameterize the
  existing `checked_plane_type` with the owning array so the typed error names the
  correct bank.
- In `crates/splot-decode/src/tile_payload/cdf.rs`: add `TileCdfArray::DcSign`,
  `TileCdfSelector::DcSign`, and the `row` / `row_mut` delegation.
- Tests: the bank loads the generated default and selects across every
  `[q][plane][group][ctx]` cell; each of the four index axes returns a typed
  `SelectorOutOfRange`; a tile copy does not alias the frame.

This follows the same store-all-`COEFF_CDF_Q_CONTEXTS` design as the merged
coefficient banks; the §6.19.1 `init_coeff_cdfs()` single-row model is the tracked
cross-cutting follow-up.

Non-goals:

- No `coeffs()` decode-loop wiring (the bank is loaded but not read), so the
  minimal-fixture decode output is unchanged.
- No §8.3.2 `dc_sign` `ctx` derivation (the Above/Left DC-context buffers do not
  exist yet), no `dc_sign_horz_vert` consumer, and no other coefficient banks.
- No reconstruction, hashes, Y4M, reference refresh, public API, AVM/dav2d
  invocation, or scheduler change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the `dc_sign` coefficient CDF bank in the tile CDF
  selection subset, while broader §8.3 coefficient CDF selection and the
  coefficient decode loop remain partial.

## Impact

- `crates/splot-decode/src/tile_payload/cdf/block_rows.rs`
- `crates/splot-decode/src/tile_payload/cdf.rs`
- `crates/splot-decode/src/tile_payload/cdf/tests.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
