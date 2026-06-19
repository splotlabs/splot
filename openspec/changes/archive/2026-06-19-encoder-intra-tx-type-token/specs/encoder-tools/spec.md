## ADDED Requirements

### Requirement: Encoder intra_tx_type TX_SET_INTRA_1 token

The encoder SHALL provide a private `intra_tx_type` transform-type token tracked by
`ENC-INTRA-TX-TYPE-TOKEN` for the `TX_SET_INTRA_1` transform set. It SHALL add the
`IntraTxType` token syntax and an `IntraTxTypeSet1 { tx_size_sqr }` CDF-row selector
for `TileIntraTxTypeSet1Cdf[Tx_Size_Sqr[txSz]]`, and an
`intra_tx_type_set1_token(tx_size_sqr, symbol)` accessor whose symbol indexes the §9
`Md_Idx_To_Type[Size_Class[txSz]][intraDir]` row. The token SHALL roundtrip through
one in-tree AV2 §8.2 symbol encoder/decoder via the generic coefficient-token CDF-row
router. It SHALL NOT compose a general `eob > 1` trace, model `sec_tx_type`, derive
non-`TX_SET_INTRA_1` sets or non-`DC_PRED` directions, or produce a coded packet.

#### Scenario: intra_tx_type symbol 0 selects DCT_DCT for a 4x4 DC_PRED block

- **WHEN** the encoder resolves the `intra_tx_type` symbol for a 4x4 (`Tx_Size_Sqr 0`,
  `Size_Class 0`) `DC_PRED` (`intraDir 0`) block
- **THEN** symbol 0 SHALL select `DCT_DCT` (`Md_Idx_To_Type[0][0][0] == 0`).

#### Scenario: The intra_tx_type token roundtrips

- **WHEN** the DCT_DCT `intra_tx_type` token (symbol 0, `Tx_Size_Sqr 0`) is
  roundtripped through the generic router and one in-tree AV2 §8.2 coder
- **THEN** the decoded symbol SHALL be 0.

#### Scenario: The token is not yet composed into a trace

- **WHEN** the `intra_tx_type` token is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a general `eob > 1` trace,
  `sec_tx_type`, or Baseline Encoder Profile v1 output from the token alone.
