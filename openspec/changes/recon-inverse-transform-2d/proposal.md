## Why

With all three § 7.15.2 1D inverse transforms (the § 7.15.2.1 kernel transform
and the § 7.15.2.2 Walsh-Hadamard and § 7.15.2.3 identity transforms) and the
§ 7.14.3 residual-addition step in place, the next residual-path step is the
§ 7.15.4.1 2D matrix transform: the row-then-column pass that wires the 1D
transforms into a full 2D inverse transform over a dequantized coefficient
block. It is a clean self-contained brick — the "2D matrix transform core" item
on the decoder roadmap — that produces the `Residual` array the existing
residual-addition step consumes.

## What Changes

- Add Feature ID `RECON-INVERSE-TRANSFORM-2D`.
- Add `crates/splot-recon/src/inverse_transform_2d.rs` with `inverse_transform_2d`
  and the `InverseTransform2d` / `InverseTransform2dDim` parameter types,
  implementing the § 7.15.4.1 row-then-column 2D matrix transform over a
  caller-supplied dequantized block.
- Carry the *original* (unadjusted) `txSz` log2 dimensions (`log2W` / `log2H`,
  each 2..=6); derive the adjusted operating size internally as
  `1 << Min(log2, 5)` per the `Adjusted_Tx_Size` table. Compute the § 7.15.4.1
  √2 rescale parity (`Abs(log2W - log2H)` odd) and the per-pass
  `get_identity_scale` from the *original* log2 dimensions, so a 64-sample side
  (whose adjusted parity differs) rescales correctly.
- Validate the log2 shape (each 2..=6; both 2 when lossless) and the `w * h`
  buffer lengths with typed `ReconError`; use fixed 32x32 stack buffers and the
  total 1D primitives so the transform is panic-free for valid shapes.
- Preserve the current runtime `splot decode` unsupported behavior.
- Add self-contained spec-exact unit tests, including an original-vs-adjusted
  parity regression.
- Update decoder support, feature tracking, roadmap, generated status docs, and
  OpenSpec artifacts.

Non-goals:

- No § 7.15.4 outer process: the `Adjusted_Tx_Size` lookup itself, the
  `Transform_Shift` / `get_transform_1d_type` derivations, the `Lossless &&
  IDTX` bit-shift shortcut, the DPCM cumulative sum, or the adjusted-size sample
  duplication.
- No § 7.14.4 dequantization process, § 7.15.3 secondary transform, or residual
  addition (separate rows).
- No tile-syntax decode, runtime decode output, hashes, Y4M, or reference
  refresh.
- No scheduler state in `splot-recon`.
- No AVM/dav2d source, dependency, wrapper, script, CI job, or required test.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decoder-support`: records source-backed AV2 § 7.15.4.1 2D matrix transform
  core support while broader reconstruction (the § 7.15.4 outer orchestration,
  the § 7.14.4 dequantization process, the § 7.15.3 secondary transform, and
  prediction/workspace integration) remains partial.

## Impact

- `crates/splot-recon/src/inverse_transform_2d.rs`
- `crates/splot-recon/src/error.rs`
- `crates/splot-recon/src/lib.rs`
- `docs/IMPLEMENTATION-MATRIX.toml`
- `docs/DECODER-SUPPORT-MATRIX.toml`
- generated status/coverage docs
- `docs/DECODER-SUPPORT-MATRIX.toml`
- `openspec/specs/decoder-support/spec.md`
