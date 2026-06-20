## ADDED Requirements

### Requirement: Encoder sec_tx_type IST transform-type token

The encoder SHALL provide a private `sec_tx_type` transform-type token, tracked by
`ENC-SEC-TX-TYPE-TOKEN`, modeling the AV2 §5.20.8.2 `transform_type()` IST secondary
transform symbol read right after `intra_tx_type` (line 16613). It SHALL expose a
token whose syntax is `sec_tx_type`, coded with the intra `TileSecTxTypeCdf[0]
[Tx_Size_Sqr]` CDF (the `is_inter = 0` bank, §8.3.2), and SHALL prove the token
writes through one in-tree AV2 §8.2 symbol encoder and decodes back for every intra
`Tx_Size_Sqr` row and every `sec_tx_type` value (`STX_TYPES = 4`). It SHALL NOT emit
a trace inserting the token, the `most_probable_stx_set` follow-up symbol, the inter
bank, blocks with eob > 2, tile payloads, coded packets, public CLI success, or modes
beyond the DC minimal tier.

#### Scenario: The sec_tx_type token carries the intra IST selector

- **WHEN** the `sec_tx_type` "IST off" token is constructed for `Tx_Size_Sqr 0`
- **THEN** its syntax SHALL be `sec_tx_type` and its selector SHALL be the intra
  `TileSecTxTypeCdf[0][0]` row
- **AND** symbol 0 SHALL represent `sec_tx_type = 0` (no `most_probable_stx_set`).

#### Scenario: The sec_tx_type token roundtrips for all rows and values

- **WHEN** the `sec_tx_type` token is roundtripped through one in-tree AV2 §8.2 coder
  for each intra `Tx_Size_Sqr` row and each of the four `sec_tx_type` values
- **THEN** every decoded symbol SHALL equal the encoded symbol.

#### Scenario: The token does not produce packets

- **WHEN** the `sec_tx_type` token is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim a trace, IST condition
  evaluation, or Baseline Encoder Profile v1 output from the token alone.
