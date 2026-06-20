## Why

The decodable-tile arc's first **coded** frame. The milestone skip frame proved the encoder
emits a decodable stream, but its all-zero residual only exercises prediction. This brick
emits a single coded luma DC coefficient so the decoder reconstructs a non-128 value — proving
the encoder produces real residual the decoder dequantizes and adds to the predictor.

## What Changes

- Add `ENC-GENERAL-INTRA-CODED-LUMA-DC` as an encoder feature (splot-encode + splot-cli
  oracle).
- Add the `eob_pt_1024` coefficient token (a new `CoefficientTokenSyntax::EobPt1024` /
  `CoefficientCdfRowSelector::EobPt1024`): the `eob_pt` symbol for the `TX_64X64` 1024-position
  EOB size class, distinct from the minimal-tier `eob_pt_16`.
- Add `general_intra_64x64_luma_dc_coded_tokens(q, magnitude, negative)`: the coded luma DC
  token sequence (`txb_skip == 0`, `eob_pt == 0`, `coeff_base_eob`, optional `coeff_br`,
  `dc_sign`) at the general `TX_64X64` contexts. Route the `eob_pt_1024` and `TX_64X64`
  `coeff_base_lf_eob` rows through `BlockSymbolTraceCdfRows`.
- Add `splot_encode::emit_minimal_intra_coded_dc_ivf()`: one 64x64 frame with a single
  negative luma DC (magnitude 6, sub-golomb) and skipped chroma.
- Add the cross-crate oracle: `splot decode` reconstructs flat luma `127` and flat chroma
  `128`.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the first decodable coded-coefficient intra frame.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization.rs` (the `eob_pt_1024`
  token + the general coded-DC tokens), `crates/splot-encode/src/closed_loop.rs` (the syntax
  no-op arm), `crates/splot-encode/src/block_symbol_trace.rs` (two CDF rows + routing),
  `crates/splot-encode/src/general_intra_trace.rs` (the composer + emit function),
  `crates/splot-encode/src/lib.rs` (re-export),
  `crates/splot-cli/tests/encode_decode_roundtrip.rs` (oracle).
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature status/spec
  coverage, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: one added `splot-encode` function. No dependency-graph change.
- Validator/CLI impact: none (a new test only).
