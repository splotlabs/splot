## ADDED Requirements

### Requirement: Encoder eob=2 trace with intra_tx_type and sec_tx_type

The encoder SHALL provide a private eob=2 multi-coefficient luma block trace that
includes BOTH §5.20.8.2 transform-type symbols — `intra_tx_type` and the `sec_tx_type`
IST secondary transform — tracked by `ENC-INTRA-BLOCK-TRACE-IST`, for the
`enable_intra_ist == 1` configuration. It SHALL compose the eob=2 tx-type trace with
the 4x4 intra `sec_tx_type` IST-off symbol (0) inserted right after `intra_tx_type`
(the position `sec_tx_type` is read, §5.20.8.2 line 16613), and SHALL prove the
twelve-token trace `[0,0,0, 0, 1, 0, 0, 0, 0, 0, 1, 1]` writes through one in-tree AV2
§8.2 symbol encoder and decodes back. This block satisfies the IST condition (`eob 2
!= 1 && !Lossless && TxType == DCT_DCT && YMode != PAETH && eob 2 <= eobLim =
IST_4X4_HEIGHT = 8`) and uses `sec_tx_type = 0`. It SHALL NOT emit the
`most_probable_stx_set` symbol, blocks with eob > 2, non-DCT_DCT transform types,
non-`TX_SET_INTRA_1` sets, tile payloads, coded packets, public CLI success, or modes
beyond the DC minimal tier.

#### Scenario: sec_tx_type sits right after intra_tx_type

- **WHEN** the eob=2 trace with `intra_tx_type` and `sec_tx_type` is composed
- **THEN** it SHALL be the tx-type trace plus exactly one `sec_tx_type` (IST,
  `Tx_Size_Sqr 0`) token immediately after the `intra_tx_type` token — symbols
  `[0,0,0,0,1,0,0,0,0,0,1,1]`.

#### Scenario: The IST trace roundtrips

- **WHEN** the composed trace is roundtripped through one in-tree AV2 §8.2 coder
- **THEN** the decoded symbols SHALL be `[0,0,0,0,1,0,0,0,0,0,1,1]`
- **AND** the roundtrip SHALL be deterministic.

#### Scenario: The trace does not produce packets

- **WHEN** the IST trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim `most_probable_stx_set`, eob > 2,
  or Baseline Encoder Profile v1 output from the trace alone.
