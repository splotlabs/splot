## ADDED Requirements

### Requirement: Encoder eob=2 trace with TX_SET_INTRA_1 intra_tx_type

The encoder SHALL provide a private eob=2 multi-coefficient luma block trace that
includes the §5.20.8.2 `intra_tx_type` transform-type symbol, tracked by
`ENC-INTRA-BLOCK-TRACE-TWO-COEFF-TX-TYPE`, for the default-`reduced_tx_set`
`TX_SET_INTRA_1` configuration. It SHALL compose the eob=2 multi-coefficient trace
with the 4x4 `DC_PRED` `intra_tx_type` DCT_DCT symbol (0) inserted after `eob_pt_16`
(the position `transform_type()` is read, §5.20.7.27), and SHALL prove the
eleven-token trace `[0,0,0, 0, 1, 0, 0, 0, 0, 1, 1]` writes through one in-tree AV2
§8.2 symbol encoder and decodes back. It still assumes `enable_intra_ist == 0`. It
SHALL NOT emit the `sec_tx_type` secondary-transform symbol, blocks with eob > 2,
non-DCT_DCT transform types, non-`TX_SET_INTRA_1` sets, tile payloads, coded packets,
public CLI success, or modes beyond the DC minimal tier.

#### Scenario: The intra_tx_type symbol sits after eob_pt_16

- **WHEN** the eob=2 trace with `intra_tx_type` is composed
- **THEN** it SHALL be the eob=2 multi-coefficient trace plus exactly one
  `intra_tx_type` (DCT_DCT, `TX_SET_INTRA_1`, `Tx_Size_Sqr 0`) token immediately
  after the `eob_pt_16` token — symbols `[0,0,0,0,1,0,0,0,0,1,1]`.

#### Scenario: The eob=2 tx-type trace roundtrips

- **WHEN** the composed trace is roundtripped through one in-tree AV2 §8.2 coder
- **THEN** the decoded symbols SHALL be `[0,0,0,0,1,0,0,0,0,1,1]`
- **AND** the roundtrip SHALL be deterministic.

#### Scenario: The trace does not produce packets

- **WHEN** the eob=2 tx-type trace is available in `splot-encode`
- **THEN** `Context::receive_packet` SHALL continue to return no coded packet
- **AND** no documentation or matrix row SHALL claim `sec_tx_type`, eob > 2, or
  Baseline Encoder Profile v1 output from the trace alone.
