## Why

The encoder's first frame where a coefficient **visibly shapes the reconstruction**. Every
prior decodable frame reconstructed flat (skip → 128, coded DC → a flat plane, the minimal
level-1 eob=2 → sub-visible flat 128). This raises the eob=2 AC coefficient to level 4 — the
largest `coeff_base_eob` base level with no `coeff_br` tail — so its dequantized residual
survives rounding and reconstructs a vertical low-frequency cosine.

## What Changes

- Add `ENC-GENERAL-INTRA-VISIBLE-AC` (splot-encode + splot-cli oracle).
- Add `general_intra_64x64_luma_visible_ac_tokens`: the eob=2 luma tokens with the AC at level 4
  (`coeff_base_eob` symbol 3, no `coeff_br`) and the DC `coeff_base` at its `Level[]`-derived
  low-frequency context 2 (the larger AC neighbour magnitude raises the DC context from 1 to 2).
- Add one `BlockSymbolTraceCdfRows` row (the DC `coeff_base` at `tx_size 4`, context 2).
- Add `compose_general_intra_visible_ac_block_trace` and `emit_minimal_intra_visible_ac_ivf()`.
- Add the cross-crate oracle: `splot decode` reconstructs a vertical cosine — each row constant
  across columns, the top 8 rows 129, the middle 48 rows 128, the bottom 8 rows 127 — with flat
  128 chroma.

## Capabilities

### Modified Capabilities

- `encoder-tools`: add a requirement for the first visibly non-flat reconstruction.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization/general_coded.rs`,
  `crates/splot-encode/src/block_symbol_trace.rs` (one row + routing),
  `crates/splot-encode/src/general_intra_trace.rs`, `crates/splot-encode/src/lib.rs`,
  `crates/splot-cli/tests/encode_decode_roundtrip.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated status/coverage,
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one added `splot-encode` function. No dependency-graph change.
