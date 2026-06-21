## Why

The encoder's first frame with **eob > 2**. Every prior coded block had eob ≤ 2, so the EOB was
signaled by `eob_pt` alone. eob=3 is the first value requiring the `eob_extra` CDF symbol — the
gateway to arbitrary-length blocks (`eob_pt >= 3` reads `eob_extra`, then `eob_pt > 3` adds
`eob_extra_bit` bypass literals).

## What Changes

- Add `ENC-GENERAL-INTRA-EOB3` (splot-encode + splot-cli oracle).
- Add the `EobExtra` token: `CoefficientTokenSyntax::EobExtra` + `CoefficientCdfRowSelector::EobExtra`
  (`TileEobExtraCdf[coeff_cdf_q_ctx]`, context-free apart from the q-context) + the `closed_loop`
  no-op arm.
- Add two `BlockSymbolTraceCdfRows` rows: `eob_extra` (`DEFAULT_EOB_EXTRA_CDF[0]`) and the
  scan-index-1 `coeff_base` low-frequency context 9.
- Add `general_intra_64x64_luma_eob3_base_tokens` (`txb_skip=0`, `eob_pt_1024=2`, `eob_extra=0`,
  then the base pass `c=2,1,0`: AC `coeff_base_eob` ctx 1, scan-1 `coeff_base` ctx 9, DC
  `coeff_base` ctx 2) and the composer + `emit_minimal_intra_eob3_ivf()` in `multi_coeff.rs`.
- Add the cross-crate oracle: `splot decode` reconstructs a horizontal cosine (each column
  constant; left 8 cols 129, middle 48 = 128, right 8 = 127), with flat 128 chroma.

## Capabilities

### Modified Capabilities

- `encoder-tools`: add a requirement for the first eob>2 frame.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization.rs`,
  `crates/splot-encode/src/coefficient_tokenization/general_coded.rs`,
  `crates/splot-encode/src/block_symbol_trace.rs`, `crates/splot-encode/src/closed_loop.rs`,
  `crates/splot-encode/src/general_intra_trace/{mod,multi_coeff}.rs`,
  `crates/splot-encode/src/lib.rs`, `crates/splot-cli/tests/encode_decode_roundtrip.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated status/coverage,
  `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one added `splot-encode` function. No dependency-graph change.
