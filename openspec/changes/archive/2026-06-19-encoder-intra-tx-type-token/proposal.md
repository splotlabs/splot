## Why

The eob=2 trace (#336) had to assume away the §5.20.7.27 `transform_type()`
signaling because the `intra_tx_type` symbol — read between `eob_pt` and the
coefficient base pass for `eob > 1` blocks — wasn't modeled. This change adds the
`intra_tx_type` token for the `TX_SET_INTRA_1` set (the default-`reduced_tx_set`
4x4 intra set), the first building block toward general `eob > 1` blocks.

## What Changes

- Add `ENC-INTRA-TX-TYPE-TOKEN` as a private `splot-encode` encoder-tool feature.
- Add `CoefficientTokenSyntax::IntraTxType` and
  `CoefficientCdfRowSelector::IntraTxTypeSet1 { tx_size_sqr }`
  (`TileIntraTxTypeSet1Cdf[Tx_Size_Sqr[txSz]]`, §8.3.2 Table 8.2).
- Add a `coefficient_tokenization/transform_type.rs` submodule with `pub(crate)`
  `intra_tx_type_set1_token(tx_size_sqr, symbol)`. The symbol indexes the §9
  `Md_Idx_To_Type[Size_Class[txSz]][intraDir]` row; for a 4x4 (`Tx_Size_Sqr 0`)
  `DC_PRED` block, symbol 0 selects `DCT_DCT` (`Md_Idx_To_Type[0][0][0] = 0`).
- Wire all three `TX_SET_INTRA_1` rows (one per `Tx_Size_Sqr`) into the generic
  CDF-row router so the token roundtrips through the in-tree AV2 §8.2 coder.
- Extract the generic CDF-row router (`CoefficientTokenCdfRows`) into a
  `coefficient_tokenization/cdf_rows.rs` submodule to keep the parent file under the
  1000-line source budget.
- The token is available but not yet composed into a trace (the general `eob > 1`
  trace brick does that). No packet output.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the `intra_tx_type` (`TX_SET_INTRA_1`)
  transform-type token.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization.rs` (syntax,
  selector, router wiring, module split), `crates/splot-encode/src/coefficient_tokenization/transform_type.rs`
  (new), `crates/splot-encode/src/coefficient_tokenization/cdf_rows.rs` (extracted
  router), `crates/splot-encode/src/closed_loop.rs` (exhaustive-match arm),
  `crates/splot-encode/src/coefficient_tokenization_tests.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; crate-private, not re-exported.
- Dependency impact: none; imports existing `splot-core` §9 tables.
- Validator/CLI impact: none.
