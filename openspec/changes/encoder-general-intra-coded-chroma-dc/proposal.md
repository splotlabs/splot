## Why

The decodable-tile arc's first **chroma** coded frame. The coded-luma frame proved luma
residual; this proves chroma residual, isolated: the U block carries a single coded DC while
luma and V stay skipped, so decoding shows the chroma plane (not luma) reconstructing a
non-128 value.

## What Changes

- Add `ENC-GENERAL-INTRA-CODED-CHROMA-DC` as an encoder feature (splot-encode + splot-cli
  oracle).
- Add `general_intra_32x32_chroma_u_dc_coded_tokens(q, magnitude)`: the coded chroma U DC
  tokens (`txb_skip == 0`, `eob_pt_1024` at chroma `eob_ctx 2`, `coeff_base_eob`) at the
  general `TX_32X32` chroma contexts. Move it and the existing general luma coded-DC tokens
  into a `general_coded` submodule (keeping `coefficient_tokenization.rs` under the line
  budget).
- Route the chroma `eob_pt_1024` (`eob_ctx 2`) row through `BlockSymbolTraceCdfRows`.
- Add `compose_general_intra_coded_chroma_u_block_trace` (luma skip + U coded + the U DC
  `sign_bit` § 8.2.5 bypass + V skip at the `EobU != 0` context) and
  `splot_encode::emit_minimal_intra_coded_chroma_ivf()`.
- Add the cross-crate oracle: `splot decode` reconstructs flat luma 128, flat U 127, flat
  V 128.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the first decodable coded-chroma intra frame.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization.rs` and the new
  `coefficient_tokenization/general_coded.rs`, `crates/splot-encode/src/block_symbol_trace.rs`
  (the chroma `eob_pt_1024` row), `crates/splot-encode/src/general_intra_trace.rs` (the
  composer + emit), `crates/splot-encode/src/lib.rs`,
  `crates/splot-cli/tests/encode_decode_roundtrip.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated status/coverage,
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one added `splot-encode` function. No dependency-graph change.
