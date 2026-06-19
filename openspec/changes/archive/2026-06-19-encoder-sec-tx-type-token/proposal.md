## Why

Transform-type signaling Sub-brick C: after the `intra_tx_type` token
(`ENC-INTRA-TX-TYPE-TOKEN`) and its trace (`ENC-INTRA-BLOCK-TRACE-TWO-COEFF-TX-TYPE`),
the next §5.20.8.2 `transform_type()` symbol is `sec_tx_type` (the IST secondary
transform), read right after `intra_tx_type` when `enable_intra_ist` is on. This
change models the `sec_tx_type` token so a later trace can carry the IST symbol.

The §5.20.8.2 read order and CDF context were verified adversarially against the
committed spec mirror before implementation: `sec_tx_type` (line 16613) is read
inside `transform_type()` — the SAME function as `intra_tx_type` (line 16529), NOT
inside `compute_tx_type()` (§5.20.7.29, which reads no symbols).

## What Changes

- Add `ENC-SEC-TX-TYPE-TOKEN` as a private `splot-encode` encoder-tool feature.
- Add `CoefficientTokenSyntax::SecTxType` and `CoefficientCdfRowSelector::SecTxTypeIntra
  { tx_size_sqr }`, routing `TileSecTxTypeCdf[0][Tx_Size_Sqr]` (the intra `is_inter =
  0` bank; §8.3.2).
- Add `sec_tx_type_intra_token(tx_size_sqr, symbol)` to the `transform_type`
  submodule and wire the intra `sec_tx_type` rows into the generic CDF-row router.
- Add the `SecTxType` arm to the closed-loop single-DC recovery helper (no-op).
- Correct the `IntraTxType` enum-variant doc to cite §5.20.8.2 (the syntax section),
  matching the #347 source/matrix attribution.
- Prove the token roundtrips through one in-tree §8.2 coder for every `Tx_Size_Sqr`
  row and every `sec_tx_type` value (`STX_TYPES = 4`). No trace yet; no packet output.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `encoder-tools`: add a requirement for the `sec_tx_type` IST transform-type token.

## Impact

- Affected code: `crates/splot-encode/src/coefficient_tokenization.rs`,
  `crates/splot-encode/src/coefficient_tokenization/transform_type.rs`,
  `crates/splot-encode/src/coefficient_tokenization/cdf_rows.rs`,
  `crates/splot-encode/src/closed_loop.rs`,
  `crates/splot-encode/src/coefficient_tokenization_tests.rs`.
- Affected docs/tracking: `docs/IMPLEMENTATION-MATRIX.toml`, generated feature
  status/spec coverage, encoder roadmap/gap audit, `openspec/specs/encoder-tools/spec.md`.
- Public API impact: none; crate-private, not re-exported beyond the crate.
- Dependency impact: none.
- Validator/CLI impact: none.
