## Why

The `dc_sign` CDF bank (`TileDcSignCdf`) was wired earlier but its §8.3.2 `ctx`
was deferred (it needs the above/left DC-context buffers). With the `coeff_base`
family complete, `dc_sign_ctx` is the next §8.3.2 context — it completes the
`dc_sign` selection and is verifiable now over caller-provided DC-context slices.

## What Changes

- Advance Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` (stays partial) by
  adding the `dc_sign` sign context.
- In `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs` add
  `dc_sign_ctx(above_dc, left_dc, x4, y4, w4, h4) -> usize` implementing §8.3.2
  `dc_sign`: net the above (`AboveDcContext[plane][x4+k]`, `k` in `0..w4`) and
  left (`LeftDcContext[plane][y4+k]`, `k` in `0..h4`) DC-sign votes — sign `1`
  decrements, sign `2` increments a running `dcSign` — and return `1` (`dcSign <
  0`), `2` (`dcSign > 0`), or `0`.

`above_dc` / `left_dc` are the plane's `AboveDcContext` / `LeftDcContext` slices,
whose lengths are the spec `MiCols` / `MiRows` bounds; the loop breaks once the
monotonic index leaves the slice (equivalent to the spec's skip-remaining), so a
pathological `w4` / `h4` cannot spin and the `const fn` is total and panic-free. A
module-level `const` spec-contract check is the non-test consumer.

Non-goals:

- No `coeffs()` decode-loop wiring and no DC-context buffer construction (the
  derivation is not read by any decode path), so the minimal-fixture decode output
  is unchanged.
- No `idtx_sign` (needs `QuantSign[]`), and no full per-transform-block level/sign
  buffers.
- No reconstruction, hashes, Y4M, reference refresh, public API, AVM/dav2d
  invocation, or scheduler change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the `dc_sign` sign context (completing the `dc_sign`
  CDF selection) in the tile CDF selection subset, while broader §8.3 coefficient
  CDF selection and the coefficient decode loop remain partial.

## Impact

- `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
