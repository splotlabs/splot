## 1. sec_tx_type token

- [x] 1.1 Add `CoefficientTokenSyntax::SecTxType` (+ `as_str`) and `CoefficientCdfRowSelector::SecTxTypeIntra { tx_size_sqr }` (+ `syntax_name`).
- [x] 1.2 Add `sec_tx_type_intra_token(tx_size_sqr, symbol)` to the `transform_type` submodule and re-export it.
- [x] 1.3 Wire the intra `TileSecTxTypeCdf[0]` rows (one per `Tx_Size_Sqr`) into the generic CDF-row router.
- [x] 1.4 Add the `SecTxType` arm to the closed-loop single-DC recovery helper (no-op); correct the `IntraTxType` enum-variant doc to §5.20.8.2.

## 2. Tests

- [x] 2.1 Prove the `sec_tx_type` "IST off" token (symbol 0, `Tx_Size_Sqr 0`) roundtrips through the §8.2 router with the right syntax/selector.
- [x] 2.2 Prove the token roundtrips for every intra `Tx_Size_Sqr` row and every `sec_tx_type` value (`STX_TYPES = 4`).

## 3. Tracking and verification

- [x] 3.1 Add `ENC-SEC-TX-TYPE-TOKEN` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming a trace, `most_probable_stx_set`, eob > 2, the IST condition's runtime evaluation, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
