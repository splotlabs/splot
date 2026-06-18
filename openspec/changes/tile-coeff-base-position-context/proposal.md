## Why

After wiring the standalone coefficient CDF banks (`eob_extra`, the `eob_pt`
family, `dc_sign`), the next step toward the §5.20.7.27 `coeffs()` decode loop is
the §8.3.2 coefficient-symbol CDF context derivations. The two `coeff_base`
position contexts — `coeff_base_eob` and `coeff_base_bob` — are the first such
derivations that need no per-transform-block `Level[]` magnitude buffer, so they
are self-contained and verifiable now against the spec.

## What Changes

- Advance Feature ID `DECODE-TILE-CDF-SELECTION-BOUNDARY` (stays partial) by
  adding the two position-only coefficient base contexts.
- Add `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs` with:
  - `coeff_base_eob_ctx(c, bwl, height)` — §8.3.2 `coeff_base_eob`: partitions the
    scan position `c` by the adjusted block coefficient count
    `height << bwl` (`Tx_Height[adjTxSz] << Tx_Width_Log2[adjTxSz]`) into the four
    `SIG_COEF_CONTEXTS_EOB - 4 ..= SIG_COEF_CONTEXTS_EOB - 1` contexts (`0..=3`),
    total over an out-of-range shift width.
  - `coeff_base_bob_ctx(bob, seg_eob)` — §8.3.2 `coeff_base_bob`: partitions the
    begin position `bob` by `seg_eob >> 3` and `seg_eob >> 2` into contexts
    `0`/`1`/`2`.
- Register the module in `crates/splot-decode/src/tile_payload/cdf.rs`.

Both are pure `const fn`s over caller-supplied scan/segment scalars and
caller-resolved adjusted geometry; they reuse the existing `TransformClass`-free
arithmetic and need no new types or errors.

Non-goals:

- No `coeffs()` decode-loop wiring (the derivations are not yet read by any decode
  path), so the minimal-fixture decode output is unchanged.
- No `Level[]`-dependent contexts (`coeff_base`, `coeff_br`, the IDTX variants),
  no sign contexts (`dc_sign` ctx, `idtx_sign`), and no per-transform-block level
  or sign buffers.
- No reconstruction, hashes, Y4M, reference refresh, public API, AVM/dav2d
  invocation, or scheduler change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records the two position-only coefficient base CDF contexts
  in the tile CDF selection subset, while broader §8.3 coefficient CDF selection
  and the coefficient decode loop remain partial.

## Impact

- `crates/splot-decode/src/tile_payload/cdf/coeff_context.rs` (new)
- `crates/splot-decode/src/tile_payload/cdf.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`, `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `openspec/specs/decoder-support/spec.md`
