## Why

The encoder's first **2-D reconstruction**. Every prior frame reconstructed a separable pattern
(flat, a vertical cosine, or a horizontal cosine — each varying in at most one dimension). This
codes two nonzero ACs in one eob=3 block — a vertical (scan 1) and a horizontal (scan 2) — whose
superposition varies in both dimensions, and exercises the first block with two nonzero non-EOB
sign bypasses.

## What Changes

- Add `ENC-GENERAL-INTRA-2D` (splot-encode + splot-cli oracle).
- Add one `BlockSymbolTraceCdfRows` row: the DC `coeff_base` low-frequency context 4 (two level-4
  AC neighbours sum to § 8.3.2 magnitude 8 -> context 4).
- Add `general_intra_64x64_luma_2d_base_tokens` (eob=3 base pass with scan-2 `coeff_base_eob` at
  ctx 1 and scan-1 `coeff_base` level 4 at ctx 9, no `coeff_br` since 4 == `LF_NUM_BASE_LEVELS`).
- Add `compose_general_intra_2d_block_trace` + `emit_minimal_intra_2d_ivf()`: the two AC
  `sign_bit` bypasses in reverse-scan order (scan 2 positive, then scan 1 negative).
- Add the cross-crate oracle: `splot decode` reconstructs a diagonal gradient — neither rows nor
  columns constant; the 3x3 band grid is `[[128,127,127],[129,128,127],[129,129,128]]`, flat 128
  chroma.

## Capabilities

### Modified Capabilities

- `encoder-tools`: add a requirement for the first 2-D reconstruction.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization{,.rs}`,
  `crates/splot-encode/src/coefficient_tokenization/general_coded.rs`,
  `crates/splot-encode/src/block_symbol_trace/{mod,cdf_rows}.rs`,
  `crates/splot-encode/src/general_intra_trace/{mod,multi_coeff}.rs`,
  `crates/splot-encode/src/lib.rs`, `crates/splot-cli/tests/encode_decode_roundtrip.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated status/coverage,
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one added `splot-encode` function. No dependency-graph change.
