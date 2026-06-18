## Why

Continuing the §8.3.2 coefficient context derivations toward the §5.20.7.27
`coeffs()` decode loop (after `coeff_base_eob`/`coeff_base_bob` and `coeff_br`),
the two identity-transform magnitude contexts `coeff_base_idtx` and
`coeff_br_idtx` are the next `Level[]`-reading contexts. They are the simplest of
the remaining ones — each reads only the left and above neighbour — and are
verifiable now against the spec over a caller-provided level slice.

## What Changes

- Advance Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` (stays partial) by
  adding the two IDTX magnitude contexts.
- In `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs` add:
  - `coeff_base_idtx_ctx(level, row, col, txw)` — §8.3.2 `coeff_base_idtx`:
    `mag = Min(3, Level[row][col-1]) + Min(3, Level[row-1][col])`.
  - `coeff_br_idtx_ctx(level, row, col, txw)` — §8.3.2 `coeff_br_idtx`: the same
    with the `MAX_BASE_BR_RANGE - 1` clamp, then `mag = Min(mag, 6)`.
  - a shared `idtx_neighbour_mag` helper.
- Both are `const fn`s over a caller-provided row-major `txw`-wide `Level[]`
  slice; the flat index uses saturating geometry and a slice-length guard, so
  out-of-range or short-slice reads contribute `0` and the functions are total
  and panic-free. A module-level `const` spec-contract check is the non-test
  consumer.

Non-goals:

- No `coeffs()` decode-loop wiring (the derivations are not read by any decode
  path), so the minimal-fixture decode output is unchanged.
- No `coeff_base`, no sign contexts (`dc_sign` ctx, `idtx_sign`), and no full
  per-transform-block level/sign buffers.
- No reconstruction, hashes, Y4M, reference refresh, public API, AVM/dav2d
  invocation, or scheduler change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the two IDTX coefficient magnitude CDF contexts in
  the tile CDF selection subset, while broader §8.3 coefficient CDF selection and
  the coefficient decode loop remain partial.

## Impact

- `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
