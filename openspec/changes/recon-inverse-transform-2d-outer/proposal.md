## Why

The § 7.15.4.1 2D matrix transform core is in place. The next residual-path step
is the § 7.15.4 **outer** process that wraps it: the adjusted-size derivation,
the `Lossless && IDTX` bit-shift shortcut, the DPCM cumulative sum, and the
adjusted-size sample duplication. It is a clean self-contained brick — the
"§ 7.15.4 outer 2D inverse transform orchestration" item on the decoder roadmap —
that turns a dequantized block into the full original-size residual array.

## What Changes

- Add Feature ID `RECON-INVERSE-TRANSFORM-2D-OUTER`.
- Add `crates/splot-recon/src/inverse_transform_2d_outer.rs` with
  `inverse_transform_2d_outer` and the `InverseTransform2dOuter` / `DpcmDirection`
  types, wrapping `inverse_transform_2d`.
- Carry the *original* (unadjusted) `txSz` log2 dims and derive the adjusted
  operating size (`1 << Min(log2, 5)`) and the original size (`1 << log2`)
  internally, so no `Adjusted_Tx_Size` / `Tx_Width` / `Tx_Height` conversion
  table is needed and there is no `splot-core` dependency.
- Implement the § 7.15.4 lossless IDTX shortcut
  (`Residual = Dequant >> (3 - shift)`, `shift = (pels > 256) + (pels > 1024)`),
  the DPCM cumulative sum (vertical for `V_PRED`, else horizontal; via
  `wrapping_add` for totality), and the sample duplication (nearest-neighbour 2x
  along any 64-sample dimension), expanding the adjusted block into the
  original-size residual.
- Keep `rowType` / `colType` / shifts caller-resolved (the
  `get_transform_1d_type` and `Transform_Shift` derivations remain out of scope).
- Validate the log2 shape and the adjusted-`dequant` / original-`residual`
  buffer lengths with typed `ReconError`; the transform is panic-free for valid
  shapes.
- Preserve the current runtime `splot decode` unsupported behavior.
- Add self-contained spec-exact unit tests.
- Update decoder support, feature tracking, roadmap, generated status docs, and
  OpenSpec artifacts.

Non-goals:

- No § 7.15.4 `Transform_Shift` or `get_transform_1d_type` derivation (the caller
  resolves `rowType` / `colType` / shifts).
- No § 7.15.3 secondary transform, § 7.14.4 dequantization process, or residual
  addition (separate rows).
- No tile-syntax decode, runtime decode output, hashes, Y4M, or reference
  refresh.
- No scheduler state in `splot-recon`.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records source-backed AV2 § 7.15.4 outer 2D inverse
  transform orchestration support while broader reconstruction (the
  `Transform_Shift` / `get_transform_1d_type` derivations, the § 7.14.4
  dequantization process, the § 7.15.3 secondary transform, and
  prediction/workspace integration) remains partial.

## Impact

- `crates/splot-recon/src/inverse_transform_2d_outer.rs`
- `crates/splot-recon/src/error.rs`
- `crates/splot-recon/src/lib.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml`
- `xtask/src/decoder_conformance_coverage.rs`
- generated status/coverage docs
- `docs/DECODER-SUPPORT-MATRIX.toml`
- `openspec/specs/decoder-support/spec.md`
