## 1. intra_tx_type token

- [x] 1.1 Add `CoefficientTokenSyntax::IntraTxType` + `CoefficientCdfRowSelector::IntraTxTypeSet1 { tx_size_sqr }` and a `transform_type.rs` submodule with `intra_tx_type_set1_token(tx_size_sqr, symbol)`.
- [x] 1.2 Wire the 4x4 `TX_SET_INTRA_1` row into the generic CDF-row router; add the `IntraTxType` arm to the closed-loop single-DC recovery helper (no-op).
- [x] 1.3 Extract `CoefficientTokenCdfRows` into a `cdf_rows.rs` submodule to keep the parent under the 1000-line budget.

## 2. Tests

- [x] 2.1 Guard the derivation: `Md_Idx_To_Type[Size_Class[4x4]=0][DC_PRED=0][0] == DCT_DCT (0)`.
- [x] 2.2 Prove the DCT_DCT token (symbol 0, `Tx_Size_Sqr 0`) roundtrips through the generic §8.2 router.

## 3. Tracking and verification

- [x] 3.1 Add `ENC-INTRA-TX-TYPE-TOKEN` to the implementation matrix and refresh generated status/coverage docs.
- [x] 3.2 Update encoder roadmap/gap audit notes without claiming a general `eob > 1` trace, `sec_tx_type`, non-`TX_SET_INTRA_1` sets, tile-body, packet, CLI, or Baseline Encoder Profile v1 behavior.
- [x] 3.3 Run OpenSpec validation, focused encoder tests, feature-status checks, and `cargo xtask ci`.
